//! Client side: list a directory or fetch one file from an agent.

use std::{
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
        Response::Error(e) => bail!("agent: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// Path of the partial download for `out`.
pub fn part_path(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Downloads `path` from the agent into `out`.
///
/// Data goes to `<out>.part` first; an existing `.part` file is resumed from
/// its current size. After the transfer the SHA-256 of the whole file is
/// checked against the agent's hash and the file is renamed to `out`.
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
    conn.close(0u32.into(), b"done");
    let (size, expected) = result?;
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
    })
}

/// Reads the header and the file bytes into `part`. Returns (size, sha256).
async fn receive(
    recv: &mut RecvStream,
    part: &Path,
    offset: u64,
) -> Result<(u64, Option<[u8; 32]>)> {
    let (size, sha256) = match read_frame(recv).await? {
        Response::File {
            size,
            offset: o,
            sha256,
        } => {
            if o != offset {
                bail!("agent answered with offset {o}, expected {offset}");
            }
            (size, sha256)
        }
        Response::Error(e) => bail!("agent: {e}"),
        other => bail!("unexpected response: {other:?}"),
    };

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
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
    Ok((size, sha256))
}
