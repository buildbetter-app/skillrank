# Security Policy

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report privately via GitHub's [private vulnerability
reporting](https://github.com/buildbetter-app/skillrank/security/advisories/new),
or email **security@buildbetter.app**.

Please include:

- what you found and where (file, command, or endpoint),
- how to reproduce it, and
- what an attacker could achieve.

We aim to acknowledge within 3 business days and to ship a fix or a mitigation
plan within 30 days. We will credit you in the advisory unless you'd rather stay
anonymous.

## Supported versions

Fixes land on the latest released version. There are no long-term support
branches while SkillRank is pre-1.0.

## What we consider a vulnerability

SkillRank installs and runs content that other people wrote, so we care most
about anything that crosses one of these boundaries:

- **Installing a skill executes code.** `skillrank install` must only ever
  verify and write files. Skill content is never executed at install time.
- **Escaping the skill surface.** Registry-supplied slugs are validated before
  they are joined onto a path; a way to write or delete outside the repo's
  skill directory is a vulnerability.
- **Content-hash bypass.** Installed content is verified against the hash the
  registry advertises. Any way to install content that does not match is a
  vulnerability.
- **The local registry (`skillrank serve`)** binds to `127.0.0.1` by default.
  Anything that exposes it more widely without the operator asking is a
  vulnerability.
- **Supply chain.** Release binaries are code-signed, notarized on macOS, and
  published with a SHA-256 checksum that `install.sh` verifies before
  installing. Report anything that lets an unverified binary be installed.

## Known and intentional: `skillrank eval` runs untrusted code

`skillrank eval` fetches an evaluation suite from the registry and runs its
**verifier scripts** and a **fixture repository** supplied by whoever published
that suite. That code executes on your machine, and **there is currently no
sandbox** — the verifier shell runs directly on the host.

Because of this, `eval` requires explicit consent every time:

- interactively, it prints what will run and asks before executing;
- non-interactively, it refuses unless you pass `--allow-verifier-exec`;
- `--yes` approves *spend only* and never authorizes code execution.

Fixture remotes are restricted to `https`/`ssh`, and git's command-executing
transports (`ext::`, `file://`) are disabled, so a suite cannot get execution
purely from the clone step.

**Only run suites you trust.** Real sandboxed execution (containerized
verifiers) is planned; until it ships, treat `eval` like running someone else's
build script. A way to execute suite code *without* that consent step is a
vulnerability — please report it.

## Scope

Out of scope: vulnerabilities in the agents SkillRank drives (Claude Code,
Codex), in third-party skills published to the registry (report those with
`skillrank review`, or to the skill's own author), and issues that require
an already-compromised local machine.
