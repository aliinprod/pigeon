# Security requirements

Owner: Warden. Anvil and Canvas must meet these; Proof and Warden check them
on every PR. Each requirement is testable and names the test that proves it.
Status: **met** / **partial** / **not yet**. Test paths are relative to
`crates/`. "e2e" = `pigeon-core/tests/e2e.rs`, "props" =
`pigeon-core/tests/props.rs`.

A PR that weakens a **met** requirement, or removes its test, is blocked.

## Access control

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-01 | No request is served before the peer is authenticated. An unauthenticated peer can only send one Hello of at most 256 bytes within 10 s. | met | e2e `wrong_token_is_rejected`, `oversized_hello_from_stranger_is_refused_and_agent_keeps_serving`; `proto::tests::hello_fits_and_oversized_frames_are_refused_early` |
| SEC-02 | Access secrets are compared in constant time. | met | code review: `subtle::ConstantTimeEq` in `pigeon-core/src/agent.rs:79` (no timing test; not reliably testable) |
| SEC-03 | Every requested path resolves inside the canonical share root: no `..`, absolute, drive, UNC, `:`/ADS, NUL, or symlink/junction escape, for any input. | met | `guard::tests::*`, `guard::proof_tests::*`, props `guard_never_escapes_root`, `guard_random_text_never_escapes` |
| SEC-04 | Listing never reveals anything (name, size, mtime, type) about a path outside the root, including link targets. | met | `agent::tests::list_hides_links_that_point_outside_the_share` |
| SEC-05 | Frame lengths are checked before allocation; no decoder input can panic or allocate more than the frame cap. | met | props `frame_decoder_never_panics`, `frame_body_fuzz_never_panics`, `oversized_prefix_is_refused` |
| SEC-06 | Ticket / invite parsing never panics on any input. | met | props `ticket_parse_never_panics`, `ticket_base32_fuzz_never_panics`, `ticket_roundtrip` |
| SEC-07 | Share is read-only: the protocol has no write, delete or rename request. | met | code review: `proto::Request` has only `List` and `Get`; TODO test once write support is ever discussed |
| SEC-08 | Only paired devices can connect: the agent checks the peer's iroh endpoint ID against its allow-list before accepting any stream; revoking a device closes its live connections and refuses new ones. | not yet | TODO tests from [PAIRING.md](PAIRING.md) (P-T1..P-T10) |

## Secrets and logging

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-09 | Secrets (tokens, pairing codes, private keys) never appear in `Debug`/`Display` of structs, logs or error messages. | partial | `ticket::tests::debug_never_prints_the_token` (Ticket, Hello, Share). TODO: pairing types; a log-capture test once logging exists |
| SEC-10 | Errors sent to a peer contain no absolute local paths (only the path the peer asked for). | met | `agent::tests::errors_sent_to_the_peer_do_not_contain_the_root_path` |
| SEC-11 | Tickets / invites are never passed on the command line in normal use (no shell history, no process list). | not yet | TODO test: CLI reads ticket from file/stdin/env; pairing removes it |
| SEC-12 | Text from the other side (file names, error messages) is printed with control and bidi characters escaped. | met | `tests::display_safe_neutralises_terminal_escapes` (pigeon-core/src/lib.rs) |

## Transfers and the home PC

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-13 | Every completed download is verified with SHA-256 of the whole file; a mismatch deletes the `.part` and fails. | partial | e2e `transfer_100mb_and_hash_matches`, `stale_part_from_changed_source_is_dropped_not_kept`. Gap: `sha256: None` is accepted (TODO: require it + test) |
| SEC-14 | Fetched files are private to the home user (Unix `0600`; Windows: owner-only ACL). | partial | e2e `transfer_100mb_and_hash_matches` asserts `0600` on Unix (runs on macOS CI). TODO Windows ACL + test |
| SEC-15 | The agent never chooses the local file name or directory; downloads never overwrite an existing file. | partial | `pigeon-cli` `default_out_*` tests, `client::tests::free_path_*`. Gap: no-clobber rename + `create_new` `.part` (TODO) |
| SEC-16 | The client refuses more bytes than announced and offsets beyond the size. | met | code: `client.rs:215,240`; e2e `part_larger_than_source_restarts_from_zero`. TODO: malicious-agent test harness |
| SEC-17 | The client caps a download at a user-visible limit / available disk space. | not yet | TODO test |

## Availability

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-18 | The agent bounds concurrent connections (total and per peer) and times out idle authenticated connections. | not yet | TODO test: N+1 connections, idle client dropped |
| SEC-19 | Pairing attempts are rate-limited: an invite is burned after 3 failed attempts; at most one pairing handshake at a time. | not yet | PAIRING.md P-T4, P-T5 |

## Network and platform

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-20 | The agent opens no listening TCP port; it only uses iroh's outbound UDP/relay connections. | partial | code review (`pigeon-agent/src/main.rs:39-42`). TODO CI check: `lsof -iTCP -sTCP:LISTEN` for the agent PID on macOS |
| SEC-21 | The client authenticates the agent by the endpoint ID from the ticket / pairing (no trust on first use, no relay trust). | met (via iroh) | e2e connects to the ticket's endpoint ID; iroh TLS. TODO: test that a different key at the same address is refused |
| SEC-22 | Device secret keys are stored only in the OS keystore (macOS Keychain, Windows Credential Manager/DPAPI), never in plain files or logs. | not yet | PAIRING.md P-T9 |
| SEC-23 | No custom cryptography: only iroh, rustls, RustCrypto primitives. | met | code review; `deny.toml` bans OpenSSL/native-tls |

## Supply chain and CI

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-24 | Dependencies: exact versions, `Cargo.lock` with `--locked`, no known vulnerabilities, permissive licenses, crates.io only. | met | `security.yml` job `cargo-deny` (`cargo deny --locked check`), weekly + per PR |
| SEC-25 | All third-party GitHub Actions are pinned to full commit SHAs, and every workflow sets minimal `permissions:`. | met | review of `.github/workflows/*.yml`. TODO: automated check (e.g. zizmor) |
| SEC-26 | No secrets in the repository history. | met | `security.yml` job `gitleaks` (full history) |
| SEC-27 | Release binaries are signed (macOS: Developer ID + notarization; Windows: Authenticode) and published with SHA-256 sums. | partial | SHA256SUMS in `binaries.yml`; signing not yet (needs owner approval: paid accounts) |
| SEC-28 | Parsers are fuzzed with coverage guidance. | not yet | proptest covers stable; cargo-fuzz targets (nightly) later |
