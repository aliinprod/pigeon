# Public repository: checklist and hardening

Owner: Warden. Written 2026-11-24 for the switch of `aliinprod/pigeon` from private to public
(GitHub Actions minutes on private repos ran out; public repos get free Actions). The owner's
conditions: security checked first, nobody else can change or break the repo, no secret keys in
it, and it can be made private again later.

Only the owner runs the commands below. No bot changes repository visibility or settings.
All commands use the [GitHub CLI](https://cli.github.com/) logged in as the owner, from Git Bash
(on PowerShell, replace the trailing `\` line continuations with a backtick or put each command
on one line). `R=aliinprod/pigeon` throughout.

## 1. Audit result (2026-11-24)

Full history, every ref: all branches, all `refs/pull/*` heads and merge refs (they become
public too), 60 commits (45 non-merge), plus the logs of all Actions runs that are still
stored (they become public as well).

| Check | Tool | Result |
|---|---|---|
| Secrets in history | gitleaks 8.21.2 (official release, SHA-256 checked), `detect --log-opts=--all` | no leaks |
| Same, by hand | regexes over `git log -p --all`: private keys, GitHub/Slack/AWS/Google/OpenAI-style tokens, `secret/token/password = "..."`, 64- and 32-hex values, Cloudflare account ids, iroh tickets, `pigeon-invite-...` links, 4-word codes, emails, home paths, IPs, `.dev.vars`/wrangler files, resume/CV names, the owner's name | see below |
| Files ever committed | `git log --all --name-only` | no `.env`, `.dev.vars`, `wrangler.toml`, key, certificate or CV/resume file ever |
| Workflows | review + `actionlint` 1.7.7 + `.github/scripts/check-workflows.sh` | see section 2 |

Hits that matter:

- **W-11 (Medium, fixed here, credential burned):** `functions/_middleware.js` held default
  SHA-256 hashes of the `/office/` Basic Auth username and a 4-digit password. Once public, the
  password falls to a 10,000-try brute force (it took 6 ms in this audit; the value is not
  written down anywhere). The hashes stay in history (commit `d5d511c`). **That username and
  password are burned:** never use them again, here or anywhere else (if the 4 digits are also a
  phone or card PIN, change that too). Fix: no defaults; the gate reads only
  `OFFICE_USER_SHA256` / `OFFICE_PASS_SHA256` from Cloudflare and answers `503` when they are
  missing or malformed (SEC-31, `office/middleware.test.mjs`). History is not rewritten: a hash
  of a dead password is not a secret worth a force-push.
- **W-12 (Low, privacy, not fixable without rewriting history):** 14 merge commits made in the
  GitHub web UI carry the owner's personal Gmail address as author. Every other commit uses the
  `users.noreply.github.com` address. Anyone can read commit metadata of a public repo (and it is
  in every clone). Owner step: GitHub **Settings -> Emails**: tick *Keep my email addresses
  private* and *Block command line pushes that expose my email*; future web merges then use the
  noreply address. Accept the 14 old commits (recommended) or rewrite history (not recommended:
  it breaks every PR ref and clone, and cannot reach `refs/pull/*`, which GitHub keeps). If the
  address must stay private, the repo must not be made public before a rewrite.
- **W-10 (Low, fixed here):** `cross-network.yml` uploaded the live pairing code in an artifact.
  In a public repo, any logged-in GitHub user can download a run's artifacts while it runs. See
  section 2.

Hits that do not matter (checked, left as is): the 4-word codes in docs and the design mockup
are examples (`otter-plaza-crumb-tulip`, `acid-acorn-acre-zoom`); `pigeon-invite-zzsecretzz` is a
test fixture; `127.0.0.1` and `192.0.2.1` (documentation range) are test addresses; `C:\Users\me`,
`C:\Users\x`, `/Users/x`, `C:\\Users\\you` are placeholders; "Resume"/"Résumé" is the
resume-a-download feature; `Ali’s MacBook Pro` (PR #14 test) is a first name only; the 64-hex
values in `Cargo.lock` are crate checksums and in `NOTICE` a wordlist checksum;
`CLOUDFLARE_ACCOUNT_ID` appears only as a secret name, never a value; `.dev.vars` appears only
in `.gitignore` and docs. No Actions log in the 98 stored runs prints a code, ticket, invite
link, token, key or email (codes and tickets were masked with `::add-mask::`); the stored
`gitleaks-results.sarif` artifacts have zero results.

## 2. Workflows in a public repo

| Rule | Status |
|---|---|
| No `pull_request_target` / `workflow_run` (they run with secrets) | none; enforced by `check-workflows.sh` |
| Fork PRs get no secrets | only `pull_request` triggers: GitHub gives forks a read-only token and no secrets. No workflow on `pull_request` uses a secret except `GITHUB_TOKEN` |
| Minimal `permissions:` | every workflow: `contents: read`; `cross-network` adds `actions: read`, `security`/gitleaks adds `pull-requests: read`. Enforced (top-level block) by `check-workflows.sh` |
| Actions pinned to commit SHAs | all; every SHA was checked against its tag on 2026-11-24 (annotated tags dereferenced). Enforced by `check-workflows.sh` |
| No `${{ github.event.* }}` / `head_ref` in scripts | none; enforced. `workflow_dispatch` sizes go through `env:` and are validated as numbers |
| `persist-credentials: false` on checkout | now on every checkout (the token is not left in `.git/config` for later steps) |
| No secret in artifacts | `cross-network`: only a public key, the invite link, a ciphertext, SHA-256 sums and a done marker (W-10, below). `security`/gitleaks: SARIF upload turned off (W-14, Low: a report of a real finding would be a public artifact) |
| `binaries.yml` artifacts | only the tarball/`.exe` files plus `SHA256SUMS.txt`; runs on push to `main` and manual dispatch only, never on PRs |
| `site-deploy.yml` (PR #12, branch `office-auto`) | deploys only on push to `main`, schedule and `workflow_dispatch`; no PR trigger. See the PR #12 requirements below |

**W-10 fix, cross-network pairing hand-off.** The two jobs run on different networks and can
only talk through the run's artifacts, which are downloadable by anyone in a public repo. New
design: the Windows client job first makes a throwaway RSA-3072 key pair and uploads only the
**public** key. The macOS agent job waits for it, encrypts the 4-word code to it (OpenSSL
RSA-OAEP: CI glue with a standard primitive, not product crypto), and uploads the invite link
(not secret by design, PAIRING.md), the ciphertext and the test files' SHA-256 sums. The client
decrypts with the private key that never left its runner. A step fails the job if a plaintext
`code` file would be uploaded. Both runners are ephemeral VMs: when the jobs end, the agent,
its Keychain entry, the client key and the RSA private key are gone, and the invite is single
use with a 1 h TTL (`--ttl 1`). So a downloaded artifact is useless after the run, and useless
during it except for the link, which lets a stranger burn the 3 attempts and fail the run
(DoS of CI only, no data: the share holds random bytes). Not yet run on real runners (Actions is
blocked by billing); first green run after the flip is the proof.

**Requirements for PR #12 (`site-deploy.yml`), before it is merged on a public repo:**

1. *(Medium, W-13)* `office/build-status.mjs` maps **any** open PR to a bot by its branch name,
   title "(Warden)" or a "Bot: x" body line, and puts its title into the office and into the
   owner queue as "Review/merge PR #N: ..." when its checks are green. In a public repo a
   stranger's fork PR can do that and look like a bot's work. Fix: keep only PRs whose head repo
   is this repo (`headRepositoryOwner.login == owner`, or `isCrossRepository == false`) and whose
   author is the owner; test it in `build-status.test.mjs`.
2. *(Low)* Put the Cloudflare secrets in an environment (`production`, deployment branches:
   `main` only) and add `environment: production` to the deploy job, so a manual dispatch from
   another branch cannot reach them. Today only the owner can dispatch, so this is defence in
   depth.

## 3. Owner steps, in order

### 3.1 Before the flip

1. Cloudflare: set the **new** office login (16+ random characters, from a password manager):
   `OFFICE_USER_SHA256` and `OFFICE_PASS_SHA256` as type *Secret*, Production (and Preview),
   as in `office/README.md`. Redeploy and check `/office/` gives `401` and the new login works.
   Until this is done, the currently deployed site still accepts the burned password.
2. GitHub **Settings -> Emails**: *Keep my email addresses private* and *Block command line
   pushes that expose my email* (W-12).
3. Merge this PR (it removes the hashes from the code and fixes W-10). Then deploy `main` once
   more (from the repo root, see `office/README.md`) and check `/office/` is `401`, not `503`.
4. Delete the old pairing hand-off artifacts. On 2026-11-24, 10 `pigeon-invite-1`, 1
   `pigeon-ticket-1` and 11 `pigeon-client-done-1` artifacts were still stored (they expire
   later that day). They hold codes / a ticket for agents on runners that no longer exist, so
   they grant nothing, but they should not be public:
   ```sh
   R=aliinprod/pigeon
   gh api --paginate "repos/$R/actions/artifacts?per_page=100" \
     -q '.artifacts[] | select(.name | test("^pigeon-(invite|ticket|client)")) | .id' |
     xargs -I{} gh api -X DELETE "repos/$R/actions/artifacts/{}"
   ```
   Old Actions **logs become public** with the repo. This audit scanned all 98 stored run logs
   (352 files): no code, ticket, invite link, token, key or email in them (codes and tickets
   were masked). Keeping them is fine. To start clean anyway:
   `gh run list -R $R -L 200 --json databaseId -q '.[].databaseId' | xargs -n1 gh run delete -R $R`
   The `gitleaks-results.sarif` artifacts are empty reports; new runs no longer upload one.

### 3.2 The flip (owner only)

```sh
R=aliinprod/pigeon
gh repo edit $R --visibility public --accept-visibility-change-consequences
```

### 3.3 Right after the flip

Rulesets and most security features need a public repo on the free plan, so they come now.

**Repository features:** wiki off, issues on, projects off, delete merged branches.

```sh
gh api -X PATCH repos/$R \
  -F has_wiki=false -F has_issues=true -F has_projects=false \
  -F delete_branch_on_merge=true
```

**Secret scanning and push protection** (free for public repos):

```sh
gh api -X PATCH repos/$R --input - <<'EOF'
{"security_and_analysis": {
  "secret_scanning": {"status": "enabled"},
  "secret_scanning_push_protection": {"status": "enabled"}
}}
EOF
```

**Dependabot alerts** (and, optionally, security-update PRs), **private vulnerability reporting**:

```sh
gh api -X PUT repos/$R/vulnerability-alerts
gh api -X PUT repos/$R/private-vulnerability-reporting
# optional, opens PRs for vulnerable crates/npm packages:
gh api -X PUT repos/$R/automated-security-fixes
```

**Actions:** default token read-only, Actions may not create or approve PRs, outside
contributors' fork PRs wait for the owner's approval before any workflow runs, and only GitHub's
own actions plus the three third-party actions in use may run.

```sh
gh api -X PUT repos/$R/actions/permissions/workflow \
  -f default_workflow_permissions=read -F can_approve_pull_request_reviews=false

gh api -X PUT repos/$R/actions/permissions/fork-pr-contributor-approval \
  -f approval_policy=all_external_contributors

gh api -X PUT repos/$R/actions/permissions -F enabled=true -f allowed_actions=selected
gh api -X PUT repos/$R/actions/permissions/selected-actions --input - <<'EOF'
{"github_owned_allowed": true, "verified_allowed": false,
 "patterns_allowed": ["dtolnay/rust-toolchain@*", "Swatinem/rust-cache@*", "gitleaks/gitleaks-action@*"]}
EOF
# optional, if your plan/API offers it: refuse any action not pinned to a full SHA
gh api -X PUT repos/$R/actions/permissions -F enabled=true -f allowed_actions=selected -F sha_pinning_required=true
```

If PR #12 adds `actions/cache` it is GitHub-owned and already allowed; a new third-party
action must be added to `patterns_allowed` on purpose.

**Ruleset on `main`** (step 1, now): PRs required, no force push, no deletion, linear history
optional. Zero required approvals, so the owner can merge his own PRs. No bypass list: the rule
applies to the owner too, so no one (including a bot that runs with the owner's token) can push
or force-push to `main` directly.

```sh
gh api -X POST repos/$R/rulesets --input - <<'EOF'
{
  "name": "main",
  "target": "branch",
  "enforcement": "active",
  "conditions": {"ref_name": {"include": ["~DEFAULT_BRANCH"], "exclude": []}},
  "bypass_actors": [],
  "rules": [
    {"type": "deletion"},
    {"type": "non_fast_forward"},
    {"type": "pull_request", "parameters": {
      "required_approving_review_count": 0,
      "dismiss_stale_reviews_on_push": false,
      "require_code_owner_review": false,
      "require_last_push_approval": false,
      "required_review_thread_resolution": false,
      "allowed_merge_methods": ["merge", "squash", "rebase"]
    }}
  ]
}
EOF
gh api repos/$R/rulesets --jq '.[] | [.id, .name, .enforcement] | @tsv'
```

Optional linear history (then use squash or rebase merges only): add `{"type": "required_linear_history"}`
to `rules` and set `"allowed_merge_methods": ["squash", "rebase"]`.

**Ruleset step 2, once CI and security have run green on a PR after the flip:** require their
checks. Names are the job names; `integration_id` 15368 is GitHub Actions, so a status posted by
anything else cannot satisfy them. `cross-network` is not required (it only runs when Rust code
changes).

```sh
ID=$(gh api repos/$R/rulesets --jq '.[] | select(.name=="main") | .id')
gh api -X PUT repos/$R/rulesets/$ID --input - <<'EOF'
{
  "rules": [
    {"type": "deletion"},
    {"type": "non_fast_forward"},
    {"type": "pull_request", "parameters": {
      "required_approving_review_count": 0,
      "dismiss_stale_reviews_on_push": false,
      "require_code_owner_review": false,
      "require_last_push_approval": false,
      "required_review_thread_resolution": false,
      "allowed_merge_methods": ["merge", "squash", "rebase"]
    }},
    {"type": "required_status_checks", "parameters": {
      "strict_required_status_checks_policy": false,
      "do_not_enforce_on_create": false,
      "required_status_checks": [
        {"context": "test (ubuntu-latest)", "integration_id": 15368},
        {"context": "test (macos-latest)", "integration_id": 15368},
        {"context": "test (windows-latest)", "integration_id": 15368},
        {"context": "cargo deny (advisories, licenses, bans, sources)", "integration_id": 15368},
        {"context": "secret scan (gitleaks)", "integration_id": 15368},
        {"context": "office login gate fails closed (SEC-31)", "integration_id": 15368},
        {"context": "workflows safe for a public repo (SEC-25, SEC-32)", "integration_id": 15368}
      ]
    }}
  ]
}
EOF
```

**Who can push.** On a personal repository only the owner and invited collaborators can push
at all; strangers can only fork and open PRs. Keep the collaborator list empty
(`gh api repos/$R/collaborators --jq '.[].login'` must print only the owner) and review
**Settings -> Deploy keys** and **Settings -> GitHub Apps** (none needed). A "restrict updates"
rule is not used: on a personal repo it would also block the owner's own PR merges unless he is
on the bypass list, which adds nothing over an empty collaborator list.

**Check it worked:**

```sh
gh api repos/$R --jq '{visibility, has_wiki, has_issues, security_and_analysis}'
gh api repos/$R/actions/permissions/workflow
gh api repos/$R/actions/permissions/fork-pr-contributor-approval
gh api repos/$R/private-vulnerability-reporting
gh api repos/$R/rules/branches/main --jq '.[].type'
```

**Accepted residual (owner's token):** the bots push branches and open PRs with the owner's
GitHub login. GitHub cannot tell a bot from the owner, so a bot could also merge a PR. The
ruleset still stops direct pushes, force pushes and deletion of `main`; the "only the owner
merges" rule stays a process rule (CLAUDE.md). To make it technical later: give the bots a
separate machine account with write access and require one approval from the owner
(`required_approving_review_count: 1`, the owner on the bypass list).

## 4. Making it private again

```sh
gh repo edit aliinprod/pigeon --visibility private --accept-visibility-change-consequences
```

What that does: the repo, its issues, PRs, Actions logs and artifacts are hidden again; stars
and watchers from non-collaborators are dropped; on the free plan rulesets stop being enforced
and secret scanning / push protection stop (they need a paid plan on private repos), and
Actions minutes count against the private quota again.

What it does **not** undo:

- **Forks** made while public stay public, become a separate network and keep the full history.
- **Clones** and downloaded release/artifact files on anyone's disk.
- **Caches and archives:** search engines, the Internet Archive (archive.org), Software
  Heritage (it archives public GitHub repos), GH Archive (public event data: commit messages,
  PR titles, issue text), code-search and mirror sites.
- Anything already read: treat everything that was ever public as public forever. Rotating a
  leaked secret is the only fix for a secret; making the repo private is not.

## 5. What could not be verified (2026-11-24)

- No workflow change in this PR has run on GitHub runners (Actions is blocked by billing until
  the flip). The OpenSSL commands were tested locally with OpenSSL 3.5 on Windows; the macOS
  runner's `/usr/bin/openssl` is LibreSSL, which should accept the same `pkeyutl -pkeyopt
  rsa_padding_mode:oaep` options. The first `cross-network` run after the flip is the proof.
- The `gh api` settings commands were written against the GitHub REST docs and not executed
  (settings are the owner's). `sha_pinning_required` and `fork-pr-contributor-approval` are
  recent API fields; if one is rejected, set the same option under **Settings -> Actions ->
  General**.
- Actions logs of runs that GitHub has already expired could not be scanned (they are gone).
