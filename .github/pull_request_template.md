## What this changes (qué cambia)

<!-- One or two sentences: what and why. -->

## How it was verified (cómo se verificó)

<!--
For a change to a GPUI crate (cincel-editor, cincel-chat,
cincel-workspace, cincel):

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings
    cargo test --workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support

For a change to a crate without GPUI (cincel-settings, cincel-project,
cincel-review, cincel-connections, cincel-perf, ...), `cargo test -p
<crate>` without features is enough. See CONTRIBUTING.md.
-->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy ... -- -D warnings` (fixed feature set, see above)
- [ ] `cargo test` (fixed feature set for GPUI crates)
- [ ] New tests added for the new behavior, in a new file where the spec
      names one
- [ ] User-facing interface text is in Spanish; code comments are in
      English (CONTRIBUTING.md, "Style")

## Related issue (issue relacionado)

<!-- Closes #... -->
