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
/// Key-binding context predicate of the composer inside the chat.
///
/// `gpui-kit`'s `Textarea` installs its own bindings in the `Input` context,
/// whose node is **deeper** than the panel's, and GPUI gives the deepest node
/// priority: a binding written for `Chat` alone therefore never sees `Enter`,
/// `Tab` or the arrows while the composer has the focus. `Chat > Input` matches
/// at the composer's own depth, so the popover keys win the tie (bindings added
/// later break a tie, and the chat's are installed after `gpui_kit::init`), and
/// the panel calls `cx.propagate()` whenever no popover is open, which hands
/// the key straight back to the textarea.
pub const CONTEXT_COMPOSER: &str = "Chat > Input";

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
    /// Moves the focus to the input (`Ctrl+L`).
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
        KeyBinding::new("ctrl-l", FocusInput, chat),
        KeyBinding::new("ctrl-shift-n", NewSession, chat),
        // The popover (`@`, `/`, the selectors) owns the arrows while it is open.
        KeyBinding::new("down", PopoverNext, chat),
        KeyBinding::new("up", PopoverPrev, chat),
        KeyBinding::new("tab", PopoverConfirm, chat),
        // A pending permission takes `Enter` and `Esc` (`chat.md`, "Permisos").
        KeyBinding::new("enter", AcceptPermission, permission),
        KeyBinding::new("escape", RejectPermission, permission),
        // The composer's own bindings, last so they outrank gpui-kit's `Input`
        // ones at the same depth (see `CONTEXT_COMPOSER`). Every handler
        // propagates when no popover is open, so the textarea keeps its caret
        // movement, its `Tab` and its `Enter`.
        KeyBinding::new("down", PopoverNext, composer),
        KeyBinding::new("up", PopoverPrev, composer),
        KeyBinding::new("enter", PopoverConfirm, composer),
        KeyBinding::new("tab", PopoverConfirm, composer),
        KeyBinding::new("escape", ClosePopover, composer),
    ]
}

/// Installs [`default_key_bindings`] into the app.
pub fn bind_default_keys(cx: &mut gpui::App) {
    cx.bind_keys(default_key_bindings());
}
