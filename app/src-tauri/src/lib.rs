//! The app's backend: both roles of one computer in one process, on **one**
//! iroh endpoint with the computer's one device key (PAIRING.md D9).
//!
//! - **Share**: an [`Agent`] serving the folders picked in the app (named
//!   folders, read-only) to paired devices, plus pairing while an invite is
//!   active.
//! - **Get**: `ls` / `get` / `pair` dial out from the *same* endpoint, so the
//!   relay never sees this computer's ID twice (closes R14 / TH-28 for the
//!   app: there is no second connection to displace the share).
//!
//! Nothing here depends on Tauri: `main.rs` maps IPC commands onto
//! [`Backend`] methods, and `tests/two_backends.rs` runs two backends in one
//! process. Every value from the webview is checked here again (length,
//! format) before it is used; the webview never sends a local path.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use iroh::{Endpoint, EndpointAddr, EndpointId, Watcher, endpoint::presets};
use pigeon_core::{
    ALPN,
    agent::{self, Agent, AgentStorage, Offer, ServeOptions, SharedFolder},
    client,
    config::{self, Home, display_path},
    devices::{AGENTS_FILE, AgentList, DEVICES_FILE, Device, DeviceList, FileLock, PairedAgent},
    fingerprint,
    identity::{self, Identity},
    pair::{self, Invite, InviteLink, PairingCode},
};
use serde::{Deserialize, Serialize};

/// The app's list of shared folders (not secret: names and local paths).
pub const SHARES_FILE: &str = "app-shares.json";
/// Same file and format as `pigeon-agent` (W-5), so an invite spent in one
/// is never accepted by the other.
const SPENT_FILE: &str = "spent-invites.json";
const SPENT_KEEP: usize = 256;

/// Longest text the webview may send for each kind of input.
pub const MAX_PATH_INPUT: usize = 4096;
pub const MAX_LINK_INPUT: usize = 4200;
pub const MAX_CODE_INPUT: usize = 200;
pub const MAX_NAME_INPUT: usize = identity::MAX_NAME * 4;

/// How long a listing may take before the computer counts as offline.
const LIST_TIMEOUT: Duration = Duration::from_secs(20);

/// Shown when something else already shares for this computer (another
/// Pigeon app with the same settings, or `pigeon-agent share`). A second
/// launch of the app itself never gets here: it focuses the first window
/// (single instance, `main.rs`).
pub const ALREADY_SHARING: &str = "Pigeon is already running on this computer: either the \
     Pigeon app, or `pigeon-agent share` in a terminal. Only one of them can share at a time. \
     Quit the other one, then open Pigeon again.";

/// How the backend starts.
#[derive(Debug, Clone)]
pub struct Options {
    pub home: Home,
    /// Where fetched files go (`Downloads/Pigeon` by default). Created
    /// owner-only on first use (SEC-14).
    pub download_dir: PathBuf,
    /// Tests: loopback only, no relay, no address lookup; invite links
    /// carry the loopback address.
    pub local_only: bool,
}

/// One folder in [`SHARES_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FolderEntry {
    name: String,
    path: PathBuf,
}

/// The invite made in this app run. The code stays in this process (shown
/// on this computer's screen only) and is dropped when the invite ends.
struct ActiveInvite {
    invite_id: [u8; 16],
    link: String,
    code: Option<PairingCode>,
    expires_at: u64,
    attempts_left: u8,
    device_name: Option<String>,
    paired: Option<Device>,
    cancelled: bool,
}

#[derive(Default)]
struct InviteState {
    current: Option<ActiveInvite>,
}

/// Persistence for the agent half: allow-list file, spent invites file, and
/// the live invite state the UI polls. The invite itself is kept in memory
/// only (the app is the one process that serves it; a restart ends it).
struct AppStorage {
    dir: PathBuf,
    invite: Arc<Mutex<InviteState>>,
}

