//! Agent side: accepts iroh connections from paired devices and answers
//! List/Get requests for one shared root folder (`pigeon-agent share <DIR>`)
//! or for several named folders (the app, see [`SharedFolder`]); runs
//! pairing while an invite is active.
//!
//! Deny by default: a connection on [`crate::ALPN`] whose endpoint ID is not
//! on the allow-list is closed before any stream is accepted (SEC-08). The
//! pairing ALPN is only offered while an invite exists (PAIRING.md section 5).

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointId,
    endpoint::{Connection, RecvStream, SendStream},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::Semaphore,
};
use zeroize::Zeroizing;

use crate::{
    ALPN, PAIR_ALPN,
    devices::{Device, DeviceList},
    guard,
    pair::{
        self, AGENT_LABEL, AgentMsg, CLIENT_LABEL, Invite, MAX_PAIR_FRAME, PAIR_VERSION,
        PairConfirm, PairStart,
    },
    proto::{Entry, MAX_REQUEST_LEN, Request, Response, read_frame_max, write_frame},
};

/// Size of the read buffer used when streaming files.
const CHUNK: usize = 256 * 1024;

/// How long a peer gets to open its stream and send its request.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// QUIC close codes the agent uses.
pub mod close {
    /// Normal end of a request.
    pub const DONE: u32 = 0;
    /// The peer's key is not on the allow-list (never paired, or revoked).
    pub const NOT_PAIRED: u32 = 403;
    /// Too many connections.
    pub const BUSY: u32 = 429;
    /// Pairing ALPN without an active invite.
    pub const NO_INVITE: u32 = 404;
}

/// Resource limits for one agent (SEC-18, TH-08).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Connections served at the same time, from all peers together. Extra
    /// connection attempts are refused before the QUIC handshake.
    pub max_connections: usize,
    /// Connections served at the same time for one peer (endpoint ID).
    pub max_per_peer: usize,
    /// Time a peer gets to open its stream and send its request.
    pub handshake_timeout: Duration,
    /// Longest time without progress on an authenticated connection: a write
    /// the peer does not read, or waiting for the peer to close at the end.
    pub idle_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 32,
            max_per_peer: 4,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            idle_timeout: Duration::from_secs(30),
        }
    }
}

/// How the agent serves connections.
#[derive(Debug, Clone, Copy, Default)]
pub struct ServeOptions {
    pub limits: Limits,
    /// Print details of failed connection attempts. Off by default: any host
    /// that can reach us may send stray packets, and those are not an alarm.
    pub verbose: bool,
}

/// Where the agent persists pairing results. The binary writes files and the
/// keystore; tests use [`NoStorage`].
pub trait AgentStorage: Send + Sync {
    /// `device` was just paired; add it to the persistent allow-list.
    fn device_paired(&self, device: &Device) -> Result<()>;
    /// The invite `invite_id` changed (`Some`: an attempt was used) or is
    /// gone (`None`: used, burned or expired). Implementations must not
    /// overwrite a newer invite with a different ID.
    fn invite_changed(&self, invite_id: &[u8; 16], invite: Option<&Invite>) -> Result<()>;
    /// Remembers that `invite_id` was used, burned, expired or replaced, so
    /// it is never accepted again, also after a restart (W-5).
    fn invite_spent(&self, invite_id: &[u8; 16]) -> Result<()>;
    /// Whether `invite_id` was spent before. Fail closed: return `true` if
    /// the record cannot be read.
    fn is_spent(&self, invite_id: &[u8; 16]) -> bool;
}

/// What [`Agent::offer_invite`] did with an invite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// It is now the active invite.
    Activated,
    /// It already was the active invite (nothing changed).
    AlreadyActive,
    /// It was used, burned, expired or replaced before: ignored.
    Spent,
    /// It has expired: ignored.
    Expired,
    /// It was made for another agent key (e.g. after `pigeon-agent reset`
    /// while this agent kept running): ignored.
    OtherAgent,
}

/// One folder shared under a name (the app shares several). Peers see the
/// names as the top-level directories of the share: `Photos/2026/a.jpg` is
/// `a.jpg` in `2026` inside the folder named `Photos`. They never see `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedFolder {
    /// What peers see. One path component; see [`check_folder_name`].
    pub name: String,
    /// Local folder (read-only; every request still goes through the guard).
    pub path: PathBuf,
}

/// Longest shared-folder name, in characters.
pub const MAX_FOLDER_NAME: usize = 128;

/// A shared-folder name must be one ordinary path component: not empty, at
/// most [`MAX_FOLDER_NAME`] characters, not `.`/`..`, no `/`, `\`, `:`, NUL,
/// control or bidi characters, and no leading or trailing space.
pub fn check_folder_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "a shared folder needs a name");
    anyhow::ensure!(
        name.chars().count() <= MAX_FOLDER_NAME,
        "a shared folder name can have at most {MAX_FOLDER_NAME} characters"
    );
    anyhow::ensure!(
        name != "." && name != "..",
        "`.` and `..` are not folder names"
    );
    anyhow::ensure!(
        name.trim() == name,
        "a shared folder name cannot start or end with a space"
    );
    anyhow::ensure!(
        !name.contains(['/', '\\', ':', '\0']) && crate::display_safe(name) == name,
        "a shared folder name cannot contain `/`, `\\`, `:` or control characters"
    );
    Ok(())
}

