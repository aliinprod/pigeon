//! Agent side: accepts iroh connections from paired devices and answers
//! List/Get requests for one shared root folder; runs pairing while an invite
//! is active.
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
    endpoint::{Connection, SendStream},
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
    root: PathBuf,
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
        endpoint.set_alpns(vec![ALPN.to_vec()]);
        Self {
            inner: Arc::new(Inner {
                root,
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

    /// Whether a pairing invite is active.
    pub fn invite_active(&self) -> bool {
        self.inner.invite.lock().expect("invite poisoned").is_some()
    }

    /// Activates `invite` (replacing any other one, which then can never be
    /// used again). Ignored if it was already spent or has expired; a call
    /// with the active invite's ID keeps the in-memory attempt count.
    pub fn offer_invite(&self, invite: Invite) {
        let now = pair::now_secs();
        let mut spent = self.inner.spent.lock().expect("spent poisoned");
        if spent.contains(&invite.invite_id) {
            return;
        }
        let mut current = self.inner.invite.lock().expect("invite poisoned");
        if let Some(cur) = current.as_ref() {
            if cur.invite_id == invite.invite_id {
                return;
            }
            spent.insert(cur.invite_id);
        }
        if invite.is_expired(now) {
            spent.insert(invite.invite_id);
            *current = None;
        } else {
            *current = Some(invite);
        }
        self.update_alpns(current.is_some());
    }

    fn update_alpns(&self, pairing: bool) {
        let mut alpns = vec![ALPN.to_vec()];
        if pairing {
            alpns.push(PAIR_ALPN.to_vec());
        }
        self.inner.endpoint.set_alpns(alpns);
    }

    /// Drops the active invite (used, burned or expired).
    fn clear_invite(&self, current: &mut Option<Invite>) {
        if let Some(old) = current.take() {
            self.inner
                .spent
                .lock()
                .expect("spent poisoned")
                .insert(old.invite_id);
            if let Err(e) = self.inner.storage.invite_changed(&old.invite_id, None) {
                eprintln!("warning: could not remove the used invite from storage: {e:#}");
            }
        }
        self.update_alpns(false);
    }

    /// Drops the invite if it has expired. Call periodically.
    pub fn expire_invite(&self) {
        let mut current = self.inner.invite.lock().expect("invite poisoned");
        if current
            .as_ref()
            .is_some_and(|i| i.is_expired(pair::now_secs()))
        {
            eprintln!("the pairing invite expired");
            self.clear_invite(&mut current);
        }
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
        let result = serve_one(&conn, &self.inner.root, &self.inner.opts.limits).await;
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
        // PAIRING.md section 5: one handshake at a time, others refused at once.
        let Ok(_busy) = self.inner.pairing.try_lock() else {
            conn.close(close::BUSY.into(), b"busy");
            bail!("another pairing is in progress; refused");
        };
        if !self.invite_active() {
            conn.close(close::NO_INVITE.into(), b"no invite");
            bail!("no active invite");
        }
        tokio::time::timeout(pair::TOTAL_TIMEOUT, self.pair_steps(conn))
            .await
            .context("pairing timed out")?
    }

    /// Uses one attempt of invite `id`. Returns its code and device name, or
    /// the reason to refuse.
    fn take_attempt(
        &self,
        id: &[u8; 16],
    ) -> std::result::Result<(Zeroizing<String>, Option<String>), String> {
        let mut current = self.inner.invite.lock().expect("invite poisoned");
        let gone = "no active invite: it expired, was used, or was replaced".to_string();
        let Some(invite) = current.as_mut() else {
            return Err(gone);
        };
        if invite.invite_id != *id {
            return Err(gone);
        }
        if invite.is_expired(pair::now_secs()) {
            self.clear_invite(&mut current);
            return Err("this invite has expired".into());
        }
        if invite.attempts_left == 0 {
            self.clear_invite(&mut current);
            return Err("this invite is used up".into());
        }
        // Decrement before any work (PAIRING.md section 4, step 4).
        invite.attempts_left -= 1;
        if let Err(e) = self.inner.storage.invite_changed(id, Some(invite)) {
            eprintln!("warning: could not save the invite's attempt count: {e:#}");
        }
        Ok((invite.code(), invite.device_name.clone()))
    }

    /// Records a wrong code. Returns the attempts left; burns the invite at 0.
    fn failed_attempt(&self, id: &[u8; 16]) -> u8 {
        let mut current = self.inner.invite.lock().expect("invite poisoned");
        let left = match current.as_ref() {
            Some(invite) if invite.invite_id == *id => invite.attempts_left,
            _ => 0,
        };
        if left == 0 {
            self.clear_invite(&mut current);
        }
        left
    }

    /// Consumes invite `id` and adds `device` to the allow-list.
    fn complete_pairing(&self, id: &[u8; 16], device: Device) -> Result<()> {
        {
            let mut current = self.inner.invite.lock().expect("invite poisoned");
            anyhow::ensure!(
                current.as_ref().is_some_and(|i| i.invite_id == *id),
                "the invite was replaced during pairing"
            );
            self.clear_invite(&mut current);
        }
        self.inner.storage.device_paired(&device)?;
        let mut list = self.devices();
        list.upsert(device);
        *self.inner.devices.write().expect("devices poisoned") = list;
        Ok(())
    }

    async fn pair_steps(&self, conn: &Connection) -> Result<()> {
        let client = conn.remote_id();
        let agent = self.inner.endpoint.id();
        let fp = crate::fingerprint(&client);
        let (mut send, mut recv) = step(async { Ok(conn.accept_bi().await?) }).await?;
        let start: PairStart = step(read_frame_max(&mut recv, MAX_PAIR_FRAME)).await?;
        let refuse = |why: String| AgentMsg::Refused(why);
        if start.version != PAIR_VERSION {
            step(write_frame(
                &mut send,
                &refuse("unsupported pairing version".into()),
            ))
            .await?;
            send.finish()?;
            bail!("unsupported pairing version {}", start.version);
        }
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

async fn step<T>(fut: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(pair::STEP_TIMEOUT, fut)
        .await
        .context("pairing step timed out")?
}

/// One request, one response, on a connection from a paired device.
async fn serve_one(conn: &Connection, root: &Path, limits: &Limits) -> Result<()> {
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
            let response = match list(root, &path).await {
                Ok(entries) => Response::List(entries),
                Err(e) => Response::Error(format!("{e:#}")),
            };
            with_idle(idle, write_frame(&mut send, &response)).await?;
        }
        Request::Get { path, offset } => match open_for_get(root, &path, offset).await {
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
