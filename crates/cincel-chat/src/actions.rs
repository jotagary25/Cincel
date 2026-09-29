//! Named actions and the default Linux key bindings of the chat
//! (`docs/specs/02-visual.md` §8, rows with context "chat").
//!
//! Every command is a GPUI action whose registered name is the one the spec
//! uses (`chat::send`, `chat::cancel_turn`, …) so `keymap.json` can rebind it.
//! The key context is `Chat`; `Chat && permission` is active while a permission
//! request is waiting for an answer, which is what makes `Enter` choose the
//! agent's first option and `Esc` its first `reject_*` one instead of sending
//! and cancelling.

use gpui::KeyBinding;

/// Key-binding context predicate of the chat panel.
pub const CONTEXT_CHAT: &str = "Chat";
/// Key-binding context predicate while a permission request is pending.
pub const CONTEXT_PERMISSION: &str = "Chat && permission";
/// Key-context identifier of the composer's input box.
pub const COMPOSER_NODE: &str = "Composer";
/// Key-binding context predicate of the composer inside the chat.
///
/// The composer is an `cincel_editor::EditorView`, whose `Editor` node sits
/// deeper than the box and outranks any binding written for the box (GPUI
/// prefers the deepest match, and the user's keymap binds `enter` in
/// `Editor`). So the keys the composer shares with the editor are not taken by
/// bindings here: the box takes the editor's own actions in the capture phase
/// (`editor::insert_newline` sends or confirms the popover, the arrows, `Tab`
/// and `Esc` drive the popover while it lists something). What is bound here
/// is only what the editor does not bind: `shift-enter` → `chat::newline`.
pub const CONTEXT_COMPOSER: &str = "Chat > Composer";

/// Declares unit-struct actions with explicit registered names.
macro_rules! named_actions {
    ($namespace:ident, [ $( $(#[$attr:meta])* $type:ident => $name:literal ),* $(,)? ]) => {
        $(
            $(#[$attr])*
            #[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
            #[action(namespace = $namespace, name = $name)]
            pub struct $type;
        )*
    };
}

named_actions!(chat, [
    /// Sends the composed message (`Enter`).
    Send => "send",
    /// Inserts a line break in the input (`Shift+Enter`).
    Newline => "newline",
    /// Cancels the running turn (`Esc`).
    CancelTurn => "cancel_turn",
    /// Moves the focus to the input. Unbound by default: `Ctrl+L` is the
    /// workspace's focus wheel (`docs/specs/07-etapa5-productividad.md` §9.2,
    /// D8), and a `Chat` binding would outrank it from inside the chat.
    FocusInput => "focus_input",
    /// Starts a new session with the active agent.
    NewSession => "new_session",
    /// Answers the pending permission with the agent's first option.
    AcceptPermission => "accept_permission",
    /// Answers the pending permission with its first `reject_*` option.
    RejectPermission => "reject_permission",
    /// Closes the open popover (`@`, `/`, agent, sessions, selectors).
    ClosePopover => "close_popover",
    /// Moves the selection down in the open popover.
    PopoverNext => "popover_next",
    /// Moves the selection up in the open popover.
    PopoverPrev => "popover_prev",
    /// Confirms the selected row of the open popover.
    PopoverConfirm => "popover_confirm",
]);

/// The default Linux key bindings of the chat (`docs/specs/02-visual.md` §8).
///
/// The workspace installs them with `cx.bind_keys(default_key_bindings())`
/// before loading the user's `keymap.json`, which can then override any of
/// them. Order matters: GPUI keeps the last binding that matches a key on the
/// same node, so the `Chat && permission` rows come after the plain `Chat` ones
/// and win whenever a permission is on screen.
#[must_use]
pub fn default_key_bindings() -> Vec<KeyBinding> {
    let chat = Some(CONTEXT_CHAT);
    let permission = Some(CONTEXT_PERMISSION);
    let composer = Some(CONTEXT_COMPOSER);
    vec![
        // 02-visual §8: "Enviar mensaje / salto de línea" and "Cancelar turno".
        KeyBinding::new("enter", Send, chat),
        KeyBinding::new("shift-enter", Newline, chat),
        KeyBinding::new("escape", CancelTurn, chat),
        KeyBinding::new("ctrl-shift-n", NewSession, chat),
        // The popover (`@`, `/`, the selectors) owns the arrows while it is open.
        KeyBinding::new("down", PopoverNext, chat),
        KeyBinding::new("up", PopoverPrev, chat),
        KeyBinding::new("tab", PopoverConfirm, chat),
        // A pending permission takes `Enter` and `Esc` (`chat.md`, "Permisos").
        KeyBinding::new("enter", AcceptPermission, permission),
        KeyBinding::new("escape", RejectPermission, permission),
        // The composer: `Shift+Enter` is a line break (the editor binds no
        // `shift-enter`); the rest of its keys are the editor's own actions,
        // taken by the composer box in the capture phase (`CONTEXT_COMPOSER`).
        KeyBinding::new("shift-enter", Newline, composer),
    ]
}

/// Installs the code editor's default bindings (the composer is an editor and
/// needs them) and then [`default_key_bindings`], so the chat's win ties.
pub fn bind_default_keys(cx: &mut gpui::App) {
    cincel_editor::bind_default_keys(cx);
    cx.bind_keys(default_key_bindings());
}
