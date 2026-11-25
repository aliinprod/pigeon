//! Client side: list a directory or fetch one file from an agent.

use std::{
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, ConnectionError, RecvStream, SendStream},
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::{
    ALPN,
    agent::close,
    proto::{Entry, Request, Response, read_frame, write_frame},
};

/// How the bytes reached us, as seen by the client at the end of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// A direct UDP path between the two devices (hole punching worked).
    Direct,
    /// Forwarded by the relay (end-to-end encrypted; the relay cannot read it).
    Relay,
    /// No selected path was reported (should not happen on a live connection).
    Unknown,
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Route::Direct => "direct",
            Route::Relay => "relay",
            Route::Unknown => "unknown",
        })
    }
}

/// Endpoint builder for the fetching side (`pigeon`): n0 relays and n0
/// address *lookup*, but no address *publishing*. The fetching side only
/// dials out, so nobody needs to find it; and since it uses the same device
/// key as a `pigeon-agent share` that may be running on this computer, a
/// publisher here would replace the agent's published address record with
/// this short-lived process's addresses (PAIRING.md D9). Same as
/// `presets::N0` otherwise.
pub fn endpoint_builder(key: iroh::SecretKey) -> iroh::endpoint::Builder {
    use iroh::address_lookup::{DnsAddressLookup, PkarrResolver};
    Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(key)
        .address_lookup(PkarrResolver::n0_dns())
        .address_lookup(DnsAddressLookup::n0_dns())
        .relay_mode(iroh::endpoint::default_relay_mode())
}

/// The path iroh currently uses to send application data on `conn`.
pub fn route(conn: &Connection) -> Route {
    let paths = conn.paths();
    match paths.iter().find(|p| p.is_selected()) {
        Some(p) if p.is_ip() => Route::Direct,
        Some(p) if p.is_relay() => Route::Relay,
        _ => Route::Unknown,
    }
}

/// Result of a successful [`get`].
#[derive(Debug, Clone)]
pub struct GetStats {
    /// Full file size.
    pub size: u64,
    /// Bytes resumed from an existing `.part` file.
    pub resumed_from: u64,
    /// Bytes received over the network in this run.
    pub received: u64,
    pub elapsed: Duration,
    /// Hex SHA-256 of the final file. Always verified against the agent's
    /// hash: a download without one fails (W-19).
    pub sha256: String,
    /// Path in use when the transfer finished.
    pub route: Route,
    /// Where the file landed: the `out` asked for, or `name (n).ext` if a
    /// file of that name appeared during the download (W-18).
    pub path: PathBuf,
    /// True if the OS "downloaded from elsewhere" mark was written (Mark of
    /// the Web on Windows, quarantine on macOS); false on Linux, or where the
    /// file system cannot hold it. See [`mark_downloaded`].
    pub marked: bool,
}

impl GetStats {
    /// Network throughput of this run in MB/s (1 MB = 1_000_000 bytes).
    pub fn mb_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64().max(1e-9);
        self.received as f64 / 1_000_000.0 / secs
    }
}

/// Opens a connection and sends the request. Returns the open streams.
/// The agent identifies us by our endpoint key (iroh TLS); nothing secret
/// goes on the wire.
async fn request(
    endpoint: &Endpoint,
    agent: &EndpointAddr,
    req: &Request,
) -> Result<(Connection, SendStream, RecvStream)> {
    let conn = endpoint
        .connect(agent.clone(), ALPN)
        .await
        .context("could not connect to the agent (is `pigeon-agent share` running?)")?;
    let streams = async {
        let (mut send, recv) = conn.open_bi().await?;
        write_frame(&mut send, req).await?;
        send.finish()?;
        anyhow::Ok((send, recv))
    }
    .await;
    match streams {
        Ok((send, recv)) => Ok((conn, send, recv)),
        Err(e) => Err(explain(&conn, e)),
    }
}

/// Turns "connection closed" into the agent's reason, when it gave one.
fn explain(conn: &Connection, err: anyhow::Error) -> anyhow::Error {
    match conn.close_reason() {
        Some(ConnectionError::ApplicationClosed(c)) if c.error_code == close::NOT_PAIRED.into() => {
            anyhow::anyhow!(
                "the agent refused this device: it is not paired (or was revoked). \
                 Run `pigeon-agent pair` on the agent and `pigeon pair <LINK>` here"
            )
        }
        Some(ConnectionError::ApplicationClosed(c)) if c.error_code == close::BUSY.into() => {
            anyhow::anyhow!("the agent is busy (too many connections); try again in a moment")
        }
        _ => err,
    }
}

