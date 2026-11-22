//! Agent side: accepts iroh connections and answers List/Get requests for one
//! shared root folder.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint,
    endpoint::{Connection, SendStream},
};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

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
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What an agent shares and who may access it.
#[derive(Debug, Clone)]
pub struct Share {
    pub root: PathBuf,
    pub token: [u8; 32],
}

/// Serves incoming connections on `endpoint` until the endpoint is closed.
pub async fn serve(endpoint: Endpoint, share: Share) -> Result<()> {
    let share = Arc::new(share);
    while let Some(incoming) = endpoint.accept().await {
        let share = share.clone();
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("incoming connection failed: {e}");
                    return;
                }
            };
            let peer = conn.remote_id();
            if let Err(e) = handle_connection(conn, &share).await {
                eprintln!("connection from {} failed: {e:#}", peer.fmt_short());
            }
        });
    }
    Ok(())
}

/// Handles one connection: hello (auth), one request, one response.
pub async fn handle_connection(conn: Connection, share: &Share) -> Result<()> {
    let (mut send, mut recv, hello) = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
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
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), conn.closed()).await;
        conn.close(1u32.into(), b"unauthorized");
        bail!("rejected peer {}: bad token", conn.remote_id().fmt_short());
    }

    let request: Request = read_frame(&mut recv).await?;
    match request {
        Request::List { path } => {
            let response = match list(&share.root, &path).await {
                Ok(entries) => Response::List(entries),
                Err(e) => Response::Error(format!("{e:#}")),
            };
            write_frame(&mut send, &response).await?;
        }
        Request::Get { path, offset } => match open_for_get(&share.root, &path, offset).await {
            Ok((file, size, offset, sha256)) => {
                let header = Response::File {
                    size,
                    offset,
                    sha256: Some(sha256),
                };
                write_frame(&mut send, &header).await?;
                send_file(file, &mut send).await?;
            }
            Err(e) => write_frame(&mut send, &Response::Error(format!("{e:#}"))).await?,
        },
    }
    send.finish()?;
    // Wait for the client to close once it has read everything, so we do not
    // drop buffered data by closing first.
    conn.closed().await;
    Ok(())
}

async fn list(root: &Path, path: &str) -> Result<Vec<Entry>> {
    let dir = guard::resolve(root, path)?;
    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .with_context(|| format!("not a directory: {path}"))?;
    let mut entries = Vec::new();
    while let Some(e) = rd.next_entry().await? {
        // Follow symlinks for metadata; Get still goes through the guard.
        let Ok(meta) = tokio::fs::metadata(e.path()).await else {
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

async fn send_file(mut file: tokio::fs::File, send: &mut SendStream) -> Result<()> {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        send.write_all(&buf[..n]).await?;
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
