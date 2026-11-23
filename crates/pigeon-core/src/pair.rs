//! Pairing, protocol P1 from `docs/security/PAIRING.md`.
//!
//! The agent creates a single-use [`Invite`]: an [`InviteLink`] (agent
//! address + invite ID + expiry, **no secret**, safe to email to yourself) and
//! a 4-word [`PairingCode`] that is only shown on the agent's screen. The
//! client dials the agent on [`crate::PAIR_ALPN`] and both run SPAKE2 with the
//! code, bound to this TLS session (RFC 5705 exporter) and to both endpoint
//! IDs, then confirm with HMAC-SHA256. A wrong guess costs one of 3 attempts;
//! seeing the link and all traffic gives no offline test for the code.
//!
//! No custom crypto: `spake2` (RustCrypto), `hkdf`, `hmac`, `sha2`.

use std::{fmt, str::FromStr, sync::LazyLock, time::Duration};

use anyhow::{Context, Result, bail};
use data_encoding::BASE32_NOPAD;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use iroh::{Endpoint, EndpointAddr, EndpointId, endpoint::Connection};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use zeroize::{Zeroize, Zeroizing};

use crate::proto::{read_frame_max, write_frame};

/// Version of the pairing frames ([`PairStart::version`]).
pub const PAIR_VERSION: u32 = 1;
/// Largest pairing frame either side accepts (PAIRING.md section 4, step 3).
pub const MAX_PAIR_FRAME: u32 = 128;
/// Timeout for each pairing step.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for a whole pairing handshake.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Time a pairing peer gets to send its first frame, before it may take the
/// one-at-a-time pairing lock (W-2).
pub const START_TIMEOUT: Duration = Duration::from_secs(3);
/// Default invite lifetime.
pub const DEFAULT_TTL: Duration = Duration::from_secs(12 * 3600);
/// Longest invite lifetime the agent accepts.
pub const MAX_TTL: Duration = Duration::from_secs(24 * 3600);
/// Wrong codes allowed per invite.
pub const ATTEMPTS: u8 = 3;
/// Words in a pairing code.
pub const CODE_WORDS: usize = 4;

/// SPAKE2 messages for Ed25519 are one group element plus a side byte.
const SPAKE_MSG_LEN: usize = 33;
const EXPORTER_LABEL: &[u8] = b"pigeon pair v1";
const CONFIRM_INFO: &[u8] = b"pigeon pair v1 confirm";

const LINK_PREFIX: &str = "pigeon-invite-";

/// EFF short wordlist 1 (CC BY 3.0 US, see NOTICE), minus `yo-yo`: `-` is our
/// separator and `yoyo` is already in the list. 1295 words, ~41.4 bits for 4.
static WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("eff_short_wordlist_1.txt")
        .lines()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect()
});

/// The words a pairing code is made of.
pub fn wordlist() -> &'static [&'static str] {
    &WORDS
}

/// A uniformly random index below `n` (rejection sampling, no modulo bias).
fn random_below(n: u32) -> Result<u32> {
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("no OS randomness: {e}"))?;
        let r = u32::from_le_bytes(b);
        if r < zone {
            return Ok(r % n);
        }
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("no OS randomness: {e}"))?;
    Ok(b)
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A pairing code like `otter-plaza-crumb-tulip`. Secret: never logged,
/// never sent over the network, zeroed on drop. There is deliberately no
/// `Display`; use [`PairingCode::reveal`] where showing it is intended.
#[derive(Clone)]
pub struct PairingCode(Zeroizing<String>);

impl PairingCode {
    /// A fresh random code.
    pub fn generate() -> Result<Self> {
        let words = wordlist();
        let n = u32::try_from(words.len())?;
        let mut picked = Vec::with_capacity(CODE_WORDS);
        for _ in 0..CODE_WORDS {
            picked.push(words[random_below(n)? as usize]);
        }
        Ok(Self(Zeroizing::new(picked.join("-"))))
    }

