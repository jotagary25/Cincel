//! Cross-checks between `docs/usuario/` (E6-J, spec 08 §8) and the code it
//! documents, so the manual cannot drift silently from what Cincel actually
//! does:
//!
//! - Every shortcut the default keymap or a widget binds by default has its
//!   Spanish description and its formatted key(s) written down in
//!   `atajos.md` (§8.2's first bullet).
//! - Every key `settings.json` ships with is explained in `ajustes.md`, and
//!   every key `ajustes.md` explains really exists in the shipped
//!   `settings.json` (§8.1's `ajustes.md`: "cada ajuste en lenguaje llano").
//! - Every relative link and image inside `docs/usuario/` points at a file
//!   that exists (§8.2's third bullet; README's links are E6-I's job).
//!
//! These are plain `#[test]`s, no `TestAppContext`: [`shortcuts_modal::
//! default_only_keymap`] and [`shortcuts_modal::build_rows`] need no `App`
//! ([`cincel_settings::Keymap::default`] only parses the built-in JSONC), and
//! the manual itself is read with `include_str!` so the test needs no
//! particular working directory.

use std::path::Path;

use crate::shortcuts_modal::{build_rows, default_only_keymap};

// ------------------------------------------------------------------ atajos.md

const ATAJOS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/usuario/atajos.md"
));

/// Every row the shortcuts modal would show with no `keymap.json` at all
/// (built-in JSONC layered over the widgets' own default bindings) has its
/// description and every one of its active keys written down in
/// `atajos.md`. This is the automatic half of spec 08 §8.2's first bullet:
/// "cada atajo... existe... con el mismo comando" is the reverse direction
/// (documented ⇒ real), checked by hand while writing the table; this test
/// is the direction that catches a *future* shortcut added to the code
/// without updating the manual.
#[test]
fn every_default_shortcut_row_is_documented_in_atajos() {
    let keymap = default_only_keymap();
    let rows = build_rows(&keymap, &keymap);
    assert!(
        !rows.is_empty(),
        "default_only_keymap() no produjo ninguna fila"
    );
    for row in &rows {
        assert!(
            ATAJOS.contains(row.description.as_str()),
            "atajos.md no menciona la descripción «{}» (comando «{}», {})",
            row.description,
            row.identity,
            row.context_label
        );
        for key in &row.keys {
            assert!(
                ATAJOS.contains(key.as_str()),
                "atajos.md no menciona la tecla «{}» del comando «{}» ({}, descripción «{}»)",
                key,
                row.identity,
                row.context_label,
                row.description
            );
        }
    }
}

// ------------------------------------------------------------------ ajustes.md

const AJUSTES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/usuario/ajustes.md"
));

/// Every dotted key `settings.json` ships with by default
/// (`cincel_settings::default_settings_jsonc`, the single source of truth
/// also checked against `Settings::default()` in `cincel-settings`), in
/// display order. Kept as an explicit list rather than parsed out of the
/// JSONC (this crate has no JSONC parser of its own; `cincel-settings`'s is
/// private): a key added there without adding it here fails the first loop
/// below at the "existe en el settings.json de verdad" check, which is
/// exactly the drift this test exists to catch.
const SETTINGS_KEYS: &[&str] = &[
    "theme.mode",
    "theme.dark",
    "theme.light",
    "ui_font_family",
    "ui_font_size",
    "buffer_font_family",
    "buffer_font_size",
    "buffer_line_height",
    "text_rendering",
    "editor.soft_wrap",
    "editor.tab_size",
    "editor.insert_spaces",
    "editor.show_whitespace",
    "editor.ruler",
    "editor.cursor_blink",
    "editor.auto_close_pairs",
    "files.exclude",
    "files.autosave",
    "files.autosave_delay_ms",
    "review.jump_to_next_on_decide",
    "review.max_file_size_kb",
    "review.max_lines",
    "review.snapshot_max_total_mb",
    "review.sensitive_paths",
    "connections.default_label",
    "connections.runtime.node_version",
    "connections.mcp_servers",
    "window.decorations",
];

