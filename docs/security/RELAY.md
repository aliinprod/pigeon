# Relay design: a Pigeon relay instead of n0's free public relay

Owner: Warden. Status: **design review, nothing built, bought or deployed.**
Date: 2026-11-24. Applies to iroh / iroh-relay **1.5.0** (the exact version in
`Cargo.toml`). Related: [THREAT_MODEL.md](THREAT_MODEL.md) (TH-17, TH-33..37,
R4, R17..R19), [REQUIREMENTS.md](REQUIREMENTS.md) (SEC-33..SEC-39).

## 0. Why

Measured on our own transfers: the **relay path ran at ~0.046 MB/s** on n0's
free public relays, the **direct path at 17-21 MB/s**. n0 says its public
relays "rate-limit traffic to prevent abuse", carry no uptime or performance
guarantees, and are meant for development and testing [S2]; its pricing page
lists the free tier as "rate-limited" [S1]. So the slow relay path is a policy
limit, not a protocol limit. It matters because the relay path is exactly what
an office network gets when it blocks outbound UDP (no hole punching), and
that is a common case for work computers. Making direct paths better (option
C) cannot help there.

What does **not** change, whatever we pick:

- The Mac still opens **no ports**. Only the relay server listens. The agent
  and the CLI dial out to it (TCP 443, plus UDP 7842 for address discovery).
- The relay stays **untrusted**. Data is QUIC + TLS 1.3 end to end between the
  two endpoint keys; the client authenticates the agent by the endpoint ID
  stored at pairing (SEC-21). A relay can drop, delay and observe metadata.
  It cannot read, change or forge traffic, and it cannot pair.

## 1. How iroh uses relays (from the 1.5.0 source)

Facts the design depends on, each checked in the crate source in
`~/.cargo/registry/src/*/iroh-1.5.0` and `iroh-relay-1.5.0`:

