//! Wire protocol: length-prefixed postcard frames on a single QUIC bi-stream.
//!
//! One request per connection, ALPN `pigeon/1` (the ALPN is the version):
//! 1. the agent checks the peer's endpoint ID (verified by iroh's TLS
//!    handshake) against its allow-list before accepting any stream;
//! 2. client -> agent: [`Request`] (no secret on the wire);
//! 3. agent -> client: [`Response`];
//! 4. for [`Response::File`], the agent then streams the raw file bytes from
//!    the requested offset and finishes the stream.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest frame we accept (file bytes are not framed, so this is plenty).
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Largest [`Request`] the agent reads (a path plus an offset).
pub const MAX_REQUEST_LEN: u32 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// List a directory (path relative to the share root, "" = root).
    List { path: String },
    /// Fetch a file starting at `offset` (for resume).
    Get { path: String, offset: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch (0 if unknown).
    pub mtime: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    List(Vec<Entry>),
    /// Header for a file download. `size` is the whole file size; the bytes that
    /// follow are `size - offset` long. `sha256` covers the whole file.
    /// `offset` is the requested offset, or 0 when the requested offset was
    /// past the end of the file (the client must then restart from scratch).
    File {
        size: u64,
        offset: u64,
        sha256: Option<[u8; 32]>,
    },
    Error(String),
}

/// Writes one length-prefixed postcard frame.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let buf = postcard::to_stdvec(msg).context("encode frame")?;
    let len = u32::try_from(buf.len()).context("frame too large")?;
    if len > MAX_FRAME_LEN {
        bail!("frame too large: {len} bytes");
    }
    w.write_u32(len).await?;
    w.write_all(&buf).await?;
    Ok(())
}

/// Reads one length-prefixed postcard frame.
pub async fn read_frame<R, T>(r: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    read_frame_max(r, MAX_FRAME_LEN).await
}

/// Reads one length-prefixed postcard frame of at most `max` bytes. The length
/// is checked before any buffer is allocated.
pub async fn read_frame_max<R, T>(r: &mut R, max: u32) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let len = r.read_u32().await.context("read frame length")?;
    if len > max {
        bail!("frame too large: {len} bytes");
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await.context("read frame body")?;
    postcard::from_bytes(&buf).context("decode frame")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_roundtrip() {
        let req = Request::Get {
            path: "a/b.txt".into(),
            offset: 42,
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).await.unwrap();
        let back: Request = read_frame(&mut buf.as_slice()).await.unwrap();
        assert_eq!(req, back);
    }

    #[tokio::test]
    async fn requests_fit_and_oversized_frames_are_refused_early() {
        let req = Request::Get {
            path: "x".repeat(4096),
            offset: u64::MAX,
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).await.unwrap();
        let back: Request = read_frame_max(&mut buf.as_slice(), MAX_REQUEST_LEN)
            .await
            .unwrap();
        assert_eq!(back, req);

        // A length prefix alone (no body) claiming a big frame must be refused
        // from the prefix, not after trying to read/allocate the body.
        for len in [MAX_REQUEST_LEN + 1, MAX_FRAME_LEN, u32::MAX] {
            let prefix = len.to_be_bytes();
            let err = read_frame_max::<_, Request>(&mut prefix.as_slice(), MAX_REQUEST_LEN)
                .await
                .unwrap_err();
            assert!(format!("{err:#}").contains("too large"), "{len}: {err:#}");
        }
        let prefix = (crate::pair::MAX_PAIR_FRAME + 1).to_be_bytes();
        let err = read_frame_max::<_, crate::pair::PairStart>(
            &mut prefix.as_slice(),
            crate::pair::MAX_PAIR_FRAME,
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("too large"), "{err:#}");
        let prefix = (MAX_FRAME_LEN + 1).to_be_bytes();
        let err = read_frame::<_, Request>(&mut prefix.as_slice())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("too large"), "{err:#}");
    }
}
