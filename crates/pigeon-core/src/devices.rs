//! Who is paired with whom, as small JSON files in the config dir.
//!
//! - Agent: `devices.json`, the allow-list of device keys that may connect.
//! - Client: `agents.json`, the agents this device paired with.
//!
//! Not secret (public keys and names), but integrity matters: written
//! owner-only and atomically ([`crate::config::write_private`]).

use std::path::Path;

use anyhow::{Context, Result, bail};
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const DEVICES_FILE: &str = "devices.json";
pub const AGENTS_FILE: &str = "agents.json";

/// A device on the agent's allow-list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// Endpoint ID (hex public key).
    pub id: EndpointId,
    pub name: String,
    /// Unix seconds.
    pub paired_at: u64,
}

/// The agent's allow-list. Deny by default: empty means nobody.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceList {
    pub devices: Vec<Device>,
}

/// An agent this client paired with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedAgent {
    pub id: EndpointId,
    pub name: String,
    /// Relay the agent used when paired: a hint only. The agent is found by
    /// its endpoint ID through iroh address lookup.
    pub relay_url: Option<RelayUrl>,
    pub paired_at: u64,
}

impl PairedAgent {
    /// Address to dial: the endpoint ID plus the relay hint (unless
    /// `relay_hint` is false). Never stale direct IPs.
    pub fn dial_addr(&self) -> EndpointAddr {
        let addr = EndpointAddr::new(self.id);
        match &self.relay_url {
            Some(url) => addr.with_relay_url(url.clone()),
            None => addr,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentList {
    pub agents: Vec<PairedAgent>,
}

fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is damaged; fix or delete it", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut json = serde_json::to_vec_pretty(value)?;
    json.push(b'\n');
    crate::config::write_private(path, &json)
}

/// Finds the one entry matching `query`: a fingerprint prefix (at least 4
/// characters) or a name (case-insensitive).
fn find<'a, T>(
    items: &'a [T],
    query: &str,
    id: impl Fn(&T) -> EndpointId,
    name: impl Fn(&T) -> &str,
) -> Result<&'a T> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        bail!("give a name or fingerprint");
    }
    let by_name: Vec<&T> = items
        .iter()
        .filter(|i| name(i).to_lowercase() == q)
        .collect();
    let hits = if by_name.is_empty() && q.len() >= 4 {
        items
            .iter()
            .filter(|i| crate::fingerprint(&id(i)).starts_with(&q))
            .collect()
    } else {
        by_name
    };
    match hits.as_slice() {
        [one] => Ok(one),
        [] => bail!("no paired device matches {:?}", crate::display_safe(query)),
        _ => bail!(
            "{:?} matches more than one device; use the fingerprint",
            crate::display_safe(query)
        ),
    }
}

impl DeviceList {
    pub fn load(path: &Path) -> Result<Self> {
        load(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save(path, self)
    }

    pub fn contains(&self, id: &EndpointId) -> bool {
        self.devices.iter().any(|d| d.id == *id)
    }

    /// Adds or renames `device` (one entry per key).
    pub fn upsert(&mut self, device: Device) {
        self.devices.retain(|d| d.id != device.id);
        self.devices.push(device);
    }

    pub fn find(&self, query: &str) -> Result<&Device> {
        find(&self.devices, query, |d| d.id, |d| &d.name)
    }

    /// Removes the device matching `query` and returns it.
    pub fn remove(&mut self, query: &str) -> Result<Device> {
        let found = self.find(query)?.clone();
        self.devices.retain(|d| d.id != found.id);
        Ok(found)
    }
}

impl AgentList {
    pub fn load(path: &Path) -> Result<Self> {
        load(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save(path, self)
    }

    pub fn upsert(&mut self, agent: PairedAgent) {
        self.agents.retain(|a| a.id != agent.id);
        self.agents.push(agent);
    }

    /// The agent to use: the one matching `query`, or the only one.
    pub fn pick(&self, query: Option<&str>) -> Result<&PairedAgent> {
        match (query, self.agents.as_slice()) {
            (Some(q), agents) => find(agents, q, |a| a.id, |a| &a.name),
            (None, [one]) => Ok(one),
            (None, []) => bail!("not paired with any agent yet: run `pigeon pair <INVITE-LINK>`"),
            (None, _) => bail!("paired with several agents; choose one with --agent <NAME>"),
        }
    }

    pub fn remove(&mut self, query: &str) -> Result<PairedAgent> {
        let found = find(&self.agents, query, |a| a.id, |a| &a.name)?.clone();
        self.agents.retain(|a| a.id != found.id);
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn device(seed: u8, name: &str) -> Device {
        Device {
            id: SecretKey::from_bytes(&[seed; 32]).public(),
            name: name.into(),
            paired_at: 1,
        }
    }

    #[test]
    fn allow_list_roundtrips_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILE);
        let empty = DeviceList::load(&path).unwrap();
        assert!(empty.devices.is_empty(), "deny by default");
        let mut list = empty;
        list.upsert(device(1, "Home PC"));
        list.upsert(device(2, "Laptop"));
        list.upsert(device(1, "Home PC renamed"));
        assert_eq!(list.devices.len(), 2);
        list.save(&path).unwrap();
        assert_eq!(DeviceList::load(&path).unwrap(), list);
    }

    #[test]
    fn find_by_name_or_fingerprint() {
        let mut list = DeviceList::default();
        list.upsert(device(1, "Home PC"));
        list.upsert(device(2, "Laptop"));
        let fp = crate::fingerprint(&list.devices[1].id);
        assert_eq!(list.find("home pc").unwrap().name, "Home PC");
        assert_eq!(list.find(&fp).unwrap().name, "Laptop");
        assert_eq!(list.find(&fp[..4]).unwrap().name, "Laptop");
        assert!(list.find("nobody").is_err());
        assert!(list.find("").is_err());
        let gone = list.remove("laptop").unwrap();
        assert_eq!(gone.name, "Laptop");
        assert!(!list.contains(&gone.id));
    }

    #[test]
    fn damaged_file_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILE);
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(DeviceList::load(&path).is_err());
    }

    #[test]
    fn client_picks_the_only_agent_or_asks() {
        let mut list = AgentList::default();
        assert!(list.pick(None).is_err());
        let a = PairedAgent {
            id: SecretKey::from_bytes(&[7; 32]).public(),
            name: "Office Mac".into(),
            relay_url: Some("https://relay.example.com".parse().unwrap()),
            paired_at: 1,
        };
        list.upsert(a.clone());
        assert_eq!(list.pick(None).unwrap(), &a);
        let addr = a.dial_addr();
        assert_eq!(addr.id, a.id);
        assert!(addr.addrs.iter().all(|t| t.is_relay()));
        list.upsert(PairedAgent {
            id: SecretKey::from_bytes(&[8; 32]).public(),
            name: "Other".into(),
            relay_url: None,
            paired_at: 2,
        });
        assert!(list.pick(None).is_err());
        assert_eq!(list.pick(Some("office mac")).unwrap(), &a);
    }
}