impl AppStorage {
    fn spent_path(&self) -> PathBuf {
        self.dir.join(SPENT_FILE)
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

impl AgentStorage for AppStorage {
    fn device_paired(&self, device: &Device) -> Result<()> {
        DeviceList::update(&self.dir.join(DEVICES_FILE), |list| {
            list.upsert(device.clone());
            Ok(())
        })?;
        let mut state = self.invite.lock().expect("invite state poisoned");
        if let Some(inv) = state.current.as_mut() {
            inv.paired = Some(device.clone());
            inv.code = None;
        }
        Ok(())
    }

    fn invite_changed(&self, invite_id: &[u8; 16], invite: Option<&Invite>) -> Result<()> {
        let mut state = self.invite.lock().expect("invite state poisoned");
        if let Some(inv) = state.current.as_mut()
            && inv.invite_id == *invite_id
        {
            match invite {
                Some(i) => inv.attempts_left = i.attempts_left,
                // Ended (paired, burned, expired or cancelled): forget the code.
                None => inv.code = None,
            }
        }
        Ok(())
    }

    fn invite_spent(&self, invite_id: &[u8; 16]) -> Result<()> {
        let id = pigeon_core::hex(invite_id);
        let mut spent = self.load_spent()?;
        if !spent.contains(&id) {
            spent.push_back(id);
            while spent.len() > SPENT_KEEP {
                spent.pop_front();
            }
            config::write_private(&self.spent_path(), &serde_json::to_vec(&spent)?)?;
        }
        Ok(())
    }

    fn is_spent(&self, invite_id: &[u8; 16]) -> bool {
        // Fail closed, like the agent binary.
        self.load_spent()
            .map(|s| s.contains(&pigeon_core::hex(invite_id)))
            .unwrap_or(true)
    }
}

/// State of one download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    Active,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferView {
    pub id: u64,
    pub computer: String,
    pub computer_name: String,
    pub remote_path: String,
    /// Local file name (what is in the download folder).
    pub file_name: String,
    /// The name it would have had if nothing was there yet.
    pub wanted_name: String,
    pub state: TransferState,
    pub size: u64,
    pub done: u64,
    pub resumed_from: u64,
    pub mb_per_sec: f64,
    pub route: Option<String>,
    /// SHA-256 of the local copy (always computed after a get).
    pub sha256: Option<String>,
    /// Whether that hash was checked against the one the other computer
    /// sent (W-19): `Some(true)` checked and equal, `Some(false)` nothing
    /// to check against, `None` not known. The UI says "SHA-256 verified"
    /// only for `Some(true)`.
    pub verified: Option<bool>,
    pub error: Option<String>,
    pub started_at: u64,
    pub finished_at: Option<u64>,
}

struct Transfer {
    view: TransferView,
    computer: EndpointId,
    /// The remote path exactly as the agent listed it.
    path: String,
    out: PathBuf,
    started: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub struct Me {
    pub name: String,
    pub fingerprint: String,
    pub id: String,
    pub os: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderView {
    pub name: String,
    pub path: String,
    pub available: bool,
}

/// A device that may get files from this computer.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub paired_at: u64,
}

/// A computer this one can get files from.
#[derive(Debug, Clone, Serialize)]
pub struct ComputerView {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub paired_at: u64,
    /// `None` until the app tried to reach it.
    pub online: Option<bool>,
    /// Paired with an old per-role key that this app cannot use.
    pub needs_new_pairing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum InviteView {
    /// Waiting for the other device. The code is shown on this screen only.
    Waiting {
        link: String,
        code: Vec<String>,
        expires_at: u64,
        attempts_left: u8,
        device_name: Option<String>,
    },
    Paired {
        device: DeviceView,
    },
    Burned,
    Expired,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub me: Me,
    /// Connected to a home relay (reachable from other networks).
    pub online: bool,
    pub folders: Vec<FolderView>,
    pub devices: Vec<DeviceView>,
    pub computers: Vec<ComputerView>,
    pub invite: Option<InviteView>,
    pub transfers: Vec<TransferView>,
    pub download_dir: String,
}

/// What the get side learns from a pasted link before asking for the code.
#[derive(Debug, Clone, Serialize)]
pub struct LinkView {
    pub computer: String,
    pub fingerprint: String,
    pub expires_at: u64,
    pub expired: bool,
    pub already_paired: bool,
}

/// Result of typing the code.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PairOutcome {
    Paired {
        computer: ComputerView,
        my_fingerprint: String,
    },
    /// A word is not a code word: checked here, costs no try.
    Typo {
        message: String,
    },
    WrongCode {
        attempts_left: u8,
    },
    UsedUp,
    Expired,
    /// Not expired, but no active invite there: it paired a device already
    /// (or was cancelled or replaced, or Pigeon is not running there).
    AlreadyUsed,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryView {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

pub struct Backend {
    home: Home,
    me_id: EndpointId,
    me_name: Mutex<String>,
    endpoint: Endpoint,
    agent: Agent,
    local_only: bool,
    download_dir: PathBuf,
    folders: Mutex<Vec<FolderEntry>>,
    invite: Arc<Mutex<InviteState>>,
    transfers: Mutex<Vec<Transfer>>,
    next_transfer: Mutex<u64>,
    online: Mutex<HashMap<EndpointId, bool>>,
    /// Loopback addresses from invite links (tests only, `local_only`).
    local_addrs: Mutex<HashMap<EndpointId, EndpointAddr>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// The runtime the backend was started on. Transfers are spawned here,
    /// so they can be started from any thread (Tauri runs sync commands on
    /// the main thread, outside any runtime).
    rt: tokio::runtime::Handle,
    _share_lock: FileLock,
}

/// Parses an endpoint ID sent by the webview.
pub fn parse_id(s: &str) -> Result<EndpointId> {
    anyhow::ensure!(s.len() <= 128, "not a device ID");
    EndpointId::from_str(s.trim()).map_err(|_| anyhow::anyhow!("not a device ID"))
}

fn check_len(what: &str, s: &str, max: usize) -> Result<()> {
    anyhow::ensure!(s.len() <= max, "{what} is too long");
    anyhow::ensure!(!s.contains('\0'), "{what} contains a NUL byte");
    Ok(())
}

/// A display name for a folder: its last component, made to pass
/// [`agent::check_folder_name`].
fn folder_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            // A drive or volume root: `C:\` -> "Drive C".
            let s = display_path(path);
            let letter: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            if letter.is_empty() {
                "Shared".into()
            } else {
                format!("Drive {letter}")
            }
        });
    let mut name: String = raw
        .chars()
        .map(|c| if "/\\:\0".contains(c) { '_' } else { c })
        .filter(|c| {
            let s = c.to_string();
            pigeon_core::display_safe(&s) == s
        })
        .take(agent::MAX_FOLDER_NAME)
        .collect();
    name = name.trim().to_string();
    if name.is_empty() || name == "." || name == ".." {
        name = "Shared".into();
    }
    name
}

fn unique_name(base: &str, taken: &[FolderEntry]) -> String {
    let used: HashSet<String> = taken.iter().map(|f| f.name.to_lowercase()).collect();
    if !used.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base} ({n})"))
        .find(|n| !used.contains(&n.to_lowercase()))
        .expect("ran out of names")
}

