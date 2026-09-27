//! The `workspace::*` commands.
//!
//! Every shortcut is a named command that `keymap.json` can rebind
//! (`docs/specs/02-visual.md` §8). GPUI actions are types, and their *name* is
//! what the keymap refers to, so each action below is named exactly as the
//! keymap spells it (`workspace::open_folder`, not `workspace::OpenFolder`).
//! That way [`crate::keymap`] can ask GPUI to build an action straight from
//! the command name, and a crate that arrives later (the editor, the chat,
//! the review panel) only has to name its own actions the same way to become
//! bindable.

/// Declares one unit action per `name => "command"` pair.
macro_rules! workspace_actions {
    ($($(#[$meta:meta])* $name:ident => $command:literal),* $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
            #[action(namespace = workspace, name = $command)]
            pub struct $name;
        )*
    };
}

/// `editor::save_all`, which `cincel-editor` does not define: saving is the
/// host's job, so the action that saves *every* tab lives here.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = editor, name = "save_all")]
pub struct SaveAll;

workspace_actions! {
    /// `Enter` in the "¿Guardar cambios?" dialog.
    ConfirmClose => "confirm_close",
    /// `Esc` in the "¿Guardar cambios?" dialog.
    CancelClose => "cancel_close",
    /// Opens a folder as the project, through the xdg portal dialog.
    OpenFolder => "open_folder",
    /// Shows or hides the chat dock.
    ToggleChat => "toggle_chat",
    /// Shows or hides the file tree dock.
    ToggleTree => "toggle_tree",
    /// Moves the focus to the chat input.
    FocusChat => "focus_chat",
    /// Closes the active tab, asking about unsaved changes.
    CloseTab => "close_tab",
    /// Toggles the active Markdown tab between the code editor and a
    /// rendered preview of the same buffer (`Ctrl+Shift+V`).
    ToggleMarkdownPreview => "toggle_markdown_preview",
    /// Enlarges the interface.
    ZoomIn => "zoom_in",
    /// Shrinks the interface.
    ZoomOut => "zoom_out",
    /// Restores the interface to 100 %.
    ZoomReset => "zoom_reset",
    /// Jumps to the next pending change, wrapping across files (`Alt+J`).
    NextChange => "next_change",
    /// Jumps to the previous pending change (`Alt+K`).
    PrevChange => "prev_change",
    /// Jumps to the next file with pending changes (`Alt+L`).
    NextFileWithChanges => "next_file_with_changes",
    /// Accepts every change of the last turn (`Ctrl+Alt+Enter`).
    AcceptTurn => "accept_turn",
    /// Rejects every change of the last turn (`Ctrl+Alt+Backspace`).
    RejectTurn => "reject_turn",
    /// Undoes the last rejection (`Alt+Shift+U`).
    UndoLastReject => "undo_last_reject",
    /// Opens or closes the review panel (`Ctrl+Shift+R`).
    OpenReviewPanel => "open_review_panel",
    /// Opens the file selected in the tree (Enter).
    OpenSelected => "open_selected",
    /// Opens the containing folder of the tree entry in the file manager.
    RevealInFolder => "reveal_in_folder",
    /// Copies the absolute path of the tree entry.
    CopyPath => "copy_path",
    /// Copies the path of the tree entry relative to the project root.
    CopyRelativePath => "copy_relative_path",
    /// Stages the tree entry as an `@` mention in the chat composer, the
    /// context-menu fallback for dragging a file into the chat
    /// (`gpui-kit`'s tree has no drag source; see `docs/etapas/etapa-2.md`).
    MentionInChat => "mention_in_chat",
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Action as _;

    #[test]
    fn action_names_match_the_keymap_commands() {
        assert_eq!(OpenFolder.name(), "workspace::open_folder");
        assert_eq!(ToggleTree.name(), "workspace::toggle_tree");
        assert_eq!(ZoomReset.name(), "workspace::zoom_reset");
        assert_eq!(CopyRelativePath.name(), "workspace::copy_relative_path");
    }
}
