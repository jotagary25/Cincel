//! What the panel asks the workspace for.
//!
//! The panel never talks to `cincel-acp`, the file system or the clipboard
//! itself: it emits a [`ChatEvent`] and the workspace, which owns the channels
//! and the worktree, does the work (`docs/specs/03-arquitectura.md` §2, "El
//! pegamento vive en `cincel` y `cincel-workspace`").

use std::path::PathBuf;

use cincel_acp::AgentCommand;

use crate::model::{ChatImageSource, NoticeLevel};

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
    /// The paperclip button (or `chat::attach_image`): open the system file
    /// dialog ("Adjuntar imagen", several files, filter "Imágenes": png, jpg,
    /// jpeg, gif, webp) and hand the chosen files to
    /// [`crate::ChatPanel::attach_paths`]
    /// (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.3.2).
    /// Only emitted when the active connection accepts images.
    PickImages,
    /// Show this as a pop-up notice (`toast`): why an image was not
    /// attached, and the like.
    Notify {
        /// How loud it is.
        level: NoticeLevel,
        /// What it says, in Spanish.
        text: String,
    },
    /// A thumbnail of a sent message was clicked: open the image viewer.
    OpenImage {
        /// Where the bytes are.
        source: ChatImageSource,
        /// File name it was attached with.
        name: String,
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Size in bytes.
        bytes_len: u64,
    },
    /// The `×` of a comment tag in the composer box: delete that comment
    /// (from the store and the margin too).
    RemoveComment {
        /// The store's comment id.
        id: u64,
    },
    /// A comment tag or a comment card's header was clicked: open `path` at
    /// `line`.
    OpenLocation {
        /// The file.
        path: PathBuf,
        /// 1-based line.
        line: u32,
    },
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
            ChatEvent::PickImages => f.write_str("PickImages"),
            ChatEvent::Notify { level, text } => f
                .debug_struct("Notify")
                .field("level", level)
                .field("text", text)
                .finish(),
            ChatEvent::OpenImage {
                source,
                name,
                width,
                height,
                bytes_len,
            } => {
                let source = match source {
                    ChatImageSource::File(path) => format!("File({})", path.display()),
                    ChatImageSource::Bytes { mime_type, data } => {
                        format!("Bytes({mime_type}, {} B)", data.len())
                    }
                };
                f.debug_struct("OpenImage")
                    .field("source", &source)
                    .field("name", name)
                    .field("width", width)
                    .field("height", height)
                    .field("bytes_len", bytes_len)
                    .finish()
            }
            ChatEvent::RemoveComment { id } => {
                f.debug_struct("RemoveComment").field("id", id).finish()
            }
            ChatEvent::OpenLocation { path, line } => f
                .debug_struct("OpenLocation")
                .field("path", path)
                .field("line", line)
                .finish(),
        }
    }
}
