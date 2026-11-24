//! Secret storage for device keys (and, later, pairing invites).
//!
//! - macOS: the login Keychain (generic password items, service `Pigeon`).
//! - Windows: Credential Manager (generic credentials, user scope; Windows
//!   protects them with DPAPI under the user's logon).
//! - Other OSes, or `PIGEON_KEYSTORE=file`: one owner-only file per secret in
//!   the config dir (`0600`). This is weaker than an OS keystore and is meant
//!   for headless or test machines only.
//!
//! No custom crypto: the OS keystore does the protecting; Pigeon only stores
//! and reads bytes (SEC-22, SEC-23).

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Context, Result, bail};
use iroh::SecretKey;
use zeroize::Zeroizing;

/// Environment variable that forces the file backend (`file`).
pub const KEYSTORE_ENV: &str = "PIGEON_KEYSTORE";

// Key names live in [`crate::identity`]: one device key per computer
// (`device-key`), plus the two per-role names from before (`agent-key`,
// `client-key`) that the migration reads.

/// Secret bytes read from a store. Zeroed on drop, never printed.
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret({})", crate::REDACTED)
    }
}

/// A place to keep small secrets by name.
pub trait SecretStore: Send + Sync {
    /// Reads `name`; `None` if it does not exist.
    fn get(&self, name: &str) -> Result<Option<Secret>>;
    /// Creates or replaces `name`.
    fn set(&self, name: &str, value: &[u8]) -> Result<()>;
    /// Removes `name`. Removing a missing secret is not an error.
    fn delete(&self, name: &str) -> Result<()>;
    /// Short human description, e.g. "macOS Keychain".
    fn describe(&self) -> String;
}

/// Loads the device key `name`, or creates and stores a new one on first use.
/// Returns the key and whether it was just created.
///
/// A stored value that is not a valid key is an error, not a reason to make a
/// new one: silently changing identity would break every pairing.
pub fn load_or_create_key(store: &dyn SecretStore, name: &str) -> Result<(SecretKey, bool)> {
    if let Some(key) = read_key(store, name)? {
        return Ok((key, false));
    }
    let key = SecretKey::generate();
    write_key(store, name, &key)?;
    Ok((key, true))
}

/// Reads the key `name`; `None` if there is none. A stored value that is not
/// 32 bytes is an error ("damaged"), never `None`.
pub fn read_key(store: &dyn SecretStore, name: &str) -> Result<Option<SecretKey>> {
    let Some(secret) = store.get(name)? else {
        return Ok(None);
    };
    let bytes: &[u8; 32] = secret.as_bytes().try_into().with_context(|| {
        format!(
            "the stored device key `{name}` in {} is damaged",
            store.describe()
        )
    })?;
    Ok(Some(SecretKey::from_bytes(bytes)))
}

/// Like [`read_key`], but "not found" is only believed after it was seen 4
/// times over ~150 ms. Windows Credential Manager can report "not found"
/// for an existing entry while other credentials are being written (seen in
/// stress tests with 8-14 parallel writers). Use this wherever "not found"
/// leads to creating or migrating a key.
pub fn read_key_settled(store: &dyn SecretStore, name: &str) -> Result<Option<SecretKey>> {
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if let Some(key) = read_key(store, name)? {
            return Ok(Some(key));
        }
    }
    Ok(None)
}

