//! Client side: list a directory or fetch one file from an agent.

use std::{
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint,
    endpoint::{Connection, RecvStream, SendStream},
};
use tokio::io::AsyncWriteExt;

use crate::{
    ALPN, Ticket,
    agent::sha256_file,
    proto::{Entry, Hello, PROTOCOL_VERSION, Request, Response, read_frame, write_frame},
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
    /// Hex SHA-256 of the final file (verified against the agent's hash).
    pub sha256: String,
    /// Path in use when the transfer finished.
    pub route: Route,
}

impl GetStats {
    /// Network throughput of this run in MB/s (1 MB = 1_000_000 bytes).
    pub fn mb_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64().max(1e-9);
        self.received as f64 / 1_000_000.0 / secs
    }
}

/// Opens a connection and sends the hello + request. Returns the open streams.
async fn request(
    endpoint: &Endpoint,
    ticket: &Ticket,
    req: &Request,
) -> Result<(Connection, SendStream, RecvStream)> {
    let conn = endpoint
        .connect(ticket.addr.clone(), ALPN)
        .await
        .context("could not connect to the agent")?;
    let (mut send, recv) = conn.open_bi().await?;
    let hello = Hello {
        version: PROTOCOL_VERSION,
        token: ticket.token,
    };
    write_frame(&mut send, &hello).await?;
    write_frame(&mut send, req).await?;
    send.finish()?;
    Ok((conn, send, recv))
}

/// Lists a directory on the agent ("" = share root).
pub async fn list(endpoint: &Endpoint, ticket: &Ticket, path: &str) -> Result<Vec<Entry>> {
    let req = Request::List { path: path.into() };
    let (conn, _send, mut recv) = request(endpoint, ticket, &req).await?;
    let response: Result<Response> = read_frame(&mut recv).await;
    conn.close(0u32.into(), b"done");
    match response? {
        Response::List(entries) => Ok(entries),
        Response::Error(e) => bail!("agent: {}", crate::display_safe(&e)),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// Path of the partial download for `out`.
pub fn part_path(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Returns `out` if nothing exists there yet, otherwise the first free
/// `name (1).ext`, `name (2).ext`, ... next to it. Never overwrites.
pub fn free_path(out: &Path) -> PathBuf {
    if !exists(out) {
        return out.to_path_buf();
    }
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = out
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1u32..)
        .map(|n| out.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| !exists(p))
        .expect("ran out of u32 file names")
}

/// Like `Path::exists`, but a dangling symlink also counts as taken.
fn exists(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}

/// Downloads `path` from the agent into `out`.
///
/// Data goes to `<out>.part` first; an existing `.part` file is resumed from
/// its current size (or dropped, if the agent says it is longer than the
/// file). After the transfer the SHA-256 of the whole file is checked against
/// the agent's hash and the file is renamed to `out`.
pub async fn get(endpoint: &Endpoint, ticket: &Ticket, path: &str, out: &Path) -> Result<GetStats> {
    let part = part_path(out);
    let offset = match tokio::fs::metadata(&part).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    let start = Instant::now();
    let req = Request::Get {
        path: path.into(),
        offset,
    };
    let (conn, _send, mut recv) = request(endpoint, ticket, &req).await?;
    let result = receive(&mut recv, &part, offset).await;
    let route = route(&conn);
    conn.close(0u32.into(), b"done");
    let (size, offset, expected) = result?;
    let elapsed = start.elapsed();

    let actual = sha256_file(part.clone()).await?;
    if let Some(expected) = expected
        && actual != expected
    {
        // A corrupt .part would never heal on resume, so drop it.
        let _ = tokio::fs::remove_file(&part).await;
        bail!(
            "SHA-256 mismatch: expected {}, got {}",
            crate::hex(&expected),
            crate::hex(&actual)
        );
    }
    tokio::fs::rename(&part, out)
        .await
        .with_context(|| format!("could not move {} to {}", part.display(), out.display()))?;

    Ok(GetStats {
        size,
        resumed_from: offset,
        received: size - offset,
        elapsed,
        sha256: crate::hex(&actual),
        route,
    })
}

/// Reads the header and the file bytes into `part`.
/// Returns (size, effective offset, sha256).
async fn receive(
    recv: &mut RecvStream,
    part: &Path,
    offset: u64,
) -> Result<(u64, u64, Option<[u8; 32]>)> {
    let (size, offset, sha256) = match read_frame(recv).await? {
        Response::File {
            size,
            offset: o,
            sha256,
        } => {
            if o == 0 && offset > 0 {
                // Our .part is longer than the file: it is stale. Start over.
                tokio::fs::remove_file(part)
                    .await
                    .with_context(|| format!("could not remove stale {}", part.display()))?;
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

    let mut opts = tokio::fs::OpenOptions::new();
    opts.create(true).append(true);
    // Fetched work files are readable by the home user only (SEC-14). On
    // Windows the file inherits the ACL of the folder it is saved in.
    #[cfg(unix)]
    opts.mode(0o600);
    let mut file = opts
        .open(part)
        .await
        .with_context(|| format!("could not open {}", part.display()))?;
    let expected = size - offset;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    // `RecvStream::read` returns `None` once the agent finished the stream.
    while let Some(n) = recv.read(&mut buf).await? {
        got += n as u64;
        if got > expected {
            bail!("agent sent more data than announced");
        }
        file.write_all(&buf[..n]).await?;
    }
    file.flush().await?;
    file.sync_all().await?;
    if got != expected {
        bail!("transfer interrupted: got {got} of {expected} bytes (run again to resume)");
    }
    Ok((size, offset, sha256))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_path_keeps_a_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("report.pdf");
        assert_eq!(free_path(&out), out);
    }

    #[test]
    fn free_path_numbers_taken_names() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("report.pdf");
        std::fs::write(&out, b"1").unwrap();
        assert_eq!(free_path(&out), dir.path().join("report (1).pdf"));
        std::fs::write(dir.path().join("report (1).pdf"), b"2").unwrap();
        assert_eq!(free_path(&out), dir.path().join("report (2).pdf"));
    }

    #[test]
    fn free_path_without_extension_and_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("Makefile");
        std::fs::write(&plain, b"").unwrap();
        assert_eq!(free_path(&plain), dir.path().join("Makefile (1)"));
        let dot = dir.path().join(".env");
        std::fs::write(&dot, b"").unwrap();
        assert_eq!(free_path(&dot), dir.path().join(".env (1)"));
    }

    #[test]
    fn free_path_treats_a_directory_as_taken() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("photos");
        std::fs::create_dir(&out).unwrap();
        assert_eq!(free_path(&out), dir.path().join("photos (1)"));
    }
}
