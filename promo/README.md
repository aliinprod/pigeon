# Pigeon promo videos (Remotion)

Owner: Frame (`.claude/agents/frame.md`). Silent cut, made to read muted.

## Render

```
cd promo
npm.cmd install            # Windows (PowerShell blocks the npm.ps1 shim); `npm install` elsewhere
npm.cmd run render         # out/pigeon-promo-16x9.mp4 (1920x1080) and out/pigeon-promo-1x1.mp4 (1080x1080), H.264, 30 fps
npm.cmd run contact        # out/contact-16x9.png and out/contact-1x1.png: one frame every 2 s (t = 1, 3, 5 ... s)
npm.cmd run stills         # single full-size stills from the MP4s into out/frames/ (git-ignored)
npm.cmd run studio         # live preview / scrubbing
```

Versions are pinned exactly in `package.json` (Remotion 4.0.542, React 19.2.0). Fonts come from Google Fonts through
`@remotion/google-fonts` (Space Grotesk, Inter, JetBrains Mono), so the render needs network access.

## Promo v1: script (33.5 s, 1005 frames)

| Time | Scene | On screen |
|---|---|---|
| 0.0–3.3 s | Hook (Ink) | "Sending [file] files to yourself on Telegram?" + a stack of self-sent file bubbles (generic chat bubbles, no Telegram branding) |
| 3.3–5.3 s | Turn | "There's a better way **home.**" |
| 5.3–8.0 s | Logo | Pigeon logo flies in · "Pigeon" · "Your files know the way home." |
| 8.0–11.3 s | Step 1 (Paper) | "On your work Mac · Share a [folder] folder. **Read-only.**" + real Mac menu-bar popover, ring on "Sharing · read-only" |
| 11.3–14.3 s | Step 2 (Ink) | "Pair once · One **4-word** code. That's it." + real pairing dialog, ring on the code |
| 14.3–18.3 s | Step 3 (Paper) | "At home, on Windows · Browse the Mac. Click **Get.**" + real Windows browse window, cursor zooms in and clicks Get |
| 18.3–21.0 s | Step 4 (Paper) | "It arrives. **Verified.**" + real Transfers "Finished" rows, ring on "SHA-256 verified · Direct" |
| 21.0–25.0 s | Proof (Ink) | "Real test · office Mac → home Windows PC" · **50 MiB in 2.44 s** (counter) · 21.5 MB/s · Direct · SHA-256 match |
| 25.0–28.0 s | Trust (Paper) | "No cloud copy. No VPN. No open ports." · [lock] End-to-end encrypted. |
| 28.0–30.0 s | Direction | "Mac ⇄ Windows" · "Any direction. Linux coming soon." |
| 30.0–33.5 s | End card (Ink over blurred real UI) | Logo · Pigeon · tagline · **Coming soon** · flypigeon.pages.dev · Built in public by @aliinprod |

Every UI scene carries "App design preview · not released yet": the desktop app is a design prototype.

## Sources

| Shown | Source |
|---|---|
| 50 MiB in 2.44 s, 21.5 MB/s, direct, SHA-256 match, office Mac → home Windows PC, two networks, 23 Nov 2026 | `DEVLOG.md` Day 4 on branch `day-4-office-test` (commit 050c671) |
| One 4-word code, single use, never sent over the network | `docs/security/THREAT_MODEL.md` (B4, A3), `crates/pigeon-agent/src/main.rs` and `crates/pigeon-cli/src/main.rs` module docs (merged in PR #6) |
| Read-only, only picked folders, files stay on the Mac; no open ports; no custom crypto / E2E | `CLAUDE.md` rules, `SECURITY.md`, iroh QUIC (TLS 1.3) |
| Nothing is overwritten | DEVLOG Day 2 ("no silent overwrite") |
| UI | `app/design/screenshots/*.png` on branch `app-design` (PR #7), copied to `public/ui/`, cropped only |
| Logo | `brand/logo.webp` |

The UI screenshots contain mock values from the design prototype (file names, sizes, times); they are not measurements.
Mock speed numbers in the Transfers screen are cropped out.

## Licenses

- **Remotion** is not MIT. Its license (`node_modules/remotion/LICENSE.md`) is free for individuals, for-profit
  organizations with up to 3 employees, and non-profits; larger for-profit companies need a paid Company License
  (remotion.pro). Pigeon today is one person, so the Free License applies. Re-check if the team grows.
- Fonts: Space Grotesk, Inter, JetBrains Mono are SIL Open Font License 1.1 (Google Fonts).
- Audio: none. The video is silent.