/// Stores `key` as `name` and reads it back: an error unless the store now
/// returns exactly this key.
///
/// Windows Credential Manager can report "not found" for an entry just
/// written by `CredWriteW` while other credentials are being written at the
/// same time (~1 in 1600 read-backs with 8 parallel writers). So "not
/// found" is retried for up to ~1 s; a *different* key is an error at once.
pub fn write_key(store: &dyn SecretStore, name: &str, key: &SecretKey) -> Result<()> {
    let bytes = Zeroizing::new(key.to_bytes());
    store.set(name, &bytes[..])?;
    for _ in 0..50 {
        match read_key(store, name)? {
            Some(back) if back.public() == key.public() => return Ok(()),
            Some(_) => break,
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    bail!(
        "saved the device key `{name}` in {}, but reading it back gave something else",
        store.describe()
    )
}

/// The store to use on this machine. `namespace` keeps separate config dirs
/// (e.g. tests, `PIGEON_HOME`) from sharing keys; it is empty for the default
/// config dir.
pub fn default_store(config_dir: &Path, namespace: &str) -> Result<Box<dyn SecretStore>> {
    match std::env::var(KEYSTORE_ENV).as_deref() {
        // W-3: no file fallback on Windows; Credential Manager is always there.
        #[cfg(windows)]
        Ok("file") => bail!(
            "{KEYSTORE_ENV}=file is not supported on Windows: keys stay in Credential Manager"
        ),
        #[cfg(not(windows))]
        Ok("file") => return Ok(Box::new(FileStore::new(config_dir.join("secrets")))),
        Ok("") | Ok("os") | Err(_) => {}
        Ok(other) => bail!("unknown {KEYSTORE_ENV}={other:?} (use `os` or `file`)"),
    }
    Ok(os_store(config_dir, namespace))
}

/// The platform's OS keystore (file store where there is none).
#[allow(unused_variables)]
pub fn os_store(config_dir: &Path, namespace: &str) -> Box<dyn SecretStore> {
    let service = if namespace.is_empty() {
        "Pigeon".to_string()
    } else {
        format!("Pigeon ({namespace})")
    };
    #[cfg(target_os = "macos")]
    return Box::new(mac::KeychainStore { service });
    #[cfg(windows)]
    return Box::new(win::CredentialStore { service });
    #[cfg(not(any(target_os = "macos", windows)))]
    return Box::new(FileStore::new(config_dir.join("secrets")));
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<Secret>> {
        let map = self.0.lock().expect("memory store poisoned");
        Ok(map.get(name).map(|v| Secret::new(v.to_vec())))
    }

    fn set(&self, name: &str, value: &[u8]) -> Result<()> {
        let mut map = self.0.lock().expect("memory store poisoned");
        map.insert(name.to_string(), Zeroizing::new(value.to_vec()));
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<()> {
        self.0.lock().expect("memory store poisoned").remove(name);
        Ok(())
    }

    fn describe(&self) -> String {
        "memory".into()
    }
}

/// One owner-only file per secret. Fallback only (see the module docs).
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "bad secret name"
        );
        Ok(self.dir.join(format!("{name}.secret")))
    }
}

impl SecretStore for FileStore {
    fn get(&self, name: &str) -> Result<Option<Secret>> {
        let path = self.path(name)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(Secret::new(bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
        }
    }

    fn set(&self, name: &str, value: &[u8]) -> Result<()> {
        crate::config::ensure_private_dir(&self.dir)?;
        crate::config::write_private(&self.path(name)?, value)
    }

    fn delete(&self, name: &str) -> Result<()> {
        let path = self.path(name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("could not remove {}", path.display())),
        }
    }

    fn describe(&self) -> String {
        format!("owner-only files in {}", self.dir.display())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use anyhow::{Result, anyhow};
    use security_framework::passwords::{
        PasswordOptions, delete_generic_password, generic_password, set_generic_password,
    };

    use super::{Secret, SecretStore};

    /// `errSecItemNotFound` from `SecBase.h`.
    const ITEM_NOT_FOUND: i32 = -25300;

    /// Generic password items in the user's default (login) keychain. The
    /// item is not marked synchronizable, so it does not go to iCloud.
    /// (`kSecAttrAccessible...ThisDeviceOnly` needs the data-protection
    /// keychain, which an unsigned command-line tool cannot use.)
    pub struct KeychainStore {
        pub service: String,
    }

    impl SecretStore for KeychainStore {
        fn get(&self, name: &str) -> Result<Option<Secret>> {
            match generic_password(PasswordOptions::new_generic_password(&self.service, name)) {
                Ok(bytes) => Ok(Some(Secret::new(bytes))),
                Err(e) if e.code() == ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(anyhow!("could not read `{name}` from the Keychain: {e}")),
            }
        }

        fn set(&self, name: &str, value: &[u8]) -> Result<()> {
            set_generic_password(&self.service, name, value)
                .map_err(|e| anyhow!("could not save `{name}` in the Keychain: {e}"))
        }

        fn delete(&self, name: &str) -> Result<()> {
            match delete_generic_password(&self.service, name) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ITEM_NOT_FOUND => Ok(()),
                Err(e) => Err(anyhow!("could not remove `{name}` from the Keychain: {e}")),
            }
        }

        fn describe(&self) -> String {
            format!("the macOS Keychain (service \"{}\")", self.service)
        }
    }
}

