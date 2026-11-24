# Security requirements

Owner: Warden. Anvil and Canvas must meet these; Proof and Warden check them
on every PR. Each requirement is testable and names the test that proves it.
Status: **met** / **partial** / **not yet**. Test files: [e2e], [pairing],
[props] (all in `crates/pigeon-core/tests/`); unit tests are named
`module::tests::name` inside [pigeon-core][core], [pigeon-agent][agentbin]
and [pigeon-cli][clibin].

A PR that weakens a **met** requirement, or removes its test, is blocked.
Week 1 (pairing) renamed two Day-1 tests because the bearer token they tested
is gone: `wrong_token_is_rejected` became
`unpaired_device_is_refused_before_any_stream`, and
`oversized_hello_from_stranger_is_refused_and_agent_keeps_serving` became
`oversized_request_is_refused_and_agent_keeps_serving` (same checks, now
against key-based access).

## Access control

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-01 | No request is served before the peer is authenticated. A peer that is not on the allow-list is closed before any stream is accepted; a paired peer can send one request of at most 64 KiB within 10 s. | met | [e2e] `unpaired_device_is_refused_before_any_stream`, `oversized_request_is_refused_and_agent_keeps_serving`; `proto::tests::requests_fit_and_oversized_frames_are_refused_early` |
| SEC-02 | Access secrets are compared in constant time. | met | code review: pairing MACs are checked with `hmac`'s `verify_slice` (constant time) in `pair::verify_mac`. There is no other shared secret any more (Day-1 token removed). |
| SEC-03 | Every requested path resolves inside the canonical share root: no `..`, absolute, drive, UNC, `:`/ADS, NUL, or symlink/junction escape, for any input. | met | `guard::tests::*`, `guard::proof_tests::*`, [props] `guard_never_escapes_root`, `guard_random_text_never_escapes` |
| SEC-04 | Listing never reveals anything (name, size, mtime, type) about a path outside the root, including link targets. | met | `agent::tests::list_hides_links_that_point_outside_the_share` |
| SEC-05 | Frame lengths are checked before allocation; no decoder input can panic or allocate more than the frame cap. | met | [props] `frame_decoder_never_panics`, `frame_body_fuzz_never_panics`, `oversized_prefix_is_refused` (file and pairing frames) |
| SEC-06 | Invite link / pairing code parsing never panics on any input. | met | [props] `invite_link_parse_never_panics`, `invite_link_base32_fuzz_never_panics`, `invite_link_roundtrip`, `pairing_code_parse_never_panics_or_echoes` |
| SEC-07 | Share is read-only: the protocol has no write, delete or rename request. | met | code review: `proto::Request` has only `List` and `Get`; TODO test once write support is ever discussed |
| SEC-08 | Only paired devices can connect: the agent checks the peer's iroh endpoint ID against its allow-list before accepting any stream; revoking a device closes its live connections and refuses new ones. | met | [pairing] `pairing_with_the_right_code_lists_the_device_and_get_works` (P-T1), `revoke_closes_a_live_transfer_within_a_second` (P-T8, in-process `Agent::revoke`, < 1 ms on loopback), `revoke_through_the_devices_file_closes_a_live_transfer` (the CLI path: `devices.json` edited as `pigeon-agent revoke` does, picked up by the real `agent::watch` loop, transfer closed within 1.5 s; about 0.43 s locally); [e2e] `unpaired_device_is_refused_before_any_stream` (P-T2); `pigeon-agent` `tests::device_paired_keeps_a_concurrent_revoke`, `devices::tests::concurrent_updates_lose_nothing` (locked read-modify-write, W-4) |

