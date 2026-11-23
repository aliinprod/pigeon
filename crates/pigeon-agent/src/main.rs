//! `pigeon-agent`: shares one folder (read-only) with paired devices.
//!
//! - `pigeon-agent share <DIR>`: serve paired devices (and pairing, while an
//!   invite is active). Keep it running.
//! - `pigeon-agent pair [--name NAME] [--ttl HOURS]`: create a single-use
//!   invite: a link (not secret) and a 4-word code (secret, screen only).
//! - `pigeon-agent devices`: list paired devices.
//! - `pigeon-agent revoke <NAME|FINGERPRINT>`: remove a device; a running
//!   agent drops its connections within a second.
//! - `pigeon-agent reset --yes`: new agent key; forgets every pairing.
//!
//! The agent only connects outward (to the iroh relay); it does not open any
//! listening socket of its own beyond the UDP sockets iroh uses.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, endpoint::presets};
use pigeon_core::{
    agent::{Agent, AgentStorage, ServeOptions},
    config::Home,
    devices::{DEVICES_FILE, Device, DeviceList},
    display_safe, fingerprint,
    keystore::{AGENT_KEY, SecretStore},
    pair::{self, Invite},
};
use serde::{Deserialize, Serialize};

/// Keystore entry holding the active invite (it contains the code).
const INVITE_SECRET: &str = "invite";
/// Non-secret status file: the running agent's ID and relay, so `pair` can
/// put a relay hint in the link without starting a second endpoint.
const STATUS_FILE: &str = "agent-status.json";