    /// Parses what a person typed: any case, words separated by `-`, spaces
    /// or both. Every word must be in the wordlist, so a typo fails here,
    /// locally, without costing one of the invite's attempts. The error never
    /// contains the input.
    pub fn parse(input: &str) -> Result<Self> {
        let lower = Zeroizing::new(input.trim().to_lowercase());
        let words: Vec<&str> = lower
            .split(|c: char| c == '-' || c.is_whitespace())
            .filter(|w| !w.is_empty())
            .collect();
        if words.len() != CODE_WORDS {
            bail!("a pairing code has {CODE_WORDS} words, got {}", words.len());
        }
        for (i, w) in words.iter().enumerate() {
            if !wordlist().contains(w) {
                bail!(
                    "word {} of the pairing code is not a code word (typo?)",
                    i + 1
                );
            }
        }
        Ok(Self(Zeroizing::new(words.join("-"))))
    }

    /// The code, for showing it on the agent's screen.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PairingCode({})", crate::REDACTED)
    }
}

/// The agent's side of one invite. Persisted in the keystore (not a file)
/// while it is active, because it contains the code.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Invite {
    pub invite_id: [u8; 16],
    /// Canonical code (`w-w-w-w`).
    code: String,
    /// Unix seconds.
    pub expires_at: u64,
    pub attempts_left: u8,
    /// Name the owner gave the device being paired (`pair --name`).
    pub device_name: Option<String>,
    /// The agent key this invite was made for (`None` in tests). A running
    /// agent with another key ignores it (M1).
    pub agent: Option<EndpointId>,
}

impl Drop for Invite {
    fn drop(&mut self) {
        self.code.zeroize();
    }
}

impl fmt::Debug for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Invite")
            .field("invite_id", &crate::hex(&self.invite_id))
            .field("code", &crate::REDACTED)
            .field("expires_at", &self.expires_at)
            .field("attempts_left", &self.attempts_left)
            .field("device_name", &self.device_name)
            .field("agent", &self.agent)
            .finish()
    }
}

impl Invite {
    /// A new invite valid for `ttl` (capped at [`MAX_TTL`]) from `now`.
    /// Returns the invite and its code (to show once).
    pub fn new(
        ttl: Duration,
        now: u64,
        device_name: Option<String>,
    ) -> Result<(Self, PairingCode)> {
        anyhow::ensure!(ttl <= MAX_TTL, "an invite can last at most 24 h");
        anyhow::ensure!(!ttl.is_zero(), "an invite must last longer than 0 s");
        let code = PairingCode::generate()?;
        let invite = Self {
            invite_id: random_bytes()?,
            code: code.reveal().to_string(),
            expires_at: now.saturating_add(ttl.as_secs()),
            attempts_left: ATTEMPTS,
            device_name,
            agent: None,
        };
        Ok((invite, code))
    }

    /// Binds the invite to the agent key `id`.
    pub fn for_agent(mut self, id: EndpointId) -> Self {
        self.agent = Some(id);
        self
    }

    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    pub(crate) fn code(&self) -> Zeroizing<String> {
        Zeroizing::new(self.code.clone())
    }

    /// The link for this invite on an agent reachable at `addr`.
    pub fn link(&self, addr: EndpointAddr) -> InviteLink {
        InviteLink {
            addr,
            invite_id: self.invite_id,
            expires_at: self.expires_at,
        }
    }

    /// Encoding for the keystore.
    pub fn to_secret_bytes(&self) -> Result<Zeroizing<Vec<u8>>> {
        Ok(Zeroizing::new(postcard::to_stdvec(self)?))
    }

    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self> {
        let invite: Self = postcard::from_bytes(bytes).context("stored invite is damaged")?;
        PairingCode::parse(&invite.code).context("stored invite is damaged")?;
        Ok(invite)
    }
}

/// What the client needs to find the agent for pairing. Not secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteLink {
    pub addr: EndpointAddr,
    pub invite_id: [u8; 16],
    /// Unix seconds (informational; the agent enforces it).
    pub expires_at: u64,
}

impl fmt::Display for InviteLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = postcard::to_stdvec(self).map_err(|_| fmt::Error)?;
        let text = BASE32_NOPAD.encode(&bytes).to_ascii_lowercase();
        write!(f, "{LINK_PREFIX}{text}")
    }
}