## Secrets and logging

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-09 | Secrets (pairing codes, private keys) never appear in `Debug`/`Display` of structs, logs or error messages. | partial | `pair::tests::secrets_are_redacted_in_debug` (`Invite`, `PairingCode`, `InviteLink`), `keystore::tests::secret_debug_is_redacted`, iroh's `SecretKey` Debug is `SecretKey(..)`; code-parse errors never echo input ([props] `pairing_code_parse_never_panics_or_echoes`, cli `tests::code_is_read_from_a_file_and_never_echoed`); `PairingCode` has no `Display`; the CLI keeps the typed code in `Zeroizing` buffers. TODO: a log-capture test over a whole pairing run. Known gaps: `$PIGEON_PAIR_CODE` stays in the process environment, and `spake2` 0.4.0 does not zeroize its copy of the password (PAIRING.md section 9) |
| SEC-10 | Errors sent to a peer contain no absolute local paths (only the path the peer asked for). | met | `agent::tests::errors_sent_to_the_peer_do_not_contain_the_root_path` |
| SEC-11 | Secrets are never passed on the command line in normal use (no shell history, no process list). | met | cli `tests::secrets_are_not_command_line_arguments` (the code has no argv form; `ls`/`get` take no ticket; only the non-secret invite link is an argument), `tests::code_is_read_from_a_file_and_never_echoed` (`--code-file`, `$PIGEON_PAIR_CODE` or prompt) |
| SEC-12 | Text from the other side (file names, error messages) is printed with control and bidi characters escaped. | met | `tests::display_safe_neutralises_terminal_escapes` (pigeon-core/src/lib.rs) |

## Transfers and the home PC

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-13 | Every completed download is verified with SHA-256 of the whole file; a mismatch deletes the `.part` and fails. | partial | [e2e] `transfer_100mb_and_hash_matches`, `stale_part_from_changed_source_is_dropped_not_kept`. Gap: `sha256: None` is accepted (TODO: require it + test) |
| SEC-14 | Fetched files are private to the home user (Unix `0600`; Windows: owner-only ACL). | partial | [e2e] `transfer_100mb_and_hash_matches` asserts `0600` on Unix (runs on macOS CI). Gap: on Windows a fetched file inherits the ACL of the folder it is saved in (TODO: owner-only DACL + test, like SEC-29) |
| SEC-29 | Pigeon's config dir (allow-list, paired agents, spent invites, status) and every file in it are owner-only on every OS, wherever `PIGEON_HOME` points. | met | Unix: `config::tests::write_private_replaces_atomically_and_is_owner_only`, `keystore::tests::file_store_roundtrip_and_owner_only` (`0700`/`0600`); Windows (W-3): `config::tests::windows_config_dir_is_owner_only` (protected DACL, current user + SYSTEM only, even under a parent that grants Authenticated Users Modify, like `C:\`) |
| SEC-15 | The agent never chooses the local file name or directory; downloads never overwrite an existing file. | partial | `pigeon-cli` `default_out_*` tests, `client::tests::free_path_*`. Gap: no-clobber rename + `create_new` `.part` (TODO) |
| SEC-16 | The client refuses more bytes than announced and offsets beyond the size. | met | code: `client::receive`; [e2e] `part_larger_than_source_restarts_from_zero`. TODO: malicious-agent test harness |
| SEC-17 | The client caps a download at a user-visible limit / available disk space. | not yet | TODO test |

## Availability

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-18 | The agent bounds concurrent connections (total and per peer) and times out idle authenticated connections. | met | [e2e] `global_connection_limit_refuses_extra_connections` (32 total by default, refused before the handshake), `per_peer_connection_limit_closes_extra_connections` (4 per peer; since W-8 it no longer assumes which connection the agent closes: exactly one of three must be closed as busy, 10/10 local runs), `idle_client_is_disconnected_after_its_request`, `stalled_reader_is_disconnected` (30 s without progress by default) |
| SEC-19 | Pairing attempts are rate-limited: an invite is burned after 3 failed attempts; at most one pairing handshake at a time; a peer without a valid invite ID cannot hold the pairing lock; one pairing connection per peer. | met | [pairing] `three_wrong_codes_burn_the_invite` (P-T4), `second_concurrent_pairing_is_refused` (P-T5), `wrong_code_costs_an_attempt_and_leaves_no_partial_state` (P-T3), `pairing_alpn_is_refused_without_an_invite` (P-T12), `invite_cannot_be_reused` (P-T6), `expired_invite_is_refused` (P-T7), `stranger_without_invite_id_does_not_block_pairing` and `wrong_invite_id_and_second_connection_per_peer_are_refused` (W-2), `stale_failed_attempt_does_not_burn_a_newer_invite` (W-1), `used_invite_does_not_come_back_after_a_restart` (W-5), `invite_for_another_agent_key_is_ignored` (M1), `client_refuses_an_agent_that_cannot_prove_the_code` (L1) |

