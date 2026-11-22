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
- **Next:** First real test: agent on the office Mac, fetch from home. Decide on our own relay after seeing whether the office allows a direct path.
