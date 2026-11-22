# Pairing design review (week 2)

Owner: Warden. Implementer: Anvil (core, agent, CLI), Canvas (UI). Status:
**approved design, not implemented**. Replaces the Day-1 bearer token in the
ticket (threat TH-02 / risk R1 in [THREAT_MODEL.md](THREAT_MODEL.md)).
Requirements: SEC-08, SEC-09, SEC-11, SEC-19, SEC-22.

## 1. Goals

1. A device gets access only through a deliberate, one-time pairing on the
   owner's request. After that, access depends on the device's **key**, not
   on any text that can leak.
2. Anything that leaks later (QR photo, email, shell history, screenshot) is
   useless once pairing is done or the invite has expired.
3. The owner can see and revoke each paired device; revocation is immediate.
4. A relay, a LAN attacker or employer IT that sees all pairing traffic
   cannot pair, and cannot brute-force the code offline.
5. No custom crypto: iroh (Ed25519 + TLS 1.3), RustCrypto `spake2`, `hkdf` /
   `hmac` / `sha2`.

Non-goals for week 2: multi-user sharing, per-device folder scopes beyond
"all shared folders, read-only", phone app.

## 2. Building blocks (checked 2026-11-23)

| Need | Choice | Notes |
|---|---|---|
| Device identity | iroh `SecretKey` (Ed25519); public key = `EndpointId` | `Endpoint::builder(..).secret_key(sk)` (iroh 1.5 `endpoint.rs:529`); `SecretKey::to_bytes/from_bytes` (iroh-base 1.5 `key.rs:332-337`, `ZeroizeOnDrop`). iroh's TLS 1.3 handshake authenticates both endpoint keys; `Connection::remote_id()` gives the peer's verified key (`connection.rs:569`). |
| Channel binding | `Connection::export_keying_material` (iroh 1.5 `connection.rs:1115`, RFC 5705 exporter) | Binds the PAKE to this exact TLS session. |
| PAKE | `spake2 = "=0.4.0"` (RustCrypto/PAKEs, released 2026-01-25, ~860k recent downloads), `Ed25519Group`, asymmetric A/B mode with identities | Widely used (magic-wormhole.rs), **not independently audited**. Alternatives looked at: `opaque-ke` 4.0.1 (audited, but an augmented client/server password protocol: more machinery than we need); `cpace` 0.1.0 (unmaintained since 2020, rejected). |
| KDF / MAC | `hkdf`, `hmac`, `sha2` (RustCrypto) | Standard key confirmation, no new constructions. |
| Key storage | `keyring` 4.x (Keychain / Windows Credential Manager) or direct `security-framework` 3.x on macOS and DPAPI via `windows-sys` | Anvil picks one; must pass P-T9. |

## 3. Why two secrets (address vs code)

The owner pairs at the office but uses the home PC later, so the invite has to
travel home. The usual channel (email/chat to self) is readable by employer IT
(T8) and by anyone who later gets the mailbox (T4). So the invite is split:

- **Invite link** (`pigeon-invite...`, safe to send through email/chat): agent
  `EndpointAddr` + 16-byte random `invite_id` + expiry. Contains **no secret**.
- **Pairing code** (shown only on the Mac screen, never sent over the
  network, never logged): 4 words from the EFF short wordlist (1296 words,
  ~41 bits), e.g. `otter-plaza-crumb-vivid`. The owner writes it down or
  photographs the screen with their phone.
- **QR** (for a phone or camera-equipped PC later): link + code together,
  labelled "scan only, do not share". Pairing uses the same protocol.

Because of the PAKE, seeing the link plus all pairing traffic gives no offline
test for the code. Each online guess costs one of 3 attempts, so a stranger's
chance is at most 3 / 1296^4 ~ 1e-12 per invite.

## 4. Recommended protocol (P1)

ALPN `pigeon/pair/1`, accepted **only while an invite is active**. Normal
traffic moves to ALPN `pigeon/1` (no token in `Hello` any more).

**Setup (once per device):** on first start, each device generates an iroh
`SecretKey` with the OS RNG and stores it in the OS keystore (macOS Keychain
item, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, not synced to
iCloud; Windows Credential Manager, user scope, DPAPI-protected). The key is
loaded into memory at start and never written anywhere else.

**Invite (agent, owner runs `pigeon-agent pair [--name "Home PC"] [--ttl 12h]`):**
creates `{invite_id: 16 random bytes, code: 4 random words, expires_at:
now + ttl (default 12 h, max 24 h), attempts_left: 3}` in memory (or in the
keystore if the agent must survive a restart). Prints the link and shows the
code. Only one invite at a time; a new one cancels the old one.

**Handshake (client runs `pigeon pair <LINK>`, then types the code):**

1. Client dials the agent's `EndpointId` from the link with ALPN
   `pigeon/pair/1`. iroh authenticates both keys. Agent reads
   `C = conn.remote_id()`; `A` = agent's own id.
2. Both compute `cb = conn.export_keying_material(32 bytes, label
   "pigeon pair v1", context = invite_id)`.
3. Client -> agent: `PairStart { invite_id, msg_a }` where
   `msg_a` comes from `Spake2::<Ed25519Group>::start_a(password = code,
   id_a = C, id_b = A)`. Frame cap 128 bytes, 10 s timeout.
4. Agent: looks up `invite_id`; refuses if missing, expired, or
   `attempts_left == 0`. **Decrements `attempts_left` before** doing any work.
   Runs `start_b(code, C, A)`, replies `PairReply { msg_b }`.