fn device_view(d: &Device) -> DeviceView {
    DeviceView {
        id: d.id.to_string(),
        name: pigeon_core::display_safe(&d.name),
        fingerprint: fingerprint(&d.id),
        paired_at: d.paired_at,
    }
}

/// Maps a core pairing error onto what the UI shows. `link_expired`: the
/// pasted link's own expiry time has passed.
fn classify_pair_error(err: &anyhow::Error, link_expired: bool) -> PairOutcome {
    let msg = format!("{err:#}");
    if let Some(rest) = msg.split("attempts left: ").nth(1) {
        let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = n.parse() {
            return PairOutcome::WrongCode { attempts_left: n };
        }
    }
    if msg.contains("used up") {
        return PairOutcome::UsedUp;
    }
    // Checked before "expired": the core's texts for these say "it expired,
    // was used, or was replaced" (no active invite there, or no pairing at
    // all once the invite ended). The link knows whether it expired, so
    // anything else means it was used, cancelled or replaced (N2).
    if msg.contains("no active invite") || msg.contains("could not pair") {
        return if link_expired {
            PairOutcome::Expired
        } else {
            PairOutcome::AlreadyUsed
        };
    }
    if msg.contains("expired") {
        return PairOutcome::Expired;
    }
    PairOutcome::Failed {
        message: friendly_error(&msg, None),
    }
}

/// Turns a core error into what a person reads in the app: no CLI commands,
/// no OS error text, no `\\?\` paths, no `agent:` prefix (N3). `computer` is
/// the other computer's name when there is one.
pub fn friendly_error(msg: &str, computer: Option<&str>) -> String {
    let them = computer.unwrap_or("That computer");
    if msg.contains("not paired (or was revoked)") {
        return format!(
            "{them} no longer lets this computer get files. To get files again, make a new \
             invite on {them} (Pair a device) and pair again."
        );
    }
    if msg.contains("is busy (too many connections)") {
        return format!("{them} is busy right now. Try again in a moment.");
    }
    if msg.contains("could not connect to the agent") {
        return format!("Couldn't connect to {them}.");
    }
    let mut msg = msg.trim();
    msg = msg.strip_prefix("agent: ").unwrap_or(msg);
    let mut out = msg.to_string();
    // "not found: a/b: The system cannot find the file specified. (os error 2)"
    if let Some(at) = out.find(" (os error ") {
        out.truncate(at);
        if let Some(colon) = out.rfind(": ") {
            out.truncate(colon);
        }
    }
    if let Some(path) = out.strip_prefix("not found: ") {
        out = format!("Not found: {path}. It may have been moved, renamed or unshared.");
    } else if out == "the shared folder is not available" {
        out = format!(
            "{them} can't open that shared folder right now (it may be on a disk that is not connected)."
        );
    }
    let out = out.replace(r"\\?\", "");
    let mut chars = out.chars();
    let out = match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => out,
    };
    pigeon_core::display_safe(&out)
}