#[test]
fn every_settings_key_is_real_and_documented() {
    let shipped = cincel_settings::default_settings_jsonc();
    for key in SETTINGS_KEYS {
        let leaf = key.rsplit('.').next().unwrap();
        assert!(
            shipped.contains(&format!("\"{leaf}\"")),
            "«{key}» no está en el settings.json de verdad (cincel_settings::default_settings_jsonc)"
        );
        let inline = format!("`{key}`");
        assert!(
            AJUSTES.contains(&inline),
            "ajustes.md no documenta la clave «{key}» (se buscó {inline})"
        );
    }
}

/// The reverse direction: every key `default_settings_jsonc()` actually
/// defines (one regex-free pass over `"snake_case_key":` object keys) is one
/// of [`SETTINGS_KEYS`]'s leaves, so a key added to the shipped document
/// without adding it to this list — and so, transitively, without adding it
/// to `ajustes.md` — fails here instead of shipping silently undocumented.
#[test]
fn no_settings_key_is_shipped_without_being_tracked_here() {
    let shipped = cincel_settings::default_settings_jsonc();
    let known: Vec<&str> = SETTINGS_KEYS
        .iter()
        .map(|key| key.rsplit('.').next().unwrap())
        .collect();
    for key in object_keys(&shipped) {
        assert!(
            known.contains(&key.as_str()),
            "settings.json define «{key}» pero SETTINGS_KEYS (y por lo tanto la comprobación de \
             ajustes.md) no lo conoce: sumalo a SETTINGS_KEYS y a docs/usuario/ajustes.md"
        );
    }
}

/// Every `"identifier": <scalar or array>` at the start of a line (ignoring
/// comments) of a JSONC document: leaf keys only, no nesting information.
/// A key whose value opens a nested object (`"theme": {`) is a section
/// header, not a setting by itself, and is skipped — its own leaf keys are
/// what get listed (`"mode"`, `"dark"`, …). Good enough to catch a forgotten
/// key without writing a JSONC parser in a test file.
fn object_keys(jsonc: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for line in jsonc.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        let Some(rest) = line.strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        let (name, after) = rest.split_at(end);
        let after_colon = after[1..].trim_start();
        let Some(value) = after_colon.strip_prefix(':') else {
            continue;
        };
        let value = value.trim_start();
        if value.starts_with('{') {
            continue;
        }
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            keys.push(name.to_owned());
        }
    }
    keys
}

// ------------------------------------------------------------------ links

/// Every user-manual file, so the link checker below needs no directory
/// listing (which would need a real filesystem path instead of
/// `include_str!`, the one thing that lets this test run from any working
/// directory).
const MANUAL_FILES: &[(&str, &str)] = &[
    (
        "README.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/usuario/README.md"
        )),
    ),
    (
        "instalacion.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/usuario/instalacion.md"
        )),
    ),
    (
        "primer-arranque.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/usuario/primer-arranque.md"
        )),
    ),
    (
        "conexiones.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/usuario/conexiones.md"
        )),
    ),
    (
        "revision.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/usuario/revision.md"
        )),
    ),
    ("atajos.md", ATAJOS),
    ("ajustes.md", AJUSTES),
    (
        "problemas.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/usuario/problemas.md"
        )),
    ),
];

/// The directory `docs/usuario/` lives in, for resolving the manual's own
/// relative links (`instalacion.md`, `../capturas/revision.png`, …) against
/// real files on disk. Only this one check needs an actual path: it is
/// confirming files exist, which `include_str!` cannot do for an image.
fn usuario_dir() -> std::path::PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/usuario")).to_owned()
}

/// Markdown links (`[text](target)`) and images (`![alt](target)`) found in
/// `text`, target only, in order of appearance.
fn markdown_targets(text: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("](") {
        let after = &rest[open + 2..];
        let Some(close) = after.find(')') else {
            break;
        };
        targets.push(&after[..close]);
        rest = &after[close + 1..];
    }
    targets
}

#[test]
fn every_relative_link_of_the_manual_points_at_a_real_file() {
    let base = usuario_dir();
    for (file, text) in MANUAL_FILES {
        for target in markdown_targets(text) {
            let target = target.split('#').next().unwrap_or(target);
            if target.is_empty()
                || target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            let resolved = base.join(target);
            assert!(
                resolved.exists(),
                "{file}: el enlace «{target}» no existe ({})",
                resolved.display()
            );
        }
    }
}
