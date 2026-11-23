# Agent office

An animated pixel-art office where the Pigeon bots work, plus a plain dashboard of who is doing what.
It lives at **flypigeon.pages.dev/office/** behind a login. Static files only: no build step, no libraries,
no trackers. Google Fonts is the only outside request.

| Path | What it is |
|---|---|
| `site/office/index.html` | The page. Holds an embedded fallback copy of `status.json`. |
| `site/office/office.js` | Draws the scene (Canvas 2D, 440x324 logical pixels, nearest-neighbour scaling) and the dashboard. All art is drawn in code. |
| `site/office/office.css` | Page chrome: brand colours and fonts, light and dark. |
| `site/office/status.json` | Single source of truth for the scene and the dashboard. |
| `functions/_middleware.js` | Cloudflare Pages Function: the login gate for `/office` (see below). |
| `office/sync-status.mjs` | Validates `status.json` and copies it into the fallback block in `index.html`. |
| `office/README.md` | This file. Kept outside `site/` so it is not deployed. |

## Run it locally

Without the login (fastest, for design work):

```
cd site
python -m http.server 8765
# open http://127.0.0.1:8765/office/
```

With the login, exactly as Cloudflare runs it (from the repo root, so `functions/` is picked up):

```
npx.cmd wrangler@4 pages dev site      # Windows; `npx wrangler@4 pages dev site` elsewhere
# open http://127.0.0.1:8788/office/
```

For local tests use throwaway credentials: put their hashes in a `.dev.vars` file at the repo root (it is git-ignored,
never commit it):

```
OFFICE_USER_SHA256=<sha256 hex of a test user>
OFFICE_PASS_SHA256=<sha256 hex of a test password>
```

Opening `site/office/index.html` straight from the file system also works: browsers block `fetch()` on `file://`,
so the page uses the embedded copy (the dashboard says "from the copy embedded in this page"). Over http it reads
`status.json` and re-reads it every 60 s.

URL options (handy for screenshots): `?theme=dark|light`, `?still=1` (no animation), `?at=SECONDS` (jump the
simulation forward), `?select=ID` (select a bot and open its tooltip), `?names=1|0` (name chips on or off).

## The login gate

`functions/_middleware.js` protects `/office`, `/office/` and everything under it (page, script, CSS and
`status.json`) with HTTP Basic Auth (realm "Pigeon office"). The rest of the site is untouched.

- Credentials are stored **only as SHA-256 hex hashes**. The middleware hashes the submitted username and password
  with Web Crypto and compares both hashes in constant time. No plaintext username or password is in the repo.
- **Defaults are hardcoded** in `functions/_middleware.js` (`DEFAULT_USER_SHA256`, `DEFAULT_PASS_SHA256`), set by the
  owner. They work with no Cloudflare configuration.
- **Warning:** a hash of a short password is not a secret. A 4-digit password has 10,000 possibilities; anyone who can
  read this repo can try them all in well under a second. That is acceptable only while the repo is private. If the
  repo ever becomes public (or before it does), rotate to a long password via the env vars below.
- Responses carry `Cache-Control: no-store`, `X-Robots-Tag: noindex` and `Referrer-Policy: no-referrer`. Plain http
  requests are redirected to https before any credentials are asked for.
- Fail closed: if a configured hash is malformed, every `/office` request gets `503`, never the page.
- The gate runs as root middleware on every request and checks the decoded, lower-cased, dot-segment-resolved path,
  because a percent-encoded path such as `/%6Fffice/` would otherwise skip an `/office/*` route and still be served
  by the asset server (found and fixed during the local test). Every site request therefore invokes a Function;
  the free plan allows 100,000 a day.
- Basic Auth has no logout: the browser remembers the login until it is closed.

### Rotate the login (no code change)

1. Compute the two hashes locally. In PowerShell, this reads the value without echoing it and without it landing in
   the command history (run it once for the username, once for the password):

   ```powershell
   $s = Read-Host -AsSecureString 'Value'; -join ([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes([Net.NetworkCredential]::new('', $s).Password)) | ForEach-Object { $_.ToString('x2') })
   ```

2. Cloudflare dashboard → **Workers & Pages** → **flypigeon** → **Settings** → **Variables and Secrets** →
   **Add**: `OFFICE_USER_SHA256` = the username hash, `OFFICE_PASS_SHA256` = the password hash. Choose type
   **Secret** (encrypted) for both, for the **Production** environment (and Preview if you use preview URLs).
3. Redeploy (variables apply to new deployments). The env vars now override the hardcoded defaults.
4. Check: `curl -I https://flypigeon.pages.dev/office/` must return `401`.

### Deploying

Functions are only deployed when the `functions/` folder sits next to where Wrangler runs, so deploy from the
**repo root**:

```
npx.cmd wrangler@4 pages deploy site --project-name flypigeon
```

(Use the project name shown in the dashboard if it is not `flypigeon`.)

Dashboard "Direct Upload" of the `site` folder does **not** include Functions: the office would then be public.
After every deploy, check that `https://flypigeon.pages.dev/office/` answers `401` without a login.

## How status.json is updated

Atlas owns this file. When Atlas delegates a task, or a bot finishes, waits on the owner, or goes idle, Atlas edits
`site/office/status.json` in the same PR as the work (or a small follow-up PR), then runs:

```
node office/sync-status.mjs          # validate + refresh the fallback copy in index.html
node office/sync-status.mjs --check  # exits 1 if the copy is stale or the JSON is invalid
```

Only the owner merges to `main`, so the office shows what is merged, not what is in flight on a branch.

## Schema

```jsonc
{
  "version": 1,
  "updated_at": "2026-11-23T19:30:00Z",   // ISO 8601, UTC
  "updated_by": "atlas",
  "agents": [
    {
      "id": "anvil",                       // lower case; known ids get a desk and a look in the scene
      "name": "Anvil",
      "role": "Rust core + CLI",
      "about": "What this bot is for.",    // optional, shown in the dashboard details
      "status": "working",                 // working | done | waiting | idle (anything else shows as idle)
      "task": "Linux support + binaries",  // current task, or null when idle
      "last_result": { "text": "PR #9 merged", "url": "https://github.com/..." },  // url: https only, or null
      "updated_at": "2026-11-23T18:20:10Z"
    }
  ],
  "owner_queue": [
    { "text": "Merge PR #7 (app design)", "from": "canvas", "url": "https://github.com/..." }
  ]
}
```

In the scene: **working** = typing, code scrolling on the monitor, a "typing" pill above the head; **done** = leaning
back with a mug, green tick on screen and above the head; **waiting** = idle hands, glancing at the room, a Signal
"!" above the head and on the screen; **idle** = reading a book, screensaver, a grey dot above the head.

Known ids with a desk: `atlas`, `anvil`, `canvas`, `plume`, `warden`, `proof`, `frame`. A new bot shows in the
dashboard right away; to give it a desk and a look, add entries to `DESKS` and `LOOKS` in `office.js`.

Walking is scene flavour: Atlas visits desks and the whiteboard while he works; idle or done bots sometimes get up
(Proof to Anvil's desk, Warden to the server rack, the coffee corner, the bookshelf, the meeting table). Every word
shown comes from `status.json`.

## Design references

Researched on 2026-11-23. Principles only: no artwork, sprites or characters were copied.

1. **The room is the hero; status must not cover it.** Pixel Agents shows agent state through what the character
   does (typing when editing, reading when searching) and only raises a bubble when it needs you
   ([pixel-agents](https://github.com/pablodelucca/pixel-agents)). Here: behaviour first, a 9-pixel marker second,
   text only in one tooltip on hover/tap and in the dashboard.
2. **Tie the picture to real state, not fake busyness.** A write-up of a 2D AI office stresses mapping visuals to
   actual system state ([dev.to: Riel 2D office](https://dev.to/junhee916/building-a-2d-office-ai-company-where-ai-works-inside-pixel-offices-the-riel-2d-office-project-dhn)).
   Every status, task and count in the scene (wall screen, whiteboard, monitors, markers) comes from `status.json`.
3. **Build everything from cuboids on the 2:1 line, light one face, shade the other.** Top face lightest, one side
   mid, one side dark; avoid inner lines that flatten volume; keep characters inside iso boxes
   ([Pixel Parmesan: Fundamentals of Isometric Pixel Art](https://pixelparmesan.com/fundamentals-of-isometric-pixel-art/),
   [Slynyrd Pixelblog 41: Isometric](https://www.slynyrd.com/blog/2022/11/28/pixelblog-41-isometric-pixel-art)).
4. **One coherent detail density, and small props that tell a story.** Mixing flat and highly detailed assets
   looks wrong; small details carry the scene ([Make: Complete Guide to Isometric Pixel Art](https://makezine.com/article/craft/the-complete-guide-to-iso)).
   Unpacking tells whole stories through objects alone ([Game Developer on Unpacking](https://www.gamedeveloper.com/design/telling-story-someones-belongings-unpacking)).
   Here each desk says who works there: Anvil's terminal and anvil paperweight, Canvas's drawing tablet and swatches,
   Proof's clipboard and magnifier, Frame's timeline and clapperboard, Warden's rack, Plume's quill and swatches.
5. **Warm light, cool shadows; add an ambient bounce.** Hue-shift shadows away from the light colour and use a
   second, subtle ambient so dark areas never go flat black
   ([Slynyrd Pixelblog 6: Light and Shadow](https://www.slynyrd.com/blog/2018/6/15/pixelblog-6-light-and-shadow),
   [OpenGameArt: Shadow and Light](https://opengameart.org/content/chapter-4-shadow-and-light)). Here: a cool
   lavender multiply over the room, warm lamps, window shafts and string lights added on top, cool monitor glow.
6. **Time of day lives in palette and shadow direction, not in a sun icon.** With no horizon, rendering carries the
   mood (Pixel Parmesan, above). Dusk sky in the windows, long warm shafts across the floor, contact shadows thrown
   away from the windows.
7. **Readable characters need contrast at the silhouette.** A 1px dark outline (sel-out style) keeps small sprites
   legible over busy floors; big heads and distinct hats/hair make seven bots tell apart at a glance
   ([Clip Studio: isometric pixel workflow](https://tips.clip-studio.com/fr-fr/articles/4969?org=1)).
8. **Frame the camera to fill the canvas.** The room is cropped tight to the frame so the scene reads as a place,
   not a diorama floating in black; detail is densest where the eye lands (wall screen, desks).
