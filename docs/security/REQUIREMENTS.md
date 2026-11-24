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
| SEC-22 | Device secret keys are stored only in the OS keystore (macOS Keychain, Windows Credential Manager/DPAPI), never in plain files or logs. | met (macOS, Windows) | `keystore::tests::os_keystore_roundtrip_leaves_no_key_on_disk` (P-T9; real Keychain on macOS CI, real Credential Manager on Windows CI; scans the config dir for the key in raw, hex and base32 form), `os_keystore_migration_roundtrip` (the one-key migration through the real keystore; no key bytes in any file, incl. `device.json`), `device_key_is_stable_across_restarts`, `damaged_key_is_an_error_not_a_new_identity`. Exception by design: OSes other than macOS/Windows, or `PIGEON_KEYSTORE=file` on macOS, use `0600` files (`file_store_roundtrip_and_owner_only`; on Linux, ubuntu-latest CI runs `keystore::tests::linux_keystore_is_owner_only_files`: key file `0600`, secrets and config dir `0700`, key stable, nothing else holds it. Follow-up: Secret Service / libsecret on Linux desktops); `PIGEON_KEYSTORE=file` is refused on Windows (W-3); see PAIRING.md D6. Limits: any process running as the same user can read the key without a prompt (Windows Credential Manager) or after one "Always Allow" (macOS), see THREAT_MODEL A2 |
| SEC-30 | One device identity per computer (per OS user and config dir) for both roles: `pigeon-agent` and `pigeon` use the same `device-key` and name, created once even when both start at the same time. Migrating a two-key install keeps every existing pairing working in both directions, deletes an old key only once the device key is stored and read back and only if it is a copy of it or no longer needed, and is idempotent (PAIRING.md D9, section 9.1). | met | `identity::tests::concurrent_first_run_creates_one_key`, `concurrent_migration_is_done_once`, `two_key_install_keeps_agent_key_and_old_client_key_for_old_agents`, `client_only_install_keeps_the_client_key`, `agent_only_install_keeps_the_agent_key`, `interrupted_migration_is_finished_and_strangers_are_kept`, `missing_old_client_key_gives_a_re_pair_hint`, `reset_replaces_the_key_for_both_roles`, `reset_retires_the_old_client_key_and_all_marks` (W-12), `hint_names_are_shell_inert` and `pigeon-agent` `tests::pair_hint_is_safe_to_paste` (W-14), `tests::reset_report_lists_all_old_keys_and_agents`; [one_identity] `reset_retires_every_old_key`; [identity] `migration_keeps_pairings_working_in_both_directions`, `both_roles_with_one_key_at_once_over_direct_paths`; real binaries: [one_identity] `both_programs_create_and_show_one_identity_at_once` (4 + 4 processes), `binaries_migrate_a_two_key_install`, `pigeon-agent` [whoami]; real keystore: `keystore::tests::os_keystore_migration_roundtrip` (Keychain on macOS CI, Credential Manager on Windows CI; the binary tests use Credential Manager on Windows and the file store elsewhere). Gap: the relay routes one endpoint ID to its newest connection, so `pigeon get` next to a running `share` on the same computer makes that share unreachable through the relay while it runs (THREAT_MODEL TH-28, R14); interim warning via `share.lock` (`agent::tests::share_lock_is_visible_and_released`, cli `tests::warns_only_while_a_share_runs_here`) |
| SEC-23 | No custom cryptography: only iroh, rustls, RustCrypto primitives. | met | code review: pairing uses `spake2`, `hkdf`, `hmac`, `sha2` (RustCrypto) and iroh's TLS exporter; `deny.toml` bans OpenSSL/native-tls |

## Relay (planned, see [RELAY.md](RELAY.md))

Design approved items only; Anvil implements them in the relay PR. Status stays **not yet** until the named test exists.

| ID | Requirement | Status | Proof |
|---|---|---|---|
| SEC-33 | The relay set comes only from build-time defaults (Pigeon relays, n0 defaults) and the owner's local `relay.json` / `pigeon relay use`; never from an invite link, a peer message, or an environment variable in release builds. A link's relay hint is stored only if it is in this computer's own relay set. | not yet | planned: `devices::tests` hint filtering against the effective relay set (extends `only_default_relays_are_kept_as_hints`, W-6) |
| SEC-34 | Relay URLs are `https://` with a DNS name; `http://` and IP literals are refused outside test builds. A custom relay token is read from a file or prompt (not argv) and stored in the OS keystore, not in `relay.json`. | not yet | planned: `relays::tests` URL validation; keystore roundtrip; scan of the config dir for the token |
| SEC-35 | The Pigeon relay server runs iroh-relay at exactly the clients' iroh version, built with `--locked` from crates.io, with its lockfile audited weekly; it runs unprivileged (only `CAP_NET_BIND_SERVICE`) and exposes only 80/443 tcp and 7842 udp (+22 key-only SSH). | not yet | planned: `security.yml` job auditing the iroh-relay lockfile; runbook acceptance check (RELAY.md section 6) |
| SEC-36 | Relay logging: `RUST_LOG=warn` (no IPs or endpoint IDs in logs), journal kept at most 7 days, no access logs, metrics bound to 127.0.0.1; rate limits (`accept_conn_limit`, per-client `rx`) are set. | not yet | planned: config in repo + runbook check (`journalctl` has no IP/ID; 9090 closed from outside) |
| SEC-37 | If no Pigeon relay is connected for 30 s, the agent adds the n0 relays at run time and stays reachable; it removes them after the Pigeon relay is healthy again. `--no-fallback` disables this. | not yet | planned: e2e with two in-process relays: stop relay A, relay-only `get` still succeeds via B; restart A, agent returns to A |
| SEC-38 | No security property depends on the Pigeon relay token (it is public); it is sent only to Pigeon relays (never to n0), and no relay config or token is ever `Debug`-printed or logged (iroh-relay 1.5.0 `RelayConfig` derives `Debug` with the token, W-16). | not yet | planned: e2e with a `shared_token` relay: no token is refused, right token works; unit test that Pigeon's relay display shows only the host |
| SEC-39 | A relay-only transfer through a self-hosted iroh-relay is end-to-end verified: the SHA-256 matches and the relay only ever saw ciphertext (relay untrusted, SEC-21 unchanged). | not yet | planned: e2e `relay_only_get_through_own_relay_matches_hash` with iroh `test-utils` `run_relay_server` |

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
[identity]: ../../crates/pigeon-core/tests/identity.rs
[one_identity]: ../../crates/pigeon-cli/tests/one_identity.rs
[whoami]: ../../crates/pigeon-agent/tests/whoami.rs
[core]: ../../crates/pigeon-core/src/
[agentbin]: ../../crates/pigeon-agent/src/main.rs
[clibin]: ../../crates/pigeon-cli/src/main.rs