fn load_folders(dir: &Path) -> Result<Vec<FolderEntry>> {
    match std::fs::read(dir.join(SHARES_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{SHARES_FILE} is damaged; fix or delete it")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).context(format!("could not read {SHARES_FILE}")),
    }
}

/// Whether the core checked the local copy's SHA-256 against the one the
/// other computer sent (W-19).
///
/// TODO(core-file-safety): today `client::GetStats` does not say. The core
/// skips the check when the agent sends no hash (R7), and `GetStats::sha256`
/// is the local hash either way, so the app cannot tell a checked copy from
/// an unchecked one: report "not known" and never claim "verified". Use the
/// flag from Anvil's `core-file-safety` change once it lands.
fn sha256_verified(_stats: &client::GetStats) -> Option<bool> {
    None
}

fn shared(folders: &[FolderEntry]) -> Vec<SharedFolder> {
    folders
        .iter()
        .map(|f| SharedFolder {
            name: f.name.clone(),
            path: f.path.clone(),
        })
        .collect()
}

impl Backend {
    /// Opens this computer's identity, binds the one endpoint and starts
    /// serving. Fails if `pigeon-agent share` already runs for this config
    /// dir (two programs with one key would fight over the relay slot).
    pub async fn start(opts: Options) -> Result<Arc<Self>> {
        let Options {
            home,
            download_dir,
            local_only,
        } = opts;
        let share_lock = match agent::hold_share_lock(&home.dir) {
            Ok(lock) => lock,
            // Busy: say who, without the lock file's path (M2).
            Err(_) if agent::share_running(&home.dir) => bail!(ALREADY_SHARING),
            Err(e) => {
                return Err(e).context(format!(
                    "Pigeon could not open its settings folder {}",
                    display_path(&home.dir)
                ));
            }
        };
        let (me, _store): (Identity, _) = home.identity()?;
        let key = me.key.clone();
        let builder = if local_only {
            Endpoint::builder(presets::Minimal)
                .secret_key(key)
                .clear_ip_transports()
                .bind_addr("127.0.0.1:0")?
        } else {
            // Publishing (n0 preset): this endpoint is also the share, and
            // the only one with this key on this computer.
            Endpoint::builder(presets::N0).secret_key(key)
        };
        let endpoint = builder.alpns(vec![ALPN.to_vec()]).bind().await?;

        let folders = load_folders(&home.dir)?;
        let devices_path = home.dir.join(DEVICES_FILE);
        let devices = DeviceList::load(&devices_path)?;
        let invite = Arc::new(Mutex::new(InviteState::default()));
        let storage = AppStorage {
            dir: home.dir.clone(),
            invite: invite.clone(),
        };
        let agent = Agent::with_folders(
            endpoint.clone(),
            shared(&folders),
            devices,
            ServeOptions::default(),
            Box::new(storage),
        )?;

        let serving = agent.clone();
        let serve = tokio::spawn(async move {
            if let Err(e) = serving.serve().await {
                eprintln!("share stopped: {e:#}");
            }
        });
        // Picks up `pigeon-agent revoke` edits and expires the invite. The
        // app makes invites in-process, so there is nothing to load.
        let watch = tokio::spawn(agent::watch(agent.clone(), devices_path, || Ok(None)));

        Ok(Arc::new(Self {
            me_id: me.id(),
            me_name: Mutex::new(me.name.clone()),
            home,
            endpoint,
            agent,
            local_only,
            download_dir,
            folders: Mutex::new(folders),
            invite,
            transfers: Mutex::new(Vec::new()),
            next_transfer: Mutex::new(1),
            online: Mutex::new(HashMap::new()),
            local_addrs: Mutex::new(HashMap::new()),
            tasks: Mutex::new(vec![serve, watch]),
            rt: tokio::runtime::Handle::current(),
            _share_lock: share_lock,
        }))
    }

    /// Stops serving and closes the endpoint.
    pub async fn shutdown(&self) {
        for t in self.tasks.lock().expect("tasks poisoned").drain(..) {
            t.abort();
        }
        self.endpoint.close().await;
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn id(&self) -> EndpointId {
        self.me_id
    }

    /// Connected to a home relay.
    pub fn online(&self) -> bool {
        if self.local_only {
            return true;
        }
        self.endpoint
            .home_relay_status()
            .get()
            .iter()
            .any(|s| s.is_connected())
    }

    fn me(&self) -> Me {
        Me {
            name: pigeon_core::display_safe(&self.me_name.lock().expect("name poisoned")),
            fingerprint: fingerprint(&self.me_id),
            id: self.me_id.to_string(),
            os: std::env::consts::OS.into(),
        }
    }

    // ---------------------------------------------------------------- status

    pub fn status(&self) -> Result<Status> {
        let folders = self
            .folders
            .lock()
            .expect("folders poisoned")
            .iter()
            .map(|f| FolderView {
                name: f.name.clone(),
                path: display_path(&f.path),
                available: f.path.is_dir(),
            })
            .collect();
        let devices = self
            .agent
            .devices()
            .devices
            .iter()
            .map(device_view)
            .collect();
        let transfers = self
            .transfers
            .lock()
            .expect("transfers poisoned")
            .iter()
            .rev()
            .map(|t| t.view.clone())
            .collect();
        Ok(Status {
            me: self.me(),
            online: self.online(),
            folders,
            devices,
            computers: self.computers()?,
            invite: self.invite_view(),
            transfers,
            download_dir: display_path(&self.download_dir),
        })
    }

    fn computers(&self) -> Result<Vec<ComputerView>> {
        let online = self.online.lock().expect("online poisoned").clone();
        Ok(AgentList::load(&self.home.dir.join(AGENTS_FILE))?
            .agents
            .iter()
            .map(|a| ComputerView {
                id: a.id.to_string(),
                name: pigeon_core::display_safe(&a.name),
                fingerprint: fingerprint(&a.id),
                paired_at: a.paired_at,
                online: online.get(&a.id).copied(),
                needs_new_pairing: a.legacy_key.is_some_and(|k| k != self.me_id),
            })
            .collect())
    }

    // ----------------------------------------------------------------- share

    /// Shares `path` (read-only) under its folder name. The path comes from
    /// the native folder picker, never from the webview.
    pub fn add_folder(&self, path: &Path) -> Result<FolderView> {
        let path = path
            .canonicalize()
            .with_context(|| format!("folder not found: {}", path.display()))?;
        anyhow::ensure!(path.is_dir(), "not a folder: {}", display_path(&path));
        let config = self.home.dir.canonicalize()?;
        // Sharing these would hand out the allow-lists (N1: say which).
        anyhow::ensure!(
            !path.starts_with(&config),
            "This folder is part of Pigeon's own settings, so it can't be shared. \
             Choose another folder."
        );
        anyhow::ensure!(
            !config.starts_with(&path),
            "This folder contains Pigeon's own settings ({}), so it can't be shared. \
             Choose a subfolder instead.",
            display_path(&config)
        );
        let mut folders = self.folders.lock().expect("folders poisoned");
        anyhow::ensure!(
            !folders.iter().any(|f| f.path == path),
            "this folder is already shared"
        );
        let mut next = folders.clone();
        let name = unique_name(&folder_name(&path), &next);
        next.push(FolderEntry {
            name: name.clone(),
            path: path.clone(),
        });
        self.agent.set_folders(shared(&next))?;
        self.save_folders(&next)?;
        *folders = next;
        Ok(FolderView {
            name,
            path: display_path(&path),
            available: true,
        })
    }

    /// Stops sharing the folder called `name`. Takes effect for the next
    /// request.
    pub fn remove_folder(&self, name: &str) -> Result<()> {
        check_len("folder name", name, MAX_NAME_INPUT)?;
        let mut folders = self.folders.lock().expect("folders poisoned");
        let next: Vec<FolderEntry> = folders.iter().filter(|f| f.name != name).cloned().collect();
        anyhow::ensure!(
            next.len() < folders.len(),
            "no shared folder with that name"
        );
        self.agent.set_folders(shared(&next))?;
        self.save_folders(&next)?;
        *folders = next;
        Ok(())
    }

    fn save_folders(&self, folders: &[FolderEntry]) -> Result<()> {
        let mut json = serde_json::to_vec_pretty(folders)?;
        json.push(b'\n');
        config::write_private(&self.home.dir.join(SHARES_FILE), &json)
    }

    /// Makes a new single-use invite (replacing any other one).
    pub fn create_invite(&self, device_name: Option<&str>) -> Result<InviteView> {
        let device_name = match device_name {
            Some(n) => {
                check_len("device name", n, MAX_NAME_INPUT)?;
                identity::clean_name(n)
            }
            None => None,
        };
        let (invite, code) = Invite::new(pair::DEFAULT_TTL, pair::now_secs(), device_name.clone())?;
        let invite = invite.for_agent(self.me_id);
        let addr = if self.local_only {
            self.local_addr()
        } else {
            // Like `pigeon-agent pair`: the ID plus a relay hint, never IPs.
            let addr = EndpointAddr::new(self.me_id);
            match self.endpoint.addr().relay_urls().next() {
                Some(url) => addr.with_relay_url(url.clone()),
                None => addr,
            }
        };
        let link = invite.link(addr).to_string();
        let active = ActiveInvite {
            invite_id: invite.invite_id,
            link,
            code: Some(code),
            expires_at: invite.expires_at,
            attempts_left: invite.attempts_left,
            device_name,
            paired: None,
            cancelled: false,
        };
        // Swap the UI state first: a pairing callback for the old invite
        // must not land on the new one (the storage checks the ID too).
        self.invite.lock().expect("invite state poisoned").current = Some(active);
        match self.agent.offer_invite(invite) {
            Offer::Activated | Offer::AlreadyActive => {}
            other => bail!("could not activate the invite ({other:?})"),
        }
        self.invite_view().context("invite vanished")
    }

    pub fn cancel_invite(&self) {
        self.agent.cancel_invite();
        if let Some(inv) = self
            .invite
            .lock()
            .expect("invite state poisoned")
            .current
            .as_mut()
        {
            inv.cancelled = true;
            inv.code = None;
        }
    }

    /// Forgets a finished invite's outcome (after the UI showed it).
    pub fn dismiss_invite(&self) {
        let mut state = self.invite.lock().expect("invite state poisoned");
        if state
            .current
            .as_ref()
            .is_some_and(|i| i.code.is_none() || i.cancelled)
        {
            state.current = None;
        }
    }

    fn invite_view(&self) -> Option<InviteView> {
        let active = self.agent.invite_active();
        let state = self.invite.lock().expect("invite state poisoned");
        let inv = state.current.as_ref()?;
        if inv.cancelled {
            return None;
        }
        if let Some(d) = &inv.paired {
            return Some(InviteView::Paired {
                device: device_view(d),
            });
        }
        let now = pair::now_secs();
        match &inv.code {
            Some(code) if active && now < inv.expires_at => Some(InviteView::Waiting {
                link: inv.link.clone(),
                code: code.reveal().split('-').map(str::to_string).collect(),
                expires_at: inv.expires_at,
                attempts_left: inv.attempts_left,
                device_name: inv.device_name.clone(),
            }),
            _ if now >= inv.expires_at => Some(InviteView::Expired),
            _ if inv.attempts_left == 0 => Some(InviteView::Burned),
            // Ended with tries left and not expired: a pairing is completing
            // (the allow-list write follows the invite's end by a moment).
            _ => Some(InviteView::Waiting {
                link: inv.link.clone(),
                code: Vec::new(),
                expires_at: inv.expires_at,
                attempts_left: inv.attempts_left,
                device_name: inv.device_name.clone(),
            }),
        }
    }

    /// Stops `id` getting files from this computer: off the allow-list file
    /// and its live connections closed now.
    pub fn revoke_device(&self, id: &str) -> Result<()> {
        let id = parse_id(id)?;
        DeviceList::update(&self.home.dir.join(DEVICES_FILE), |list| {
            list.devices.retain(|d| d.id != id);
            Ok(())
        })?;
        self.agent.revoke(&id);
        Ok(())
    }

    fn local_addr(&self) -> EndpointAddr {
        self.endpoint
            .bound_sockets()
            .into_iter()
            .fold(EndpointAddr::new(self.me_id), |a, s| a.with_ip_addr(s))
    }

    // ------------------------------------------------------------------- get

    pub fn check_link(&self, link: &str) -> Result<LinkView> {
        check_len("link", link, MAX_LINK_INPUT)?;
        let link = InviteLink::from_str(link)?;
        let already = AgentList::load(&self.home.dir.join(AGENTS_FILE))?
            .agents
            .iter()
            .any(|a| a.id == link.addr.id);
        Ok(LinkView {
            computer: link.addr.id.to_string(),
            fingerprint: fingerprint(&link.addr.id),
            expires_at: link.expires_at,
            expired: pair::now_secs() >= link.expires_at,
            already_paired: already,
        })
    }

    /// Pairs with the computer in `link` using the typed `code` (which goes
    /// nowhere but `pair::pair`). `name` is what to call that computer here.
    pub async fn pair(&self, link: &str, code: &str, name: Option<&str>) -> Result<PairOutcome> {
        check_len("link", link, MAX_LINK_INPUT)?;
        check_len("code", code, MAX_CODE_INPUT)?;
        let link = InviteLink::from_str(link)?;
        anyhow::ensure!(
            link.addr.id != self.me_id,
            "this is this computer's own invite; paste it on the other computer"
        );
        let code = match PairingCode::parse(code) {
            Ok(c) => c,
            Err(e) => {
                return Ok(PairOutcome::Typo {
                    message: format!("{e:#}"),
                });
            }
        };
        let name = match name {
            Some(n) => {
                check_len("name", n, MAX_NAME_INPUT)?;
                identity::clean_name(n)
            }
            None => None,
        };
        let paired = match pair::pair(&self.endpoint, &link, &code).await {
            Ok(p) => p,
            Err(e) => {
                let expired = pair::now_secs() >= link.expires_at;
                return Ok(classify_pair_error(&e, expired));
            }
        };
        let fp = fingerprint(&paired.agent);
        let agent = PairedAgent {
            id: paired.agent,
            name: name.unwrap_or_else(|| format!("Computer {}", fp.to_uppercase())),
            relay_url: pigeon_core::devices::trusted_relay_hint(link.addr.relay_urls().next()),
            paired_at: pair::now_secs(),
            legacy_key: None,
        };
        AgentList::update(&self.home.dir.join(AGENTS_FILE), |list| {
            list.upsert(agent.clone());
            Ok(())
        })?;
        if self.local_only {
            self.local_addrs
                .lock()
                .expect("addrs poisoned")
                .insert(agent.id, link.addr.clone());
        }
        self.online
            .lock()
            .expect("online poisoned")
            .insert(agent.id, true);
        Ok(PairOutcome::Paired {
            computer: ComputerView {
                id: agent.id.to_string(),
                name: pigeon_core::display_safe(&agent.name),
                fingerprint: fp,
                paired_at: agent.paired_at,
                online: Some(true),
                needs_new_pairing: false,
            },
            my_fingerprint: fingerprint(&self.me_id),
        })
    }

    /// Stops getting files from `id` here (the other side's allow-list is
    /// theirs to change).
    pub fn forget_computer(&self, id: &str) -> Result<()> {
        let id = parse_id(id)?;
        AgentList::update(&self.home.dir.join(AGENTS_FILE), |list| {
            list.agents.retain(|a| a.id != id);
            Ok(())
        })?;
        self.online.lock().expect("online poisoned").remove(&id);
        Ok(())
    }

    fn computer(&self, id: &str) -> Result<(PairedAgent, EndpointAddr)> {
        let id = parse_id(id)?;
        let agent = AgentList::load(&self.home.dir.join(AGENTS_FILE))?
            .agents
            .into_iter()
            .find(|a| a.id == id)
            .context("not paired with that computer")?;
        // One endpoint, one key: an agent that only knows this computer's
        // old per-role key cannot be reached from here.
        anyhow::ensure!(
            agent.legacy_key.is_none_or(|k| k == self.me_id),
            "{} was paired with this computer's old key; pair again",
            pigeon_core::display_safe(&agent.name)
        );
        let addr = match self.local_addrs.lock().expect("addrs poisoned").get(&id) {
            Some(a) => a.clone(),
            None => agent.dial_addr(),
        };
        Ok((agent, addr))
    }

    /// Lists `path` on a paired computer ("" = its shared folders).
    pub async fn list(&self, computer: &str, path: &str) -> Result<Vec<EntryView>> {
        check_len("path", path, MAX_PATH_INPUT)?;
        let (agent, addr) = self.computer(computer)?;
        let result = tokio::time::timeout(LIST_TIMEOUT, client::list(&self.endpoint, &addr, path))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "no answer in time; it may be asleep or offline"
                ))
            });
        let raw_error = result.as_ref().err().map(|e| format!("{e:#}"));
        let reached = match &raw_error {
            None => true,
            // The agent answered with an error, or refused this device (it
            // was revoked): either way it is online.
            Some(e) => e.starts_with("agent:") || e.contains("not paired (or was revoked)"),
        };
        self.online
            .lock()
            .expect("online poisoned")
            .insert(agent.id, reached);
        if let Some(e) = raw_error {
            bail!(friendly_error(
                &e,
                Some(&pigeon_core::display_safe(&agent.name))
            ));
        }
        let prefix = path.trim_matches('/');
        Ok(result?
            .into_iter()
            .map(|e| EntryView {
                path: if prefix.is_empty() {
                    e.name.clone()
                } else {
                    format!("{prefix}/{}", e.name)
                },
                name: pigeon_core::display_safe(&e.name),
                is_dir: e.is_dir,
                size: e.size,
                mtime: e.mtime,
            })
            .collect())
    }

    /// Starts fetching `path` from a paired computer into the download
    /// folder. Never overwrites: a taken name becomes `name (1).ext`; a
    /// `.part` left by an earlier attempt is resumed.
    pub fn get(self: &Arc<Self>, computer: &str, path: &str) -> Result<u64> {
        check_len("path", path, MAX_PATH_INPUT)?;
        let (agent, _) = self.computer(computer)?;
        let wanted = client::local_file_name(path)?;
        config::ensure_private_dir(&self.download_dir)?;
        let base = self.download_dir.join(&wanted);
        let mut transfers = self.transfers.lock().expect("transfers poisoned");
        let out = client::free_path(&base);
        anyhow::ensure!(
            !transfers
                .iter()
                .any(|t| t.view.state == TransferState::Active && t.out == out),
            "{wanted} is already downloading"
        );
        let id = {
            let mut n = self.next_transfer.lock().expect("ids poisoned");
            *n += 1;
            *n - 1
        };
        let file_name = out
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| wanted.clone());
        transfers.push(Transfer {
            view: TransferView {
                id,
                computer: agent.id.to_string(),
                computer_name: pigeon_core::display_safe(&agent.name),
                remote_path: pigeon_core::display_safe(path),
                file_name,
                wanted_name: wanted,
                state: TransferState::Active,
                size: 0,
                done: 0,
                resumed_from: 0,
                mb_per_sec: 0.0,
                route: None,
                sha256: None,
                verified: None,
                error: None,
                started_at: pair::now_secs(),
                finished_at: None,
            },
            computer: agent.id,
            path: path.to_string(),
            out,
            started: Instant::now(),
        });
        drop(transfers);
        self.spawn_transfer(id, path.to_string());
        Ok(id)
    }

    /// Runs a failed transfer again; it resumes from its `.part` file.
    pub fn resume(self: &Arc<Self>, id: u64) -> Result<()> {
        let path = {
            let mut transfers = self.transfers.lock().expect("transfers poisoned");
            let busy = transfers.iter().any(|o| {
                o.view.id != id
                    && o.view.state == TransferState::Active
                    && transfers.iter().any(|t| t.view.id == id && t.out == o.out)
            });
            let t = transfers
                .iter_mut()
                .find(|t| t.view.id == id)
                .context("no such transfer")?;
            anyhow::ensure!(
                t.view.state == TransferState::Failed,
                "not a failed transfer"
            );
            anyhow::ensure!(
                !busy,
                "{} is downloading again; wait for it to finish",
                t.view.file_name
            );
            anyhow::ensure!(
                t.out.symlink_metadata().is_err(),
                "{} exists now; get it again instead",
                t.view.file_name
            );
            t.view.state = TransferState::Active;
            t.view.error = None;
            t.view.verified = None;
            t.view.finished_at = None;
            t.started = Instant::now();
            t.path.clone()
        };
        self.spawn_transfer(id, path);
        Ok(())
    }

    fn spawn_transfer(self: &Arc<Self>, id: u64, path: String) {
        let me = self.clone();
        self.rt.spawn(async move {
            let result = me.run_transfer(id, &path).await;
            let mut transfers = me.transfers.lock().expect("transfers poisoned");
            let Some(t) = transfers.iter_mut().find(|t| t.view.id == id) else {
                return;
            };
            t.view.finished_at = Some(pair::now_secs());
            match result {
                Ok(stats) => {
                    t.view.state = TransferState::Done;
                    t.view.size = stats.size;
                    t.view.done = stats.size;
                    t.view.resumed_from = stats.resumed_from;
                    t.view.mb_per_sec = stats.mb_per_sec();
                    t.view.route = Some(stats.route.to_string());
                    t.view.verified = sha256_verified(&stats);
                    t.view.sha256 = Some(stats.sha256);
                }
                Err(e) => {
                    t.view.state = TransferState::Failed;
                    t.view.error = Some(friendly_error(
                        &format!("{e:#}"),
                        Some(&t.view.computer_name),
                    ));
                }
            }
        });
    }

    async fn run_transfer(&self, id: u64, path: &str) -> Result<client::GetStats> {
        let (computer, out) = {
            let transfers = self.transfers.lock().expect("transfers poisoned");
            let t = transfers
                .iter()
                .find(|t| t.view.id == id)
                .context("no such transfer")?;
            (t.computer, t.out.clone())
        };
        let (_, addr) = self.computer(&computer.to_string())?;
        let progress = |p: client::Progress| {
            let mut transfers = self.transfers.lock().expect("transfers poisoned");
            if let Some(t) = transfers.iter_mut().find(|t| t.view.id == id) {
                t.view.size = p.size;
                t.view.done = p.done;
                t.view.resumed_from = p.resumed_from;
                let secs = t.started.elapsed().as_secs_f64().max(1e-3);
                t.view.mb_per_sec = (p.done - p.resumed_from) as f64 / 1e6 / secs;
            }
        };
        let result = client::get_with_progress(&self.endpoint, &addr, path, &out, &progress).await;
        self.online
            .lock()
            .expect("online poisoned")
            .insert(computer, result.is_ok());
        result
    }

    /// Removes finished and failed transfers from the list (files stay).
    pub fn clear_finished(&self) {
        self.transfers
            .lock()
            .expect("transfers poisoned")
            .retain(|t| t.view.state == TransferState::Active);
    }

    /// The local file of a finished transfer, for Open / Show in folder.
    /// Only paths this backend wrote can be opened this way.
    pub fn finished_file(&self, id: u64) -> Result<PathBuf> {
        let transfers = self.transfers.lock().expect("transfers poisoned");
        let t = transfers
            .iter()
            .find(|t| t.view.id == id)
            .context("no such transfer")?;
        anyhow::ensure!(t.view.state == TransferState::Done, "not finished yet");
        anyhow::ensure!(t.out.is_file(), "the file was moved or deleted");
        Ok(t.out.clone())
    }

    /// Renames this computer (what it calls itself).
    pub fn rename(&self, name: &str) -> Result<String> {
        check_len("name", name, MAX_NAME_INPUT)?;
        let name = identity::set_name(&self.home.dir, name)?;
        *self.me_name.lock().expect("name poisoned") = name.clone();
        Ok(name)
    }

    /// The words a pairing code is made of (public list, for autocomplete).
    pub fn wordlist() -> &'static [&'static str] {
        pair::wordlist()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_pass_the_core_check() {
        for (path, want) in [
            ("C:\\Users\\a\\Photos 2026", "Photos 2026"),
            ("/home/a/Renders", "Renders"),
        ] {
            let got = folder_name(Path::new(path));
            if cfg!(windows) || !path.contains('\\') {
                assert_eq!(got, want);
            }
            assert!(agent::check_folder_name(&got).is_ok(), "{got:?}");
        }
        let odd = folder_name(Path::new("/x/ a\u{202e}b "));
        assert!(agent::check_folder_name(&odd).is_ok(), "{odd:?}");
        let taken = vec![FolderEntry {
            name: "Photos".into(),
            path: "x".into(),
        }];
        assert_eq!(unique_name("photos", &taken), "photos (2)");
        assert_eq!(unique_name("Scans", &taken), "Scans");
    }

    #[test]
    fn pair_errors_map_to_ui_states() {
        let e = |s: &str| classify_pair_error(&anyhow::anyhow!("{s}"), false);
        assert!(matches!(
            e("wrong pairing code; attempts left: 2"),
            PairOutcome::WrongCode { attempts_left: 2 }
        ));
        assert!(matches!(
            e("wrong pairing code; the invite is now used up. Run `pigeon-agent pair` again"),
            PairOutcome::UsedUp
        ));
        assert!(matches!(
            e("the agent refused: this invite has expired"),
            PairOutcome::Expired
        ));
        // N2: a link that has not expired but finds no invite was used
        // (or cancelled / replaced), whatever the core's text says.
        let no_invite =
            "the agent refused: no active invite: it expired, was used, or was replaced";
        let no_alpn = "could not pair: either `pigeon-agent share` is not running, or this invite \
                       expired, was already used, or was replaced. Make a new one with `pigeon-agent pair`";
        for msg in [no_invite, no_alpn] {
            assert!(matches!(e(msg), PairOutcome::AlreadyUsed), "{msg}");
            assert!(matches!(
                classify_pair_error(&anyhow::anyhow!("{msg}"), true),
                PairOutcome::Expired
            ));
        }
        // Anything else never carries CLI text into the app.
        match e("pairing step timed out") {
            PairOutcome::Failed { message } => assert_eq!(message, "Pairing step timed out"),
            other => panic!("{other:?}"),
        }
    }

    /// N3: what the Browse banner and Transfers show.
    #[test]
    fn core_errors_read_like_the_app() {
        let f = |s: &str| friendly_error(s, Some("Work Mac"));
        let revoked = f(
            "the agent refused this device: it is not paired (or was revoked). \
                         Run `pigeon-agent pair` on the agent and `pigeon pair <LINK>` here",
        );
        assert!(revoked.starts_with("Work Mac no longer lets this computer get files"));
        let os =
            f("agent: not found: nope: The system cannot find the file specified. (os error 2)");
        assert_eq!(
            os,
            "Not found: nope. It may have been moved, renamed or unshared."
        );
        assert_eq!(
            f("agent: `..` is not allowed: a/.."),
            "`..` is not allowed: a/.."
        );
        assert!(f("could not move \\\\?\\C:\\x.part").contains("C:\\x.part"));
        for msg in [
            revoked,
            os,
            f("agent: the agent is busy (too many connections); try again"),
        ] {
            for bad in ["pigeon-agent", "pigeon pair", "os error", "agent:", r"\\?\"] {
                assert!(!msg.contains(bad), "{bad} in {msg}");
            }
        }
    }

    #[test]
    fn webview_ids_and_lengths_are_checked() {
        assert!(parse_id("not-an-id").is_err());
        assert!(parse_id(&"a".repeat(500)).is_err());
        let id = iroh::SecretKey::from_bytes(&[7; 32]).public();
        assert_eq!(parse_id(&id.to_string()).unwrap(), id);
        assert!(check_len("x", &"a".repeat(10), 5).is_err());
        assert!(check_len("x", "a\0b", 5).is_err());
    }
}
