# Pigeon threat model

Owner: Warden. Status: **pre-release, not audited**. Covers the code on `main`
as of Day 2 (bearer-ticket auth) plus the week-2 pairing plan
([PAIRING.md](PAIRING.md)). Requirements live in
[REQUIREMENTS.md](REQUIREMENTS.md) (SEC-xx). Update this file whenever the
protocol, auth, storage or relay usage changes.

Pigeon moves files **out of a work computer**. A security mistake here means a
stranger reads someone's work files. That is the worst failure the product can
have, so the default answer to "should a peer get access?" is no.

## 1. System in one picture

```mermaid
flowchart LR
  subgraph Office["Office network (untrusted LAN, employer IT)"]
    subgraph Mac["Work Mac (trusted: owner's user account)"]
      Share[("Shared folder<br/>(allow-listed, read-only)")]
      Agent["pigeon-agent<br/>Ed25519 endpoint key"]
      Agent -- "guard::resolve()" --> Share
    end
  end
  subgraph Internet["Internet (untrusted)"]
    Relay["iroh relay (n0 public, third party)<br/>sees: endpoint IDs, IPs, timing, sizes<br/>cannot read: QUIC/TLS payload"]
    Disc["n0 address lookup (DNS/pkarr)"]
  end
  subgraph Home["Home network"]
    subgraph PC["Home PC (trusted: owner's user account)"]
      CLI["pigeon CLI / app<br/>Ed25519 endpoint key"]
      Copy[("Fetched copies<br/>+ .part files")]
      CLI --> Copy
    end
  end
  Agent -- "outbound only<br/>QUIC + TLS 1.3" --> Relay
  CLI -- "outbound only" --> Relay
  Agent <-. "direct QUIC if hole punching works" .-> CLI
  Agent --> Disc
  CLI --> Disc
  Ticket{{"Ticket / pairing invite<br/>(copy-paste, QR, email-to-self)"}}
  Agent -. "printed once" .-> Ticket -. "pasted" .-> CLI
```

**Trust boundaries** (every arrow that crosses one is checked):

| # | Boundary | What crosses it | Check today |
|---|---|---|---|
| B1 | Network peer -> agent | iroh connection, Hello, Request | iroh TLS (peer key), token, frame caps, path guard |
| B2 | Agent -> file system | requested path | `guard::resolve` canonicalize + `starts_with(root)` |
| B3 | Agent -> client | Response, file bytes, names, errors | iroh TLS pins agent key from ticket; size/offset checks; SHA-256; `display_safe` |
| B4 | Human channel | ticket text (chat, email, shell, QR) | none: bearer secret (week 2 replaces it) |
| B5 | Relay / discovery | encrypted packets, endpoint IDs, IPs | iroh end-to-end encryption; relay is untrusted |
| B6 | Build / supply chain | crates, GitHub Actions, binaries | `Cargo.lock --locked`, pinned crate versions, cargo-deny, SHA-pinned actions |

## 2. Assets

| Asset | Where | Why it matters |
|---|---|---|
| A1 Work files | Shared folder on the Mac (and everything else on the Mac) | Confidential company data; the main thing to protect |
| A2 Device keys | iroh `SecretKey` per device. Today: random per run, in memory only (`pigeon-agent/src/main.rs:39`, `pigeon-cli/src/main.rs:48`). Week 2: persistent, OS keystore | Whoever holds a paired key *is* that device |
| A3 Tickets / pairing secrets | Today: 32-byte bearer token inside the ticket (`ticket.rs:15-20`), printed to stdout (`pigeon-agent/src/main.rs:56`) | Today: anyone with the ticket reads the whole share until the agent stops |
| A4 Home copies | Fetched files + `.part` files on the home PC | Same sensitivity as A1, now on a less-managed machine |
| A5 Metadata | File names, sizes, mtimes, endpoint IDs, IP addresses, transfer timing | Leaks what the owner works on and when |
| A6 Build integrity | Source, `Cargo.lock`, CI workflows, release binaries | A backdoored binary bypasses every other control |

## 3. Actors

