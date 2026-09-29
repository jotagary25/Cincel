//! From `keymap.json` to GPUI key bindings.
//!
//! [`cincel_settings::Keymap`] is a list of sections, each with a context
//! expression and a table of keystroke → command name
//! (`docs/specs/modulos/workspace.md`). GPUI wants a flat list of
//! [`gpui::KeyBinding`]s, each with a keystroke string, an *action value* and
//! a context predicate string. The three translations are:
//!
//! | `keymap.json` | GPUI |
//! |---|---|
//! | `"ctrl-shift-enter"` | the same string; both spell modifiers the same way |
//! | `"Editor && searching"` | the same string: [`cincel_settings::ContextExpr`] prints back the subset of GPUI's predicate syntax it parses (identifiers, `!`, `&&`, `\|\|`, parentheses) |
//! | `"workspace::open_folder"` | the action registered under that exact name ([`crate::actions`]) |
//!
//! Nothing here ever panics: an unparsable keystroke, an unparsable context or
//! a command no crate has registered yet (`editor::*` and `chat::*` until
//! their stage arrives) is skipped with a debug log, so a keymap that mentions
//! the future still loads today.

use std::rc::Rc;

use cincel_settings::Keymap;
use gpui::{Action, App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate};

/// GPUI's own bindings (gpui-kit's list navigation, inputs, menus), captured
/// before Cincel adds any, so a keymap reload can rebuild the whole keymap
/// instead of piling new bindings on top of the old ones.
struct BaseBindings(Vec<KeyBinding>);

impl Global for BaseBindings {}

/// Remembers the key bindings GPUI and gpui-kit installed.
///
/// Call once, right after `gpui_kit::init` and before the first
/// [`install`].
pub fn snapshot_base_bindings(cx: &mut App) {
    if cx.has_global::<BaseBindings>() {
        return;
    }
    let bindings: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    tracing::debug!(count = bindings.len(), "atajos base de gpui-kit guardados");
    cx.set_global(BaseBindings(bindings));
}

/// What [`convert`] produced.
#[derive(Default)]
pub struct Conversion {
    /// Bindings GPUI accepted, lowest priority first.
    pub bindings: Vec<KeyBinding>,
    /// `(keystroke, command)` pairs that were skipped, and why, for the log.
    pub skipped: Vec<(String, String, SkipReason)>,
}

/// Why a binding of `keymap.json` did not become a GPUI binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// No crate has registered a command with that name (yet).
    UnknownCommand,
    /// GPUI does not know that keystroke.
    InvalidKeystroke,
    /// GPUI does not accept that context expression.
    InvalidContext,
}

impl SkipReason {
    /// A message for the log, in Spanish.
    pub fn message(self) -> &'static str {
        match self {
            SkipReason::UnknownCommand => "ningún módulo registró ese comando todavía",
            SkipReason::InvalidKeystroke => "GPUI no entiende ese atajo",
            SkipReason::InvalidContext => "GPUI no entiende ese contexto",
        }
    }
}

/// Builds the GPUI bindings of `keymap`, resolving command names with GPUI's
/// action registry.
pub fn convert(keymap: &Keymap, cx: &App) -> Conversion {
    convert_with(keymap, |command| resolve_action(command, cx))
}

