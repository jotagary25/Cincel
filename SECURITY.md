# Security Policy

## Reporting a vulnerability

Please **do not** open a public issue for a security vulnerability.

Use GitHub's private vulnerability reporting for this repository instead:
go to the **Security** tab → **Report a vulnerability**, or open
`https://github.com/<usuario>/cincel/security/advisories/new` directly
once the repository is public (`<usuario>`: see `docs/publicacion.md`).
This opens a private draft advisory that only the maintainers can see
until it is resolved and published together.

Include, if you can:
- Cincel's version (`cincel --version`) and how you installed it (tarball
  or `.deb`).
- A minimal way to reproduce the issue.
- What you'd expect to happen instead.

Please do not include real personal data, credentials, or tokens in a
report; redact them the same way `tools/privacy-check.sh` would.

## Supported versions

Cincel follows a single rolling `main` branch; the most recent tagged
release is the one that gets security fixes.

## Scope

This covers the Cincel editor itself (this repository). It does not cover
the coding agents it connects to (Claude Code, Codex, Antigravity, or any
other ACP-compatible agent) — report those to their own projects.
