# Roadmap

v1 does not need to be perfect. Each milestone has a "done" check that a bot proves.

| Week | Milestone | Done when | Owner |
|---|---|---|---|
| 1 | Two machines find each other; fetch one file via CLI | CI green on macOS + Windows; 100 MB file transfers with matching SHA-256; path-escape tests pass | Anvil |
| 2 | Pairing by ticket/QR + simple desktop app | 500 MB file resumes after a network drop; revoked device refused | Canvas, Proof |
| 4 | Menu-bar agent, request queue while the Mac sleeps | Queued request arrives within 5 min of wake | Anvil, Canvas |
| 6 | Hot files (opt-in prefetch) + `/fetch` Telegram bot | 90% of a week's requests served while the Mac sleeps | All |
| 8 | v1 release | 5 GB relay-only transfer, hash match, nightly e2e green 7 nights | Atlas |

Later: local AI search, more platforms, own relay hosting, monetization.