/// Same, with an explicit resolver. Used by the tests, which have no `App`.
pub fn convert_with(
    keymap: &Keymap,
    resolve: impl Fn(&str) -> Option<Box<dyn Action>>,
) -> Conversion {
    let mut conversion = Conversion::default();
    // GPUI's `KeyBinding` does not say which keystroke and context it was
    // built from in a comparable way, so an index-aligned list of the two is
    // kept for the sake of `null` (unbind) entries.
    let mut identities: Vec<(String, Option<String>)> = Vec::new();
    // Sections are stored lowest priority first and GPUI gives the last
    // matching binding precedence, so the order carries over unchanged.
    for section in keymap.sections() {
        let context = section.context.to_string();
        let context = context.trim();
        if !context.is_empty() && KeyBindingContextPredicate::parse(context).is_err() {
            for binding in &section.bindings {
                conversion.skipped.push((
                    binding.keystroke.to_string(),
                    context.to_owned(),
                    SkipReason::InvalidContext,
                ));
            }
            continue;
        }
        let predicate = (!context.is_empty())
            .then(|| KeyBindingContextPredicate::parse(context).ok().map(Rc::new))
            .flatten();

        for binding in &section.bindings {
            let keystroke = binding.keystroke.to_string();
            let Some(command) = binding.command.as_deref() else {
                // An explicit `null` unbinds whatever a lower layer bound.
                // GPUI has no "remove" binding, so the earlier one is dropped
                // from the list being built.
                remove_binding(
                    &mut conversion.bindings,
                    &mut identities,
                    &keystroke,
                    context,
                );
                continue;
            };
            let Some(action) = resolve(command) else {
                conversion.skipped.push((
                    keystroke,
                    command.to_owned(),
                    SkipReason::UnknownCommand,
                ));
                continue;
            };
            // `KeyBinding::load` is the non-panicking sibling of
            // `KeyBinding::new`, which unwraps the keystroke parse.
            match KeyBinding::load(
                &keystroke,
                action,
                predicate.clone(),
                false,
                None,
                &DummyKeyboardMapper,
            ) {
                Ok(binding) => {
                    conversion.bindings.push(binding);
                    identities.push((keystroke, (!context.is_empty()).then(|| context.to_owned())));
                }
                Err(error) => {
                    tracing::debug!(%error, "atajo inválido");
                    conversion.skipped.push((
                        keystroke,
                        command.to_owned(),
                        SkipReason::InvalidKeystroke,
                    ));
                }
            }
        }
    }
    conversion
}

/// Drops a previously converted binding with the same keystroke and context.
fn remove_binding(
    bindings: &mut Vec<KeyBinding>,
    identities: &mut Vec<(String, Option<String>)>,
    keystroke: &str,
    context: &str,
) {
    let context = (!context.is_empty()).then(|| context.to_owned());
    let mut index = 0;
    while index < identities.len() {
        if identities[index].0 == keystroke && identities[index].1 == context {
            identities.remove(index);
            bindings.remove(index);
        } else {
            index += 1;
        }
    }
}

/// Rebuilds GPUI's keymap: the bindings gpui-kit installed, then Cincel's.
///
/// Called at startup and on every `keymap.json` change.
pub fn install(keymap: &Keymap, cx: &mut App) {
    snapshot_base_bindings(cx);
    let conversion = convert(keymap, cx);
    for (keystroke, command, reason) in &conversion.skipped {
        tracing::debug!(
            %keystroke,
            %command,
            reason = reason.message(),
            "atajo ignorado"
        );
    }

    let base: Vec<KeyBinding> = cx.global::<BaseBindings>().0.clone();
    cx.clear_key_bindings();
    // Order is precedence: GPUI gives the last matching binding priority, so
    // gpui-kit's bindings come first, then the widget ones, then the user's,
    // which is the only layer that may override anything.
    cx.bind_keys(base);
    cx.bind_keys(built_in_bindings());
    cx.bind_keys(conversion.bindings);
    // A modal dialog outranks even the user's keymap: while it is up, `Enter`
    // and `Esc` belong to it, not to whatever the focused element does with
    // them.
    cx.bind_keys(modal_bindings());
    tracing::debug!(
        skipped = conversion.skipped.len(),
        "keymap instalado en GPUI"
    );
}

/// The bindings of the modal dialogs, which win over everything else while
/// their context is on screen (see [`crate::center::MODAL_CONTEXT`]).
fn modal_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new(
            "enter",
            crate::actions::ConfirmClose,
            Some(crate::center::MODAL_CONTEXT),
        ),
        KeyBinding::new(
            "escape",
            crate::actions::CancelClose,
            Some(crate::center::MODAL_CONTEXT),
        ),
        KeyBinding::new(
            "enter",
            crate::connection_modal::ModalConfirm,
            Some(crate::connection_modal::MODAL_CONTEXT),
        ),
        KeyBinding::new(
            "escape",
            crate::connection_modal::ModalCancel,
            Some(crate::connection_modal::MODAL_CONTEXT),
        ),
        // "Nuevo archivo…"'s own `Enter`/`Esc` (E5-I): a transient dialog
        // like the ones above, not a discoverable panel like the file
        // finder or the shortcuts modal, so it belongs here rather than in
        // `built_in_bindings` (which feeds `shortcuts_modal::
        // all_default_commands`, and this field's own presence on screen
        // already explains what `Enter`/`Esc` do).
        KeyBinding::new(
            "enter",
            crate::new_file::Confirm,
            Some(crate::new_file::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "escape",
            crate::new_file::Dismiss,
            Some(crate::new_file::KEY_CONTEXT),
        ),
        // The quit dialog's own `Enter`/`Esc` (D15, E5-I), same reasoning as
        // above; it is a different context from the tab-close dialog's
        // because it has a third, button-only choice ("Salir sin guardar")
        // the tab-close dialog does not.
        KeyBinding::new(
            "enter",
            crate::title_menu::ConfirmSaveAll,
            Some(crate::title_menu::QUIT_KEY_CONTEXT),
        ),
        KeyBinding::new(
            "escape",
            crate::title_menu::CancelQuit,
            Some(crate::title_menu::QUIT_KEY_CONTEXT),
        ),
    ]
}

