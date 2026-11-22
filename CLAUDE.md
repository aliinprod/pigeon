# Pigeon

Pigeon lets you fetch files from your work Mac while you are at home on Windows (and later any OS), with no open ports. Tagline: "Your files know the way home." Global product, English UI and docs. Built in public on X ("Building Pigeon · Day N"). Monetization is out of scope for now. v1 does not need to be perfect; ship small, working slices daily.

## How it works (one paragraph)
A small **Agent** runs on the Mac and shares only folders the owner picks. The **App** on the home PC asks for files. Both connect *outward* to a **relay** (iroh), which introduces them so they can talk directly over QUIC; if a firewall blocks that, the relay forwards end-to-end-encrypted bytes it cannot read. Devices are paired once (QR / ticket) and identified by Ed25519 keys. Files stay on the Mac; fetched copies land on the home PC.

## Stack
- Rust workspace, `iroh` for connectivity (pin exact version in Cargo.toml).
- `crates/pigeon-core` protocol + path guard, `crates/pigeon-agent` (Mac/any OS), `crates/pigeon-cli` (client CLI). Tauri app comes later (Canvas).
- CI: GitHub Actions on `macos-latest` and `windows-latest`.

## Team (focused bots, one job each)
| Bot | Job | Done means |
|---|---|---|
| Atlas (main session) | Coordinator: splits work, delegates, reports to the owner, drafts the daily X post. Writes no product code. | Every task closed with proof (CI link, test output, screenshot). |
| Anvil (`.claude/agents/anvil.md`) | Core + Mac agent + CLI in Rust. | `cargo test` green on macOS + Windows CI; e2e transfer hash matches. |
| Canvas (`.claude/agents/canvas.md`) | Desktop app UI (Tauri 2). Starts week 2. | App builds; screenshots of each screen attached. |
| Plume (`.claude/agents/plume.md`) | Website in `site/` (landing, coming soon). Static HTML/CSS, no trackers. | Screenshots at 390px and 1440px; no console errors; links checked. |
| Proof (`.claude/agents/proof.md`) | Verifier: turns risks into automated tests, reviews every PR, can reject. | All gate tests green; written verdict on the PR. |

Trust ladder: owner watches every result → bots verify themselves → bots run in parallel → nightly runs. Add bots only after the previous one is trusted.

## Rules
- Work on a branch, open a PR. **Only the owner merges to `main`.**
- Owner approval required for: merging, releases, paid services (servers, Apple Developer), publishing anything (X posts are drafts only; the owner posts), deleting data, changing repo visibility.
- Never open listening ports on the Mac; agent connects outward only.
- Share only allow-listed folders, read-only by default. Every path is canonicalized and checked against the root (no `..`, no symlink escape).
- No custom crypto. Use iroh / rustls primitives.
- Each day ends with: a `DEVLOG.md` entry (Did / Broke / Next) and a post draft via the `day-post` skill.
- Keep things simple; v1 can be rough. Prefer a working slice over a perfect design.

## Brand
See `brand/BRAND.md`. Logo: `brand/logo.webp` (geometric black pigeon).
