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

/// Creates `dir` (and parents) if needed and makes it owner-only: `0700` on
/// Unix; on Windows a protected DACL (no inheritance from the parent) that
/// grants full control to the current user and SYSTEM only, inherited by
/// every file created inside (W-3). If that fails we refuse to go on.
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("could not restrict {}", dir.display()))?;
    }
    #[cfg(windows)]
    winacl::restrict_to_current_user(dir)
        .with_context(|| format!("could not make {} private to this user", dir.display()))?;
    Ok(())
}

/// Owner-only DACL on Windows through the Win32 API. `unsafe` is unavoidable
/// (no std API for ACLs); every call is a thin FFI wrapper with its pointer
/// rules next to it.
#[cfg(windows)]
#[allow(unsafe_code)]
mod winacl {
    use std::{os::windows::ffi::OsStrExt, path::Path, ptr::null_mut};

    use anyhow::{Result, bail};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE, LocalFree},
        Security::{
            ACL,
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
            },
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    /// The current user's SID as a string, e.g. `S-1-5-21-...`.
    pub fn current_user_sid() -> Result<String> {
        let mut token: HANDLE = null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo handle that needs no
        // closing; `token` is a valid out-pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            // SAFETY: plain FFI call with no arguments.
            bail!("OpenProcessToken failed (error {})", unsafe {
                GetLastError()
            });
        }
        let result = (|| {
            let mut len = 0u32;
            // SAFETY: a size query with a null buffer of length 0 is the
            // documented way to learn the needed size; `len` is valid.
            unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len) };
            // u64 elements keep the buffer aligned for TOKEN_USER.
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            // SAFETY: `buf` has at least `len` writable bytes.
            let ok = unsafe {
                GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len)
            };
            if ok == 0 {
                // SAFETY: plain FFI call with no arguments.
                bail!("GetTokenInformation failed (error {})", unsafe {
                    GetLastError()
                });
            }
            // SAFETY: on success the buffer starts with a TOKEN_USER whose SID
            // points into the same buffer, which outlives this use.
            let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
            let mut text: *mut u16 = null_mut();
            // SAFETY: `sid` is valid (see above); `text` is a valid out-pointer.
            if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
                // SAFETY: plain FFI call with no arguments.
                bail!("ConvertSidToStringSidW failed (error {})", unsafe {
                    GetLastError()
                });
            }
            // SAFETY: on success `text` is a NUL-terminated UTF-16 string
            // allocated with LocalAlloc; we read up to the NUL, then free it once.
            let s = unsafe {
                let mut n = 0;
                while *text.add(n) != 0 {
                    n += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
                LocalFree(text.cast());
                s
            };
            Ok(s)
        })();
        // SAFETY: `token` is a real handle from OpenProcessToken, closed once.
        unsafe { CloseHandle(token) };
        result
    }

    pub fn restrict_to_current_user(dir: &Path) -> Result<()> {
        let sid = current_user_sid()?;
        anyhow::ensure!(
            sid.starts_with("S-") && sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "unexpected user SID"
        );
        // P = protected (do not inherit the parent's ACEs). OICI = inherited
        // by files and subfolders. FA = full access. SY = LocalSystem.
        let sddl = wide(format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)").as_ref());
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16; `sd` is a valid
        // out-pointer and is freed with LocalFree below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 {
            // SAFETY: plain FFI call with no arguments.
            bail!(
                "could not build the security descriptor (error {})",
                unsafe { GetLastError() }
            );
        }
        let path = wide(dir.as_os_str());
        // SAFETY: `sd` is a valid descriptor until LocalFree; the DACL
        // pointer we get points into it. `path` is NUL-terminated UTF-16.
        let err = unsafe {
            let mut present = 0;
            let mut defaulted = 0;
            let mut dacl: *mut ACL = null_mut();
            let got = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
            let err = if got == 0 || present == 0 || dacl.is_null() {
                u32::MAX
            } else {
                SetNamedSecurityInfoW(
                    path.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    dacl,
                    std::ptr::null(),
                )
            };
            LocalFree(sd);
            err
        };
        if err != 0 {
            bail!("SetNamedSecurityInfoW failed (error {err})");
        }
        Ok(())
    }
}

/// A path for people to read: on Windows without the `\\?\` prefix that
/// `canonicalize` adds (N6).
pub fn display_path(path: &Path) -> String {
    let s = path.display().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

/// The current Windows user's SID (for tests and diagnostics).
#[cfg(windows)]
pub fn current_user_sid() -> Result<String> {
    winacl::current_user_sid()
}

/// Replaces `path` with `bytes` atomically (temp file in the same dir, then
/// rename). The file is owner-only: `0600` on Unix; on Windows it inherits
/// the owner-only DACL that [`ensure_private_dir`] puts on its folder.
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

    /// W-3 / SEC-14: on Windows the config dir gets a protected DACL (user +
    /// SYSTEM only), even inside a parent that lets Authenticated Users
    /// modify everything below it (like `C:\` on a default install), and
    /// files written there inherit only those two entries.
    #[cfg(windows)]
    #[test]
    fn windows_config_dir_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("loose");
        std::fs::create_dir(&parent).unwrap();
        // Authenticated Users (S-1-5-11): Modify, inherited by children.
        let out = std::process::Command::new("icacls")
            .arg(&parent)
            .args(["/grant", "*S-1-5-11:(OI)(CI)M"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");

        let home = parent.join("pigeon-home");
        ensure_private_dir(&home).unwrap();
        let file = home.join("devices.json");
        write_private(&file, b"{}").unwrap();
        let me = current_user_sid().unwrap();
        for (path, must_be_protected) in [(&home, true), (&file, false)] {
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(format!("(Get-Acl -LiteralPath '{}').Sddl", path.display()))
                .output()
                .unwrap();
            let sddl = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let dacl = &sddl[sddl.find("D:").expect(&sddl)..];
            if must_be_protected {
                assert!(dacl.starts_with("D:P"), "not protected: {sddl}");
            }
            let trustees: Vec<&str> = dacl
                .split('(')
                .skip(1)
                .map(|ace| ace.trim_end_matches(')').rsplit(';').next().unwrap())
                .collect();
            assert!(!trustees.is_empty(), "{sddl}");
            for t in trustees {
                assert!(
                    t == me || t == "SY",
                    "{} grants access to {t}: {sddl}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn display_path_drops_the_verbatim_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\Users\x")), r"C:\Users\x");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\srv\share")),
            r"\\srv\share"
        );
        assert_eq!(display_path(Path::new("/Users/x")), "/Users/x");
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