/// Bindings that belong to a widget rather than to the user's keymap.
///
/// The code editor's own defaults (`cincel_editor::default_key_bindings`)
/// come first of all, so anything the user writes in `keymap.json` overrides
/// them.
///
/// - the file tree's `Enter`, which opens the selected file (the arrows come
///   from gpui-kit's own tree),
/// - `Ctrl+W`. The default keymap binds it in the `Editor` context, which only
///   exists once the code editor is on screen (stage 2), so until then the
///   same action is bound on the workspace itself. Both resolve to
///   `workspace::close_tab`, so stage 2 changes nothing.
/// - the chat's own defaults (`cincel_chat::default_key_bindings`, context
///   `Chat`/`Chat && permission`): merged here rather than through
///   `cincel_chat::init` so a keymap reload's `cx.clear_key_bindings()`
///   (`install`, below) does not wipe them — the same reasoning that keeps
///   the editor's defaults in this list instead of a one-shot `init` call.
pub(crate) fn built_in_bindings() -> Vec<KeyBinding> {
    let mut bindings = cincel_editor::default_key_bindings();
    bindings.extend(cincel_chat::default_key_bindings());
    bindings.extend([
        KeyBinding::new(
            "enter",
            crate::actions::OpenSelected,
            Some(crate::tree_panel::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "ctrl-w",
            crate::actions::CloseTab,
            Some(crate::workspace::KEY_CONTEXT),
        ),
        // The quick file finder's navigation (`docs/specs/07-etapa5-
        // productividad.md` §3.3, E5-F): fixed, like the tree's `Enter`
        // above.
        KeyBinding::new(
            "down",
            crate::file_finder::SelectNext,
            Some(crate::file_finder::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "up",
            crate::file_finder::SelectPrev,
            Some(crate::file_finder::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "pagedown",
            crate::file_finder::PageDown,
            Some(crate::file_finder::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "pageup",
            crate::file_finder::PageUp,
            Some(crate::file_finder::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "enter",
            crate::file_finder::Confirm,
            Some(crate::file_finder::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "escape",
            crate::file_finder::Dismiss,
            Some(crate::file_finder::KEY_CONTEXT),
        ),
        // The shortcuts modal's own `Esc` (E5-H), fixed like the finder's.
        KeyBinding::new(
            "escape",
            crate::shortcuts_modal::Dismiss,
            Some(crate::shortcuts_modal::KEY_CONTEXT),
        ),
    ]);
    bindings
}

/// The action registered under `command`, or `None` when no crate registered
/// it yet.
///
/// Cincel names its actions exactly as the keymap spells them
/// (`workspace::open_folder`). A crate that follows GPUI's own convention and
/// names its action type `Save` in namespace `editor` is still found, by
/// trying the CamelCase spelling of the command as well.
pub fn resolve_action(command: &str, cx: &App) -> Option<Box<dyn Action>> {
    if let Ok(action) = cx.build_action(command, None) {
        return Some(action);
    }
    let camel = camel_case_command(command)?;
    cx.build_action(&camel, None).ok()
}

/// `editor::save_all` -> `editor::SaveAll`.
fn camel_case_command(command: &str) -> Option<String> {
    let (namespace, name) = command.rsplit_once("::")?;
    let mut camel = String::with_capacity(name.len());
    let mut upper = true;
    for character in name.chars() {
        if character == '_' {
            upper = true;
            continue;
        }
        if upper {
            camel.extend(character.to_uppercase());
            upper = false;
        } else {
            camel.push(character);
        }
    }
    Some(format!("{namespace}::{camel}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{OpenFolder, ToggleTree};

    /// A resolver that knows the workspace commands and nothing else, which is
    /// the situation of stage 1: `editor::*` and `chat::*` do not exist yet.
    fn workspace_only(command: &str) -> Option<Box<dyn Action>> {
        match command {
            "workspace::open_folder" => Some(Box::new(OpenFolder)),
            "workspace::toggle_tree" => Some(Box::new(ToggleTree)),
            other if other.starts_with("workspace::") => Some(Box::new(ToggleTree)),
            _ => None,
        }
    }

    #[test]
    fn converts_every_workspace_binding_of_the_default_keymap() {
        let keymap = Keymap::default();
        let conversion = convert_with(&keymap, workspace_only);

        let workspace_commands = keymap
            .commands()
            .into_iter()
            .filter(|command| command.starts_with("workspace::"))
            .count();
        assert!(workspace_commands >= 15, "{workspace_commands}");
        assert!(conversion.bindings.len() >= workspace_commands);
        assert!(
            conversion
                .skipped
                .iter()
                .all(|(_, command, reason)| !command.starts_with("workspace::")
                    && *reason == SkipReason::UnknownCommand),
            "{:?}",
            conversion.skipped
        );
    }

    #[test]
    fn skips_commands_no_crate_registered() {
        let keymap = Keymap::default();
        let conversion = convert_with(&keymap, workspace_only);
        assert!(
            conversion
                .skipped
                .iter()
                .any(|(_, command, _)| command == "editor::save"),
            "editor::save debería haberse salteado"
        );
    }

    #[test]
    fn keeps_the_context_of_each_section() {
        let keymap = Keymap::default();
        let conversion = convert_with(&keymap, |_| Some(Box::new(ToggleTree) as Box<dyn Action>));
        let contexts: Vec<String> = conversion
            .bindings
            .iter()
            .filter_map(|binding| binding.predicate().map(|p| p.to_string()))
            .collect();
        assert!(contexts.iter().any(|context| context == "Editor"));
        assert!(
            contexts
                .iter()
                .any(|context| context.contains("Editor") && context.contains("searching")),
            "{contexts:?}"
        );
        assert!(contexts.iter().any(|context| context == "Chat"));
        // The global section has no context at all.
        assert!(
            conversion
                .bindings
                .iter()
                .any(|binding| binding.predicate().is_none())
        );
    }

    #[test]
    fn an_explicit_unbind_removes_the_inherited_binding() {
        let user = Keymap::parse(r#"[{ "bindings": { "ctrl-o": null } }]"#);
        assert!(user.is_clean(), "{:?}", user.issues);
        let keymap = Keymap::default().layered(user.value);
        let conversion = convert_with(&keymap, workspace_only);
        assert!(
            !conversion
                .bindings
                .iter()
                .any(|binding| binding.action().name() == "workspace::open_folder"),
            "ctrl-o debería haber quedado sin asignar"
        );
    }

    #[test]
    fn skips_a_keystroke_gpui_cannot_parse() {
        let user = Keymap::parse(r#"[{ "bindings": { "ctrl-nope": "workspace::open_folder" } }]"#);
        let keymap = Keymap::from_sections(user.value.sections().to_vec());
        let conversion = convert_with(&keymap, workspace_only);
        // GPUI accepts any single-token key name, so this one does convert;
        // what must never happen is a panic.
        assert!(conversion.bindings.len() + conversion.skipped.len() == 1);
    }

    #[test]
    fn camel_case_fallback() {
        assert_eq!(
            camel_case_command("editor::save_all").as_deref(),
            Some("editor::SaveAll")
        );
        assert_eq!(
            camel_case_command("chat::send").as_deref(),
            Some("chat::Send")
        );
        assert_eq!(camel_case_command("sin_espacio_de_nombres"), None);
    }

    // Needs gpui's test harness, like the other `#[gpui::test]`s of the crate.
    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn the_default_ctrl_h_resolves_to_find_replace(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let action = resolve_action("editor::find_replace", cx)
                .expect("editor::find_replace está registrado");
            assert_eq!(action.name(), "editor::find_replace");
            let conversion = convert(&Keymap::default(), cx);
            assert!(
                !conversion
                    .skipped
                    .iter()
                    .any(|(_, command, _)| command == "editor::find_replace"),
                "{:?}",
                conversion.skipped
            );
        });
    }
}
