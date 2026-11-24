//! `pigeon-agent whoami` through the real binary, in a temporary
//! `PIGEON_HOME`. On Windows this uses the real Credential Manager (a
//! namespace of its own, deleted afterwards); elsewhere the owner-only file
//! store, so a macOS Keychain prompt can never block CI.

use std::{path::Path, process::Command};

use pigeon_core::{config::Home, identity::DEVICE_KEY};

fn run(home: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pigeon-agent"));
    cmd.args(args).env("PIGEON_HOME", home);
    if !cfg!(windows) {
        cmd.env("PIGEON_KEYSTORE", "file");
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "pigeon-agent {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn field(out: &str, name: &str) -> String {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("no {name} in {out}"))
        .trim()
        .to_string()
}

/// Deletes the test's Credential Manager entry on Windows.
struct Cleanup(Home);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if cfg!(windows) {
            let store = pigeon_core::keystore::os_store(&self.0.dir, &self.0.namespace);
            let _ = store.delete(DEVICE_KEY);
        }
    }
}

#[test]
fn whoami_is_stable_renames_and_matches_devices() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("pigeon");
    let _cleanup = Cleanup(Home::at(home.clone()).unwrap());
    let first = run(&home, &["whoami"]);
    let fp = field(&first, "fingerprint");
    assert_eq!(fp.len(), 8, "{first}");
    assert!(!field(&first, "name").is_empty());
    let again = run(&home, &["whoami", "--name", "Office Mac"]);
    assert_eq!(field(&again, "fingerprint"), fp);
    assert_eq!(field(&again, "name"), "Office Mac");
    let devices = run(&home, &["devices"]);
    assert!(
        devices.contains(&format!("this computer: Office Mac ({fp})")),
        "{devices}"
    );
}
