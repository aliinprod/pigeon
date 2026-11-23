# Pigeon app design (v0)

A clickable design prototype for the **macOS agent** and the **Windows client**,
made before any Tauri code. It is a single static file: `index.html` (inline
CSS/JS, Google Fonts only, logo from `../../brand/logo.webp`). Nothing in it
talks to a network or to `pigeon-core`; all data is fake.

**Open it:** double-click `app/design/index.html`, or serve the repo root
(`python -m http.server` from the repo root, then `/app/design/`).

- Top switcher: **macOS (agent)** / **Windows (client)**, and **Auto / Light / Dark**
  (Auto follows the OS).
- Chips under it jump to every screen and state. Chips with a dot are problem states.
  On a phone the chips become one dropdown.
- Deep links: `index.html#p=win&s=w-transfers&t=dark`. Add `&bare=1` to hide the
  prototype chrome (used for screenshots).

## Screens

### macOS (agent)
| Id | Screen | What it shows |
|---|---|---|
| `m-welcome` | Welcome: pick folders | First run. Add folders to share, read-only. Warns against sharing the whole home folder. |
| `m-invite` | Pair a device | Sheet over Devices: (1) invite link + Copy, "safe to email"; (2) the 4-word code in big mono boxes, "don't send it". Expiry countdown, single use, 3 tries, "Waiting for a device…". QR button present but disabled ("comes later"). |
| `m-invite-wrong` | Wrong code tried | Same sheet with a banner: "Someone typed a wrong code. 2 tries left. If that wasn't you, cancel this invite." |
| `m-paired` | Paired + notification | Success sheet with the new device's fingerprint, and a macOS notification "New device paired: Home PC · 4HPN Q2ZA. Not you? Revoke." |
| `m-popover` | Menu bar popover | Daily use. Status (Online, relay connected), shared folders, recent activity (incl. a live fetch), Pair a device, Open Pigeon, Pause sharing, Quit. A Signal dot on the menu bar icon = a device is fetching now. |
| `m-folders` | Folders tab | Shared folders with path, file count, size, Read-only tag, Stop sharing. Note on symlinks and hidden files. |
| `m-devices` | Devices tab | Paired devices: name, fingerprint, paired date, last seen, route, Revoke. |
| `m-revoke` | Revoke confirm | macOS alert: "It's disconnected now… A transfer in progress stops." |
| `m-activity` | Activity tab | Fetches (with route), pairing events, wrong codes, refused unknown devices. "Kept for 30 days on this Mac. Never sent anywhere." |
| `m-general` | General tab | Mac name, Mac fingerprint, open at login, sharing on/off, keep awake during transfers, new-pairing alerts (locked on), relay, Reset. |
| `m-popover-offline` | No connection | Popover says the relay can't be reached, retry countdown, and what devices will see. |
| `m-invite-burned` | Invite used up | 3 wrong codes: "Nothing was shared. If you didn't type them, someone else has your link." |
| `m-invite-expired` | Invite expired | 12 h passed. Normal event, not an error. |

### Windows (client)
| Id | Screen | What it shows |
|---|---|---|
| `w-link` | Pair: paste link | Step 1 of 2. Paste link; it is checked locally and shows "Invite from Work Mac · expires in 9 h 12 m". |
| `w-code` | Pair: type code | Step 2 of 2. Four word boxes with autocompletion (**interactive**, see below). Shows Mac name, expiry and tries. |
| `w-paired` | Paired: fingerprints | Both fingerprints side by side (Work Mac, This PC) and "check these match what the Mac shows". |
| `w-browse` | Browse | Nav (device + status, shared folders, Transfers with count, Settings), breadcrumbs, name search, Online badge, file list (name, size, modified), Get on hover/selection, status bar "Read-only. Files stay on the Mac · Saving to Downloads\Pigeon". |
| `w-search` | Search | File-name matches across all shared folders, with the folder each lives in. |
| `w-transfers` | Transfers | In progress: progress bar, %, bytes, speed, time left, Direct / Relay badge, Pause. Finished: "SHA-256 verified", Open, Show in folder, and "Saved as a new copy: report-q4.pdf already existed". **Animates.** |
| `w-relay` | Relay explained | Teaching tip from the Relay badge: why, a 3-box path diagram, "Slower, not less safe." |
| `w-settings` | Settings | Download folder + the no-overwrite rule, paired Macs with Forget, This PC fingerprint, appearance, version. |
| `w-code-wrong` | Wrong code | "That code didn't match. 2 tries left." Boxes keep their contents. |
| `w-burned` / `w-expired` / `w-used` | Invite problems | One layout: what happened, what to do. "Already used" tells the owner to check Devices on the Mac and revoke. |
| `w-offline` | Mac offline | Banner with last-seen time; the last known list stays, greyed, Get disabled. |
| `w-dropped` | Dropped, resuming | "Connection lost at 61%. Reconnecting… resumes from 113 MB" and "Paused until Work Mac is reachable. Resumes from 412 MB; nothing is downloaded twice." |
| `w-changed` | File changed | "Changed on the Mac while downloading. The partial copy was deleted…" plus a checksum-mismatch row. |
| `w-revoked` | Revoked | "Work Mac doesn't accept this PC any more." Downloads stay. Pair again / Forget. |

### What is interactive
- Every chip, nav item, tab, breadcrumb and most buttons navigate.
- **Code input:** type 2+ letters for suggestions; `Tab`/`Enter` accepts; space or `-`
  jumps to the next box; pasting `otter plaza crumb vivid` (any separators) fills all four;
  `Backspace` in an empty box goes back. Pair with `otter-plaza-crumb-vivid` succeeds;
  any other code costs a try; the third wrong code lands on "Invite used up".
  The demo word list is a ~280-word subset; the app will embed the EFF list.