| Fact | Where |
|---|---|
| Each endpoint picks **one home relay**: the relay in its relay map with the best recent latency. There is **no priority order**; "ours first, n0 as fallback" cannot be expressed in a relay map. | `iroh/src/net_report.rs` `add_report_history_and_set_preferred_relay` |
| A peer is reached through **its** home relay, which it publishes in its address record (pkarr). The dialer connects to that relay URL even if it is not in its own relay map. | `iroh/src/socket/transports/relay/actor.rs` `start_active_relay`; [S4] |
| An auth token is sent only to relays that are **in the dialer's own relay map** (`RelayConfig::auth_token`). A token-protected relay that is not in the map is dialed without it and refuses. | `actor.rs` `start_active_relay` (`relay_map.get(&url).and_then(|c| c.auth_token)`) |
| The token travels as `Authorization: Bearer` on native targets (`?token=` only under Wasm). | `iroh-relay/src/client.rs` |
| Relay map can change at run time: `Endpoint::insert_relay` / `remove_relay`; liveness via `home_relay_status()`. | `iroh/src/endpoint.rs` |
| The relay authenticates each client's endpoint ID (signature over a server challenge or a TLS exporter value). So access rules keyed on endpoint ID are sound. | `iroh-relay/src/protos/handshake.rs` |
| Relay server access modes: `everyone` (default), `allowlist` / `denylist` of endpoint IDs, `shared_token`, `http` callout. | `iroh-relay/src/main.rs` `AccessConfig`, README |
| Limits: `accept_conn_limit` / `accept_conn_burst` (new connections), per-client `rx` token bucket (`bytes_per_second`, `max_burst_bytes`). **All off by default.** | `main.rs` `Limits` |
| Ports: HTTP `[::]:80` (captive-portal probe), HTTPS `443` (the relay itself), QUIC address discovery UDP **7842** (needs TLS, `enable_quic_addr_discovery`, default **false**), metrics `9090`, **enabled by default on all interfaces** (`[::]:9090`). | `iroh-relay/src/defaults.rs`, `main.rs` (the README's "7824" is a typo; the code says 7842) |
| TLS: `cert_mode = "LetsEncrypt"` (ACME TLS-ALPN-01 on 443, needs `hostname` + `contact`), `Manual`, `Reloading`. | `main.rs` `CertMode`; `server/http_server.rs` |
| Logging: `tracing` with `EnvFilter::from_default_env()`; default is errors only. At `debug` it logs client IP:port per connection; at `trace` short endpoint IDs. | `main.rs`, `server.rs`, `server/http_server.rs`, `server/clients.rs` |
| Address lookup (pkarr) is separate from the relay. Records are signed by the endpoint key, so a lookup server can withhold or serve stale records but cannot forge one. Pigeon today publishes to and resolves from n0's DNS (`presets::N0` on the agent; `PkarrResolver::n0_dns()` + `DnsAddressLookup::n0_dns()` in `client::endpoint_builder`). | `iroh/src/endpoint/presets.rs`, `crates/pigeon-core/src/client.rs` |

## 2. Options and cost

Bandwidth rule of thumb: a relayed file of N GB costs about **N GB of relay
egress** (plus the same as ingress, usually free). Prices below are list
prices seen on 2026-11-24, excluding VAT; the owner must re-check them on the
provider's page before buying.

| | A1. Hetzner Cloud (CX23 / CAX11) | A2. Oracle Cloud Always Free (A1 Flex) | A3. Fly.io | B. n0 Pro (shared) | B'. n0 Dedicated | C. Keep public relay, improve direct |
|---|---|---|---|---|---|---|
| Monthly cost | **about EUR 5.5** (CX23 EUR 5.49 after the 15 June 2026 increase, per third-party lists [S6][S7]; check, the CX line was intermittently not orderable in autumn 2026 [S6]) | **0** | ~USD 2 VM + **USD 2 dedicated IPv4** (needed for UDP) + egress [S10] | **USD 19** [S1] | USD 19 + **199 per region** [S1] | 0 |
| Egress included | 20 TB/month in EU, then EUR 1/TB [S8] | 10 TB/month per tenancy [S9] | none on current plans; USD 0.02/GB NA/EU, 0.04 APAC, 0.12 India/Africa [S10] | 100 GB, then USD 0.09/GB [S1] | 250 GB, then USD 0.09/GB [S1] | n/a |
| Throughput cap | none from the relay (we set it) | none from the relay | none from the relay | **5 MB/s** [S1] | none [S1] | stays ~0.046 MB/s on relay |
| Regions near the owner (Uzbekistan) | EU: Falkenstein, Nuremberg, Helsinki (also US, Singapore with smaller traffic) | Home region chosen at sign-up only [S9] (e.g. Frankfurt, Middle East, India) | Several; no Central Asia | US, EU, Singapore [S1] | custom | n/a |
| Ops work | ours (patching, certs, monitoring) | ours, plus Oracle risks below | ours, plus Fly UDP specifics | none | none | none |
| Risks | price rises (Hetzner +38% in 2026); account verification | **idle reclamation**: Oracle may reclaim an Always Free VM whose 95th-percentile CPU, network and memory stay under 20% for 7 days [S9], which a quiet relay will; free A1 is now 2 OCPU / 12 GB [S9]; A1 capacity often unavailable; needs a card that Oracle accepts | UDP only on a dedicated IPv4 with `fly-global-services` binding; whether QUIC address discovery sees the real client address through Fly's proxy is **not verified** | metadata goes to n0; 5 MB/s cap is 4x below our direct path | cost | does not fix UDP-blocked offices at all |

Expected throughput of a self-hosted relay: **not measured yet.** The relayed
path is QUIC packets inside a WebSocket over TLS over TCP, so it is bounded
by the TCP connection between each endpoint and the relay (RTT, loss) and by
the VPS's network share, not by iroh's protocol (64 KiB packets,
`MAX_PACKET_SIZE`). On a small VPS a few MB/s to tens of MB/s is plausible;
we only claim a number after the hourly test in section 6 (owner step O2).

Option C (better direct path) is still worth doing because roughly 9 in 10
network setups allow a direct connection [S2], but it is not a substitute: a
UDP-blocked office only ever gets the relay path.

## 3. Threat model for running our own relay

### 3.1 What our relay sees and does not see

| Sees (metadata) | Does not see |
|---|---|
| Public IP:port of every agent and client that connects | File names, paths, contents, sizes of individual files, listing results |
| Endpoint ID of every connected device (authenticated by signature, so it is a stable, real identifier) | Pairing codes or SPAKE2 traffic in the clear |
| Which endpoint IDs exchange packets, when, for how long, how many bytes (traffic analysis: "Mac X sent 2 GB to PC Y at 21:00") | Private keys, tokens of the user's devices |
| Its QUIC address-discovery answers (it tells each endpoint its observed address) | |

The difference from today is **who** holds this metadata: today n0, after
the change **the Pigeon project itself**. That makes us a data controller for
IP addresses (personal data under GDPR and similar laws), a target for legal
requests, and responsible for what we keep. The design answer is to keep
almost nothing (3.4).

### 3.2 New threats (added to THREAT_MODEL.md as TH-33..TH-37)

| ID | Threat | Exploit scenario | Severity | Mitigation | Requirement |
|---|---|---|---|---|---|
| TH-33 | **Freeloading / abuse of an open relay** | Our relay URL is in a public repo. Any iroh app (or a script with a fresh key) uses it as a free relay: our egress bill grows, our IP gets abuse complaints for traffic we cannot read. | Medium | `access.shared_token` with a Pigeon token (filters generic iroh apps that do not send it; rotatable); per-client `rx` rate limit; `accept_conn_limit`; provider billing alert; egress cap via the provider's included traffic. The token is **public** (it ships in open-source binaries) and no security property may depend on it. | SEC-36, SEC-38 |
| TH-34 | **Relay outage = Pigeon outage on UDP-blocked networks** | VPS down, cert expired, provider suspends the account, or a DoS. Agents whose home relay is ours become unreachable for clients that cannot go direct. | Medium | Agent watchdog adds n0's relays at run time when our relay is not connected (`insert_relay`), removes them when ours is healthy again; external `/healthz` check; ACME renewal monitored. Users who opt out of fallback accept the outage. | SEC-37 |
| TH-35 | **Relay server compromise** (SSH, OS bug, iroh-relay bug) | Attacker roots the VPS. Gets: all metadata from now on, ability to drop or delay traffic, to lie in QUIC address discovery (breaks hole punching), to serve its own TLS cert for the relay name. Does **not** get: file contents, the ability to impersonate a Mac to a PC or vice versa (endpoint-key TLS, SEC-21), pairing. | Medium (privacy + availability, no confidentiality loss) | SSH keys only, no root login, provider firewall with only 22/80/443 tcp + 7842 udp; unattended security updates; iroh-relay as an unprivileged systemd service with `CAP_NET_BIND_SERVICE` only; same pinned iroh version as the clients, upgraded within 7 days of an advisory; nothing valuable stored on the box (no logs worth stealing, 3.4). | SEC-35, SEC-36 |
| TH-36 | **Domain loss / takeover** | `flypigeon.app` lapses or the registrar account is phished. The new holder points `relay.flypigeon.app` at their server, gets a valid certificate, and every shipped binary connects to it: metadata of all users, and a full relay DoS. | Medium | Register for several years with auto-renew; registrar 2FA and transfer lock; CAA record limiting issuance to Let's Encrypt; the relay hostname is also overridable in config so a fixed build can move without the domain. | owner checklist (O1) |
| TH-37 | **Malicious relay configured by the user** (custom relay, social engineering: "set your relay to relay.fast-pigeon.example") | The attacker's relay sees the user's IPs, IDs and timing; can DoS. Cannot read files (same as TH-35). | Low | Custom relays only from the owner's local config/CLI, never from an invite link or a peer (extends W-6); `https` only; `pigeon relay` prints the relay in use; docs say what a relay operator can see. | SEC-33, SEC-34 |

Existing threats that change: **TH-17** (metadata to third parties) now names
the Pigeon relay operator as the main holder, with n0 still seeing address
lookups (pkarr) and fallback traffic; **TH-03** is unchanged (still relies on
iroh TLS, relay untrusted); **TH-28** is unchanged (same "newest connection
per endpoint ID wins" logic in iroh-relay `server/clients.rs`).

### 3.3 Access control decision

| Mode | Gives | Problem for Pigeon |
|---|---|---|
| `everyone` | simplest | TH-33 open relay |
| `allowlist` of endpoint IDs | strong | every install makes its own key; we cannot list them without a sign-up service |
| `http` callout | per-ID decisions (e.g. paid accounts later) | needs an auth service we must run and secure; per-connection dependency |
| **`shared_token`** | filters every iroh app that does not know our token; rotate by config | token is public; it is a **speed bump, not authentication**; rotation needs a new client build (keep two tokens during rotation; the server accepts a list) |

**Decision: `shared_token` + rate limits for v1.** Revisit `http` callout only
if abuse shows up or if accounts arrive. Clients send the token only to our
relay (it is set per `RelayConfig`, never on n0's relays). The token value
lives in source as a named constant with a comment that it is public, and in a
gitleaks allow rule so secret scanning does not cry wolf; the real secrets on
the server are only its TLS key and SSH host key.

### 3.4 Logging and data retention policy (the relay, not the apps)

- `RUST_LOG=warn` in the systemd unit. `debug` and `trace` log IP:port and
  endpoint IDs and are **never** enabled in production; for a debugging
  session, at most 1 hour, then off, and the journal for that window rotated
  out.
- journald: `MaxRetentionSec=7day`, `Storage=persistent` capped
  (`SystemMaxUse=200M`). No reverse proxy, no HTTP access logs (iroh-relay
  terminates TLS itself, so there is no nginx/Caddy in front).
- Metrics (`9090`) bound to `127.0.0.1` only; they are aggregate counters, but
  must not be on the internet. If we scrape them later, over SSH or WireGuard.
- Provider-side: no flow logs / VPC logs turned on.
- Published in `SECURITY.md` / the privacy note: what the relay sees, that we
  keep warn-level logs for 7 days, that we cannot read files.

### 3.5 Keys, certificates, updates, monitoring

- **TLS**: Let's Encrypt via iroh-relay's built-in ACME (TLS-ALPN-01 on 443),
  `prod_tls = true`, `cert_dir` under the service's `StateDirectory` (owner-only).
  CAA record `0 issue "letsencrypt.org"`. Certificate Transparency monitoring
  of `flypigeon.app` (free CT alerting service, or a weekly scheduled check)
  detects a mis-issued cert (TH-36).
- **No long-lived secret is valuable** except the TLS key (renewed every ~60 days)
  and SSH keys. Nothing is stored in GitHub secrets for v1 (deploys are manual
  by the owner); if CI deploys later, that is a new design review.
- **Binary**: `cargo install iroh-relay --version =1.5.0 --locked --features server`
  (crates.io source, checksum-verified, published `Cargo.lock`), the same
  version as the clients. `cargo audit` of that lockfile in `security.yml`
  (weekly). Upgrade relay and clients together; iroh 1.x keeps the relay
  protocol compatible within the major version, but we test before switching.
- **OS**: Debian 12 / Ubuntu 24.04 LTS, `unattended-upgrades` for security,
  SSH key-only, `PermitRootLogin no`, provider firewall + host firewall.
- **Monitoring**: external `/healthz` check every 5 minutes (a scheduled
  GitHub Actions job with no secrets is enough: `curl --fail
  https://relay.flypigeon.app/healthz`), certificate expiry check, provider
  billing / traffic alert at 50% of included egress.

## 4. Client configuration

### 4.1 Default

| Role | Relay map | Why |
|---|---|---|
| **Agent (Mac)** | **Pigeon relays only** (with token and `quic` port 7842). Watchdog: if no home relay is connected for 30 s, `insert_relay` the n0 default relays; when a Pigeon relay has been connected for 5 min, remove them again. | Home relay choice is latency-based (section 1), so a mixed map would often pick n0 and stay slow. The agent's home relay is the one that matters: clients reach the agent through it. |
| **Client (home PC)** | **Pigeon relays + n0 defaults** | The client never publishes (D9 / TH-28), so its own home relay barely matters; it needs our relay **in its map** so the token is sent when it dials an agent homed on ours, and n0 so it can still reach an agent that fell back. |
| Address lookup | unchanged: agent publishes to n0 DNS (pkarr), client resolves from n0 DNS | Signed records; n0 sees lookups, not traffic. Self-hosting `iroh-dns-server` is a possible later step (R19), not needed for speed. |

With a single Pigeon relay region, users far from it get a slower but still
uncapped relay; adding a region later is just one more entry in the built-in
list, and iroh's latency selection then picks the nearest Pigeon relay.

### 4.2 User-chosen relay (the self-host story)

One setting per computer, stored in `relay.json` in the Pigeon config dir
(owner-only, SEC-29), changed only by a local command:

```
pigeon relay                         # show the relay in use and the mode
pigeon relay use pigeon              # default: Pigeon relays, n0 fallback
pigeon relay use n0                  # n0 public relays only (today's behaviour)
pigeon relay use https://relay.me.example [--token-file F] [--no-fallback]
```

`pigeon-agent` reads the same file (same config dir, one identity per
computer). Rules:

- **https only**; `http://` and IP-literal URLs are refused (SEC-34), except
  in test builds that use iroh's in-process test relay.
- A custom token is read from a file or prompt, never from argv (SEC-11), and
  is stored in the OS keystore, not in `relay.json`. Finding **W-16 (Low)**:
  iroh-relay 1.5.0's `RelayConfig` uses a plain `#[derive(Debug)]`
  (`relay_map.rs`), so `{:?}` of a `RelayConfig`, `RelayMap` or anything
  holding one prints the token. Pigeon must never Debug-print relay
  configs, and must print relays only as their URL host (SEC-38 test).
- Both sides must be able to reach the agent's relay. If the Mac uses a
  custom relay that needs a token, the home PC must be configured with the
  same relay and token, or it cannot dial through it. `pigeon relay` says so.
- Self-hosters run the same `iroh-relay` binary with the config in section 6;
  the README gets a short "Run your own relay" page.

### 4.3 Invite links and hints (W-6)

Invite links carry the agent's relay URL as a hint. Today
`devices::trusted_relay_hint` keeps the hint only if it is an n0 default
relay. New rule (SEC-33): keep it only if it is in **this computer's own
effective relay set** (built-in Pigeon relays, n0 defaults, the user's
configured custom relay). A relay named only by a link is used for the one
pairing connection and never stored. Relay settings never come from a link,
a peer message, or an environment variable in release builds.

### 4.4 Version pinning

`iroh = "=1.5.0"` stays exact in `Cargo.toml`; the server installs exactly
`=1.5.0` too. A version bump PR changes both and is tested against a
`run_relay_server` relay in CI before anyone upgrades the VPS.

## 5. Recommendation

**Self-host `iroh-relay` 1.5.0 on one Hetzner Cloud VM in the EU (about
EUR 5.5/month + a domain), access `shared_token` + rate limits, warn-only
logs kept 7 days, agent homed on our relay with automatic n0 fallback.**

Why not the others: n0 Pro (USD 19) is zero-ops but caps at 5 MB/s, 4x
below our direct path, and keeps metadata with a third party; n0 Dedicated
(USD 218) is too expensive for a pre-revenue project; Oracle Always Free is
free but a quiet relay meets its idle-reclamation rule; Fly.io's UDP path
needs extra parts we have not verified. Option C alone does nothing for
UDP-blocked offices.

If the owner prefers zero operations for now, **n0 Pro is the acceptable
fallback**: 100x faster than today, same code change (only the relay list and
token differ).

### 5.1 Plan for Anvil (code; after this design is approved)

1. `pigeon-core::relays`: `enum RelayChoice { Pigeon, N0, Custom { url, fallback } }`;
   built-in `PIGEON_RELAYS` (URL + QUIC port 7842) and `PIGEON_RELAY_TOKEN`
   (documented as public); `fn relay_map(choice, role) -> RelayMap` per
   section 4.1; https-only URL validation.
2. `relay.json` load/save (owner-only file, atomic write); custom token in the
   keystore; `pigeon relay` / `pigeon relay use ...` commands.
3. Agent: `Endpoint::builder(presets::N0).relay_mode(RelayMode::Custom(map))`
   (keeps pkarr publishing); fallback watchdog on `home_relay_status()` using
   `insert_relay` / `remove_relay`; status file shows the relay host.
4. Client: `client::endpoint_builder(key, map)` with Pigeon + n0.
5. `trusted_relay_hint` takes the effective relay set (SEC-33).
6. Tests (in-process relays via iroh `test-utils` `run_relay_server`, no
   network): relay-only `get` through a token relay matches the SHA-256;
   missing / wrong token is refused; agent falls back to a second relay when
   the first is stopped and returns when it is back; hint filtering;
   URL validation; token never in `Debug`/logs/`relay.json`.
7. Keep the build passing with the Pigeon relay URL as a placeholder until
   the owner has a domain; nothing in CI talks to the real relay.

### 5.2 For the owner (nothing is bought or deployed without you)

| # | Decision / purchase | Cost |
|---|---|---|
| O1 | Buy `flypigeon.app` (or another domain), multi-year, auto-renew, registrar 2FA + transfer lock | about USD 15-20/year for `.app` (check the registrar) |
| O2 | Hetzner account; create a CX23 (or ARM CAX11) **hourly** in Falkenstein/Nuremberg/Helsinki for a 1-day test: we measure RTT from Tashkent and `pigeon get --relay-only` throughput; delete it if the result is poor | cents (hourly billing) |
| O3 | Keep it: VM + DNS `relay.flypigeon.app` A/AAAA + CAA | about EUR 5.5/month |
| O4 | Approve the policy: shared token (public), warn logs kept 7 days, n0 fallback on by default | - |
| O5 | Approve Anvil's code PR (section 5.1) | - |
| Alt | Instead of O2/O3: n0 Pro | USD 19/month, 5 MB/s cap |

**Total for the recommendation: about EUR 5.5/month + about USD 1.5/month
for the domain, i.e. roughly USD 7-8/month.** Egress is covered by the 20 TB
included (about 20,000 GB of relayed files a month).

## 6. Server runbook (for O2/O3, run by the owner or with the owner watching)

Firewall (provider + host): inbound **22/tcp** (SSH, key only), **80/tcp**
(captive-portal probe), **443/tcp** (relay + ACME), **7842/udp** (QUIC
address discovery). Nothing else; 9090 stays closed.

`/etc/iroh-relay/config.toml`:

```toml
enable_relay = true
http_bind_addr = "[::]:80"
enable_quic_addr_discovery = true
enable_metrics = true
metrics_bind_addr = "127.0.0.1:9090"   # default is [::]:9090: never expose

access.shared_token = ["<PIGEON_RELAY_TOKEN, public>"]

[tls]
https_bind_addr = "[::]:443"
quic_bind_addr = "[::]:7842"
cert_mode = "LetsEncrypt"
hostname = "relay.flypigeon.app"
contact = "<owner contact address>"
prod_tls = true
cert_dir = "/var/lib/iroh-relay/certs"

[limits]
accept_conn_limit = 50.0      # new connections per second (starting value)
accept_conn_burst = 200

[limits.client.rx]
bytes_per_second = 25000000   # 25 MB/s per client; above our direct path
max_burst_bytes = 50000000
```

Check every key against `iroh-relay-1.5.0/src/main.rs` before use (the
upstream self-hosting guide says it may lag the code [S3]).

systemd unit essentials: `DynamicUser=yes`, `StateDirectory=iroh-relay`,
`AmbientCapabilities=CAP_NET_BIND_SERVICE`, `CapabilityBoundingSet=CAP_NET_BIND_SERVICE`,
`NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`,
`PrivateTmp=yes`, `Environment=RUST_LOG=warn`, `Restart=on-failure`.

Acceptance checks (all against our own server only): `curl --fail
https://relay.flypigeon.app/healthz`; from outside, 9090 is closed; a
`pigeon get --relay-only` of a 1 GB file reports `route: relay`, the hash
matches, and the speed is recorded in DEVLOG; `journalctl -u iroh-relay`
shows no IP addresses or endpoint IDs.

## 7. Not verified

- Real throughput of a self-hosted relay from Uzbekistan (needs O2).
- Hetzner's current CX23 price and orderability (third-party sources only),
  and whether Hetzner / Oracle accept the owner's payment method and country.
- Fly.io: whether UDP through its proxy preserves the client address for
  QUIC address discovery.
- `.app` domain price at the owner's registrar.
- That iroh-relay 1.5.0 has no advisories beyond what `cargo deny` already
  checks in our lockfile (the server build adds the `server` feature's extra
  crates: `tokio-rustls-acme`, `rcgen`, `tokio-websockets`, ...; to be
  audited in CI with SEC-35).

## Sources

- [S1] n0 / iroh pricing: https://www.iroh.computer/pricing (Community free,
  rate-limited; Pro USD 19, 100 GB, 5 MB/s; Dedicated +USD 199/region, 250 GB)
- [S2] iroh docs, Relays: https://docs.iroh.computer/concepts/relays
- [S3] iroh docs, self-hosted relays: https://docs.iroh.computer/iroh-services/relays/self-hosted.md
- [S4] iroh docs, configuring networks (ports, home relay): https://docs.iroh.computer/configuring-networks.md
- [S5] iroh-relay 1.5.0 and iroh 1.5.0 source (crates.io), files named in section 1
- [S6] Hetzner 2026 pricing summaries (third party): https://www.bitdoze.com/hetzner-cloud-cost-optimized-plans/
- [S7] Hetzner price adjustment (official): https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/
- [S8] Hetzner traffic: https://docs.hetzner.com/robot/general/traffic/
- [S9] Oracle Always Free resources: https://docs.oracle.com/en-us/iaas/Content/FreeTier/resourceref.htm
- [S10] Fly.io pricing: https://fly.io/pricing