/// Names must be valid and unique (case-insensitive, so the share behaves
/// the same on every OS).
fn check_folders(folders: &[SharedFolder]) -> Result<()> {
    let mut seen = HashSet::new();
    for f in folders {
        check_folder_name(&f.name)?;
        anyhow::ensure!(
            seen.insert(f.name.to_lowercase()),
            "two shared folders are called {:?}",
            crate::display_safe(&f.name)
        );
    }
    Ok(())
}

/// What the agent shares.
enum Roots {
    /// `pigeon-agent share <DIR>`: request paths are relative to this folder.
    Single(PathBuf),
    /// The app: request paths start with a folder name.
    Named(Vec<SharedFolder>),
}

/// Where a request path points once the folder name is looked up.
enum Target {
    /// The list of named folders itself (`""` with [`Roots::Named`]).
    Folders(Vec<SharedFolder>),
    /// `rest` inside `root`, still to be checked by the guard.
    Inside { root: PathBuf, rest: String },
}

impl Roots {
    fn target(&self, path: &str) -> Result<Target> {
        let folders = match self {
            Roots::Single(root) => {
                return Ok(Target::Inside {
                    root: root.clone(),
                    rest: path.to_string(),
                });
            }
            Roots::Named(folders) => folders,
        };
        if path.starts_with(['/', '\\']) {
            bail!("absolute paths are not allowed: {path}");
        }
        let (first, rest) = path.split_once(['/', '\\']).unwrap_or((path, ""));
        if first.is_empty() || first == "." {
            if rest.is_empty() {
                return Ok(Target::Folders(folders.clone()));
            }
            // `./x`: keep paths simple and refuse.
            bail!("not found: {path}");
        }
        match folders.iter().find(|f| f.name == first) {
            Some(f) => Ok(Target::Inside {
                root: f.path.clone(),
                rest: rest.to_string(),
            }),
            // Same message as a missing file (SEC-10: no local paths).
            None => bail!("not found: {path}"),
        }
    }
}

/// Keeps everything in memory.
pub struct NoStorage;

impl AgentStorage for NoStorage {
    fn device_paired(&self, _: &Device) -> Result<()> {
        Ok(())
    }

    fn invite_changed(&self, _: &[u8; 16], _: Option<&Invite>) -> Result<()> {
        Ok(())
    }

    fn invite_spent(&self, _: &[u8; 16]) -> Result<()> {
        Ok(())
    }

    fn is_spent(&self, _: &[u8; 16]) -> bool {
        false
    }
}

/// Counts live connections per peer. A [`PeerSlot`] gives its place back on drop.
#[derive(Default)]
struct PeerCounts(Mutex<HashMap<EndpointId, usize>>);

struct PeerSlot {
    counts: Arc<PeerCounts>,
    peer: EndpointId,
}

impl PeerCounts {
    fn try_take(self: &Arc<Self>, peer: EndpointId, max: usize) -> Option<PeerSlot> {
        let mut map = self.0.lock().expect("peer counts poisoned");
        let n = map.entry(peer).or_insert(0);
        if *n >= max {
            return None;
        }
        *n += 1;
        Some(PeerSlot {
            counts: self.clone(),
            peer,
        })
    }
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut map = self.counts.0.lock().expect("peer counts poisoned");
        if let Some(n) = map.get_mut(&self.peer) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.peer);
            }
        }
    }
}

/// A running agent. Cheap to clone.
#[derive(Clone)]
pub struct Agent {
    inner: Arc<Inner>,
}

struct Inner {
    roots: RwLock<Roots>,
    opts: ServeOptions,
    endpoint: Endpoint,
    storage: Box<dyn AgentStorage>,
    devices: RwLock<DeviceList>,
    invite: Mutex<Option<Invite>>,
    /// Invites that were used, burned or expired: never accepted again, even
    /// if a stale copy shows up in storage.
    spent: Mutex<HashSet<[u8; 16]>>,
    /// At most one pairing handshake at a time.
    pairing: tokio::sync::Mutex<()>,
    /// Live file-serving connections, so revoking closes them.
    conns: Mutex<HashMap<usize, (EndpointId, Connection)>>,
    slots: Arc<Semaphore>,
    peers: Arc<PeerCounts>,
    /// Pairing connections per peer (at most one each, W-2).
    pair_peers: Arc<PeerCounts>,
}

/// Removes a connection from the live map when its handler ends.
struct Registered<'a> {
    agent: &'a Agent,
    id: usize,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        self.agent
            .inner
            .conns
            .lock()
            .expect("conns poisoned")
            .remove(&self.id);
    }
}

impl Agent {
    /// An agent sharing `root` on `endpoint` for the devices in `devices`.
    pub fn new(
        endpoint: Endpoint,
        root: PathBuf,
        devices: DeviceList,
        opts: ServeOptions,
        storage: Box<dyn AgentStorage>,
    ) -> Self {
        Self::build(endpoint, Roots::Single(root), devices, opts, storage)
    }

    /// An agent sharing several named folders (the app). Peers see the
    /// names as top-level directories. Fails on an invalid or duplicate name.
    pub fn with_folders(
        endpoint: Endpoint,
        folders: Vec<SharedFolder>,
        devices: DeviceList,
        opts: ServeOptions,
        storage: Box<dyn AgentStorage>,
    ) -> Result<Self> {
        check_folders(&folders)?;
        Ok(Self::build(
            endpoint,
            Roots::Named(folders),
            devices,
            opts,
            storage,
        ))
    }