| Actor | Capabilities | Main goal |
|---|---|---|
| T1 Internet attacker | Can reach the agent via relay/discovery if it learns the endpoint ID; can send arbitrary bytes | Read files, crash or exhaust the agent |
| T2 Malicious / compromised relay | Sees and drops/delays all relayed packets, endpoint IDs, IPs | Read or tamper with traffic, MITM a connection, profile users |
| T3 LAN attacker at the office | On-path for direct UDP, can spoof/inject packets, sees DNS | Same as T2 for direct paths; find Pigeon users |
| T4 Stranger with a leaked ticket | Has the ticket text (chat log, email, screenshot, shell history) | Read the share |
| T5 Thief of the home PC / phone | Physical access, maybe unlocked session | Read home copies, reuse device keys / stored tickets |
| T6 Malicious agent | Controls everything the agent sends back | Write outside the download folder, overwrite files, fill disk, exploit parser or terminal |
| T7 Supply-chain attacker | Compromises a crate, a GitHub Action or a CI runner | Backdoor binaries, steal CI secrets |
| T8 Curious employer IT | Admin on the Mac and network; DLP, TLS inspection, EDR | Detect/inspect exfiltration; read keys from the Mac |
| T9 Local user / process on the Mac | Write access to the shared folder (e.g. a synced team folder) | Plant symlinks or race the agent to read outside the share |

T8 is not an attacker in the legal sense: Pigeon must not try to hide from the
employer. The work Mac belongs to them. We document what they can see and do
not promise secrecy against them (see R9).

## 4. Threats and current mitigations (STRIDE)

Status: **M** mitigated, **P** partial, **O** open. File:line refers to the
`security-foundation` branch.

| ID | STRIDE | Threat | Actor | Mitigation in code | Status |
|---|---|---|---|---|---|
| TH-01 | Spoofing | Peer connects without being allowed | T1 | 32-byte random token (`lib.rs:48`), constant-time compare (`agent.rs:79`), rejected before any request is read (`agent.rs:80-86`); test `wrong_token_is_rejected` | M (Day-1 model) |
| TH-02 | Spoofing | Leaked ticket = full access, no expiry, no per-device revoke | T4, T8 | Token and endpoint key are new on every agent start (`pigeon-agent/src/main.rs:39,54`), so stopping the agent revokes everything | **O, High** -> PAIRING.md |
| TH-03 | Spoofing / Tampering | Relay or LAN MITM | T2, T3 | Client dials the endpoint ID from the ticket; iroh TLS 1.3 authenticates that key (`client.rs:83-84`); relay only sees ciphertext | M (relies on iroh) |
| TH-04 | Info disclosure | Path traversal (`..`, absolute, drive, UNC, ADS, NUL) | T1/T4 | `guard.rs:17-41` rejects syntactically; canonicalize + `starts_with` (`guard.rs:43-53`); unit + proof tests + proptest `guard_never_escapes_root` | M |
| TH-05 | Info disclosure | Symlink / junction inside the share points outside | T9 | Canonical path must stay under canonical root (`guard.rs:51`); `list` hides entries whose real path is outside (`agent.rs:128-136`, fixed today); tests `rejects_symlink_escape`, `rejects_junction_escape`, `list_hides_links_that_point_outside_the_share` | M |
| TH-06 | Info disclosure | TOCTOU: component swapped for a symlink between `resolve` and `open` | T9 | None (check and open are separate calls, `agent.rs:165-175`) | O, Low (F9) |
| TH-07 | DoS | Huge frame length / memory exhaustion before auth | T1 | Length checked before allocation (`proto.rs:105`); hello capped at 256 B (`proto.rs:22`); 10 s handshake timeout (`agent.rs:30,72`); proptests on the decoder | M |
| TH-08 | DoS | Many concurrent connections / slow peers; authenticated task waits on `conn.closed()` forever (`agent.rs:113`) | T1, T4 | None beyond per-connection timeout | O, Medium (F3) |
| TH-09 | Tampering | Corrupted or mixed file (stale `.part`, changed source) | T6, network | SHA-256 of the whole file checked (`client.rs:168-179`); offset/size checks (`client.rs:207-218`); more-than-announced bytes refused (`client.rs:240`) | M (P if agent sends `sha256: None`, F10) |
| TH-10 | Elevation (client) | Malicious agent picks the local file name / path | T6 | Local name comes from the *user's* PATH argument, sanitized (`pigeon-cli/src/main.rs:72-105`); never overwrites (`client.rs:120-136`) | M |
| TH-11 | Tampering (client) | Terminal escape injection via file names or error text | T6, T9 | `display_safe` (`lib.rs:28`) on names (`pigeon-cli/src/main.rs:114`) and agent errors (`client.rs:106,220`), fixed today | M |
| TH-12 | DoS (client) | Agent announces a huge size and fills the home disk | T6 | None | O, Low (F10) |
| TH-13 | Info disclosure | Secrets in logs | all | Tickets only on stdout once; `Debug` of `Ticket`/`Hello`/`Share` redacts the token (`ticket.rs:23`, fixed today); CI masks tickets (`cross-network.yml`) | M |
| TH-14 | Info disclosure | Ticket in shell history / process list / clap error echo | T4, T5 | None | O, Medium (F2) |
| TH-15 | Info disclosure | Error messages reveal the Mac's absolute paths | T4 | Peer-facing errors use only the requested relative path; root error has no path (`guard.rs:46`, fixed today) | M |
| TH-16 | Info disclosure | Home copies readable by other local users | T5 | `.part` created `0600` on Unix (`client.rs:229`, fixed today); Windows inherits the folder ACL | P |
| TH-17 | Info disclosure | Metadata to third parties (n0 relay/discovery: IDs, IPs, timing, volume; ticket embeds LAN/public IPs) | T2, T8 | None by design for v1 | O, accepted (R9) |
| TH-18 | Tampering | Backdoored dependency / action / binary | T7 | Exact crate pins + `--locked`; `cargo deny` (advisories, licenses, bans, sources) weekly + per PR; actions pinned to SHAs; minimal `permissions:`; gitleaks | P (binaries unsigned) |
| TH-19 | Repudiation | No record of who fetched what | T4 | Agent logs peer short ID on failure only | O, Low |
| TH-20 | Elevation (Mac) | Agent runs with more rights than needed | T1 | Read-only; only one folder; no listening ports (iroh UDP only); no root needed | M (no sandbox yet) |