/// Windows Credential Manager through the Win32 API. `unsafe` is unavoidable
/// here: there is no safe std API for it, and each call is a thin FFI wrapper
/// whose pointer rules are documented next to it.
#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::ptr::null_mut;

    use anyhow::{Result, bail};
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_FOUND, FILETIME, GetLastError},
        Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
            CredReadW, CredWriteW,
        },
    };

    use super::{Secret, SecretStore};

    /// Generic credentials named `<service>/<name>`, in the current user's
    /// credential set (protected by DPAPI with the user's logon secret).
    pub struct CredentialStore {
        pub service: String,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    impl CredentialStore {
        fn target(&self, name: &str) -> Vec<u16> {
            wide(&format!("{}/{name}", self.service))
        }
    }

    impl SecretStore for CredentialStore {
        fn get(&self, name: &str) -> Result<Option<Secret>> {
            let target = self.target(name);
            let mut cred: *mut CREDENTIALW = null_mut();
            // SAFETY: `target` is a NUL-terminated UTF-16 string that outlives
            // the call; `cred` is a valid out-pointer.
            let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) };
            if ok == 0 {
                // SAFETY: plain FFI call with no arguments.
                let err = unsafe { GetLastError() };
                if err == ERROR_NOT_FOUND {
                    return Ok(None);
                }
                bail!("could not read `{name}` from Windows Credential Manager (error {err})");
            }
            // SAFETY: on success `cred` points to a CREDENTIALW allocated by
            // the system, whose blob is `CredentialBlobSize` readable bytes
            // (or null when the size is 0). We copy it, wipe the system copy,
            // and free it exactly once with CredFree.
            let bytes = unsafe {
                let c = &*cred;
                let len = c.CredentialBlobSize as usize;
                let bytes = if len == 0 || c.CredentialBlob.is_null() {
                    Vec::new()
                } else {
                    let copy = std::slice::from_raw_parts(c.CredentialBlob, len).to_vec();
                    std::ptr::write_bytes(c.CredentialBlob, 0, len);
                    copy
                };
                CredFree(cred as *const _);
                bytes
            };
            Ok(Some(Secret::new(bytes)))
        }

        fn set(&self, name: &str, value: &[u8]) -> Result<()> {
            let mut target = self.target(name);
            let mut user = wide("pigeon");
            let blob_len = u32::try_from(value.len())?;
            let cred = CREDENTIALW {
                Flags: 0,
                Type: CRED_TYPE_GENERIC,
                TargetName: target.as_mut_ptr(),
                Comment: null_mut(),
                LastWritten: FILETIME {
                    dwLowDateTime: 0,
                    dwHighDateTime: 0,
                },
                CredentialBlobSize: blob_len,
                // CredWriteW only reads the blob; the API type is just not const.
                CredentialBlob: value.as_ptr() as *mut u8,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                AttributeCount: 0,
                Attributes: null_mut(),
                TargetAlias: null_mut(),
                UserName: user.as_mut_ptr(),
            };
            // SAFETY: every pointer in `cred` points into a live local buffer
            // (`target`, `user`, `value`) for the duration of the call, and
            // CredWriteW copies what it needs.
            let ok = unsafe { CredWriteW(&cred, 0) };
            if ok == 0 {
                // SAFETY: plain FFI call with no arguments.
                let err = unsafe { GetLastError() };
                bail!("could not save `{name}` in Windows Credential Manager (error {err})");
            }
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<()> {
            let target = self.target(name);
            // SAFETY: `target` is a NUL-terminated UTF-16 string that outlives the call.
            let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
            if ok == 0 {
                // SAFETY: plain FFI call with no arguments.
                let err = unsafe { GetLastError() };
                if err != ERROR_NOT_FOUND {
                    bail!(
                        "could not remove `{name}` from Windows Credential Manager (error {err})"
                    );
                }
            }
            Ok(())
        }

        fn describe(&self) -> String {
            format!("Windows Credential Manager (\"{}/...\")", self.service)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DEVICE_KEY;

    fn roundtrip(store: &dyn SecretStore) {
        let name = "roundtrip-test";
        store.delete(name).unwrap();
        assert!(store.get(name).unwrap().is_none());
        store.set(name, b"first").unwrap();
        assert_eq!(store.get(name).unwrap().unwrap().as_bytes(), b"first");
        store.set(name, b"second value").unwrap();
        assert_eq!(
            store.get(name).unwrap().unwrap().as_bytes(),
            b"second value"
        );
        store.delete(name).unwrap();
        assert!(store.get(name).unwrap().is_none());
        store.delete(name).unwrap();
    }

    #[test]
    fn memory_store_roundtrip() {
        roundtrip(&MemoryStore::default());
    }

    #[test]
    fn file_store_roundtrip_and_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path().join("secrets"));
        roundtrip(&store);
        store.set("k", b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir.path().join("secrets/k.secret")), 0o600);
            assert_eq!(mode(&dir.path().join("secrets")), 0o700);
        }
        assert!(store.get("../escape").is_err(), "names must not be paths");
    }

    #[test]
    fn device_key_is_stable_across_restarts() {
        let store = MemoryStore::default();
        let (first, created) = load_or_create_key(&store, DEVICE_KEY).unwrap();
        assert!(created);
        let (again, created) = load_or_create_key(&store, DEVICE_KEY).unwrap();
        assert!(!created);
        assert_eq!(first.public(), again.public());
    }

    #[test]
    fn damaged_key_is_an_error_not_a_new_identity() {
        let store = MemoryStore::default();
        store.set(DEVICE_KEY, b"short").unwrap();
        let err = load_or_create_key(&store, DEVICE_KEY).unwrap_err();
        assert!(format!("{err:#}").contains("damaged"), "{err:#}");
        assert_eq!(store.get(DEVICE_KEY).unwrap().unwrap().as_bytes(), b"short");
    }

    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new(vec![0xAB; 32]);
        let dbg = format!("{s:?} {s:#?}");
        assert!(dbg.contains("<redacted>"), "{dbg}");
        assert!(
            !dbg.contains("171") && !dbg.to_lowercase().contains("ab"),
            "{dbg}"
        );
    }

    /// P-T9 on the real OS keystore (Keychain on macOS CI, Credential
    /// Manager on Windows CI), in a namespace of its own: the key round-trips
    /// and no key bytes end up in any file under the config dir.
    /// Linux (and other OSes without a supported keystore): the device key
    /// is an owner-only file. Proves, through the same path the binaries
    /// use (`Home::identity`), that the key file is `0600`, its folder and
    /// the config dir are `0700`, the key is stable, and nothing else in the
    /// config dir holds it. Runs on ubuntu-latest CI.
    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    fn linux_keystore_is_owner_only_files() {
        use crate::identity::Origin;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = crate::config::Home::at(dir.path().join("pigeon")).unwrap();
        let (me, store) = home.identity().unwrap();
        assert_eq!(me.origin, Origin::Created);
        assert!(
            store.describe().contains("owner-only files"),
            "{}",
            store.describe()
        );
        let (again, _) = home.identity().unwrap();
        assert_eq!(again.origin, Origin::Existing);
        let key = me.key;
        assert_eq!(key.public(), again.key.public());

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let secrets = home.dir.join("secrets");
        assert_eq!(mode(&home.dir), 0o700, "config dir");
        assert_eq!(mode(&secrets), 0o700, "secrets dir");
        assert_eq!(mode(&secrets.join("device-key.secret")), 0o600, "key file");
        // The key is in its own file only (raw bytes, as stored).
        let raw = key.to_bytes();
        let files: Vec<_> = std::fs::read_dir(&secrets).unwrap().collect();
        assert_eq!(files.len(), 1, "unexpected files in {}", secrets.display());
        assert_eq!(
            std::fs::read(secrets.join("device-key.secret")).unwrap(),
            raw.to_vec()
        );
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn os_keystore_roundtrip_leaves_no_key_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut tag = [0u8; 6];
        getrandom::fill(&mut tag).unwrap();
        let namespace = format!("test-{}", crate::hex(&tag));
        let store = os_store(dir.path(), &namespace);
        roundtrip(store.as_ref());

        let (key, created) = load_or_create_key(store.as_ref(), DEVICE_KEY).unwrap();
        assert!(created);
        let (again, _) = load_or_create_key(store.as_ref(), DEVICE_KEY).unwrap();
        assert_eq!(key.public(), again.public());

        let raw = key.to_bytes();
        let forms = [
            raw.to_vec(),
            crate::hex(&raw).into_bytes(),
            data_encoding::BASE32_NOPAD.encode(&raw).into_bytes(),
            data_encoding::BASE32_NOPAD
                .encode(&raw)
                .to_ascii_lowercase()
                .into_bytes(),
        ];
        crate::config::assert_no_file_contains(dir.path(), &forms);
        store.delete(DEVICE_KEY).unwrap();
        assert!(store.get(DEVICE_KEY).unwrap().is_none());
    }

    /// One identity through the real OS keystore (Keychain on macOS CI,
    /// Credential Manager on Windows CI): a two-key install (old `agent-key`
    /// and `client-key`) migrates to one `device-key` equal to the old agent
    /// key, the old client key stays as the fallback for the remembered
    /// agent, a second load is a no-op, and no key bytes land in any file.
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn os_keystore_migration_roundtrip() {
        use crate::identity::{self, LEGACY_AGENT_KEY, LEGACY_CLIENT_KEY, Origin};
        let dir = tempfile::tempdir().unwrap();
        let mut tag = [0u8; 6];
        getrandom::fill(&mut tag).unwrap();
        let store = os_store(dir.path(), &format!("test-{}", crate::hex(&tag)));
        let store = store.as_ref();
        let agent = SecretKey::generate();
        let client = SecretKey::generate();
        write_key(store, LEGACY_AGENT_KEY, &agent).unwrap();
        write_key(store, LEGACY_CLIENT_KEY, &client).unwrap();
        identity::tests_support::remember_agent(dir.path(), 9);

        let me = identity::load(dir.path(), store).unwrap();
        assert_eq!(me.origin, Origin::FromAgentKey);
        assert_eq!(me.key.public(), agent.public());
        assert_eq!(me.legacy_client, Some(client.public()));
        assert!(read_key(store, LEGACY_AGENT_KEY).unwrap().is_none());
        let again = identity::load(dir.path(), store).unwrap();
        assert_eq!(again.origin, Origin::Existing);
        assert_eq!(again.key.public(), agent.public());
        assert_eq!(
            read_key(store, LEGACY_CLIENT_KEY)
                .unwrap()
                .unwrap()
                .public(),
            client.public()
        );

        let mut forms = Vec::new();
        for k in [&agent, &client] {
            let raw = k.to_bytes();
            forms.push(raw.to_vec());
            forms.push(crate::hex(&raw).into_bytes());
        }
        crate::config::assert_no_file_contains(dir.path(), &forms);
        for name in [DEVICE_KEY, LEGACY_AGENT_KEY, LEGACY_CLIENT_KEY] {
            store.delete(name).unwrap();
        }
    }
}
