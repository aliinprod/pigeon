//! Wire protocol: length-prefixed postcard frames on a single QUIC bi-stream.
//!
//! One request per connection (v1):
//! 1. client -> agent: [`Hello`] (carries the access token)
//! 2. client -> agent: [`Request`]
//! 3. agent -> client: [`Response`]
//! 4. for [`Response::File`], the agent then streams the raw file bytes from
//!    the requested offset and finishes the stream.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Protocol version sent in [`Hello`].
pub const PROTOCOL_VERSION: u32 = 0;

/// Largest frame we accept (file bytes are not framed, so this is plenty).
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// First frame on every stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    pub token: [u8; 32],
}

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
    let len = r.read_u32().await.context("read frame length")?;
    if len > MAX_FRAME_LEN {
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
}