    fn build(
        endpoint: Endpoint,
        roots: Roots,
        devices: DeviceList,
        opts: ServeOptions,
        storage: Box<dyn AgentStorage>,
    ) -> Self {
        endpoint.set_alpns(vec![ALPN.to_vec()]);
        Self {
            inner: Arc::new(Inner {
                roots: RwLock::new(roots),
                opts,
                slots: Arc::new(Semaphore::new(opts.limits.max_connections)),
                endpoint,
                storage,
                devices: RwLock::new(devices),
                invite: Mutex::new(None),
                spent: Mutex::new(HashSet::new()),
                pairing: tokio::sync::Mutex::new(()),
                conns: Mutex::new(HashMap::new()),
                peers: Arc::new(PeerCounts::default()),
                pair_peers: Arc::new(PeerCounts::default()),
            }),
        }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.inner.endpoint
    }

    /// The current allow-list.
    pub fn devices(&self) -> DeviceList {
        self.inner.devices.read().expect("devices poisoned").clone()
    }

    /// Replaces the allow-list (e.g. after `pigeon-agent revoke` edited the
    /// file). Live connections of devices that are no longer on it are
    /// closed at once.
    pub fn set_devices(&self, list: DeviceList) {
        *self.inner.devices.write().expect("devices poisoned") = list.clone();
        let conns = self.inner.conns.lock().expect("conns poisoned");
        for (peer, conn) in conns.values() {
            if !list.contains(peer) {
                conn.close(close::NOT_PAIRED.into(), b"not paired");
                eprintln!(
                    "closed a connection from revoked device {}",
                    crate::fingerprint(peer)
                );
            }
        }
    }

    /// Removes `id` from the allow-list and closes its connections. Does not
    /// touch storage (the binary edits the file, then calls `set_devices`).
    pub fn revoke(&self, id: &EndpointId) {
        let mut list = self.devices();
        list.devices.retain(|d| d.id != *id);
        self.set_devices(list);
    }

    /// Replaces the shared folders (the app's add / stop sharing) and
    /// switches a single-root agent to named folders. New requests see the
    /// new list at once; a download that already started keeps its open
    /// file. On an invalid or duplicate name nothing changes.
    pub fn set_folders(&self, folders: Vec<SharedFolder>) -> Result<()> {
        check_folders(&folders)?;
        *self.inner.roots.write().expect("roots poisoned") = Roots::Named(folders);
        Ok(())
    }

    /// Drops the active invite, if any (the owner cancelled it). It is
    /// recorded as spent, so it is never accepted again.
    pub fn cancel_invite(&self) {
        let cleared = {
            let mut current = self.inner.invite.lock().expect("invite poisoned");
            if current.is_some() {
                self.clear_invite(&mut current)
            } else {
                None
            }
        };
        self.persist_cleared(cleared);
    }

    /// Whether a pairing invite is active.
    pub fn invite_active(&self) -> bool {
        self.inner.invite.lock().expect("invite poisoned").is_some()
    }

    /// Activates `invite` (replacing any other one, which then can never be
    /// used again). Ignored if it was already spent or has expired; a call
    /// with the active invite's ID keeps the in-memory attempt count.
    pub fn offer_invite(&self, invite: Invite) -> Offer {
        // M1: an invite made for another key (e.g. after `pigeon-agent reset`
        // while this agent kept running) can never be used here.
        if invite.agent.is_some_and(|a| a != self.inner.endpoint.id()) {
            return Offer::OtherAgent;
        }
        // Storage I/O happens outside the locks (W-5).
        let persisted_spent = self.inner.storage.is_spent(&invite.invite_id);
        let now = pair::now_secs();
        let mut newly_spent = Vec::new();
        let outcome = {
            // Lock order everywhere: `invite`, then `spent` (see `clear_invite`).
            let mut current = self.inner.invite.lock().expect("invite poisoned");
            let mut spent = self.inner.spent.lock().expect("spent poisoned");
            let outcome = if persisted_spent || spent.contains(&invite.invite_id) {
                Offer::Spent
            } else if current
                .as_ref()
                .is_some_and(|c| c.invite_id == invite.invite_id)
            {
                Offer::AlreadyActive
            } else {
                if let Some(old) = current.take() {
                    spent.insert(old.invite_id);
                    newly_spent.push(old.invite_id);
                }
                if invite.is_expired(now) {
                    spent.insert(invite.invite_id);
                    newly_spent.push(invite.invite_id);
                    Offer::Expired
                } else {
                    *current = Some(invite);
                    Offer::Activated
                }
            };
            self.update_alpns(current.is_some());
            outcome
        };
        for id in newly_spent {
            self.persist_cleared(Some(id));
        }
        outcome
    }

    fn update_alpns(&self, pairing: bool) {
        let mut alpns = vec![ALPN.to_vec()];
        if pairing {
            alpns.push(PAIR_ALPN.to_vec());
        }
        self.inner.endpoint.set_alpns(alpns);
    }