/// Lists a directory on the agent ("" = share root).
pub async fn list(endpoint: &Endpoint, agent: &EndpointAddr, path: &str) -> Result<Vec<Entry>> {
    let req = Request::List { path: path.into() };
    let (conn, _send, mut recv) = request(endpoint, agent, &req).await?;
    let response: Result<Response> = read_frame(&mut recv).await;
    let response = response.map_err(|e| explain(&conn, e));
    conn.close(0u32.into(), b"done");
    match response? {
        Response::List(entries) => Ok(entries),
        Response::Error(e) => bail!("agent: {}", crate::display_safe(&e)),
        other => bail!("unexpected response: {other:?}"),
    }
}

pub use crate::download::{
    choose_out, free_path, is_risky_to_open, local_file_name, mark_downloaded, part_in_use,
    part_path,
};

/// How [`get_with_mode`] treats `<out>.part`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Continue an existing `<out>.part` (a run that was cut off), or start
    /// one if there is none. What [`get`] and [`get_with_progress`] do.
    #[default]
    Resume,
    /// Start a new `<out>.part`; fail if one exists. For a new download at a
    /// name from [`free_path`].
    Fresh,
}

/// Downloads `path` from the agent into `out` (resuming `<out>.part`).
///
/// Data goes to `<out>.part` first, which this download locks so no other
/// download can write it at the same time. An existing `.part` is resumed
/// from its current size (or started over, if the agent says it is longer
/// than the file). The agent must send the file's SHA-256 (W-19); the whole
/// file is checked against it, and only then is the `.part` moved to `out`,
/// or to the next free `name (n).ext` if `out` appeared meanwhile; an
/// existing file is never replaced (W-18). [`GetStats::path`] says where it
/// landed. Finally the file gets the OS "downloaded" mark
/// ([`mark_downloaded`]).
pub async fn get(
    endpoint: &Endpoint,
    agent: &EndpointAddr,
    path: &str,
    out: &Path,
) -> Result<GetStats> {
    get_with_progress(endpoint, agent, path, out, &|_| {}).await
}

/// How far a download is, reported by [`get_with_progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Whole file size, as announced by the agent.
    pub size: u64,
    /// Bytes of the file that are on disk now (resumed bytes included).
    pub done: u64,
    /// Bytes resumed from an existing `.part` file.
    pub resumed_from: u64,
}

