//! What the panel asks the workspace for.
//!
//! The panel never talks to `cincel-acp`, the file system or the clipboard
//! itself: it emits a [`ChatEvent`] and the workspace, which owns the channels
//! and the worktree, does the work (`docs/specs/03-arquitectura.md` §2, "El
//! pegamento vive en `cincel` y `cincel-workspace`").

use std::path::PathBuf;

use cincel_acp::AgentCommand;

/// Everything [`crate::ChatPanel`] emits.
#[non_exhaustive]
pub enum ChatEvent {
    /// Forward this to the ACP worker's command channel.
    Command(AgentCommand),
    /// Open `path` in the editor and jump to its first pending hunk
    /// ("Ver en el editor" of an edit tool card).
    OpenFileAtHunk {
        /// Absolute path of the file.
        path: PathBuf,
    },
    /// The user picked a row of the "Conectar" popover
    /// (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4 F1): the workspace
    /// stops the previous connection's process (nothing is deleted), launches
    /// this one with its profile and opens a new, empty conversation bound to
    /// it.
    ConnectionSelected {
        /// Connection id (a UUID, as text).
        id: String,
    },
    /// "Conectar nuevo agente…" of the popover: open the F2 modal.
    NewConnection,
    /// "Eliminar conexión…" of the popover: open the F3 modal.
    DeleteConnections,
    /// "Renombrar" of a row's context menu.
    RenameConnection {
        /// Connection id.
        id: String,
    },
    /// "Volver a conectar" (F4): repeat the login on the same profile.
    Reconnect {
        /// Connection id.
        id: String,
    },
    /// "Reparar" (F5): download what is missing, without touching the
    /// credentials.
    Repair {
        /// Connection id.
        id: String,
    },
    /// "Nueva conversación": the same thing for the connection already active.
    NewConversation,
    /// A row of the history popover was clicked: save the current
    /// conversation, load this one and try to reopen its ACP session.
    OpenConversation {
        /// Id of the stored conversation.
        id: String,
    },
    /// The ✕ of a history row: delete the conversation (with a 5 s undo).
    DeleteConversation {
        /// Id of the stored conversation.
        id: String,
    },
    /// The `@` picker needs the project's files. The workspace answers from the
    /// worktree by calling `reply` (on the UI thread, now or later); the panel
    /// keeps showing the previous list until it does.
    RequestFileList {
        /// Substring the user has typed after `@` (may be empty).
        query: String,
        /// Hands the candidate paths back to the panel.
        reply: Box<dyn FnOnce(Vec<PathBuf>)>,
    },
    /// Launch the user's terminal with this command already typed
    /// ("Abrir terminal" of the authentication card).
    OpenTerminalWithCommand(String),
    /// Put this text in the clipboard (a code block's "Copiar", the auth
    /// command).
    CopyToClipboard(String),
}

impl std::fmt::Debug for ChatEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatEvent::Command(command) => f.debug_tuple("Command").field(command).finish(),
            ChatEvent::OpenFileAtHunk { path } => f
                .debug_struct("OpenFileAtHunk")
                .field("path", path)
                .finish(),
            ChatEvent::ConnectionSelected { id } => f
                .debug_struct("ConnectionSelected")
                .field("id", id)
                .finish(),
            ChatEvent::NewConnection => f.write_str("NewConnection"),
            ChatEvent::DeleteConnections => f.write_str("DeleteConnections"),
            ChatEvent::RenameConnection { id } => {
                f.debug_struct("RenameConnection").field("id", id).finish()
            }
            ChatEvent::Reconnect { id } => f.debug_struct("Reconnect").field("id", id).finish(),
            ChatEvent::Repair { id } => f.debug_struct("Repair").field("id", id).finish(),
            ChatEvent::NewConversation => f.write_str("NewConversation"),
            ChatEvent::OpenConversation { id } => {
                f.debug_struct("OpenConversation").field("id", id).finish()
            }
            ChatEvent::DeleteConversation { id } => f
                .debug_struct("DeleteConversation")
                .field("id", id)
                .finish(),
            ChatEvent::RequestFileList { query, .. } => f
                .debug_struct("RequestFileList")
                .field("query", query)
                .finish_non_exhaustive(),
            ChatEvent::OpenTerminalWithCommand(command) => f
                .debug_tuple("OpenTerminalWithCommand")
                .field(command)
                .finish(),
            ChatEvent::CopyToClipboard(text) => {
                f.debug_tuple("CopyToClipboard").field(text).finish()
            }
        }
    }
}