impl FromStr for InviteLink {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let body = s
            .trim()
            .strip_prefix(LINK_PREFIX)
            .context("not a pigeon invite link (it starts with `pigeon-invite-`)")?;
        anyhow::ensure!(body.len() <= 4096, "invite link is too long");
        let bytes = BASE32_NOPAD
            .decode(body.to_ascii_uppercase().as_bytes())
            .context("invite link is not valid base32")?;
        postcard::from_bytes(&bytes).context("invite link is corrupted")
    }
}

// ---- Frames (postcard, length-prefixed, at most MAX_PAIR_FRAME bytes) ----

/// Client -> agent, first frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairStart {
    pub version: u32,
    pub invite_id: [u8; 16],
    pub msg_a: Vec<u8>,
}

/// Client -> agent, after [`AgentMsg::Reply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairConfirm {
    pub mac: [u8; 32],
}

/// Agent -> client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentMsg {
    /// SPAKE2 message B.
    Reply { msg_b: Vec<u8> },
    /// Key confirmation from the agent: pairing done.
    Ok { mac: [u8; 32] },
    /// The code was wrong.
    Failed { attempts_left: u8 },
    /// No usable invite (missing, expired, used up), busy, or bad frame.
    Refused(String),
}

// ---- Key schedule ----

/// The RFC 5705 exporter value of this TLS session, for `invite_id`.
pub fn channel_binding(conn: &Connection, invite_id: &[u8; 16]) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, invite_id)
        .map_err(|e| anyhow::anyhow!("TLS exporter failed: {e:?}"))?;
    Ok(out)
}

/// The two confirmation keys (client, agent) from the SPAKE2 key and the
/// channel binding.
pub fn confirm_keys(spake_key: &[u8], cb: &[u8; 32]) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let hk = Hkdf::<Sha256>::new(Some(cb), spake_key);
    let mut okm = Zeroizing::new([0u8; 64]);
    hk.expand(CONFIRM_INFO, &mut okm[..])
        .expect("64 bytes is a valid HKDF-SHA256 length");
    let mut client = Zeroizing::new([0u8; 32]);
    let mut agent = Zeroizing::new([0u8; 32]);
    client.copy_from_slice(&okm[..32]);
    agent.copy_from_slice(&okm[32..]);
    (client, agent)
}

/// `T = invite_id || A || C || msg_a || msg_b` (fixed-length parts).
pub fn transcript(
    invite_id: &[u8; 16],
    agent: &EndpointId,
    client: &EndpointId,
    msg_a: &[u8],
    msg_b: &[u8],
) -> Vec<u8> {
    let mut t = Vec::with_capacity(16 + 32 + 32 + msg_a.len() + msg_b.len());
    t.extend_from_slice(invite_id);
    t.extend_from_slice(agent.as_bytes());
    t.extend_from_slice(client.as_bytes());
    t.extend_from_slice(msg_a);
    t.extend_from_slice(msg_b);
    t
}

fn hmac_for(key: &[u8; 32], label: &[u8], t: &[u8]) -> Hmac<Sha256> {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(label);
    mac.update(t);
    mac
}

/// HMAC-SHA256(key, label || t).
pub fn confirm_mac(key: &[u8; 32], label: &[u8], t: &[u8]) -> [u8; 32] {
    hmac_for(key, label, t).finalize().into_bytes().into()
}

/// Constant-time check of [`confirm_mac`].
pub fn verify_mac(key: &[u8; 32], label: &[u8], t: &[u8], mac: &[u8; 32]) -> bool {
    hmac_for(key, label, t).verify_slice(mac).is_ok()
}

pub(crate) const CLIENT_LABEL: &[u8] = b"client";
pub(crate) const AGENT_LABEL: &[u8] = b"agent";

fn identities(agent: &EndpointId, client: &EndpointId) -> (Identity, Identity) {
    (
        Identity::new(client.as_bytes()),
        Identity::new(agent.as_bytes()),
    )
}

/// Agent side, step 4-5: answers `msg_a` for `code`. Returns `msg_b` and the
/// SPAKE2 key, or `None` if `msg_a` is malformed.
pub(crate) fn agent_spake(
    code: &str,
    agent: &EndpointId,
    client: &EndpointId,
    msg_a: &[u8],
) -> Option<(Vec<u8>, Zeroizing<Vec<u8>>)> {
    if msg_a.len() != SPAKE_MSG_LEN {
        return None;
    }
    let (id_a, id_b) = identities(agent, client);
    let (state, msg_b) =
        Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &id_a, &id_b);
    let key = state.finish(msg_a).ok()?;
    Some((msg_b, Zeroizing::new(key)))
}

