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
| `office/build-status.mjs` | Builds `status.json` from GitHub and the "Office status" issue at deploy time (tests: `office/build-status.test.mjs`). |
| `office/office-status-issue.md` | Template body for the "Office status" issue. |
| `.github/workflows/site-deploy.yml` | Builds the status and deploys `site/` + `functions/` to Cloudflare Pages. |
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

Normally the **`site-deploy` workflow** (`.github/workflows/site-deploy.yml`) deploys for you: on every push to
`main` that touches `site/`, `functions/` or `office/`, every 15 minutes (only when the status changed), and on
demand (Actions → site-deploy → Run workflow). After each deploy it checks that `/office/` and
`/office/status.json` answer `401` and `/` answers `200`, and fails the run otherwise. Until the two secrets below
exist it builds the status and skips the deploy with a notice instead of failing.

The post-deploy check passes only on `401`, which needs both the gate (`functions/`) and the login env vars
`OFFICE_USER_SHA256` / `OFFICE_PASS_SHA256` in Cloudflare (see "Rotate the login"). If those vars are missing or
malformed the gate answers `503` (office closed, not public) and the run fails with a message saying so; set the
vars and re-run. A `200` on `/office/` fails the run as "office is PUBLIC".

The workflow has two jobs: `plan` (no secrets) builds the status and decides; `deploy` runs in the GitHub
environment **`production`**, which holds the Cloudflare secrets and only accepts the `main` branch. No other
branch and no pull request can read the secrets.

#### One-time setup by the owner: the `production` environment and its two secrets

1. Cloudflare dashboard → **My Profile** (top right) → **API Tokens** → **Create Token**.
2. Either pick the **Edit Cloudflare Workers** template, or (tighter, preferred) **Create Custom Token** with exactly
   one permission: **Account** › **Cloudflare Pages** › **Edit**. Under **Account Resources** choose
   *Include* › your account only. Leave Zone resources empty. Optional: set a TTL.
3. **Continue to summary** → **Create Token**. Copy the token (it is shown once).
4. Copy your **Account ID**: Cloudflare dashboard → **Workers & Pages** → it is in the right column
   ("Account details"), or in the URL `dash.cloudflare.com/<account id>/...`.
5. GitHub → `aliinprod/pigeon` → **Settings** → **Environments** → **New environment** → name `production` →
   **Configure environment**.
6. Under **Deployment branches and tags**, choose **Selected branches and tags** → **Add deployment branch or tag
   rule** → ref type **Branch**, name pattern `main` → **Add rule**. (No required reviewers: the schedule must run
   unattended.)
7. Still on that page, under **Environment secrets** → **Add environment secret**:
   - Name `CLOUDFLARE_API_TOKEN`, value: the token from step 3.
   - Name `CLOUDFLARE_ACCOUNT_ID`, value: the account id from step 4.

   Do not add them as repository secrets (Settings → Secrets and variables → Actions); if they already exist there,
   delete them so only the `production` environment has them.