#[derive(Parser)]
#[command(name = "pigeon-agent", version, about = "Share a folder with Pigeon")]
struct Cli {
    /// Print details of failed connection attempts.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Share DIR (read-only) with paired devices. Keep it running.
    Share { dir: PathBuf },
    /// Create a single-use pairing invite for one new device.
    Pair {
        /// Name for the device you are pairing, e.g. "Home PC".
        #[arg(long)]
        name: Option<String>,
        /// How long the invite stays valid, in hours (max 24).
        #[arg(long, default_value_t = 12)]
        ttl: u64,
    },
    /// List paired devices.
    Devices,
    /// Remove a paired device (by name or fingerprint).
    Revoke { device: String },
    /// Create a new agent key and forget every paired device.
    Reset {
        /// Confirm: every paired device has to pair again.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Serialize, Deserialize)]
struct Status {
    id: EndpointId,
    relay_url: Option<RelayUrl>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = Home::from_env()?;
    match cli.command {
        Command::Share { dir } => share(home, dir, cli.verbose).await,
        Command::Pair { name, ttl } => pair_cmd(home, name, ttl),
        Command::Devices => devices_cmd(&home),
        Command::Revoke { device } => revoke_cmd(&home, &device),
        Command::Reset { yes } => reset_cmd(&home, yes),
    }
}

/// Files + keystore persistence for the running agent.
struct Storage {
    home: Home,
    store: Box<dyn SecretStore>,
}

impl AgentStorage for Storage {
    fn device_paired(&self, device: &Device) -> Result<()> {
        // Read-modify-write, so a `revoke` made meanwhile is not undone.
        let path = self.home.dir.join(DEVICES_FILE);
        let mut list = DeviceList::load(&path)?;
        list.upsert(device.clone());
        list.save(&path)
    }

    fn invite_changed(&self, invite_id: &[u8; 16], invite: Option<&Invite>) -> Result<()> {
        // Never overwrite or delete a newer invite from a later `pair`.
        if let Some(stored) = load_invite(self.store.as_ref())?
            && stored.invite_id != *invite_id
        {
            return Ok(());
        }
        match invite {
            Some(invite) => self
                .store
                .set(INVITE_SECRET, &invite.to_secret_bytes()?[..]),
            None => self.store.delete(INVITE_SECRET),
        }
    }
}

fn load_invite(store: &dyn SecretStore) -> Result<Option<Invite>> {
    match store.get(INVITE_SECRET)? {
        Some(secret) => Ok(Some(Invite::from_secret_bytes(secret.as_bytes())?)),
        None => Ok(None),
    }
}

async fn share(home: Home, dir: PathBuf, verbose: bool) -> Result<()> {
    let root = dir
        .canonicalize()
        .with_context(|| format!("folder not found: {}", dir.display()))?;
    anyhow::ensure!(root.is_dir(), "not a folder: {}", root.display());

    // A stable device identity: the same endpoint ID after every restart.
    let (key, store, created) = home.device_key(AGENT_KEY)?;
    eprintln!(
        "agent id {} ({} key in {})",
        fingerprint(&key.public()),
        if created { "new" } else { "existing" },
        store.describe()
    );
    let devices_path = home.dir.join(DEVICES_FILE);
    let devices = DeviceList::load(&devices_path)?;

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(key)
        .alpns(vec![pigeon_core::ALPN.to_vec()])
        .bind()
        .await?;
    // Wait until we have a home relay so we are reachable from anywhere.
    if tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .is_err()
    {
        eprintln!("warning: relay not reachable yet; other networks may not find this agent");
    }
    write_status(&home, &endpoint.addr());

    let storage = Storage {
        home: home.clone(),
        store: home.store()?,
    };
    let agent = Agent::new(
        endpoint.clone(),
        root.clone(),
        devices.clone(),
        ServeOptions {
            verbose,
            ..Default::default()
        },
        Box::new(storage),
    );
    if let Some(invite) = load_invite(store.as_ref())? {
        agent.offer_invite(invite);
    }
    eprintln!(
        "sharing {} with {} paired device(s) (Ctrl+C to stop)",
        root.display(),
        devices.devices.len()
    );
    if devices.devices.is_empty() && !agent.invite_active() {
        eprintln!("no device is paired yet: run `pigeon-agent pair` in another terminal");
    }
    if agent.invite_active() {
        eprintln!("a pairing invite is active");
    }

    let store: Arc<dyn SecretStore> = Arc::from(store);
    let watcher = tokio::spawn(watch(agent.clone(), home.clone(), store));
    tokio::select! {
        res = agent.serve() => res?,
        _ = tokio::signal::ctrl_c() => eprintln!("stopping"),
    }
    watcher.abort();
    endpoint.close().await;
    Ok(())
}

/// Picks up changes made by the other commands while `share` runs:
/// `revoke` (devices file, every 500 ms) and `pair` (invite, every 2 s).
async fn watch(agent: Agent, home: Home, store: Arc<dyn SecretStore>) {
    let devices_path = home.dir.join(DEVICES_FILE);
    let mut known = agent.devices();
    let mut tick = 0u64;
    let mut warned = false;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        tick += 1;
        match DeviceList::load(&devices_path) {
            Ok(list) if list != known => {
                for d in &known.devices {
                    if !list.contains(&d.id) {
                        eprintln!("revoked {} ({})", display_safe(&d.name), fingerprint(&d.id));
                    }
                }
                agent.set_devices(list);
                warned = false;
            }
            Ok(_) => {}
            Err(e) if !warned => {
                // Keep the last good list; a broken file must not open access.
                eprintln!("warning: {e:#}; keeping the previous device list");
                warned = true;
            }
            Err(_) => {}
        }
        // Pairing in this process may have changed the list too.
        known = agent.devices();
        if tick.is_multiple_of(4) {
            agent.expire_invite();
            match load_invite(store.as_ref()) {
                Ok(Some(invite)) => {
                    let was_active = agent.invite_active();
                    agent.offer_invite(invite);
                    if !was_active && agent.invite_active() {
                        eprintln!("a new pairing invite is active");
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("warning: could not read the pairing invite: {e:#}"),
            }
        }
    }
}

fn write_status(home: &Home, addr: &EndpointAddr) {
    let status = Status {
        id: addr.id,
        relay_url: addr.relay_urls().next().cloned(),
    };
    let result = serde_json::to_vec_pretty(&status)
        .map_err(anyhow::Error::from)
        .and_then(|json| pigeon_core::config::write_private(&home.dir.join(STATUS_FILE), &json));
    if let Err(e) = result {
        eprintln!("warning: could not write {STATUS_FILE}: {e:#}");
    }
}

fn pair_cmd(home: Home, name: Option<String>, ttl_hours: u64) -> Result<()> {
    anyhow::ensure!(
        (1..=24).contains(&ttl_hours),
        "--ttl must be between 1 and 24 hours"
    );
    let (key, store, _) = home.device_key(AGENT_KEY)?;
    let id = key.public();
    // Relay hint from a running (or the last) `share`, if it used this key.
    let mut addr = EndpointAddr::new(id);
    if let Ok(bytes) = std::fs::read(home.dir.join(STATUS_FILE))
        && let Ok(status) = serde_json::from_slice::<Status>(&bytes)
        && status.id == id
        && let Some(url) = status.relay_url
    {
        addr = addr.with_relay_url(url);
    }
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let ttl = Duration::from_secs(ttl_hours * 3600);
    let (invite, code) = Invite::new(ttl, pair::now_secs(), name.clone())?;
    // A new invite replaces the old one (only one at a time).
    store.set(INVITE_SECRET, &invite.to_secret_bytes()?[..])?;
    let link = invite.link(addr);

    let who = name.as_deref().unwrap_or("a new device");
    eprintln!(
        "Invite for {} (single use, valid {ttl_hours} h, 3 attempts).",
        display_safe(who)
    );
    eprintln!();
    eprintln!("Link (not secret; you may email it to yourself):");
    println!("link: {link}");
    eprintln!();
    eprintln!("Code (secret: keep it on this screen or a photo, never send it):");
    println!("code: {}", code.reveal());
    eprintln!();
    eprintln!("On the other device run `pigeon pair <LINK>` and type the code.");
    eprintln!("Keep `pigeon-agent share <DIR>` running; it picks up this invite within seconds.");
    Ok(())
}

fn devices_cmd(home: &Home) -> Result<()> {
    let list = DeviceList::load(&home.dir.join(DEVICES_FILE))?;
    if list.devices.is_empty() {
        println!("no paired devices (deny by default). Pair one with `pigeon-agent pair`.");
        return Ok(());
    }
    let now = pair::now_secs();
    println!("{:<10}  {:<14}  name", "fingerprint", "paired");
    for d in &list.devices {
        println!(
            "{:<10}  {:<14}  {}",
            fingerprint(&d.id),
            ago(now, d.paired_at),
            display_safe(&d.name)
        );
    }
    Ok(())
}

fn ago(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}

fn revoke_cmd(home: &Home, query: &str) -> Result<()> {
    let path = home.dir.join(DEVICES_FILE);
    let mut list = DeviceList::load(&path)?;
    let gone = list.remove(query)?;
    list.save(&path)?;
    println!(
        "revoked {} ({}). A running agent drops its connections within a second.",
        display_safe(&gone.name),
        fingerprint(&gone.id)
    );
    Ok(())
}

fn reset_cmd(home: &Home, yes: bool) -> Result<()> {
    anyhow::ensure!(
        yes,
        "this makes a new agent key and forgets every paired device; run again with --yes"
    );
    let store = home.store()?;
    store.delete(AGENT_KEY)?;
    store.delete(INVITE_SECRET)?;
    for file in [DEVICES_FILE, STATUS_FILE] {
        match std::fs::remove_file(home.dir.join(file)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context(format!("could not remove {file}")),
        }
    }
    println!("reset done: the next `pigeon-agent share` makes a new key; pair your devices again.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pigeon_core::keystore::MemoryStore;

    #[test]
    fn storage_never_overwrites_a_newer_invite() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at(dir.path().to_path_buf()).unwrap();
        let storage = Storage {
            home,
            store: Box::new(MemoryStore::default()),
        };
        let (old, _) = Invite::new(pair::DEFAULT_TTL, 1, None).unwrap();
        let (new, _) = Invite::new(pair::DEFAULT_TTL, 1, None).unwrap();
        storage
            .store
            .set(INVITE_SECRET, &new.to_secret_bytes().unwrap()[..])
            .unwrap();
        // The running agent finishing with the old invite must not touch the new one.
        storage.invite_changed(&old.invite_id, None).unwrap();
        storage.invite_changed(&old.invite_id, Some(&old)).unwrap();
        assert_eq!(
            load_invite(storage.store.as_ref()).unwrap(),
            Some(new.clone())
        );
        // Its own invite it does remove.
        storage.invite_changed(&new.invite_id, None).unwrap();
        assert_eq!(load_invite(storage.store.as_ref()).unwrap(), None);
    }

    #[test]
    fn device_paired_keeps_a_concurrent_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at(dir.path().to_path_buf()).unwrap();
        let path = home.dir.join(DEVICES_FILE);
        let storage = Storage {
            home,
            store: Box::new(MemoryStore::default()),
        };
        let dev = |seed: u8, name: &str| Device {
            id: iroh::SecretKey::from_bytes(&[seed; 32]).public(),
            name: name.into(),
            paired_at: 1,
        };
        storage.device_paired(&dev(1, "a")).unwrap();
        storage.device_paired(&dev(2, "b")).unwrap();
        // `revoke a` from another process edits the file...
        let mut list = DeviceList::load(&path).unwrap();
        list.remove("a").unwrap();
        list.save(&path).unwrap();
        // ...and a later pairing in the running agent must not bring it back.
        storage.device_paired(&dev(3, "c")).unwrap();
        let names: Vec<_> = DeviceList::load(&path)
            .unwrap()
            .devices
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, ["b", "c"]);
    }
}
