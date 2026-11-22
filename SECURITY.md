# Security policy

## Status

Pigeon is **pre-release and has not been audited**. Builds are unsigned CI
artifacts for testing. Do not use it to move files you cannot afford to leak,
and follow your employer's rules about moving work files.

Current access model (until week-2 pairing lands): the agent prints a
**ticket that works like a password**. Anyone who has it can read the shared
folder until the agent is stopped. Stopping the agent revokes every ticket.

## Reporting a vulnerability

Please report privately. **Do not open a public issue, PR or post** for a
security problem.

- Use GitHub's private reporting: the repository's **Security** tab ->
  **Report a vulnerability** (GitHub Security Advisories). The maintainer will
  enable private vulnerability reporting when the repository becomes public.
- While the repository is private, collaborators open a **draft security
  advisory** in the same tab.

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
  without a valid ticket / pairing, a malicious agent harming the client
  machine, secrets in logs, and the GitHub Actions workflows in this repo.

Out of scope:

- iroh, rustls and other dependencies themselves (report upstream; tell us if
  Pigeon is affected).
- The public n0 relay and discovery services operated by third parties.
- Attacks that need the owner's unlocked user account on either computer.
- Denial of service by flooding the network or the relay.
- Social engineering, and an employer monitoring their own computers.

No testing against machines or accounts that are not your own, please.

Design documents: [threat model](docs/security/THREAT_MODEL.md),
[security requirements](docs/security/REQUIREMENTS.md),
[pairing design](docs/security/PAIRING.md).