    /// Drops the active invite (used, burned or expired) in memory. The
    /// caller holds the `invite` lock; this takes `spent` after it. Returns
    /// its ID, to pass to [`Self::persist_cleared`] *after* the lock is gone.
    fn clear_invite(&self, current: &mut Option<Invite>) -> Option<[u8; 16]> {
        let old = current.take().map(|old| old.invite_id);
        if let Some(id) = old {
            self.inner.spent.lock().expect("spent poisoned").insert(id);
        }
        self.update_alpns(false);
        old
    }

    /// Records a cleared invite in storage: spent for good (W-5), and its
    /// keystore copy removed. Call without holding any lock.
    fn persist_cleared(&self, id: Option<[u8; 16]>) {
        let Some(id) = id else { return };
        if let Err(e) = self.inner.storage.invite_spent(&id) {
            eprintln!("warning: could not record the used invite: {e:#}");
        }
        if let Err(e) = self.inner.storage.invite_changed(&id, None) {
            eprintln!("warning: could not remove the used invite from storage: {e:#}");
        }
    }

    /// Drops the invite if it has expired. Call periodically.
    pub fn expire_invite(&self) {
        let cleared = {
            let mut current = self.inner.invite.lock().expect("invite poisoned");
            if current
                .as_ref()
                .is_some_and(|i| i.is_expired(pair::now_secs()))
            {
                eprintln!("the pairing invite expired");
                self.clear_invite(&mut current)
            } else {
                None
            }
        };
        self.persist_cleared(cleared);
    }

    /// Serves incoming connections until the endpoint is closed.
    pub async fn serve(&self) -> Result<()> {
        let opts = self.inner.opts;
        while let Some(incoming) = self.inner.endpoint.accept().await {
            // Global cap: refuse before spending any work on the handshake.
            let Ok(slot) = self.inner.slots.clone().try_acquire_owned() else {
                if opts.verbose {
                    eprintln!(
                        "connection refused: all {} slots in use",
                        opts.limits.max_connections
                    );
                }
                incoming.refuse();
                continue;
            };
            let agent = self.clone();
            tokio::spawn(async move {
                let _slot = slot;
                let conn = match incoming.await {
                    Ok(conn) => conn,
                    Err(e) => {
                        // Usually a scanner, a stray packet or a client that
                        // gave up. Nothing was exposed, so do not alarm.
                        if opts.verbose {
                            eprintln!(
                                "a connection attempt did not complete the handshake (ignored): {e}"
                            );
                        } else {
                            eprintln!(
                                "a connection attempt did not complete the handshake (ignored)"
                            );
                        }
                        return;
                    }
                };
                agent.handle(conn).await;
            });
        }
        Ok(())
    }

    async fn handle(&self, conn: Connection) {
        let peer = conn.remote_id();
        let fp = crate::fingerprint(&peer);
        if conn.alpn() == PAIR_ALPN {
            if let Err(e) = self.handle_pairing(&conn).await {
                eprintln!("pairing attempt from {fp} failed: {e:#}");
            }
            conn.close(close::DONE.into(), b"done");
            return;
        }
        // Register first, then check: a revoke that lands in between still
        // finds and closes this connection.
        let _registered = self.register(&conn);
        if !self.devices().contains(&peer) {
            eprintln!("refused a connection from unpaired device {fp}");
            conn.close(close::NOT_PAIRED.into(), b"not paired");
            return;
        }
        let Some(_peer_slot) = self
            .inner
            .peers
            .try_take(peer, self.inner.opts.limits.max_per_peer)
        else {
            eprintln!("too many connections from {fp}; closed one");
            conn.close(close::BUSY.into(), b"busy");
            return;
        };
        let result = serve_one(&conn, &self.inner.roots, &self.inner.opts.limits).await;
        conn.close(close::DONE.into(), b"done");
        if let Err(e) = result {
            eprintln!("connection from {fp} failed: {e:#}");
        }
    }

