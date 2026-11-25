# Pigeon app (Tauri 2), part 1

One app per computer, both roles: **Share** (folders this computer lets paired
devices get, read-only) and **Get** (computers this one can get files from).
Design: [`design/`](design/README.md) (v2, approved).

```
app/
  ui/          plain HTML + CSS + JS, no build step, no npm, no network
  src-tauri/   Rust: lib.rs = backend (no Tauri), main.rs = window, tray, IPC
  screenshots/ Windows 11, light + dark, real data (see below)
```

## One endpoint for both roles (R14)

`Backend::start` opens this computer's one identity (`Home::identity`, key in
the OS keystore) and binds **one** iroh endpoint with it. That endpoint runs
the share (an `Agent` with the app's named folders, pairing ALPN only while an
invite is active) and is the one `ls` / `get` / `pair` dial out from. The relay
never sees this computer's ID twice, so getting a file no longer displaces the
share (TH-28 / R14 exit criterion). The app takes `share.lock`, so it refuses
to start next to `pigeon-agent share` (or a second app) for the same config dir.

Proof: `tests/two_backends.rs` runs two complete backends in one process, pairs
them both ways, and gets files in both directions on one endpoint each, with
the other direction's share still answering.

## Run it

```sh
cd app/src-tauri
cargo run                 # debug build, uses your normal config dir
cargo test                # backend unit tests + two-backend e2e (headless)
```

Two instances on one PC (what the screenshots used): give each its own config
dir, download dir and WebView2 profile.

```powershell
$env:PIGEON_HOME="C:\tmp\office\home"; $env:PIGEON_DOWNLOAD_DIR="C:\tmp\office\dl"
$env:WEBVIEW2_USER_DATA_FOLDER="C:\tmp\office\webview"; .\target\debug\pigeon-app.exe
```

Linux needs `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev` to build.

## Why plain JS

Five screens' worth of UI does not need a framework, and every npm package is
code someone can change under us. Plain ES2020 with `// @ts-check` JSDoc types:
zero JS dependencies, nothing to audit or update, no bundler, and CI needs no
Node. The IPC goes through `window.__TAURI__.core.invoke` (`withGlobalTauri`),
not `@tauri-apps/api`.

## Security

- **Capabilities** (`src-tauri/capabilities/main.json`): the window may call the
  18 app commands listed in `build.rs` and nothing else. No `core:*`, no shell,
  no fs, no plugin commands. The folder picker and Open / Show in folder run in
  Rust (`tauri-plugin-dialog`, `tauri-plugin-opener` used from Rust only).
- **The webview never sends a local path.** Folders come from the native picker;
  Open / Show in folder take a transfer ID and only work for files this backend
  finished writing. Remote paths go to the core's guard on the other side.
- **Inputs are checked again in Rust**: IDs parsed as endpoint IDs, length caps
  on every string, names cleaned with `identity::clean_name`, shared-folder
  names checked by `agent::check_folder_name`. Text from the other computer is
  passed through `display_safe` and HTML-escaped in the UI.
- **CSP**: `default-src 'self'; script-src 'self'; style-src 'self'; connect-src
  ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; form-action
  'none'; frame-ancestors 'none'` (no inline styles or handlers anywhere);
  `freezePrototype`. No remote scripts, fonts or trackers (brand fonts are used
  if installed; bundling them is a follow-up).
- **Secrets**: the pairing code is shown only on the sharing computer (it stays
  in the backend until the invite ends). On the getting side the typed code
  goes from the four boxes to `pair_with` and the boxes are cleared; it is not
  stored, logged or sent back. Keys stay in the OS keystore.
- **Downloads** go to `Downloads/Pigeon`, created owner-only (Windows: protected
  DACL, user + SYSTEM; Unix `0700`), never overwrite (`name (1).ext`), resume
  from `.part`, and are SHA-256 checked by the core before the rename.
- **Sharing Pigeon's own config dir** (or a folder containing it) is refused.

## What is in part 1

- First run (Share / Get), My shared folders (native picker, read-only,
  Stop sharing), Pair a device (which way), Invite (link + Copy, 4-word code,
  countdown, tries left, wrong-code warning, used up / expired, cancel), paired
  screens on both sides with fingerprints, Devices (both directions per device,
  Revoke / Forget / Browse), Get: paste link, type code (autocomplete from the
  real EFF list, paste whole code, typo check costs no try), Browse (root
  chips, breadcrumbs, filter, online badge, Get), Transfers (progress, speed,
  Direct / Relay, SHA-256 verified, new-copy note, Open, Show in folder,
  Resume), Settings (rename, fingerprint, download folder, theme).
- Tray (Windows) / menu bar (macOS): status line, Open Pigeon, Quit. Closing
  the window keeps sharing. Linux: window only.

## Not yet (part 2+)

Activity log, search across all folders (Browse filters the current folder),
offline name cache, pause sharing, change download folder, keep awake, open at
login, OS notifications, QR, installers / signing, bundled brand fonts,
cancel of a running transfer, the relay tip.

## Screenshots

`screenshots/` (Windows 11, WebView2, two app instances on one PC, real
pairing and transfers; light + dark): welcome, s-folders(-empty), pair-choose,
s-invite(-wrong), s-paired, g-link, g-code(-wrong), g-paired, g-browse(-root),
g-transfers(-progress), devices (get side, and both ways), revoke, settings;
plus `win-window-native-dark.png` (real window frame) and
`win-tray-menu-dark.png` (real tray menu).
