//! Pigeon core: protocol types, path guard and the shared transfer code used by
//! both the agent and the CLI.

pub mod agent;
pub mod client;
pub mod guard;
pub mod proto;
pub mod ticket;

pub use ticket::Ticket;

/// ALPN identifying the Pigeon protocol on an iroh connection.
pub const ALPN: &[u8] = b"pigeon/0";

/// Hex-encodes a SHA-256 digest.
pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// Generates a fresh random 32-byte access token (Day-1 auth).
pub fn new_token() -> anyhow::Result<[u8; 32]> {
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).map_err(|e| anyhow::anyhow!("no OS randomness: {e}"))?;
    Ok(token)
}
