# Pigeon app design (v1: one app, Share + Get)

A clickable design prototype, made before any Tauri code. **One app per OS, two roles
on every OS:** *Share* (my folders, who may get from me, invites, activity) and *Get*
(computers I can get from, browse, search, transfers). Mac to PC, PC to Mac, Linux to
anything.

It is a single static file: `index.html` (inline CSS/JS, Google Fonts only, logo from
`../../brand/logo.webp`). Nothing talks to a network or to `pigeon-core`; all data is fake.

**Open it:** double-click `app/design/index.html`, or serve the repo root
(`python -m http.server`, then `/app/design/`).

- Top switcher: **macOS / Windows / Linux (soon)** and **Auto / Light / Dark** (Auto follows the OS).
- Chips jump to every screen and state, grouped First run / Both / Share / Get / Problems
  (dot = problem state). On a phone the chips become one dropdown.
- Deep links: `index.html#p=mac&s=g-browse&t=dark`. Add `&bare=1` to hide the prototype
  chrome (used for screenshots).

## The cast (same story from three points of view)

| Computer | OS | Shares | Can get from |
|---|---|---|---|
| Work Mac | macOS | ForHome, Projects/Q4 | Home PC |
| Home PC | Windows | Scans, Photos 2026 | Work Mac |
| Studio | Linux (coming soon) | Renders | Work Mac |

Plus one device per platform that only goes one way (Old ThinkPad gets from the Mac;
Studio gets from Home PC; Home PC gets from Studio), so the Devices list always shows a
missing direction.

## Platform frames

- **macOS:** menu bar icon + popover, and one window with a sidebar. Traffic lights, SF for chrome.
- **Windows 11:** tray icon + flyout above the taskbar, and one window (Mica title bar,
  caption buttons, left nav with the active bar). Toasts bottom-right.
- **Linux (coming soon):** GNOME-style window (header bar, round close button), GNOME top
  bar, notification top-centre. No tray: stock GNOME has none. Marked "Coming soon".

The window is the same everywhere: a sidebar with **Get from** (computers, Transfers),
**Share** (My shared folders, Activity), **Devices**, **Pair a device**, **Settings**.
Under 760 px the sidebar becomes tabs.

## Screens

| Id | Screen | Notes |
|---|---|---|
| `welcome` | Welcome | "What do you want to do on this Mac/PC?": Share folders / Get files. Both allowed, either later. |
| `pair-choose` | Pair: which way? | Two cards with a files-flow arrow: *Let a device get files from this Mac* (Share) or *Get files from another device* (Get). "Each pairing works one way… pair twice, once from each side." |
| `tray` | Menu bar / tray flyout | Status; **Sharing from this Mac** (folders, who can get); **Get from** (computers + Browse); Recent with ↑/↓ arrows; Get files, Pair a device; Pause sharing, Open, Quit. Not on Linux. |
| `devices` | Devices (both ways) | One card per computer, two rows: "↑ Home PC can get files from this Mac [Revoke]" and "↓ This Mac can get files from Home PC [Browse] [Forget]". A missing direction is a dashed row that says how to add it. |
| `settings` | Settings | This computer's name + fingerprint; Share options (sharing on/off, keep awake during transfers, new-pairing alerts locked on); Get options (download folder, never overwrite); relay; appearance; Reset (removes every pairing, both directions). |
| `s-folders` | My shared folders | Read-only folders, and "Who can get these" with Revoke. |
| `s-invite` | Invite | Made on the sharing computer. Header "Let a device get files from this Mac", flow arrow, link (+ Copy, "safe to email"), 4-word code ("don't send it"), expiry, single use, 3 tries, QR later. |
| `s-paired` | Paired (share side) | "Home PC can now get files from this Mac", fingerprint, OS notification with direction, and the follow-up "Want to get files from Home PC too? Start on Home PC…". |
| `s-activity` | Activity | What others got from this computer, pairing events, wrong codes, refused devices. |
| `revoke` | Revoke | "Stop Home PC getting files from this Mac? … This Mac can still get files from Home PC." |
| `g-link` | Get: paste link | Shows which computer and the direction before asking for the code. |
| `g-code` | Get: type code | Interactive autocompleting word boxes, tries left. |
| `g-paired` | Get: paired | Both fingerprints, direction, and the follow-up "Should Home PC get files from this Mac too? That's a second pairing, and it starts here." → `s-invite`. |
| `g-browse` | Browse | **On macOS this is the reverse flow: the Mac browsing Home PC's Scans.** Root chips, breadcrumbs, search, online badge, Get. |
| `g-search` | Search | File names across everything that computer shares with you. |
| `g-transfers` | Transfers | Progress, speed, time left, Direct / Relay, Pause, SHA-256 verified, Open / Show in folder, `name (1).ext` spelled out. Animates. |
| `g-relay` | Relay explained | Tip: why, a 3-box path, "Slower, not less safe." |