## Network and platform

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-20 | The agent opens no listening TCP port; it only uses iroh's outbound UDP/relay connections. | partial | code review (`pigeon-agent` builds one iroh endpoint; no other socket; no IPC socket either, see PAIRING.md D5). TODO CI check: `lsof -iTCP -sTCP:LISTEN` for the agent PID on macOS |
| SEC-21 | The client authenticates the agent by the endpoint ID from pairing (no trust on first use, no relay trust). | met (via iroh) | The client dials the stored `EndpointId`; iroh TLS 1.3 verifies it; pairing additionally checks `conn.remote_id() == link.addr.id` and the agent's confirmation MAC (`pair::tests::confirm_mac_is_bound_to_session_and_identities`, P-T10). TODO: test that a different key at the same address is refused |
| SEC-22 | Device secret keys are stored only in the OS keystore (macOS Keychain, Windows Credential Manager/DPAPI), never in plain files or logs. | met (macOS, Windows) | `keystore::tests::os_keystore_roundtrip_leaves_no_key_on_disk` (P-T9; real Keychain on macOS CI, real Credential Manager on Windows CI; scans the config dir for the key in raw, hex and base32 form), `device_key_is_stable_across_restarts`, `damaged_key_is_an_error_not_a_new_identity`. Exception by design: OSes other than macOS/Windows, or `PIGEON_KEYSTORE=file` on macOS, use `0600` files (`file_store_roundtrip_and_owner_only`; on Linux, ubuntu-latest CI runs `keystore::tests::linux_keystore_is_owner_only_files`: key file `0600`, secrets and config dir `0700`, key stable, nothing else holds it. Follow-up: Secret Service / libsecret on Linux desktops); `PIGEON_KEYSTORE=file` is refused on Windows (W-3); see PAIRING.md D6. Limits: any process running as the same user can read the key without a prompt (Windows Credential Manager) or after one "Always Allow" (macOS), see THREAT_MODEL A2 |
| SEC-23 | No custom cryptography: only iroh, rustls, RustCrypto primitives. | met | code review: pairing uses `spake2`, `hkdf`, `hmac`, `sha2` (RustCrypto) and iroh's TLS exporter; `deny.toml` bans OpenSSL/native-tls |

## Supply chain and CI

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-24 | Dependencies: exact versions, `Cargo.lock` with `--locked`, no known vulnerabilities, permissive licenses, crates.io only. | met | `security.yml` job `cargo-deny` (`cargo deny --locked check`), weekly + per PR |
| SEC-25 | All third-party GitHub Actions are pinned to full commit SHAs, and every workflow sets minimal `permissions:`. | met | `.github/scripts/check-workflows.sh` (SEC-32); pins checked against their tags 2026-11-24 |
| SEC-26 | No secrets in the repository history. | met | `security.yml` job `gitleaks` (full history) |
| SEC-27 | Release binaries are signed (macOS: Developer ID + notarization; Windows: Authenticode) and published with SHA-256 sums. | partial | SHA256SUMS in `binaries.yml`; signing not yet (needs owner approval: paid accounts) |
| SEC-28 | Parsers are fuzzed with coverage guidance. | not yet | proptest covers stable; cargo-fuzz targets (nightly) later |
| SEC-31 | The `/office/` login gate has no credentials or hashes in code, reads only `OFFICE_USER_SHA256` / `OFFICE_PASS_SHA256`, and fails closed (`503`, page never served) when either is missing or malformed; encoded paths to `/office` are gated too. | met | `office/middleware.test.mjs` (`security.yml` job `office-gate`) |
| SEC-32 | Workflows are safe for a public repo: no `pull_request_target` / `workflow_run`, no `${{ github.event.* }}` / `head_ref` interpolation, no `secrets: inherit`, a top-level `permissions:` block, every action pinned to a 40-hex SHA, and no artifact holds a secret. | met (static), pending first run | `.github/scripts/check-workflows.sh` (`security.yml` job `workflow-check`; negative fixture tested locally); artifacts: `cross-network.yml` step "encrypt the code to the client's public key" refuses to upload a plaintext code; gitleaks SARIF upload off |

[e2e]: ../../crates/pigeon-core/tests/e2e.rs
[pairing]: ../../crates/pigeon-core/tests/pairing.rs
[props]: ../../crates/pigeon-core/tests/props.rs
[core]: ../../crates/pigeon-core/src/
[agentbin]: ../../crates/pigeon-agent/src/main.rs
[clibin]: ../../crates/pigeon-cli/src/main.rs
