//! Ticket: everything a client needs to reach an agent (address + access token),
//! encoded as one copy-pasteable string.

use std::{fmt, str::FromStr};

use anyhow::{Context, Result};
use data_encoding::BASE32_NOPAD;
use iroh::EndpointAddr;
use serde::{Deserialize, Serialize};

/// Prefix so tickets are recognisable at a glance.
const PREFIX: &str = "pigeon";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    /// The agent's iroh address (endpoint id + relay url + direct addresses).
    pub addr: EndpointAddr,
    /// Day-1 shared secret. Replaced by proper pairing later.
    pub token: [u8; 32],
}

/// Never prints the token, so a stray `{:?}` cannot leak it into logs (SEC-09).
impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ticket")
            .field("addr", &self.addr)
            .field("token", &crate::REDACTED)
            .finish()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    #[test]
    fn debug_never_prints_the_token() {
        // A fixed key: a random one's hex can contain "171" or "abab" by
        // chance, which made this test flaky (~1.5% of runs).
        let key = SecretKey::from_bytes(&[1; 32]).public();
        let t = Ticket {
            addr: EndpointAddr::new(key),
            token: [0xAB; 32],
        };
        assert!(!format!("{key:?}").contains("171") && !key.to_string().contains("abab"));
        let dbg = format!("{t:?} {t:#?}");
        assert!(dbg.contains("<redacted>"), "{dbg}");
        assert!(!dbg.contains("171"), "token bytes leaked: {dbg}");
        assert!(
            !dbg.to_lowercase().contains("abab"),
            "token bytes leaked: {dbg}"
        );
        let hello = crate::proto::Hello {
            version: 0,
            token: [0xAB; 32],
        };
        assert!(!format!("{hello:?}").contains("171"));
        let share = crate::agent::Share {
            root: "x".into(),
            token: [0xAB; 32],
        };
        assert!(!format!("{share:?}").contains("171"));
    }
}
