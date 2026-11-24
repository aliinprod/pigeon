# Security policy

## Status

Pigeon is **pre-release and has not been audited**. The source is public;
builds are unsigned CI artifacts for testing (macOS and Windows will warn
about an unidentified developer; check the `SHA256SUMS.txt` that comes with
each build). Do not use it to move files you cannot afford to leak,
and follow your employer's rules about moving work files.

Current access model: devices are **paired once** (`pigeon-agent pair` shows
a single-use, 12-hour invite link and a 4-word code; see
[docs/security/PAIRING.md](docs/security/PAIRING.md)). After that the agent
lets in only device keys on its allow-list, and `pigeon-agent revoke` removes
one at once. Treat the 4-word code like a password until it is used: anyone
who has both the link and the code before you pair can pair instead of you
(you would notice, because your own pairing then fails).

## Reporting a vulnerability

Please report privately. **Do not open a public issue, PR or post** for a
security problem.

- Use GitHub's private vulnerability reporting: the repository's
  **Security** tab -> **Report a vulnerability**
  (<https://github.com/aliinprod/pigeon/security/advisories/new>). Only the
  maintainer sees the report.
- If that form is not available to you, open an issue that says only
  "security contact please" (no details), and we will reply with a private
  channel.

Include: affected version or commit, platform, steps to reproduce or a proof
of concept, and the impact you expect. We aim to acknowledge within 3 days and
to agree on a fix and disclosure date with you. Please give us up to 90 days
before public disclosure. We credit reporters in the advisory unless you ask
us not to.

## Supported versions

| Version | Supported |
|---|---|
| `main` (latest commit) | yes |
| anything older | no (pre-release) |

## Scope

In scope:

- `pigeon-agent`, `pigeon` CLI and `pigeon-core` (this repository), including
  the wire protocol, the path guard, tickets / pairing and file handling on
  both sides.
- Reading or listing anything outside a shared folder, reaching the agent
  without a valid pairing, a malicious agent harming the client machine,
  secrets in logs, and the GitHub Actions workflows in this repo (for
  example: a fork PR that gets a secret or a write token, an artifact that
  holds a secret, a way to push to `main`).
- The `/office/` login gate in `functions/_middleware.js`.

Out of scope:

- iroh, rustls and other dependencies themselves (report upstream; tell us if
  Pigeon is affected).
- The public n0 relay and discovery services operated by third parties.
- Attacks that need the owner's unlocked user account on either computer.
- Denial of service by flooding the network or the relay.
- Social engineering, and an employer monitoring their own computers.
- Credentials found in old commits that are documented as burned in
  [docs/security/PUBLIC_REPO.md](docs/security/PUBLIC_REPO.md) (for example
  the first `/office/` login). Do tell us about anything else.

No testing against machines or accounts that are not your own, please.

## Secrets and this repository

This repository is public. It holds no keys, tokens, passwords or password
hashes: deploy credentials live only in GitHub Actions secrets and the
Cloudflare project, device keys only in each computer's OS keystore.
`security.yml` runs gitleaks over the full history on every PR. Pull requests
from forks get no secrets and a read-only token. The repository settings
that go with a public repo (secret scanning with push protection, Dependabot
alerts, private vulnerability reporting, fork-PR workflows only after the
maintainer's approval, a read-only default token, and a ruleset that keeps
`main` PR-only with no force push or deletion) are listed with the exact
commands in [PUBLIC_REPO.md](docs/security/PUBLIC_REPO.md) section 3.3. Only
the maintainer can push.

Design documents: [threat model](docs/security/THREAT_MODEL.md),
[security requirements](docs/security/REQUIREMENTS.md),
[pairing design](docs/security/PAIRING.md),
[public repository checklist](docs/security/PUBLIC_REPO.md).
