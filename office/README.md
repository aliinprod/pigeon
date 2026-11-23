# Agent office

An animated pixel-art office where the Pigeon bots work, plus a plain dashboard of who is doing what.
Static files only: no build step, no libraries, no trackers. Google Fonts is the only outside request.

| File | What it is |
|---|---|
| `index.html` | The page. Contains an embedded fallback copy of `status.json`. |
| `office.js` | Draws the scene (Canvas 2D, 480x320 logical pixels, scaled with nearest-neighbour) and renders the dashboard. All art is drawn in code. |
| `office.css` | Page chrome: brand colours and fonts, light and dark. |
| `status.json` | The single source of truth for the scene and the dashboard. |
| `sync-status.mjs` | Validates `status.json` and copies it into the fallback block in `index.html`. |
| `logo.webp` | Copy of `brand/logo.webp`. |

## Run it locally

```
cd office
python -m http.server 8765        # or: npx serve, or any static server
# open http://127.0.0.1:8765/
```

Opening `index.html` straight from the file system also works: browsers block `fetch()` on `file://`, so the page
uses the copy embedded in `index.html` (the dashboard says "from the copy embedded in this page").
Over http the page reads `status.json` and re-reads it every 60 s.

URL options (handy for screenshots): `?theme=dark|light`, `?still=1` (no animation), `?at=SECONDS` (jump the
simulation forward), `?select=ID` (select an agent).

## How status.json is updated

Atlas owns this file. When Atlas delegates a task, or a bot finishes, waits on the owner, or goes idle, Atlas edits
`status.json` in the same PR as the work (or in a small follow-up PR), then runs:

```
node office/sync-status.mjs          # validate + refresh the fallback copy in index.html
node office/sync-status.mjs --check  # CI-style check: exits 1 if the copy is stale or the JSON is invalid
```

Only the owner merges to `main`, so the office shows what is merged, not what is in flight on a branch.

## Schema

```jsonc
{
  "version": 1,
  "updated_at": "2026-11-23T18:30:00Z",   // ISO 8601, UTC
  "updated_by": "atlas",
  "agents": [
    {
      "id": "anvil",                       // lower case; known ids get a desk and a look in the scene
      "name": "Anvil",
      "role": "Rust core + CLI",           // short, shown on the label above the head
      "about": "What this bot is for.",    // optional, shown in the dashboard details
      "status": "working",                 // working | done | waiting | idle (anything else shows as idle)
      "task": "Linux support + binaries",  // current task, or null when idle
      "last_result": { "text": "PR #9 merged", "url": "https://github.com/..." },  // url: https only, or null
      "updated_at": "2026-11-23T18:20:10Z"
    }
  ],
  "owner_queue": [
    { "text": "Merge PR #7 (app design)", "from": "canvas", "url": "https://github.com/..." }
    // plain strings are accepted too
  ]
}
```

Status meaning: **working** (pulsing dot, typing, code scrolling on the monitor), **done** (check mark, green tick on
the monitor), **waiting** (Signal orange dot: blocked on the owner), **idle** (hollow dot, screensaver).

Known ids with a desk: `atlas`, `anvil`, `canvas`, `plume`, `warden`, `proof`, `frame`. A new bot appears in the
dashboard right away; to give it a desk and a look, add entries to `DESKS` and `LOOKS` in `office.js`.

The walking is scene flavour: Atlas visits desks while he is working, and idle or done bots sometimes get up
(Proof to Anvil's desk, the coffee counter, the bookshelf). The words on the labels always come from `status.json`.

## Accessibility

The canvas has a text alternative and the dashboard carries the same information as real HTML. Labels are buttons;
clicking a bot or a label selects that bot's row. `Esc` clears the selection. With `prefers-reduced-motion` the
scene starts as a still frame; the Pause / Play button works in any case.
