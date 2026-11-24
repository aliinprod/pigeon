//! One key per computer, through both real binaries (`pigeon` and
//! `pigeon-agent`) in a temporary `PIGEON_HOME`.
//!
//! Keystore: on Windows the real Credential Manager (a namespace of its own,
//! deleted afterwards); elsewhere the owner-only file store, so a macOS
//! Keychain prompt for "another program" can never block CI. The in-process
//! Keychain round-trip is `keystore::tests::os_keystore_migration_roundtrip`.
//!
//! `pigeon-agent` is found next to `pigeon` in the target dir; `cargo test
//! --workspace` builds it (its package has integration tests too).

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

use iroh::SecretKey;
use pigeon_core::{
    config::Home,
    devices::{AGENTS_FILE, AgentList, DEVICES_FILE, Device, DeviceList, PairedAgent},
    fingerprint,
    identity::{DEVICE_KEY, LEGACY_AGENT_KEY, LEGACY_CLIENT_KEY},
    keystore::{FileStore, SecretStore, read_key, write_key},
};

fn agent_bin() -> PathBuf {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_pigeon"));
    let agent = cli.with_file_name(format!("pigeon-agent{}", std::env::consts::EXE_SUFFIX));
    assert!(
        agent.exists(),
        "{} is missing: run `cargo test --workspace` (it builds both programs)",
        agent.display()
    );
    agent
}

fn cli_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pigeon"))
}

fn spawn(bin: &Path, home: &Path, args: &[&str]) -> Child {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("PIGEON_HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !cfg!(windows) {
        cmd.env("PIGEON_KEYSTORE", "file");
    }
    cmd.spawn().unwrap()
}

fn finish(child: Child, what: &str) -> Output {
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{what} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn run(bin: &Path, home: &Path, args: &[&str]) -> String {
    let out = finish(spawn(bin, home, args), &format!("{bin:?} {args:?}"));
    String::from_utf8(out.stdout).unwrap()
}

fn field(out: &str, name: &str) -> String {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("no {name} in {out}"))
        .trim()
        .to_string()
}

/// The store the binaries use for `home` (see the module docs).
fn store(home: &Home) -> Box<dyn SecretStore> {
    if cfg!(windows) {
        pigeon_core::keystore::os_store(&home.dir, &home.namespace)
    } else {
        Box::new(FileStore::new(home.dir.join("secrets")))
    }
}

/// A temporary home whose keystore entries are deleted afterwards.
struct TestHome {
    _dir: tempfile::TempDir,
    home: Home,
}

impl TestHome {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at(dir.path().join("pigeon")).unwrap();
        Self { _dir: dir, home }
    }

    fn path(&self) -> &Path {
        &self.home.dir
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let store = store(&self.home);
        for name in [DEVICE_KEY, LEGACY_AGENT_KEY, LEGACY_CLIENT_KEY] {
            let _ = store.delete(name);
        }
    }
}

/// Fresh install, both programs started at the same time (4 each): one
/// key, one fingerprint and one name for all of them.
#[test]
fn both_programs_create_and_show_one_identity_at_once() {
    let t = TestHome::new();
    let children: Vec<(Child, &str)> = (0..4)
        .flat_map(|_| {
            [
                (spawn(&agent_bin(), t.path(), &["whoami"]), "pigeon-agent"),
                (spawn(&cli_bin(), t.path(), &["whoami"]), "pigeon"),
            ]
        })
        .collect();
    let outs: Vec<String> = children
        .into_iter()
        .map(|(c, what)| String::from_utf8(finish(c, what).stdout).unwrap())
        .collect();
    let fp = field(&outs[0], "fingerprint");
    let name = field(&outs[0], "name");
    for out in &outs {
        assert_eq!(field(out, "fingerprint"), fp, "{outs:?}");
        assert_eq!(field(out, "name"), name, "{outs:?}");
    }
    let key = read_key(store(&t.home).as_ref(), DEVICE_KEY)
        .unwrap()
        .unwrap();
    assert_eq!(fingerprint(&key.public()), fp);

    // A rename in one program shows in the other; so does the fingerprint
    // in `devices` and `agents`.
    run(&cli_bin(), t.path(), &["whoami", "--name", "Home PC"]);
    let agent = run(&agent_bin(), t.path(), &["whoami"]);
    assert_eq!(field(&agent, "name"), "Home PC");
    let header = format!("this computer: Home PC ({fp})");
    assert!(run(&agent_bin(), t.path(), &["devices"]).contains(&header));
    assert!(run(&cli_bin(), t.path(), &["agents"]).contains(&header));
}