    fn register(&self, conn: &Connection) -> Registered<'_> {
        let id = conn.stable_id();
        self.inner
            .conns
            .lock()
            .expect("conns poisoned")
            .insert(id, (conn.remote_id(), conn.clone()));
        Registered { agent: self, id }
    }

    async fn handle_pairing(&self, conn: &Connection) -> Result<()> {
        let peer = conn.remote_id();
        // One pairing connection per peer at a time (W-2).
        let Some(_peer_slot) = self.inner.pair_peers.try_take(peer, 1) else {
            conn.close(close::BUSY.into(), b"busy");
            bail!("this device already has a pairing connection open; refused");
        };
        if !self.invite_active() {
            conn.close(close::NO_INVITE.into(), b"no invite");
            bail!("no active invite");
        }
        // Read and check PairStart *before* taking the pairing lock (W-2): a
        // peer that knows only the agent ID (no invite ID) must not be able
        // to block the owner's pairing by connecting and sending nothing.
        let (mut send, recv, start) = tokio::time::timeout(pair::START_TIMEOUT, async {
            let (send, mut recv) = conn.accept_bi().await?;
            let start: PairStart = read_frame_max(&mut recv, MAX_PAIR_FRAME).await?;
            anyhow::Ok((send, recv, start))
        })
        .await
        .context("no pairing request in time")??;
        self.expire_invite();
        let refusal = if start.version != PAIR_VERSION {
            Some("unsupported pairing version".to_string())
        } else if !self.invite_matches(&start.invite_id) {
            Some("no active invite: it expired, was used, or was replaced".to_string())
        } else {
            None
        };
        if let Some(why) = refusal {
            step(write_frame(&mut send, &AgentMsg::Refused(why.clone()))).await?;
            send.finish()?;
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            bail!("{why}");
        }
        // PAIRING.md section 5: one handshake at a time, others refused at
        // once (and without using an attempt).
        let Ok(_busy) = self.inner.pairing.try_lock() else {
            let why = "another pairing is in progress; try again in a moment";
            step(write_frame(&mut send, &AgentMsg::Refused(why.into()))).await?;
            send.finish()?;
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            bail!("{why}");
        };
        tokio::time::timeout(
            pair::TOTAL_TIMEOUT,
            self.pair_steps(conn, send, recv, start),
        )
        .await
        .context("pairing timed out")?
    }

    /// Whether `id` is the active, unexpired invite (uses no attempt).
    fn invite_matches(&self, id: &[u8; 16]) -> bool {
        let current = self.inner.invite.lock().expect("invite poisoned");
        current
            .as_ref()
            .is_some_and(|i| i.invite_id == *id && !i.is_expired(pair::now_secs()))
    }

    /// Uses one attempt of invite `id`. Returns its code and device name, or
    /// the reason to refuse.
    fn take_attempt(
        &self,
        id: &[u8; 16],
    ) -> std::result::Result<(Zeroizing<String>, Option<String>), String> {
        let mut cleared = None;
        let mut to_save = None;
        let result = {
            let mut current = self.inner.invite.lock().expect("invite poisoned");
            let gone = "no active invite: it expired, was used, or was replaced".to_string();
            match current.as_mut() {
                Some(invite) if invite.invite_id == *id => {
                    if invite.is_expired(pair::now_secs()) {
                        cleared = self.clear_invite(&mut current);
                        Err("this invite has expired".into())
                    } else if invite.attempts_left == 0 {
                        cleared = self.clear_invite(&mut current);
                        Err("this invite is used up".into())
                    } else {
                        // Decrement before any work (PAIRING.md section 4, step 4).
                        invite.attempts_left -= 1;
                        to_save = Some(invite.clone());
                        Ok((invite.code(), invite.device_name.clone()))
                    }
                }
                _ => Err(gone),
            }
        };
        self.persist_cleared(cleared);
        if let Some(invite) = to_save
            && let Err(e) = self.inner.storage.invite_changed(id, Some(&invite))
        {
            eprintln!("warning: could not save the invite's attempt count: {e:#}");
        }
        result
    }

    /// Records a wrong code for invite `id`. Returns its attempts left and
    /// burns it at 0. If `id` is no longer the active invite (replaced while
    /// this handshake ran), the active one is left alone (W-1) and 0 is
    /// returned: the old invite is gone either way.
    fn failed_attempt(&self, id: &[u8; 16]) -> u8 {
        let (left, cleared) = {
            let mut current = self.inner.invite.lock().expect("invite poisoned");
            match current.as_ref() {
                Some(invite) if invite.invite_id == *id => {
                    let left = invite.attempts_left;
                    let cleared = if left == 0 {
                        self.clear_invite(&mut current)
                    } else {
                        None
                    };
                    (left, cleared)
                }
                _ => (0, None),
            }
        };
        self.persist_cleared(cleared);
        left
    }

    /// Consumes invite `id` and adds `device` to the allow-list.
    fn complete_pairing(&self, id: &[u8; 16], device: Device) -> Result<()> {
        let cleared = {
            let mut current = self.inner.invite.lock().expect("invite poisoned");
            anyhow::ensure!(
                current.as_ref().is_some_and(|i| i.invite_id == *id),
                "the invite was replaced during pairing"
            );
            self.clear_invite(&mut current)
        };
        self.persist_cleared(cleared);
        self.inner.storage.device_paired(&device)?;
        let mut list = self.devices();
        list.upsert(device);
        *self.inner.devices.write().expect("devices poisoned") = list;
        Ok(())
    }

    async fn pair_steps(
        &self,
        conn: &Connection,
        mut send: SendStream,
        mut recv: RecvStream,
        start: PairStart,
    ) -> Result<()> {
        let client = conn.remote_id();
        let agent = self.inner.endpoint.id();
        let fp = crate::fingerprint(&client);
        let refuse = |why: String| AgentMsg::Refused(why);
        let cb = pair::channel_binding(conn, &start.invite_id)?;
        let (code, device_name) = match self.take_attempt(&start.invite_id) {
            Ok(found) => found,
            Err(why) => {
                step(write_frame(&mut send, &refuse(why.clone()))).await?;
                send.finish()?;
                let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
                bail!("{why}");
            }
        };
        let Some((msg_b, key)) = pair::agent_spake(&code, &agent, &client, &start.msg_a) else {
            let attempts_left = self.failed_attempt(&start.invite_id);
            step(write_frame(&mut send, &AgentMsg::Failed { attempts_left })).await?;
            send.finish()?;
            bail!("malformed pairing message ({attempts_left} attempts left)");
        };
        step(write_frame(
            &mut send,
            &AgentMsg::Reply {
                msg_b: msg_b.clone(),
            },
        ))
        .await?;
        let confirm: PairConfirm = step(read_frame_max(&mut recv, MAX_PAIR_FRAME)).await?;
        let (kc_client, kc_agent) = pair::confirm_keys(&key, &cb);
        let t = pair::transcript(&start.invite_id, &agent, &client, &start.msg_a, &msg_b);
        if !pair::verify_mac(&kc_client, CLIENT_LABEL, &t, &confirm.mac) {
            let attempts_left = self.failed_attempt(&start.invite_id);
            step(write_frame(&mut send, &AgentMsg::Failed { attempts_left })).await?;
            send.finish()?;
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            if attempts_left == 0 {
                bail!("wrong pairing code; the invite is used up");
            }
            bail!("wrong pairing code ({attempts_left} attempts left)");
        }
        let device = Device {
            id: client,
            name: device_name.unwrap_or_else(|| format!("device {fp}")),
            paired_at: pair::now_secs(),
        };
        let name = device.name.clone();
        self.complete_pairing(&start.invite_id, device)?;
        let ok = AgentMsg::Ok {
            mac: pair::confirm_mac(&kc_agent, AGENT_LABEL, &t),
        };
        step(write_frame(&mut send, &ok)).await?;
        send.finish()?;
        eprintln!("paired with {} ({fp})", crate::display_safe(&name));
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
        Ok(())
    }
}

