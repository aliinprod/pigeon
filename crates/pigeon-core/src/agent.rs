//! Agent side: accepts iroh connections and answers List/Get requests for one
//! shared root folder.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointId,
    endpoint::{Connection, SendStream},
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::Semaphore,
};

use crate::{
    guard,
    proto::{
        Entry, Hello, MAX_HELLO_LEN, PROTOCOL_VERSION, Request, Response, read_frame,
        read_frame_max, write_frame,
    },
};

/// Size of the read buffer used when streaming files.
const CHUNK: usize = 256 * 1024;

/// How long an unauthenticated peer gets to open a stream and send its hello.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Resource limits for one agent (SEC-18, TH-08).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Connections served at the same time, from all peers together. Extra
    /// connection attempts are refused before the QUIC handshake.
    pub max_connections: usize,
    /// Connections served at the same time for one peer (endpoint ID).
    pub max_per_peer: usize,
    /// Time a peer gets to open its stream and send its first frames.
    pub handshake_timeout: Duration,
    /// Longest time without progress on an authenticated connection: a write
    /// the peer does not read, or waiting for the peer to close at the end.
    pub idle_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 32,
            max_per_peer: 4,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            idle_timeout: Duration::from_secs(30),
        }
    }
}

/// How the agent serves connections.
#[derive(Debug, Clone, Copy, Default)]
pub struct ServeOptions {
    pub limits: Limits,
    /// Print details of failed connection attempts. Off by default: any host
    /// that can reach us may send stray packets, and those are not an alarm.
    pub verbose: bool,
}

/// What an agent shares and who may access it.
#[derive(Clone)]
pub struct Share {
    pub root: PathBuf,
    pub token: [u8; 32],
}

impl std::fmt::Debug for Share {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Share")
            .field("root", &self.root)
            .field("token", &crate::REDACTED)
            .finish()
    }
}

/// Counts live connections per peer. A [`PeerSlot`] gives its place back on drop.
#[derive(Default)]
struct PeerCounts(Mutex<HashMap<EndpointId, usize>>);

struct PeerSlot {
    counts: Arc<PeerCounts>,
    peer: EndpointId,
}

impl PeerCounts {
    fn try_take(self: &Arc<Self>, peer: EndpointId, max: usize) -> Option<PeerSlot> {
        let mut map = self.0.lock().expect("peer counts poisoned");
        let n = map.entry(peer).or_insert(0);
        if *n >= max {
            return None;
        }
        *n += 1;
        Some(PeerSlot {
            counts: self.clone(),
            peer,
        })
    }
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut map = self.counts.0.lock().expect("peer counts poisoned");
        if let Some(n) = map.get_mut(&self.peer) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.peer);
            }
        }
    }
}

/// Serves incoming connections on `endpoint` until the endpoint is closed.
pub async fn serve(endpoint: Endpoint, share: Share, opts: ServeOptions) -> Result<()> {
    let share = Arc::new(share);
    let slots = Arc::new(Semaphore::new(opts.limits.max_connections));
    let peers = Arc::new(PeerCounts::default());
    while let Some(incoming) = endpoint.accept().await {
        // Global cap: refuse before spending any work on the handshake.
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            if opts.verbose {
                eprintln!(
                    "connection refused: all {} slots in use",
                    opts.limits.max_connections
                );
            }
            incoming.refuse();
            continue;
        };
        let share = share.clone();
        let peers = peers.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(e) => {
                    // Usually a scanner, a stray packet or a client that gave
                    // up. Nothing was exposed, so do not alarm the owner.
                    if opts.verbose {
                        eprintln!(
                            "a connection attempt did not complete the handshake (ignored): {e}"
                        );
                    } else {
                        eprintln!("a connection attempt did not complete the handshake (ignored)");
                    }
                    return;
                }
            };
            let peer = conn.remote_id();
            let Some(_peer_slot) = peers.try_take(peer, opts.limits.max_per_peer) else {
                eprintln!("too many connections from {}; closed one", peer.fmt_short());
                conn.close(2u32.into(), b"busy");
                return;
            };
            if let Err(e) = handle_connection(conn, &share, &opts.limits).await {
                eprintln!("connection from {} failed: {e:#}", peer.fmt_short());
            }
        });
    }
    Ok(())
}

/// Handles one connection: hello (auth), one request, one response. The
/// connection is always closed when this returns.
pub async fn handle_connection(conn: Connection, share: &Share, limits: &Limits) -> Result<()> {
    let result = serve_one(&conn, share, limits).await;
    conn.close(0u32.into(), b"done");
    result
}