/// W-12 through the real binaries: on a migrated two-key install,
/// `pigeon-agent reset --yes` retires both old keys (and prints the revoke
/// command for each), lists the remembered agent, and afterwards `pigeon`
/// dials every agent with the new key only.
#[test]
fn reset_retires_every_old_key() {
    let t = TestHome::new();
    let store = store(&t.home);
    let agent_key = SecretKey::generate();
    let client_key = SecretKey::generate();
    write_key(store.as_ref(), LEGACY_AGENT_KEY, &agent_key).unwrap();
    write_key(store.as_ref(), LEGACY_CLIENT_KEY, &client_key).unwrap();
    AgentList::update(&t.path().join(AGENTS_FILE), |l| {
        l.upsert(PairedAgent {
            id: SecretKey::generate().public(),
            name: "Office Mac".into(),
            relay_url: None,
            paired_at: 1,
            legacy_key: None,
        });
        Ok(())
    })
    .unwrap();
    run(&cli_bin(), t.path(), &["whoami"]); // migrates
    let out = run(&agent_bin(), t.path(), &["reset", "--yes"]);
    for old in [&agent_key, &client_key] {
        let line = format!("pigeon-agent revoke {}", fingerprint(&old.public()));
        assert!(out.contains(&line), "{out}");
    }
    assert!(out.contains("Office Mac"), "{out}");
    assert!(
        read_key(store.as_ref(), LEGACY_CLIENT_KEY)
            .unwrap()
            .is_none()
    );
    assert!(
        read_key(store.as_ref(), LEGACY_AGENT_KEY)
            .unwrap()
            .is_none()
    );
    let new = read_key(store.as_ref(), DEVICE_KEY).unwrap().unwrap();
    let whoami = run(&cli_bin(), t.path(), &["whoami"]);
    assert_eq!(field(&whoami, "fingerprint"), fingerprint(&new.public()));
    assert!(!whoami.contains("old key"), "{whoami}");
    let agents = run(&cli_bin(), t.path(), &["agents"]);
    assert!(!agents.contains("old key"), "{agents}");
}

/// The Day-5 state (agent key + client key, a device on the allow-list, an
/// agent remembered) migrated by the real binaries, both at once: the agent
/// key becomes the one key, the old client key stays for the remembered
/// agent only, nothing is lost, and running again changes nothing.
#[test]
fn binaries_migrate_a_two_key_install() {
    let t = TestHome::new();
    let store = store(&t.home);
    let agent_key = SecretKey::generate();
    let client_key = SecretKey::generate();
    write_key(store.as_ref(), LEGACY_AGENT_KEY, &agent_key).unwrap();
    write_key(store.as_ref(), LEGACY_CLIENT_KEY, &client_key).unwrap();
    let pc = SecretKey::generate().public();
    DeviceList::update(&t.path().join(DEVICES_FILE), |l| {
        l.upsert(Device {
            id: pc,
            name: "Home PC".into(),
            paired_at: 1,
        });
        Ok(())
    })
    .unwrap();
    let office = SecretKey::generate().public();
    AgentList::update(&t.path().join(AGENTS_FILE), |l| {
        l.upsert(PairedAgent {
            id: office,
            name: "Office Mac".into(),
            relay_url: None,
            paired_at: 1,
            legacy_key: None,
        });
        Ok(())
    })
    .unwrap();

    let a = spawn(&agent_bin(), t.path(), &["whoami"]);
    let c = spawn(&cli_bin(), t.path(), &["whoami"]);
    let (a, c) = (finish(a, "pigeon-agent"), finish(c, "pigeon"));
    let (a, c) = (
        String::from_utf8(a.stdout).unwrap(),
        String::from_utf8(c.stdout).unwrap(),
    );
    let fp = fingerprint(&agent_key.public());
    assert_eq!(field(&a, "fingerprint"), fp, "{a}");
    assert_eq!(field(&c, "fingerprint"), fp, "{c}");
    let old_fp = fingerprint(&client_key.public());
    assert!(field(&c, "old key").starts_with(&old_fp), "{c}");
    assert!(c.contains("Office Mac"), "{c}");

    assert!(
        read_key(store.as_ref(), LEGACY_AGENT_KEY)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        read_key(store.as_ref(), LEGACY_CLIENT_KEY)
            .unwrap()
            .unwrap()
            .public(),
        client_key.public()
    );
    let agents = run(&cli_bin(), t.path(), &["agents"]);
    assert!(agents.contains(&format!("old key {old_fp}")), "{agents}");
    let devices = run(&agent_bin(), t.path(), &["devices"]);
    assert!(devices.contains(&fingerprint(&pc)), "{devices}");

    // Idempotent: nothing more to say, same identity.
    let out = finish(spawn(&cli_bin(), t.path(), &["whoami"]), "pigeon");
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        field(&String::from_utf8(out.stdout).unwrap(), "fingerprint"),
        fp
    );

    // Forgetting the only agent that needed the old key drops that key.
    run(&cli_bin(), t.path(), &["forget", "Office Mac"]);
    assert!(
        read_key(store.as_ref(), LEGACY_CLIENT_KEY)
            .unwrap()
            .is_none()
    );
    let after = run(&cli_bin(), t.path(), &["whoami"]);
    assert!(!after.contains("old key"), "{after}");
    assert_eq!(field(&after, "fingerprint"), fp);
}
