//! Ticket: everything a client needs to reach an agent (address + access token),
//! encoded as one copy-pasteable string.

use std::{fmt, str::FromStr};

use anyhow::{Context, Result};
use data_encoding::BASE32_NOPAD;
use iroh::EndpointAddr;
use serde::{Deserialize, Serialize};

/// Prefix so tickets are recognisable at a glance.
const PREFIX: &str = "pigeon";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    /// The agent's iroh address (endpoint id + relay url + direct addresses).
    pub addr: EndpointAddr,
    /// Day-1 shared secret. Replaced by proper pairing later.
    pub token: [u8; 32],
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = postcard::to_stdvec(self).map_err(|_| fmt::Error)?;
        let text = BASE32_NOPAD.encode(&bytes).to_ascii_lowercase();
        write!(f, "{PREFIX}{text}")
    }
}

impl FromStr for Ticket {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        let body = s
            .strip_prefix(PREFIX)
            .context("not a pigeon ticket (missing `pigeon` prefix)")?;
        let bytes = BASE32_NOPAD
            .decode(body.to_ascii_uppercase().as_bytes())
            .context("ticket is not valid base32")?;
        postcard::from_bytes(&bytes).context("ticket is corrupted")
    }
}
