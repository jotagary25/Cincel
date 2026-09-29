//! Tests of `docs/specs/08-etapa6-cierre-1-0.md` §5.4 (D14): the search bar's
//! `Tab`, `Shift+Tab`, `Enter` and `Ctrl+Enter` moved from
//! `cincel_editor::search_bar_bindings` / `crate::keymap::search_bar_bindings`
//! (both retired) into the default `keymap.json` document
//! (`cincel_settings::defaults::DEFAULT_KEYMAP_JSONC`), in their own
//! `Editor && searching` / `Editor && searching && replacing` sections placed
//! after `Editor`.
//!
//! Kept out of `keymap.rs`'s own test module (a new file, per the rules of
//! `08-etapa6-cierre-1-0.md` §10.1): these are pure `Keymap`/`shortcuts_modal`
//! checks, no `App` needed. The functional side (does `Tab` really move the
//! search bar's focus without inserting a tab, does `Enter` really replace)
//! is `cincel-editor`'s `search_tests.rs`, untouched by this move: it binds
//! `default_key_bindings()` directly and never depended on either retired
//! function.

use cincel_settings::{ContextExpr, Keymap, Keystroke};

use crate::shortcuts_modal::{ShortcutCategory, category, context_label, description};

const SEARCHING: &[&str] = &["Editor", "searching"];
const REPLACING: &[&str] = &["Editor", "searching", "replacing"];
const PLAIN_EDITOR: &[&str] = &["Editor"];

/// 1: the four keys resolve in the default keymap, in their narrow contexts.
#[test]
fn the_four_keys_resolve_in_the_default_keymap() {
    let keymap = Keymap::default();
    let cases: &[(&str, &[&str], &str)] = &[
        ("tab", SEARCHING, "editor::search_next_field"),
        ("shift-tab", SEARCHING, "editor::search_prev_field"),
        ("enter", REPLACING, "editor::replace_next"),
        ("ctrl-enter", REPLACING, "editor::replace_all"),
    ];
    for (stroke, contexts, command) in cases {
        let keystroke = Keystroke::parse(stroke).unwrap();
        assert_eq!(
            keymap.resolve(&keystroke, contexts),
            Some(*command),
            "{stroke} en {contexts:?}"
        );
    }
    // Plain editing, outside the search bar, keeps its usual meaning.
    assert_eq!(
        keymap.resolve(&Keystroke::parse("tab").unwrap(), PLAIN_EDITOR),
        Some("editor::tab")
    );
    assert_eq!(
        keymap.resolve(&Keystroke::parse("enter").unwrap(), PLAIN_EDITOR),
        Some("editor::insert_newline")
    );
}

/// 2: `--print-default-keymap` (`default_keymap_jsonc`) shows the four keys.
#[test]
fn the_default_keymap_document_shows_the_four_keys() {
    let text = cincel_settings::default_keymap_jsonc();
    for command in [
        "editor::search_next_field",
        "editor::search_prev_field",
        "editor::replace_next",
        "editor::replace_all",
    ] {
        assert!(text.contains(command), "falta «{command}» en:\n{text}");
    }
    assert!(text.contains("Editor && searching && replacing"), "{text}");
}

/// 3: the shortcuts modal (`F1`) lists the four keys under "Editor", with the
/// contexts "buscando" / "reemplazando" and a Spanish description.
#[test]
fn the_shortcuts_modal_lists_the_four_keys_with_their_context() {
    let searching_label = context_label(&ContextExpr::parse("Editor && searching").unwrap());
    let replacing_label =
        context_label(&ContextExpr::parse("Editor && searching && replacing").unwrap());
    assert_eq!(searching_label, "buscando");
    assert_eq!(replacing_label, "reemplazando");

    for command in [
        "editor::search_next_field",
        "editor::search_prev_field",
        "editor::replace_next",
        "editor::replace_all",
    ] {
        assert_eq!(category(command), ShortcutCategory::Editor, "{command}");
        assert!(description(command).is_some(), "{command}");
    }
}

/// 4 (D14): a user keymap that reassigns `ctrl-enter` in
/// `Editor && searching && replacing` wins over the default.
#[test]
fn a_user_override_of_the_replacing_context_wins() {
    let user = Keymap::parse(
        r#"[{ "context": "Editor && searching && replacing",
              "bindings": { "ctrl-enter": "editor::find_next" } }]"#,
    );
    assert!(user.is_clean(), "{:?}", user.issues);
    let keymap = Keymap::default().layered(user.value);
    assert_eq!(
        keymap.resolve(&Keystroke::parse("ctrl-enter").unwrap(), REPLACING),
        Some("editor::find_next")
    );
}

/// 5 (D14, accepted consequence): a user who reassigns `tab` in their own
/// plain `Editor` section (without `searching`) wins that key everywhere,
/// including with the search bar open — the same behaviour Zed has, and the
/// spec's explicit trade-off for dropping the code-level workaround.
#[test]
fn a_user_override_of_plain_editor_wins_even_while_searching() {
    let user =
        Keymap::parse(r#"[{ "context": "Editor", "bindings": { "tab": "editor::find_next" } }]"#);
    assert!(user.is_clean(), "{:?}", user.issues);
    let keymap = Keymap::default().layered(user.value);
    assert_eq!(
        keymap.resolve(&Keystroke::parse("tab").unwrap(), SEARCHING),
        Some("editor::find_next"),
        "el «Editor» del usuario, instalado después, gana también buscando (D14)"
    );
}