/// What a successful pairing tells the client.
#[derive(Debug, Clone)]
pub struct Paired {
    /// The agent's verified endpoint ID.
    pub agent: EndpointId,
}

/// Client side: pairs with the agent in `link` using `code`.
pub async fn pair(endpoint: &Endpoint, link: &InviteLink, code: &PairingCode) -> Result<Paired> {
    tokio::time::timeout(TOTAL_TIMEOUT, pair_inner(endpoint, link, code))
        .await
        .context("pairing timed out")?
}

async fn step<T>(fut: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(STEP_TIMEOUT, fut)
        .await
        .context("pairing step timed out")?
}

async fn pair_inner(endpoint: &Endpoint, link: &InviteLink, code: &PairingCode) -> Result<Paired> {
    let conn = endpoint
        .connect(link.addr.clone(), crate::PAIR_ALPN)
        .await
        .context(
            "could not pair: either `pigeon-agent share` is not running, or this invite \
             expired, was already used, or was replaced. Make a new one with `pigeon-agent pair`",
        )?;
    let result = pair_on(&conn, endpoint.id(), link, code).await;
    conn.close(0u32.into(), b"done");
    result
}

async fn pair_on(
    conn: &Connection,
    me: EndpointId,
    link: &InviteLink,
    code: &PairingCode,
) -> Result<Paired> {
    // iroh's TLS handshake authenticated the agent's key; it must be the one
    // in the link (it is, since we dialed that ID, but check anyway).
    let agent = conn.remote_id();
    anyhow::ensure!(agent == link.addr.id, "connected to the wrong agent");
    let cb = channel_binding(conn, &link.invite_id)?;
    let (mut send, mut recv) = conn.open_bi().await?;

    let (id_a, id_b) = identities(&agent, &me);
    let (state, msg_a) =
        Spake2::<Ed25519Group>::start_a(&Password::new(code.reveal().as_bytes()), &id_a, &id_b);
    let start = PairStart {
        version: PAIR_VERSION,
        invite_id: link.invite_id,
        msg_a: msg_a.clone(),
    };
    step(write_frame(&mut send, &start)).await?;
    let msg_b = match step(read_frame_max::<_, AgentMsg>(&mut recv, MAX_PAIR_FRAME)).await? {
        AgentMsg::Reply { msg_b } if msg_b.len() == SPAKE_MSG_LEN => msg_b,
        AgentMsg::Refused(why) => bail!("the agent refused: {}", crate::display_safe(&why)),
        other => bail!("unexpected pairing reply: {other:?}"),
    };
    let key = Zeroizing::new(
        state
            .finish(&msg_b)
            .map_err(|e| anyhow::anyhow!("pairing failed: {e:?}"))?,
    );
    let (kc_client, kc_agent) = confirm_keys(&key, &cb);
    let t = transcript(&link.invite_id, &agent, &me, &msg_a, &msg_b);
    let confirm = PairConfirm {
        mac: confirm_mac(&kc_client, CLIENT_LABEL, &t),
    };
    step(write_frame(&mut send, &confirm)).await?;
    send.finish()?;
    match step(read_frame_max::<_, AgentMsg>(&mut recv, MAX_PAIR_FRAME)).await? {
        AgentMsg::Ok { mac } => {
            anyhow::ensure!(
                verify_mac(&kc_agent, AGENT_LABEL, &t, &mac),
                "the agent could not prove it knows the code; not paired"
            );
            Ok(Paired { agent })
        }
        AgentMsg::Failed { attempts_left: 0 } => {
            bail!("wrong pairing code; the invite is now used up. Run `pigeon-agent pair` again")
        }
        AgentMsg::Failed { attempts_left } => {
            bail!("wrong pairing code; attempts left: {attempts_left}")
        }
        AgentMsg::Refused(why) => bail!("the agent refused: {}", crate::display_safe(&why)),
        other => bail!("unexpected pairing reply: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    #[test]
    fn wordlist_is_the_eff_short_list_without_yo_yo() {
        let words = wordlist();
        assert_eq!(words.len(), 1295);
        let mut sorted = words.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), words.len(), "duplicate words");
        assert!(
            words
                .iter()
                .all(|w| w.chars().all(|c| c.is_ascii_lowercase()))
        );
        assert_eq!(words.first(), Some(&"acid"));
        assert_eq!(words.last(), Some(&"zoom"));
    }

    /// N1 (Proof): every example code in the docs must be a real code, or a
    /// user who types it to try things out gets a confusing "typo" error.
    #[test]
    fn example_codes_in_the_docs_are_valid() {
        let docs = [
            include_str!("../../../README.md"),
            include_str!("../../../docs/security/PAIRING.md"),
            include_str!("pair.rs"),
        ];
        let mut found = 0;
        for doc in docs {
            for token in doc.split('`').skip(1).step_by(2) {
                let parts: Vec<&str> = token.split('-').collect();
                let looks_like_code = parts.len() == CODE_WORDS
                    && parts
                        .iter()
                        // Code words have 3-5 letters (`w-w-w-w` is a placeholder).
                        .all(|p| p.len() >= 3 && p.chars().all(|c| c.is_ascii_lowercase()));
                if looks_like_code {
                    found += 1;
                    assert!(
                        PairingCode::parse(token).is_ok(),
                        "example code `{token}` is not made of wordlist words"
                    );
                }
            }
        }
        assert!(
            found >= 3,
            "expected example codes in README, PAIRING.md and pair.rs"
        );
    }

    #[test]
    fn codes_parse_back_in_any_spelling() {
        for _ in 0..50 {
            let code = PairingCode::generate().unwrap();
            assert_eq!(code.reveal().split('-').count(), CODE_WORDS);
            let back = PairingCode::parse(code.reveal()).unwrap();
            assert_eq!(back.reveal(), code.reveal());
            let spaced = code.reveal().replace('-', "  ").to_uppercase();
            assert_eq!(PairingCode::parse(&spaced).unwrap().reveal(), code.reveal());
        }
        assert!(PairingCode::parse("acid acorn acre").is_err());
        assert!(PairingCode::parse("acid acorn acre zoom zone").is_err());
        let err = PairingCode::parse("acid acorn acre zzzsecretzzz").unwrap_err();
        assert!(!format!("{err:#}").contains("zzzsecretzzz"), "{err:#}");
    }

    #[test]
    fn random_below_is_in_range_and_covers_it() {
        let mut seen = [false; 7];
        for _ in 0..500 {
            let r = random_below(7).unwrap();
            seen[r as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn secrets_are_redacted_in_debug() {
        let (invite, code) = Invite::new(DEFAULT_TTL, 1000, Some("Home PC".into())).unwrap();
        let shown = format!("{invite:?} {invite:#?} {code:?} {code:#?}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(!shown.contains(code.reveal()), "{shown}");
        let link = invite.link(EndpointAddr::new(SecretKey::from_bytes(&[2; 32]).public()));
        assert!(!format!("{link:?} {link}").contains(code.reveal()));
    }

    #[test]
    fn invite_ttl_is_bounded_and_expiry_uses_the_given_clock() {
        assert!(Invite::new(MAX_TTL + Duration::from_secs(1), 0, None).is_err());
        assert!(Invite::new(Duration::ZERO, 0, None).is_err());
        let (invite, _) = Invite::new(Duration::from_secs(60), 1000, None).unwrap();
        assert_eq!(invite.attempts_left, ATTEMPTS);
        assert!(!invite.is_expired(1059));
        assert!(invite.is_expired(1060));
    }

    #[test]
    fn invite_roundtrips_through_the_keystore_encoding() {
        let (invite, _) = Invite::new(DEFAULT_TTL, 5, Some("x".into())).unwrap();
        let bytes = invite.to_secret_bytes().unwrap();
        assert_eq!(Invite::from_secret_bytes(&bytes).unwrap(), invite);
        assert!(Invite::from_secret_bytes(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn invite_link_roundtrips_and_holds_no_code() {
        let (invite, code) = Invite::new(DEFAULT_TTL, 5, None).unwrap();
        let addr = EndpointAddr::new(SecretKey::generate().public())
            .with_relay_url("https://relay.example.com".parse().unwrap());
        let link = invite.link(addr);
        let text = link.to_string();
        assert!(text.starts_with("pigeon-invite-"));
        assert_eq!(text.parse::<InviteLink>().unwrap(), link);
        let bytes = BASE32_NOPAD
            .decode(text[LINK_PREFIX.len()..].to_ascii_uppercase().as_bytes())
            .unwrap();
        assert!(
            !bytes
                .windows(code.reveal().len())
                .any(|w| w == code.reveal().as_bytes()),
            "the link must not contain the code"
        );
        assert!("pigeonabc".parse::<InviteLink>().is_err());
    }

    /// Runs the SPAKE2 + key confirmation math for one simulated session and
    /// returns (client confirm MAC, agent's view of it is valid?).
    fn session(
        client_code: &str,
        agent_code: &str,
        cb: [u8; 32],
        agent: EndpointId,
        client: EndpointId,
    ) -> ([u8; 32], Zeroizing<[u8; 32]>, Vec<u8>, bool) {
        let invite_id = [9u8; 16];
        let (id_a, id_b) = identities(&agent, &client);
        let (sa, msg_a) =
            Spake2::<Ed25519Group>::start_a(&Password::new(client_code.as_bytes()), &id_a, &id_b);
        let (msg_b, agent_key) = agent_spake(agent_code, &agent, &client, &msg_a).unwrap();
        let client_key = sa.finish(&msg_b).unwrap();
        let t = transcript(&invite_id, &agent, &client, &msg_a, &msg_b);
        let (kc_client, _) = confirm_keys(&client_key, &cb);
        let mac = confirm_mac(&kc_client, CLIENT_LABEL, &t);
        let (agent_kc_client, _) = confirm_keys(&agent_key, &cb);
        let ok = verify_mac(&agent_kc_client, CLIENT_LABEL, &t, &mac);
        (mac, agent_kc_client, t, ok)
    }

    #[test]
    fn right_code_confirms_and_wrong_code_does_not() {
        let a = SecretKey::from_bytes(&[3; 32]).public();
        let c = SecretKey::from_bytes(&[4; 32]).public();
        assert!(
            session(
                "acid-acorn-acre-zoom",
                "acid-acorn-acre-zoom",
                [1; 32],
                a,
                c
            )
            .3
        );
        assert!(
            !session(
                "acid-acorn-acre-zone",
                "acid-acorn-acre-zoom",
                [1; 32],
                a,
                c
            )
            .3
        );
    }

    /// P-T10: a confirmation MAC from one session is useless in another
    /// (different TLS session = different channel binding, or different
    /// endpoint IDs).
    #[test]
    fn confirm_mac_is_bound_to_session_and_identities() {
        let a = SecretKey::from_bytes(&[3; 32]).public();
        let c = SecretKey::from_bytes(&[4; 32]).public();
        let other = SecretKey::from_bytes(&[5; 32]).public();
        let code = "acid-acorn-acre-zoom";
        let (mac1, _, _, ok1) = session(code, code, [1; 32], a, c);
        assert!(ok1);
        // Replay into a second session with another TLS exporter value.
        let (_, kc2, t2, ok2) = session(code, code, [2; 32], a, c);
        assert!(ok2);
        assert!(!verify_mac(&kc2, CLIENT_LABEL, &t2, &mac1));
        // Replay into a session with a different client identity.
        let (_, kc3, t3, _) = session(code, code, [1; 32], a, other);
        assert!(!verify_mac(&kc3, CLIENT_LABEL, &t3, &mac1));
        // The agent label is not interchangeable with the client label.
        let (mac4, kc4, t4, _) = session(code, code, [1; 32], a, c);
        assert!(!verify_mac(&kc4, AGENT_LABEL, &t4, &mac4));
    }

    #[test]
    fn malformed_spake_message_is_rejected() {
        let a = SecretKey::from_bytes(&[3; 32]).public();
        let c = SecretKey::from_bytes(&[4; 32]).public();
        assert!(agent_spake("acid-acorn-acre-zoom", &a, &c, &[0u8; 5]).is_none());
        assert!(agent_spake("acid-acorn-acre-zoom", &a, &c, &[0u8; 64]).is_none());
    }
}