8. Make sure the Cloudflare Pages project has `OFFICE_USER_SHA256` and `OFFICE_PASS_SHA256` set (see "Rotate the
   login"), otherwise the post-deploy check fails on `503`.
9. Actions → **site-deploy** → **Run workflow** (branch `main`). The run summary says "Gate OK" when it worked.

The schedule only runs from `main`, so it starts after this workflow is merged. GitHub pauses scheduled
workflows after 60 days without repository activity; re-enable it in the Actions tab if that happens.

#### By hand

Functions are only deployed when the `functions/` folder sits next to where Wrangler runs, so deploy from the
**repo root**:

```
npx.cmd wrangler@4 pages deploy site --project-name flypigeon --branch main
```

Dashboard "Direct Upload" of the `site` folder does **not** include Functions: the office would then be public.
After every deploy, check that `https://flypigeon.pages.dev/office/` answers `401` without a login.

## How status.json is updated

Nobody commits status updates. `office/build-status.mjs` builds `site/office/status.json` at deploy time from two
sources, then `office/sync-status.mjs` refreshes the embedded fallback copy in `index.html`. Both files change only
inside the deploy runner. The committed `status.json` is the roster (ids, names, roles, about) and the default for
any bot neither source mentions.

**Source B: Atlas's notes.** One open issue titled exactly **Office status** (pinned), opened by the owner or a
collaborator. Its body holds one line per bot:

```
anvil: working | One key per computer (both roles)
warden: idle | Will review one-key migration
owner: Test pairing build on the office Mac
```

`id: status | task`, status one of `working`, `done`, `waiting`, `idle`; the task is optional. `owner: text` adds an
item to "Waiting for the owner". Only known ids count; the first line per bot wins; every other line (headings,
prose, typos) is ignored. All text is treated as plain text: control characters are stripped, length is capped at
160 characters, and the page renders it with `textContent` only. Atlas edits it whenever delegating or finishing:

```
gh issue edit <N> --body-file office-status.md
```

One-time creation (template in `office/office-status-issue.md`):

```
gh issue create --title "Office status" --body-file office/office-status-issue.md
gh issue pin <N>
```

**Source A: GitHub.** Open PRs, PRs merged in the last 7 days, the CI result of each open PR, and CI on `main`
(the latest run of each workflow on main's head commit, except site-deploy itself).

Only PRs **opened by the owner account (`aliinprod`) from a branch of this repo** count (`trustedPr()`; W-13).
Fork PRs and PRs by anyone else are dropped before anything else, so a stranger can never put text in the office
or in the owner queue by naming a branch `site-...` or writing "(Plume)" in a title. A bot's PR therefore has to
be opened with the owner's `gh` login, as all of them are today. A trusted PR belongs to a bot by, in
order: a `bot:<id>` label; a branch named `<id>/...` or `<id>-...`; a known branch prefix (`site-`, `office-`,
`agent-office` → plume, `app-` → canvas, `security-` → warden, `promo-` → frame, `day-`, `week`, `linux-` → anvil,
and so on, see `BRANCH_PREFIXES`); `(Name)` in the title; a `Bot: <id>` line in the body.

The labels, one time:

```
for b in atlas anvil canvas plume warden proof frame; do gh label create "bot:$b" --color 555555 --description "Work by $b"; done
```

**Merge rules.** The note gives status and task. Then, per bot:
- open PR with failing CI → status **waiting** (the "!" in the scene), result links to the failing run;
- open PR, CI green, not a draft → result "green, waiting for the owner"; the owner queue gets
  "Review/merge PR #N: title" (status stays as the note says, or **waiting** without a note);
- open PR, CI running or draft → result links to the PR (**working** without a note);
- no open PR but one merged in the last 7 days → result "PR #N merged" (**done** without a note).

The dashboard says "Updated <time> from GitHub" and shows main CI (green, failing, running). The page re-reads
`status.json` every 60 s. Scheduled runs skip the deploy when nothing changed, so "Updated" is when the data last
changed (checked every 15 minutes).

Local preview, read-only on GitHub (needs `gh auth login`):

```
node --test office/build-status.test.mjs                                            # unit tests for the builder
node office/build-status.mjs --print                             # status from the real issue + PRs
node office/build-status.mjs --print --notes office/office-status-issue.md   # with notes from a file
node office/build-status.mjs && node office/sync-status.mjs      # write both files (do not commit them)
```

## Schema

```jsonc
{
  "version": 1,
  "updated_at": "2026-11-23T19:30:00Z",   // ISO 8601, UTC
  "updated_by": "github",
  "source": "github",                     // optional; the dashboard then says "from GitHub"
  "notes_issue": "https://github.com/...", // optional, the Office status issue
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
  ],
  "main_ci": { "state": "failure", "url": "https://github.com/.../actions/runs/..." }  // optional: success | failure | pending | none
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