/// Like [`get`], and calls `progress` once the agent's header arrived and
/// after every chunk written to disk. `progress` runs on the download task:
/// keep it short (update a counter, not the UI).
pub async fn get_with_progress(
    endpoint: &Endpoint,
    agent: &EndpointAddr,
    path: &str,
    out: &Path,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<GetStats> {
    get_with_mode(endpoint, agent, path, out, Mode::Resume, progress).await
}

/// Like [`get_with_progress`], with an explicit [`Mode`] for the `.part`.
pub async fn get_with_mode(
    endpoint: &Endpoint,
    agent: &EndpointAddr,
    path: &str,
    out: &Path,
    mode: Mode,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<GetStats> {
    let part = part_path(out);
    // Locked from here until it is closed below.
    let file = crate::download::open_part(&part, mode == Mode::Fresh)?;
    let result = transfer(endpoint, agent, path, file, &part, progress).await;
    let (size, offset, received, elapsed, actual, route) = match result {
        Ok(r) => r,
        Err(e) => {
            // Leave no empty `.part` behind (e.g. the agent refused us).
            if std::fs::metadata(&part).is_ok_and(|m| m.len() == 0) {
                let _ = std::fs::remove_file(&part);
            }
            return Err(e);
        }
    };
    let landed = crate::download::finish_part(&part, out)?;
    // A file system without streams / xattrs (FAT, some shares) cannot hold
    // the mark; the download itself is fine, and `marked` says so.
    let marked = mark_downloaded(&landed).unwrap_or(false);
    Ok(GetStats {
        size,
        resumed_from: offset,
        received,
        elapsed,
        sha256: crate::hex(&actual),
        route,
        path: landed,
        marked,
    })
}

type Transferred = (u64, u64, u64, Duration, [u8; 32], Route);

/// Asks for the file, writes it into the locked `.part` and checks its hash.
/// The `.part` is closed (and so unlocked) when this returns.
async fn transfer(
    endpoint: &Endpoint,
    agent: &EndpointAddr,
    path: &str,
    file: std::fs::File,
    part: &Path,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Transferred> {
    let offset = file
        .metadata()
        .with_context(|| format!("could not read {}", part.display()))?
        .len();
    let mut file = tokio::fs::File::from_std(file);

    let start = Instant::now();
    let req = Request::Get {
        path: path.into(),
        offset,
    };
    let (conn, _send, mut recv) = request(endpoint, agent, &req).await?;
    let result = receive(&mut recv, &mut file, part, offset, progress)
        .await
        .map_err(|e| explain(&conn, e));
    let route = route(&conn);
    conn.close(0u32.into(), b"done");
    // Close the `.part` before it is moved or removed (Windows needs that).
    drop(file.into_std().await);
    let (size, offset, actual, expected) = result?;
    let elapsed = start.elapsed();
    if actual != expected {
        // A corrupt .part would never heal on resume, so drop it.
        let _ = tokio::fs::remove_file(part).await;
        bail!(
            "SHA-256 mismatch: expected {}, got {}",
            crate::hex(&expected),
            crate::hex(&actual)
        );
    }
    Ok((size, offset, size - offset, elapsed, actual, route))
}

/// Reads the header and the file bytes into `file` (the `.part`, positioned
/// at its start). Returns (size, effective offset, actual sha256, the
/// agent's sha256). The hash covers the resumed bytes already on disk plus
/// everything written now.
async fn receive(
    recv: &mut RecvStream,
    file: &mut tokio::fs::File,
    part: &Path,
    offset: u64,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<(u64, u64, [u8; 32], [u8; 32])> {
    let (size, offset, expected) = match read_frame(recv).await? {
        Response::File {
            size,
            offset: o,
            sha256,
        } => {
            // W-19: every pigeon-agent since Day 1 sends the hash, so a
            // header without one is refused before any byte is written:
            // "verified" must always be true.
            let Some(sha256) = sha256 else {
                bail!(
                    "the agent sent no SHA-256 for this file, so it cannot be checked; \
                     update pigeon on the other computer"
                );
            };
            if o == 0 && offset > 0 {
                // Our .part is longer than the file: it is stale. Start over.
                file.set_len(0)
                    .await
                    .with_context(|| format!("could not empty stale {}", part.display()))?;
            } else if o != offset {
                bail!("agent answered with offset {o}, expected {offset}");
            }
            if o > size {
                bail!("agent answered with offset {o} beyond size {size}");
            }
            (size, o, sha256)
        }
        Response::Error(e) => bail!("agent: {}", crate::display_safe(&e)),
        other => bail!("unexpected response: {other:?}"),
    };

    let mut buf = vec![0u8; 256 * 1024];
    let mut hasher = Sha256::new();
    // Hash the bytes we resume from, through the same (locked) handle.
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let mut left = offset;
    while left > 0 {
        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            bail!("{} got shorter while resuming", part.display());
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }

    let expected_len = size - offset;
    let mut got = 0u64;
    let report = |got: u64| {
        progress(Progress {
            size,
            done: offset + got,
            resumed_from: offset,
        })
    };
    report(0);
    // `RecvStream::read` returns `None` once the agent finished the stream.
    while let Some(n) = recv.read(&mut buf).await? {
        got += n as u64;
        if got > expected_len {
            bail!("agent sent more data than announced");
        }
        file.write_all(&buf[..n]).await?;
        hasher.update(&buf[..n]);
        report(got);
    }
    file.flush().await?;
    file.sync_all().await?;
    if got != expected_len {
        bail!("transfer interrupted: got {got} of {expected_len} bytes (run again to resume)");
    }
    let mut actual = [0u8; 32];
    actual.copy_from_slice(&hasher.finalize());
    Ok((size, offset, actual, expected))
}