/// Lock target in the config dir held by a running `pigeon-agent share`
/// (the OS lock is on `share.lock`, see [`crate::devices::FileLock`]).
pub const SHARE_LOCK: &str = "share";

/// Taken by `pigeon-agent share` for its whole lifetime (W-11). The OS
/// drops it when the process ends, even after a crash. No socket: other
/// programs only *try* the lock ([`share_running`]). A second `share` for
/// the same computer is refused: it would fight the first for the relay slot.
pub fn hold_share_lock(config_dir: &Path) -> Result<crate::devices::FileLock> {
    // On a fresh install `share` runs before anything created the config dir.
    crate::config::ensure_private_dir(config_dir)?;
    crate::devices::FileLock::acquire_for(
        &config_dir.join(SHARE_LOCK),
        Duration::ZERO,
        "`pigeon-agent share` is already running for this computer",
    )
}

/// Whether a `pigeon-agent share` for this config dir is running now.
/// Never blocks; any error counts as "not running" (it only decides a
/// warning).
pub fn share_running(config_dir: &Path) -> bool {
    let path = config_dir.join(format!("{SHARE_LOCK}.lock"));
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
    else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// The one line `pigeon ls`/`get`/`pair` print while a share runs on this
/// computer with the same key (TH-28, R14). Either the app or
/// `pigeon-agent share` can hold `share.lock`, so it names neither (W-23).
pub const SHARE_RUNNING_WARNING: &str = "note: another Pigeon share (the app or pigeon-agent) is \
     running on this computer with the same key; while this command runs, other computers can reach that share only \
     over direct connections, not through the relay";

/// How often [`watch`] re-reads the allow-list file.
pub const DEVICES_POLL: Duration = Duration::from_millis(500);
/// How often [`watch`] looks for a new invite (and expires the old one).
pub const INVITE_POLL: Duration = Duration::from_secs(2);

/// Lets the commands that run as separate processes reach a running agent:
/// `pigeon-agent revoke` edits `devices_path` (picked up within
/// [`DEVICES_POLL`], live connections of removed devices are closed), and
/// `pigeon-agent pair` stores an invite that `load_invite` returns (picked
/// up within [`INVITE_POLL`]). Runs until the task is dropped.
pub async fn watch<F>(agent: Agent, devices_path: PathBuf, load_invite: F)
where
    F: Fn() -> Result<Option<Invite>> + Send + Sync + 'static,
{
    let load_invite = Arc::new(load_invite);
    let invite_every = (INVITE_POLL.as_millis() / DEVICES_POLL.as_millis()).max(1) as u64;
    let mut known = agent.devices();
    let mut tick = 0u64;
    let mut warned = false;
    let mut reported_other: Option<[u8; 16]> = None;
    loop {
        tokio::time::sleep(DEVICES_POLL).await;
        tick += 1;
        match DeviceList::load(&devices_path) {
            Ok(list) if list != known => {
                for d in &known.devices {
                    if !list.contains(&d.id) {
                        eprintln!(
                            "revoked {} ({})",
                            crate::display_safe(&d.name),
                            crate::fingerprint(&d.id)
                        );
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
        if !tick.is_multiple_of(invite_every) {
            continue;
        }
        agent.expire_invite();
        // Keystore reads can block (or prompt): keep them off the runtime.
        let load = load_invite.clone();
        let loaded = match tokio::task::spawn_blocking(move || load()).await {
            Ok(r) => r,
            Err(e) => Err(anyhow::anyhow!("invite reader panicked: {e}")),
        };
        match loaded {
            Ok(Some(invite)) => {
                let id = invite.invite_id;
                match agent.offer_invite(invite) {
                    Offer::Activated => eprintln!("a new pairing invite is active"),
                    Offer::OtherAgent if reported_other != Some(id) => {
                        eprintln!(
                            "a pairing invite was made for a different agent key (after \
                             `pigeon-agent reset`?). Stop this agent (Ctrl+C), start \
                             `pigeon-agent share` again, then run `pigeon-agent pair` again."
                        );
                        reported_other = Some(id);
                    }
                    _ => {}
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("warning: could not read the pairing invite: {e:#}"),
        }
    }
}

async fn step<T>(fut: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(pair::STEP_TIMEOUT, fut)
        .await
        .context("pairing step timed out")?
}

/// One request, one response, on a connection from a paired device.
async fn serve_one(conn: &Connection, roots: &RwLock<Roots>, limits: &Limits) -> Result<()> {
    let (mut send, request) = tokio::time::timeout(limits.handshake_timeout, async {
        let (send, mut recv) = conn.accept_bi().await?;
        let request: Request = read_frame_max(&mut recv, MAX_REQUEST_LEN).await?;
        anyhow::Ok((send, request))
    })
    .await
    .context("request timed out")??;

    let idle = limits.idle_timeout;
    match request {
        Request::List { path } => {
            let response = match list_target(roots, &path).await {
                Ok(entries) => Response::List(entries),
                Err(e) => Response::Error(format!("{e:#}")),
            };
            with_idle(idle, write_frame(&mut send, &response)).await?;
        }
        Request::Get { path, offset } => match get_target(roots, &path, offset).await {
            Ok((file, size, offset, sha256)) => {
                let header = Response::File {
                    size,
                    offset,
                    sha256: Some(sha256),
                };
                with_idle(idle, write_frame(&mut send, &header)).await?;
                send_file(file, &mut send, idle).await?;
            }
            Err(e) => {
                let response = Response::Error(format!("{e:#}"));
                with_idle(idle, write_frame(&mut send, &response)).await?
            }
        },
    }
    send.finish()?;
    // Wait for the client to close once it has read everything, so we do not
    // drop buffered data by closing first. A client that never closes is cut
    // off after the idle timeout (TH-08).
    let _ = tokio::time::timeout(idle, conn.closed()).await;
    Ok(())
}

/// Runs one network step and fails if it makes no progress within `idle`.
async fn with_idle<T>(idle: Duration, fut: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(idle, fut)
        .await
        .context("peer stopped reading (idle timeout)")?
}

/// Looks `path` up in the shared folders (no lock is held across I/O).
fn target(roots: &RwLock<Roots>, path: &str) -> Result<Target> {
    roots.read().expect("roots poisoned").target(path)
}

async fn list_target(roots: &RwLock<Roots>, path: &str) -> Result<Vec<Entry>> {
    match target(roots, path)? {
        Target::Folders(folders) => Ok(folders
            .into_iter()
            .map(|f| Entry {
                name: f.name,
                is_dir: true,
                size: 0,
                mtime: 0,
            })
            .collect()),
        Target::Inside { root, rest } => list(&root, &rest).await,
    }
}

async fn get_target(
    roots: &RwLock<Roots>,
    path: &str,
    offset: u64,
) -> Result<(tokio::fs::File, u64, u64, [u8; 32])> {
    match target(roots, path)? {
        Target::Folders(_) => bail!("not a file: {path}"),
        Target::Inside { root, rest } => open_for_get(&root, &rest, offset).await,
    }
}

async fn list(root: &Path, path: &str) -> Result<Vec<Entry>> {
    let dir = guard::resolve(root, path)?;
    let canon_root = tokio::fs::canonicalize(root)
        .await
        .context("the shared folder is not available")?;
    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .with_context(|| format!("not a directory: {path}"))?;
    let mut entries = Vec::new();
    while let Some(e) = rd.next_entry().await? {
        // Follow symlinks, but only to targets inside the share: an entry
        // whose real path is outside must not leak its size, mtime or type,
        // and is hidden like a missing file (SEC-04). Get still goes through
        // the guard.
        let Ok(real) = tokio::fs::canonicalize(e.path()).await else {
            continue;
        };
        if !real.starts_with(&canon_root) {
            continue;
        }
        let Ok(meta) = tokio::fs::metadata(&real).await else {
            continue;
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push(Entry {
            name: e.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            mtime,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Opens `path` for a download from `offset`. Returns (file positioned at the
/// effective offset, size, effective offset, sha256). An offset past the end
/// (e.g. a stale `.part` from a bigger, older file) restarts at 0.
async fn open_for_get(
    root: &Path,
    path: &str,
    offset: u64,
) -> Result<(tokio::fs::File, u64, u64, [u8; 32])> {
    let full = guard::resolve(root, path)?;
    let meta = tokio::fs::metadata(&full).await?;
    if !meta.is_file() {
        bail!("not a file: {path}");
    }
    let size = meta.len();
    let offset = if offset > size { 0 } else { offset };
    let sha256 = sha256_file(full.clone()).await?;
    let mut file = tokio::fs::File::open(&full).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    Ok((file, size, offset, sha256))
}

async fn send_file(mut file: tokio::fs::File, send: &mut SendStream, idle: Duration) -> Result<()> {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        with_idle(idle, async { Ok(send.write_all(&buf[..n]).await?) }).await?;
    }
}

/// SHA-256 of a whole file, computed on a blocking thread.
pub async fn sha256_file(path: PathBuf) -> Result<[u8; 32]> {
    tokio::task::spawn_blocking(move || -> Result<[u8; 32]> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&hasher.finalize());
        Ok(out)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Makes `link` point at the directory `target`: a symlink on Unix, a
    /// junction on Windows (no admin rights needed there).
    fn dir_link(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let out = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(out.status.success(), "mklink /J failed: {out:?}");
        }
    }

    #[tokio::test]
    async fn list_hides_links_that_point_outside_the_share() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("share");
        std::fs::create_dir_all(root.join("inner")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"top secret").unwrap();
        dir_link(&outside, &root.join("out"));
        dir_link(&root.join("inner"), &root.join("in"));
        let names: Vec<String> = list(&root, "")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["in", "inner"], "escaping link must not be listed");
    }

    /// W-11: the share lock is seen by other programs (a separate handle,
    /// as another process has), refuses a second share, and is gone when
    /// the share stops.
    #[test]
    fn share_lock_is_visible_and_released() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!share_running(dir.path()), "no lock file yet");
        let held = hold_share_lock(dir.path()).unwrap();
        assert!(share_running(dir.path()));
        let err = hold_share_lock(dir.path()).err().unwrap();
        assert!(format!("{err:#}").contains("already running"), "{err:#}");
        drop(held);
        assert!(!share_running(dir.path()));
        assert!(hold_share_lock(dir.path()).is_ok());
    }

    /// Fresh install (caught by the macOS cross-network CI job): the default
    /// config dir does not exist yet when `share` takes the lock.
    #[test]
    fn share_lock_creates_a_missing_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = dir.path().join("Library/Application Support/Pigeon");
        assert!(!share_running(&fresh));
        let _held = hold_share_lock(&fresh).unwrap();
        assert!(share_running(&fresh));
    }

    fn named(dir: &Path) -> RwLock<Roots> {
        let photos = dir.join("photos-on-disk");
        let scans = dir.join("scans-on-disk");
        std::fs::create_dir_all(photos.join("2026")).unwrap();
        std::fs::create_dir_all(&scans).unwrap();
        std::fs::write(photos.join("2026/a.jpg"), b"jpeg").unwrap();
        std::fs::write(scans.join("scan.pdf"), b"pdf").unwrap();
        std::fs::write(dir.join("secret.txt"), b"outside").unwrap();
        RwLock::new(Roots::Named(vec![
            SharedFolder {
                name: "Photos".into(),
                path: photos,
            },
            SharedFolder {
                name: "Scans".into(),
                path: scans,
            },
        ]))
    }

    #[tokio::test]
    async fn named_folders_are_the_top_level_of_the_share() {
        let dir = tempfile::tempdir().unwrap();
        let roots = named(dir.path());
        let top: Vec<(String, bool)> = list_target(&roots, "")
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(top, [("Photos".into(), true), ("Scans".into(), true)]);
        assert_eq!(list_target(&roots, ".").await.unwrap().len(), 2);
        // The local folder names never show up.
        let inside: Vec<String> = list_target(&roots, "Photos")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(inside, ["2026"]);
        let (_, size, _, _) = get_target(&roots, "Photos/2026/a.jpg", 0).await.unwrap();
        assert_eq!(size, 4);
        let (_, size, _, _) = get_target(&roots, "Scans\\scan.pdf", 0).await.unwrap();
        assert_eq!(size, 3);
    }

    #[tokio::test]
    async fn named_folders_refuse_escapes_and_unknown_names() {
        let dir = tempfile::tempdir().unwrap();
        let roots = named(dir.path());
        for bad in [
            "/Photos",
            "\\Photos",
            "..",
            "../secret.txt",
            "Photos/../../secret.txt",
            "Photos/../Scans/scan.pdf",
            "photos-on-disk/2026/a.jpg",
            "photos/2026/a.jpg",
            "Nope",
            "./Photos",
            "C:/x",
            "Photos:x",
        ] {
            assert!(list_target(&roots, bad).await.is_err(), "list {bad:?}");
            assert!(get_target(&roots, bad, 0).await.is_err(), "get {bad:?}");
        }
        // The folder list and a folder are not files.
        assert!(get_target(&roots, "", 0).await.is_err());
        assert!(get_target(&roots, "Photos", 0).await.is_err());
        // Errors name only what the peer asked for (SEC-10).
        let err = format!("{:#}", get_target(&roots, "Nope/x", 0).await.unwrap_err());
        assert!(!err.contains("disk"), "{err}");
    }

    #[test]
    fn folder_names_are_single_plain_components() {
        for ok in ["Photos", "Projects Q4", "Отчёты", "a.b", "Scans (2)"] {
            assert!(check_folder_name(ok).is_ok(), "{ok:?}");
        }
        let long = "x".repeat(MAX_FOLDER_NAME + 1);
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "C:",
            " x",
            "x ",
            "a\u{1b}b",
            "x\u{202e}y",
            long.as_str(),
        ] {
            assert!(check_folder_name(bad).is_err(), "{bad:?}");
        }
        let f = |n: &str| SharedFolder {
            name: n.into(),
            path: PathBuf::from("x"),
        };
        assert!(check_folders(&[f("A"), f("B")]).is_ok());
        assert!(check_folders(&[f("Photos"), f("photos")]).is_err());
    }

    #[tokio::test]
    async fn errors_sent_to_the_peer_do_not_contain_the_root_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("gone-share");
        let err = format!("{:#}", list(&root, "").await.unwrap_err());
        assert!(!err.contains("gone-share"), "{err}");
        let err = format!("{:#}", open_for_get(&root, "x", 0).await.unwrap_err());
        assert!(!err.contains("gone-share"), "{err}");
    }
}