**Problem states:** `tray-offline` (no internet: both roles affected), `s-invite-wrong`,
`s-burned`, `s-expired`, `g-code-wrong`, `g-burned`, `g-expired`, `g-used`, `g-offline`
(other computer asleep: last known list, greyed), `g-dropped`, `g-changed` (+ checksum
mismatch), `g-revoked` ("Home PC no longer lets this Mac get files… the other direction
is separate").

### What is interactive
- All chips, sidebar items, tabs, cards, breadcrumbs and most buttons navigate.
- **Code input:** 2+ letters show suggestions; `Tab`/`Enter` accepts; space or `-` moves on;
  pasting the full code (any separators) fills all four; `Backspace` in an empty box goes back.
  `otter-plaza-crumb-vivid` pairs; anything else costs a try; the third wrong code ends on
  "Invite used up". Demo word list is a ~280-word subset.
- **Browse / Get:** open folders and roots, search as you type, Get adds a transfer
  (small files finish at once; getting the same name twice shows the `(1)` copy).
- Transfers animate; toggles toggle.

## Flows

**First run (any OS).** Welcome → pick *Share folders* (→ My shared folders) and/or
*Get files* (→ paste link). Nothing is shared until folders are picked.

**Pair so Home PC can get from Work Mac** (the original direction).
On the Mac: Pair a device → *Let a device get files from this Mac* → copy link, keep code on
screen. On the PC: Pair a device → *Get files from another device* → paste link (shows
"Invite from Work Mac", Work Mac → Home PC) → type code → both fingerprints → Browse.
Mac shows a notification "Home PC can get files from this Mac".

**Reverse: Work Mac gets a file from Home PC.** Same steps with roles swapped: the invite
is made on **Home PC** (it shares), pasted on **the Mac**. Then on the Mac:
Get from → Home PC → Scans → `scan-041.pdf` → Get → Transfers (saved to
`~/Downloads/Pigeon`, SHA-256 verified). See `mac-g-browse-*.png` and `mac-g-transfers-*.png`.

**Both ways.** After pairing one way, both success screens offer the other direction:
on the getting side, "Let Home PC get from this Mac" opens an invite *here*; on the
sharing side, the copy says to start on the other computer. Devices shows each direction
as its own row with its own Revoke / Forget.

**Daily use.** Tray/menu bar: what's moving out (Share), computers you can get from (Get),
recent both ways. Window for browsing, transfers, devices.

**When things go wrong.** Other computer asleep → greyed list + last seen, transfers wait
and resume. Firewall → Relay badge + tip. File edited mid-download → partial deleted,
Get again. Lost computer → revoke that direction on the sharing side; the other side sees
"no longer lets this Mac get files"; the reverse direction is untouched.

## Design decisions

- **Direction is always spelled out in words,** "X can get files from Y", never
  "connected" or "paired with". Arrows (↑ out of this computer, ↓ into it) back up the
  words; the flow diagram's arrow always follows the files.
- **The invite is always made on the computer that shares.** That matches PAIRING.md
  (the agent role issues the invite and holds the allow-list). No new protocol is implied.
- **One device card, two rows.** Revoke (my share) and Forget (my access) are separate
  actions on separate rows, so removing one direction never silently removes the other.
- **Brand:** black and white; primary buttons Ink (Paper in dark). **Signal `#FF5A1F` only for
  "needs your attention"** (wrong codes, burned invite, offline, failures, destructive
  actions) and the small "transfer running" dot on the tray/menu bar icon. Progress is Ink.
- **Type:** Space Grotesk titles, Inter text, JetBrains Mono for codes, fingerprints, paths,
  sizes. OS chrome uses the system font (SF, Segoe UI, Cantarell).
- **Contrast:** secondary text uses darker Mist `#5C6168` (light) / `#A7ABB2` (dark); Signal
  text on white `#C2410C`.
- **Honest copy:** what happened, what Pigeon is doing, what you can do. Relay is "slower,
  not less safe". Never overwrite is spelled out where it happens.
- **Fingerprints:** first 8 base32 chars, `4HPN Q2ZA`, shown the same way on both sides.

## Open questions

New with the two-role design (for the owner, Warden and Anvil):

1. **"Also let this device share with me" shortcut.** Today the design reuses the normal
   flow: the second direction is a second invite made on the other computer (link + code).
   Could an already-paired, authenticated connection carry a reverse invite so the user only
   confirms on both screens (no link, maybe no code)? That's a protocol change: Warden to
   review the trust model, Anvil to say if it fits `pigeon/pair/1`. Not designed as fact.
2. **One key per computer for both roles?** The design assumes each computer has one
   identity/fingerprint used for sharing and getting. Confirm with Anvil.
3. **Run in background for Get-only computers?** A computer that only gets doesn't need to
   be online for others. Should the tray icon still run at login by default?
4. **Linux tray:** stock GNOME has no tray. Window only, or ship an AppIndicator for
   desktops that support it?
5. **Device names:** both sides name each other (e.g. "Home PC"). Send the OS hostname by
   default, or ask during pairing?

Still open from v0:

6. **Word list:** use EFF *short list 2* (unique 3-letter prefixes) for better
   autocompletion, same entropy?
7. **Keep awake during transfers:** opt-in toggle, default off. OK?
8. **Pause sharing** in v1, or later?
9. **Activity log:** 30 days, on the sharing computer only, never uploaded. OK?
10. **Download folder default:** `Downloads/Pigeon` or plain `Downloads`?
11. **Search scope:** file names only, across everything that computer shares with you.
12. **Offline list:** show the last known listing (greyed)? Needs a local name cache.
13. **Several computers:** the sidebar has room for many; the demo shows one per direction.

## Screenshots

`screenshots/` (headless Edge, 1240×762, `&bare=1`):

- **Main screens, light and dark, macOS and Windows** (56):
  `{mac,win}-{welcome,pair-choose,tray,devices,s-folders,s-invite,s-paired,s-activity,g-link,g-code,g-paired,g-browse,g-transfers,settings}-{light,dark}.png`
- **Linux, light and dark** (8): `linux-{welcome,devices,g-browse,s-invite}-{light,dark}.png`
- **Problem states, light** (14): `mac-{tray-offline,s-invite-wrong,s-burned,g-offline,g-revoked,revoke}-light.png`,
  `win-{g-code-wrong,g-used,g-expired,g-dropped,g-changed,g-relay,g-search,s-expired}-light.png`
- **Phone width, 390 px, two screens each** (3): `phone-{mac,win,linux}-light-dark.png`

The v0 screenshots (`mac-popover-*`, `win-browse-*`, …) were replaced; anything that
linked them needs the new names.

Not verified: real macOS / Windows / GNOME rendering (frames are drawn in HTML), Safari,
screen readers beyond basic labels.