## 5. Open risks, ranked

| Rank | Risk | Severity | Plan |
|---|---|---|---|
| R1 | Bearer ticket: anyone who ever saw it reads the share while the agent runs; no per-device revoke (TH-02) | **High** | Week 2 pairing, [PAIRING.md](PAIRING.md). Until then: README tells users to treat the ticket like a password; restart = revoke |
| R2 | Ticket on the command line: PSReadLine/bash history, process list, clap echoes invalid values (TH-14) | Medium | Anvil: `--ticket-file`, `PIGEON_TICKET`, or stdin; disappears with pairing |
| R3 | No connection/concurrency limits; authenticated tasks never time out (TH-08) | Medium | Anvil: semaphore (e.g. 32), per-peer cap, idle timeout after `finish()` |
| R4 | Third-party relay and discovery metadata; employer DLP visibility (TH-17) | Medium (privacy) | Document in README; self-hosted relay option later |
| R5 | Unsigned binaries from CI artifacts | Medium | Sign + notarize at release (owner approval: Apple Developer) |
| R6 | TOCTOU between guard and open (TH-06) | Low | Anvil: open once, verify the handle's real path, hash and stream from the same handle (or `cap-std`) |
| R7 | Malicious agent can fill disk; `sha256: None` accepted (TH-12, TH-09) | Low | Anvil: require a hash, free-space check, `--max-size` |
| R8 | `.part` follows a pre-existing symlink; `rename` can overwrite on Unix if `out` appears mid-transfer | Low | Anvil: `create_new` for fresh `.part`, no-clobber rename |
| R9 | Pigeon is an exfiltration channel from the employer's view | Policy | Not hidden, not a bypass tool. SECURITY.md scope says so. Users must follow their employer's rules |
| R10 | heapless 0.7 unsound advisories via postcard; chrono unmaintained via iroh | Low | Ignored in `deny.toml` with reason + review date |

## 6. Assumptions

- The owner's user accounts on the Mac and the home PC are not compromised.
  If they are, the attacker already has the files and keys (out of scope).
- iroh 1.5's TLS 1.3 handshake authenticates both endpoints' Ed25519 keys and
  the relay cannot decrypt traffic. We rely on iroh/rustls for this and do not
  re-implement it (no custom crypto).
- The OS RNG (`getrandom`) is sound.