5. Both: `k = spake.finish(peer_msg)`; derive with HKDF-SHA256(ikm = k,
   salt = cb, info = "pigeon pair v1 confirm") two 32-byte keys `kc_client`,
   `kc_agent`. Transcript `T = invite_id || A || C || msg_a || msg_b`.
6. Client -> agent: `PairConfirm { mac = HMAC-SHA256(kc_client, "client" || T) }`.
   Agent verifies in constant time. On failure: reply `PairFailed { attempts_left }`
   and close; at 0 attempts the invite is deleted.
7. Agent -> client: `PairOk { mac = HMAC-SHA256(kc_agent, "agent" || T), agent_name }`.
   Client verifies. Only now:
   - agent adds `{endpoint_id: C, name, paired_at, last_seen}` to its
     allow-list and **deletes the invite** (single use);
   - client stores `{agent EndpointAddr, agent name}` as a paired agent.
8. Both show a short fingerprint (first 8 base32 chars of the other side's
   `EndpointId`) and "paired". The agent also raises an OS notification.

**Normal sessions (`pigeon/1`):** the agent checks `conn.remote_id()` against
the allow-list **before `accept_bi()`**, and closes the connection otherwise.
No secret is sent in-band. The client dials the stored `EndpointId`, so a
different key at the same address is refused by TLS.

**Revocation:** `pigeon-agent devices` lists paired devices (name,
fingerprint, paired_at, last_seen); `pigeon-agent revoke <fingerprint|name>`
removes the entry and closes that device's live connections (agent keeps a
map `EndpointId -> Vec<Connection>`). A lost home PC is revoked from the Mac.
A lost Mac: the client can `pigeon forget <agent>`; the agent key in the Mac's
keychain is the employer's problem (see T8). `pigeon-agent reset` rotates the
agent key, which invalidates every pairing at once.

**Allow-list storage:** not secret, but integrity matters. JSON file in the
app's config dir (`~/Library/Application Support/Pigeon/devices.json`,
`%APPDATA%\Pigeon\devices.json`), created `0600` / owner-only ACL, written via
temp file + atomic rename. Anyone who can edit it already runs as the owner.

## 5. Rate limiting

| Limit | Value | Why |
|---|---|---|
| Attempts per invite | 3, decremented before the PAKE runs | Bounds online guessing; burned invite = visible DoS at worst |
| Concurrent pairing handshakes | 1 (others refused immediately) | No parallel guessing |
| Pairing ALPN | only registered/accepted while an invite exists | Nothing to attack the rest of the time |
| Invite lifetime | default 12 h, max 24 h | Long enough to get home, short enough to bound leaks |
| Handshake timeout | 10 s per step, 30 s total | Slow-loris |
| Global (normal ALPN) | see SEC-18: e.g. 32 connections, 4 per peer | Unpaired peers are dropped right after the TLS handshake |

## 6. What an attacker who sees the QR / link later can do

| Seen | When | Can do |
|---|---|---|
| Link only (email/chat) | any time | Learns the agent `EndpointId`, relay URL and IPs (metadata). Can try 3 code guesses (~1e-12 success), then the invite is burned. Cannot pair. Cannot connect normally (not on allow-list). |
| QR or link + code | after the owner paired, or after expiry | Nothing: the invite was deleted on success / expired. Metadata as above. |
| QR or link + code | **before** the owner pairs and before expiry | **Can pair as a device** and read the shares. Mitigations: short TTL, single use, the owner's own pairing then fails with "invite already used" (a loud signal), the agent notifies on every new pairing, `pigeon-agent devices` shows it, revoke is one command. Treat the code like a password; show it only on screen. |
| A stored device key (stolen home PC) | any time | Acts as that device until revoked. Keystore protection means a powered-off, encrypted disk does not reveal it; an unlocked session does. Revoke from the Mac. |

## 7. Tests that must exist before merge

| ID | Test | Requirement |
|---|---|---|
| P-T1 | Pair with the right code -> device listed; `get` works with no token on the wire | SEC-08 |
| P-T2 | Unpaired endpoint is refused on `pigeon/1` before any stream is accepted | SEC-08 |
| P-T3 | Wrong code -> `PairFailed`, attempts decrease; agent keeps no partial state | SEC-19 |
| P-T4 | 3 wrong codes -> invite deleted; the 4th try with the right code fails | SEC-19 |
| P-T5 | Second concurrent pairing handshake is refused while one is running | SEC-19 |
| P-T6 | Invite reuse: after a successful pairing, the same link + code fails | goal 2 |
| P-T7 | Expired invite fails (inject a clock) | goal 2 |
| P-T8 | Revoke closes a live transfer within 1 s and blocks reconnects | SEC-08 |
| P-T9 | Device key round-trips through the keystore; no key bytes in any file under the config dir or in logs; `Debug` of key/code types is redacted | SEC-09, SEC-22 |
| P-T10 | Transcript binding: a `PairConfirm` MAC from one session is rejected in another (different `cb` / endpoint IDs) | goal 4 |
| P-T11 | proptest: `PairStart`/`PairConfirm` decoders and invite-link parser never panic | SEC-05, SEC-06 |
| P-T12 | Pairing ALPN is refused when no invite is active | §5 |

## 8. Open points for Anvil

- Persisting an invite across agent restarts is optional; if done, keep it in
  the keystore, not a plain file.
- Version the frames (`PairStart.version`) so v2 can change the PAKE.
- Remove `Hello.token` and the `pigeon/0` ALPN in the same PR; no fallback to
  token auth.
- Wordlist: embed the EFF short wordlist (CC BY 3.0 US, attribution in
  `NOTICE`); select words with `getrandom` + rejection sampling.