- **Browse:** open folders, breadcrumbs, search as you type, Get adds a transfer
  (small files finish at once, and a second Get of the same name shows the `(1)` copy).
- **Transfers:** progress moves; finished items move to Finished.
- macOS: popover buttons, Devices → Revoke, the invite sheet, toggles in General.

## Flows

**First run, Mac (agent).** Install, open → *Welcome: choose what to share* (add
`ForHome`, read-only) → Continue → *Pair a device* sheet: copy the link and email it
to yourself; leave the code on screen or photograph it → app lives in the menu bar.

**First run, Windows (client).** Open → *Pair with your Mac* → paste the link
(we show which Mac it is from and when it expires) → *Type the code* (4 words,
autocompleted) → *Paired*: compare fingerprints → *Browse*.
On the Mac at the same moment: notification "New device paired: Home PC · 4HPN Q2ZA".

**Daily use.** Mac: nothing to do; the menu bar icon shows a Signal dot while a device
fetches; the popover shows who got what. Windows: open Pigeon → Browse or search →
Get → Transfers shows progress, route, and "SHA-256 verified" → Open / Show in folder.

**When things go wrong.** Mac asleep/offline → greyed list + last seen, transfers wait
and resume. Firewall → Relay badge, explained in one tip. File edited mid-download →
partial copy deleted, Get again. Lost home PC → Revoke on the Mac → the PC sees
*Revoked*.

## Design decisions

- **Brand:** black and white throughout. Primary buttons are Ink (Paper in dark).
  **Signal `#FF5A1F` is reserved for "needs your attention"**: wrong codes, burned
  invites, offline, failed transfers, destructive actions, plus the tiny "fetching now"
  dot on the menu bar icon. Progress bars are Ink, not Signal, so a busy screen stays calm.
- **Type:** Space Grotesk for titles, Inter for UI text, JetBrains Mono for anything a
  person compares or copies (codes, fingerprints, paths, sizes, speeds).
  Window chrome uses the system font (SF / Segoe UI) so frames feel native.
- **Contrast:** Mist `#8A8F98` fails AA as body text on white, so secondary text uses a
  darker Mist (`#5C6168`) in light and a lifted one (`#A7ABB2`) in dark. Signal text on
  white uses `#C2410C`.
- **Pairing copy follows PAIRING.md:** two secrets, two channels. The link says
  "safe to email; the link alone can't pair". The code says "don't send it; read it from
  here, or take a photo with your phone". Tries left and expiry are always visible on
  both sides. Every new pairing raises a Mac notification that cannot be turned off.
- **Fingerprints** are the first 8 base32 chars, shown as two groups of four
  (`4HPN Q2ZA`), on both devices, in the same words ("This PC").
- **Honest states:** we say what happened, what Pigeon is doing, and what you can do,
  in that order. No spinner without a reason; offline shows last-seen time.
- **Relay** is never called an error. Badge: "Relay · slower" with a tip: "Slower, not
  less safe."
- **Never overwrite** is stated where it matters: the finished transfer spells out the
  new name and why; Settings states the rule once.
- **Mac = menu bar app**, plus one settings window with four tabs (Folders, Devices,
  Activity, General). Pairing is a sheet on Devices.
- **Windows = one window** with a left nav (device + status, folders, Transfers,
  Settings). Under 760 px the nav becomes tabs; tables drop the Modified column.

## Open questions for the owner

1. **Word list:** PAIRING.md says "EFF short wordlist (1296 words)". EFF has two short
   lists; *short list 2* has unique 3-letter prefixes, so 3 letters always identify a word.
   That makes autocompletion and typo-proofing much better. OK to ask Warden/Anvil for
   list 2? (Same entropy.)
2. **Keep the Mac awake during transfers?** Designed as an opt-in toggle. Some employers
   manage power settings; default off is the safe choice.
3. **Pause sharing** in the popover: wanted for v1, or later?
4. **Activity log retention:** 30 days on the Mac, never uploaded. OK?
5. **Download folder default:** `Downloads\Pigeon` (proposed) or plain `Downloads`?
6. **Device name on pairing:** the Mac calls the PC "Home PC" from `--name`. Should the
   client send its OS hostname by default, or ask on the code screen?
7. **Search scope:** names only across all shared folders (proposed). Content search is
   out of scope.
8. **Offline list:** we show the last known listing, greyed. That means the client caches
   file names locally. OK, or show nothing when offline?
9. **Several Macs:** the nav has room for more than one Mac; v1 designs for one.

## Screenshots

`screenshots/` (rendered with headless Edge at 1180×676 / 1100×652, `&bare=1`):

- Main screens, light and dark: `mac-{welcome,invite,paired,popover,folders,devices,revoke,activity,general}-{light,dark}.png`,
  `win-{link,code,paired,browse,search,transfers,relay,settings}-{light,dark}.png`
- Problem states, light: `mac-{invite-wrong,popover-offline,invite-burned,invite-expired}-light.png`,
  `win-{code-wrong,burned,expired,used,offline,dropped,changed,revoked}-light.png`
- Phone width (390 px, two screens per image): `phone-mac-light-dark.png`, `phone-win-light-dark.png`

Not verified: real macOS / Windows rendering (all frames are drawn in HTML), Safari,
screen readers beyond basic labels.
