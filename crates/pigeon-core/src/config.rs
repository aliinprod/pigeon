//! Where Pigeon keeps its state on a device, and how it writes it.
//!
//! - macOS: `~/Library/Application Support/Pigeon`
//! - Windows: `%APPDATA%\Pigeon`
//! - others: `$XDG_CONFIG_HOME/pigeon` or `~/.config/pigeon`
//! - `PIGEON_HOME=<dir>` overrides all of these (tests, CI, a second identity).
//!
//! Files here are not secret (keys live in the OS keystore, see
//! [`crate::keystore`]) but their integrity matters, so they are created
//! owner-only and replaced atomically.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use iroh::SecretKey;

use crate::keystore::{self, SecretStore};

/// Environment variable that overrides the config dir.
pub const HOME_ENV: &str = "PIGEON_HOME";

/// One device's Pigeon state: a config dir plus the keystore namespace that
/// belongs to it.
#[derive(Debug, Clone)]
pub struct Home {
    pub dir: PathBuf,
    /// Empty for the default dir. For any other dir, a short hash of its path,
    /// so two homes on one machine never share keys.
    pub namespace: String,
}

impl Home {
    /// The home from `PIGEON_HOME`, or the platform default.
    pub fn from_env() -> Result<Self> {
        match std::env::var_os(HOME_ENV) {
            Some(dir) if !dir.is_empty() => Self::at(PathBuf::from(dir)),
            _ => Ok(Self {
                dir: default_dir()?,
                namespace: String::new(),
            }),
        }
    }

    /// A home in `dir` (created if needed) with its own keystore namespace.
    pub fn at(dir: PathBuf) -> Result<Self> {
        ensure_private_dir(&dir)?;
        let canon = dir.canonicalize()?;
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(canon.to_string_lossy().as_bytes());
        Ok(Self {
            dir: canon,
            namespace: crate::hex(&digest[..6]),
        })
    }

    /// The secret store for this home.
    pub fn store(&self) -> Result<Box<dyn SecretStore>> {
        ensure_private_dir(&self.dir)?;
        keystore::default_store(&self.dir, &self.namespace)
    }

    /// Loads (or creates on first use) the device key `name`.
    pub fn device_key(&self, name: &str) -> Result<(SecretKey, Box<dyn SecretStore>, bool)> {
        let store = self.store()?;
        let (key, created) = keystore::load_or_create_key(store.as_ref(), name)?;
        Ok((key, store, created))
    }
}

fn default_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home).join("Library/Application Support/Pigeon"))
    }
    #[cfg(windows)]
    {
        let appdata = std::env::var_os("APPDATA").context("APPDATA is not set")?;
        Ok(PathBuf::from(appdata).join("Pigeon"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(xdg).join("pigeon"));
        }
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home).join(".config/pigeon"))
    }
}

/// Creates `dir` (and parents) if needed; on Unix the dir itself is `0700`.
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("could not restrict {}", dir.display()))?;
    }
    Ok(())
}

/// Replaces `path` with `bytes` atomically (temp file in the same dir, then
/// rename). The file is owner-only (`0600`) on Unix; on Windows it inherits
/// the user-only ACL of the profile folder it lives in.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("file has no parent dir")?;
    let name = path
        .file_name()
        .context("file has no name")?
        .to_string_lossy();
    let mut tag = [0u8; 6];
    getrandom::fill(&mut tag).map_err(|e| anyhow::anyhow!("no OS randomness: {e}"))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", crate::hex(&tag)));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("could not create {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path).with_context(|| format!("could not replace {}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Test helper: fails if any file under `dir` contains one of `needles`.
#[cfg(test)]
pub(crate) fn assert_no_file_contains(dir: &Path, needles: &[Vec<u8>]) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_no_file_contains(&path, needles);
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        for n in needles {
            assert!(
                !data.windows(n.len()).any(|w| w == n.as_slice()),
                "secret bytes found in {}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_private_replaces_atomically_and_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        write_private(&path, b"one").unwrap();
        write_private(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "temp files left behind: {names:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn separate_homes_get_separate_namespaces() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ha = Home::at(a.path().to_path_buf()).unwrap();
        let hb = Home::at(b.path().to_path_buf()).unwrap();
        assert_ne!(ha.namespace, hb.namespace);
        assert_eq!(
            Home::at(a.path().to_path_buf()).unwrap().namespace,
            ha.namespace
        );
    }
}