async fn serve_one(conn: &Connection, share: &Share, limits: &Limits) -> Result<()> {
    let (mut send, mut recv, hello) = tokio::time::timeout(limits.handshake_timeout, async {
        let (send, mut recv) = conn.accept_bi().await?;
        let hello: Hello = read_frame_max(&mut recv, MAX_HELLO_LEN).await?;
        anyhow::Ok((send, recv, hello))
    })
    .await
    .context("handshake timed out")??;
    let token_ok: bool = hello.token[..].ct_eq(&share.token[..]).into();
    if !token_ok || hello.version != PROTOCOL_VERSION {
        write_frame(&mut send, &Response::Error("unauthorized".into())).await?;
        send.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
        conn.close(1u32.into(), b"unauthorized");
        bail!("rejected peer {}: bad token", conn.remote_id().fmt_short());
    }

    let request: Request = tokio::time::timeout(limits.handshake_timeout, read_frame(&mut recv))
        .await
        .context("request timed out")??;
    let idle = limits.idle_timeout;
    match request {
        Request::List { path } => {
            let response = match list(&share.root, &path).await {
                Ok(entries) => Response::List(entries),
                Err(e) => Response::Error(format!("{e:#}")),
            };
            with_idle(idle, write_frame(&mut send, &response)).await?;
        }
        Request::Get { path, offset } => match open_for_get(&share.root, &path, offset).await {
            Ok((file, size, offset, sha256)) => {
                let header = Response::File {
                    size,
                    offset,
                    sha256: Some(sha256),
                };
                with_idle(idle, write_frame(&mut send, &header)).await?;
                send_file(file, &mut send, idle).await?;
            }
            Err(e) => {
                let response = Response::Error(format!("{e:#}"));
                with_idle(idle, write_frame(&mut send, &response)).await?
            }
        },
    }
    send.finish()?;
    // Wait for the client to close once it has read everything, so we do not
    // drop buffered data by closing first. A client that never closes is cut
    // off after the idle timeout (TH-08).
    let _ = tokio::time::timeout(idle, conn.closed()).await;
    Ok(())
}

/// Runs one network step and fails if it makes no progress within `idle`.
async fn with_idle<T>(idle: Duration, fut: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(idle, fut)
        .await
        .context("peer stopped reading (idle timeout)")?
}

async fn list(root: &Path, path: &str) -> Result<Vec<Entry>> {
    let dir = guard::resolve(root, path)?;
    let canon_root = tokio::fs::canonicalize(root)
        .await
        .context("the shared folder is not available")?;
    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .with_context(|| format!("not a directory: {path}"))?;
    let mut entries = Vec::new();
    while let Some(e) = rd.next_entry().await? {
        // Follow symlinks, but only to targets inside the share: an entry
        // whose real path is outside must not leak its size, mtime or type,
        // and is hidden like a missing file (SEC-04). Get still goes through
        // the guard.
        let Ok(real) = tokio::fs::canonicalize(e.path()).await else {
            continue;
        };
        if !real.starts_with(&canon_root) {
            continue;
        }
        let Ok(meta) = tokio::fs::metadata(&real).await else {
            continue;
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push(Entry {
            name: e.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            mtime,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Opens `path` for a download from `offset`. Returns (file positioned at the
/// effective offset, size, effective offset, sha256). An offset past the end
/// (e.g. a stale `.part` from a bigger, older file) restarts at 0.
async fn open_for_get(
    root: &Path,
    path: &str,
    offset: u64,
) -> Result<(tokio::fs::File, u64, u64, [u8; 32])> {
    let full = guard::resolve(root, path)?;
    let meta = tokio::fs::metadata(&full).await?;
    if !meta.is_file() {
        bail!("not a file: {path}");
    }
    let size = meta.len();
    let offset = if offset > size { 0 } else { offset };
    let sha256 = sha256_file(full.clone()).await?;
    let mut file = tokio::fs::File::open(&full).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    Ok((file, size, offset, sha256))
}

async fn send_file(mut file: tokio::fs::File, send: &mut SendStream, idle: Duration) -> Result<()> {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        with_idle(idle, async { Ok(send.write_all(&buf[..n]).await?) }).await?;
    }
}

/// SHA-256 of a whole file, computed on a blocking thread.
pub async fn sha256_file(path: PathBuf) -> Result<[u8; 32]> {
    tokio::task::spawn_blocking(move || -> Result<[u8; 32]> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&hasher.finalize());
        Ok(out)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Makes `link` point at the directory `target`: a symlink on Unix, a
    /// junction on Windows (no admin rights needed there).
    fn dir_link(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let out = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(out.status.success(), "mklink /J failed: {out:?}");
        }
    }

    #[tokio::test]
    async fn list_hides_links_that_point_outside_the_share() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("share");
        std::fs::create_dir_all(root.join("inner")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"top secret").unwrap();
        dir_link(&outside, &root.join("out"));
        dir_link(&root.join("inner"), &root.join("in"));
        let names: Vec<String> = list(&root, "")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["in", "inner"], "escaping link must not be listed");
    }

    #[tokio::test]
    async fn errors_sent_to_the_peer_do_not_contain_the_root_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("gone-share");
        let err = format!("{:#}", list(&root, "").await.unwrap_err());
        assert!(!err.contains("gone-share"), "{err}");
        let err = format!("{:#}", open_for_get(&root, "x", 0).await.unwrap_err());
        assert!(!err.contains("gone-share"), "{err}");
    }
}
