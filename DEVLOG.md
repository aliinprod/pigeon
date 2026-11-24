# Devlog

Format per day: **Did / Broke / Next**, plus numbers and a screenshot when there is one.

## Day 0 — 2026-11-22
- **Did:** Named the project Pigeon, picked the logo, wrote the plan, set up the repo, the bot team (Atlas, Anvil, Canvas, Proof) and the rules.
- **Broke:** Nothing yet.
- **Next:** Day 1: Rust workspace, iroh connection, fetch one file from the CLI, CI on macOS + Windows.

## Day 1 — 2026-11-22
- **Did:** Rust workspace on iroh 1.5.0. `pigeon-agent share <dir>` prints a ticket, `pigeon get <ticket> <file>` fetches it with resume and SHA-256 check. 100 MiB in ~1.4 s locally, hash match. CI on macOS + Windows.
- **Broke:** The verifier bot found that a stranger could make the agent hold 16 MiB per connection before logging in. Fixed with a 256-byte hello limit and a 10 s handshake timeout, plus a test that failed before the fix.
- **Next:** Day 2: run agent and CLI on two different networks through the relay; start pairing.

## Day 2 — 2026-11-23
- **Did:** Cross-network CI test (agent on a cloud Mac, client on a cloud Windows box), downloadable binaries for macOS + Windows, `--relay-only` flag, route shown after every transfer, no silent overwrite.
- **Broke:** iroh's free public relay is slow: 2 MiB took 75 s (~30 KB/s). Direct connections: ~180 MB/s. The verifier also caught two injection-style bugs (workflow inputs in bash arithmetic, unsafe output file names on Windows) and fixed them with tests.
- **Result (CI, cloud Mac → cloud Windows, two networks):** 10 MiB direct in 0.87 s (12 MB/s), relay-only 1 MiB in 22.6 s (0.046 MB/s). Both SHA-256 match.
- **Next:** First real test: agent on the office Mac, fetch from home.

## Day 3 — 2026-11-23
- **Did:** Coming-soon site live at https://flypigeon.pages.dev (EN/UZ/RU, FAQ). New bots: Plume (web design) and Warden (security). Security foundation: threat model, 28 requirements, pairing design, cargo-deny + gitleaks in CI, all actions pinned to SHAs, 9 property tests.
- **Broke:** Warden found the Day-1 ticket is effectively a password with no expiry (fix: week-2 pairing) and fixed 5 small issues; Proof caught Warden's own filter mangling Persian names and emoji.
- **Next:** Agent on the real office Mac, fetch from home; then pairing. Decide on our own relay after seeing whether the office allows a direct path.

## Day 5 — 2026-11-24
- **Did:** First real test of pairing, both directions, office Mac ↔ home Windows PC on two networks (Mac binaries from run 45233355571; Windows built locally from f273292). No ticket anywhere: invite link + 4-word code, then `ls`/`get` with no secret on the command line.
- **Office Mac → home PC:** paired first try; 50 MiB in 3.06 s (17.2 MB/s, direct), SHA-256 OK. No-overwrite worked (`big-test (1).bin`).
- **Home PC → office Mac:** paired first try; 50 MiB in 68.6 s (0.77 MB/s, direct), SHA-256 OK. Still direct, so the limit is most likely the home connection's upload speed, not Pigeon (not measured separately yet).
- **Keychain:** no macOS prompt on first start or during pairing.
- **Found:** each machine uses two different keys for its two roles (Mac: agent `pudlfeb6`, client `h2233blt`; PC: agent `fijzhrns`, client `pqnvbqnt`). The owner approved "one name and one key per computer" for the app, so this needs a change (Anvil, reviewed by Warden).
- **Also shipped:** agent office at flypigeon.pages.dev/office/ behind a login (401 checked live), site app screenshots, promo video v1, Linux CI + binaries, app design v2 (one app, Share + Get).
- **Next:** one key per computer; measure home upload speed to confirm the 0.77 MB/s cause; desktop app (Tauri) starts.
