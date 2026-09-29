# Contributing to Cincel

Thanks for considering a contribution. Cincel is built one stage at a
time from a written spec (`docs/specs/`); reading the relevant spec before
a change usually answers "why is it built this way" faster than reading
the code.

*En español: para compilar y correr las verificaciones alcanza con seguir
los comandos de abajo tal cual están escritos; el resto de esta guía está
en inglés, como el resto del repositorio.*

## Building

```sh
rustup show                 # installs the toolchain pinned in rust-toolchain.toml
cargo build -p cincel
cargo run -p cincel -- --smoke-test .
```

On Debian/Ubuntu you also need `libxkbcommon-x11-dev` (see the README's
Requirements section for the full list the packaged binary depends on).

## The fixed test-feature set

Every `cargo test` or `cargo clippy` invocation on a crate that uses GPUI
(`cincel-editor`, `cincel-chat`, `cincel-workspace`, `cincel`) uses exactly
this feature set, never a different one:

```sh
--features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support
```

Why fixed: these features add test-only instrumentation (input handlers
that behave like a real editor, `gpui-kit/test-support` so a test can
simulate a real click on a widget). A different combination either misses
that instrumentation (spurious failures) or recompiles the whole
workspace under a new feature set (wasted disk and CI minutes). Crates
without GPUI (`cincel-settings`, `cincel-project`, `cincel-review`,
`cincel-connections`, `cincel-perf`, `cincel-text`, `cincel-syntax`,
`cincel-log`, `cincel-acp`) take no test features at all.

## Verification commands

The same commands `.github/workflows/ci.yml` runs, so a red CI never
surprises you:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings
cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
cargo test --workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support
cargo deny check licenses advisories bans sources
shellcheck install.sh packaging/*.sh packaging/**/*.sh tools/*.sh tools/**/*.sh
```

A slow, real-network test (`cargo test -p cincel-connections --test
slow_download -- --ignored`) is not part of this list: it starts a local
HTTP server and downloads real bytes over it, so it only runs on demand
and on a weekly schedule (`.github/workflows/slow-tests.yml`), never on
every push.

Building the distributable packages (tarball and `.deb`) is a separate
step, described in `packaging/README.md`; it needs Docker and is not part
of `ci.yml`.

## Style

- Code comments: English.
- User-facing interface text (menus, dialogs, settings, error messages a
  person reading Spanish will see): Spanish. This includes new strings in
  `cincel-workspace`, `cincel-editor`, `cincel-chat` and `cincel-settings`'
  default settings/keymap comments.
- Rust formatting: `cargo fmt` (`rustfmt.toml`, if any, is authoritative);
  lints: `cargo clippy -- -D warnings`, no `#[allow]` without a comment
  explaining why.
- New tests go in new files where the spec you're implementing names one
  (`docs/specs/*.md` lists the expected test file per feature); this keeps
  concurrent work on disjoint files.

## Proposing a change

1. Check `docs/specs/` and `docs/etapas/` for whether the area you want to
   touch already has a written design — following it (or explaining why it
   should change) saves a round trip.
2. Open an issue first for anything larger than a small fix, using the
   bug report or feature request template
   (`.github/ISSUE_TEMPLATE/`).
3. Keep a pull request's tests in the same PR as the code they cover; a
   change that touches a GPUI crate should still pass the fixed feature
   set above.
4. `cargo fmt --all --check` and `cargo clippy ... -D warnings` must be
   clean before requesting review — the CI enforces both.

## Reporting a security issue

See `SECURITY.md` — please don't open a public issue for a vulnerability.
