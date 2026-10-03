//! `cincel-chat`: the chat panel of Cincel, on GPUI.
//! See `docs/specs/modulos/chat.md` and `docs/specs/02-visual.md` §7–§9.
//!
//! # Public API (stable for the workspace)
//!
//! ```no_run
//! use cincel_chat::{ChatConnection, ChatPanel, ChatSettings, ChatTheme, ConnectionBadge};
//! # use gpui::{App, AppContext, Window};
//! # fn demo(window: &mut Window, cx: &mut App) {
//! let panel = cx.new(|cx| {
//!     ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
//! });
//! panel.update(cx, |panel, cx| {
//!     panel.set_connections(
//!         vec![ChatConnection {
//!             id: "3f0c…".into(),
//!             agent_id: "claude-acp".into(),
//!             label: "Claude · personal".into(),
//!             agent_name: "Claude".into(),
//!             identity: Some("ana@… · Max".into()),
//!             last_used: "Usado hace 2 h".into(),
//!             badge: ConnectionBadge::Connected,
//!         }],
//!         cx,
//!     );
//! });
//! # let _ = panel;
//! # }
//! ```
//!
//! - **Entity**: [`ChatPanel::new(theme, settings, window, cx)`](ChatPanel::new).
//!   The workspace owns the two `async-channel`s of
//!   `03-arquitectura.md` §3 and forwards every `AgentEvent` with
//!   [`ChatPanel::handle_event`]; the panel never talks to `cincel-acp`
//!   itself.
//! - **Events**: the panel is an `EventEmitter<`[`ChatEvent`]`>`:
//!   `Command(AgentCommand)` (prompt, cancel, new session, set mode / config
//!   option, respond permission, authenticate), `OpenFileAtHunk { path }` (the
//!   "Ver en el editor" of an edit card), `RequestFileList { query, reply }` (the `@` picker; the workspace answers
//!   from the worktree, or calls [`ChatPanel::set_file_candidates`] directly),
//!   `OpenTerminalWithCommand(String)` and `CopyToClipboard(String)`.
//! - **Header**: the "Conectar" control
//!   (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4 F1, §6). Without an
//!   active connection it is a "Conectar" button; with one
//!   ([`ChatPanel::set_active_connection`]) it shows the provider icon
//!   ([`provider_icon`]), the label, the agent type in muted text (omitted
//!   when the label is exactly the type), the status badge and a caret; never
//!   the account's email, plan or "Usado hace…" (Settings → Conexiones shows
//!   those). Both open the popover of [`ChatConnection`]s
//!   ([`ChatPanel::set_connections`]): one line per connection with icon,
//!   label, type and a [`ConnectionBadge`] (Conectada / Sesión vencida / No
//!   disponible) with "Volver a conectar" / "Reparar", a context menu with
//!   "Renombrar", and
//!   "Conectar nuevo agente…" / "Eliminar conexión…" at the foot. The panel
//!   never connects anything by itself: every gesture is a [`ChatEvent`]
//!   (`ConnectionSelected`, `NewConnection`, `DeleteConnections`,
//!   `RenameConnection`, `Reconnect`, `Repair`). Every header and footer
//!   popover closes by itself: a click outside it, the keyboard focus moving
//!   anywhere else, `Esc` or the window losing focus
//!   (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §4). A [`ConnectionBanner`]
//!   ("La sesión de «X» venció") sits above the transcript
//!   ([`ChatPanel::set_banner`]). [`ChatPanel::set_conversations`] fills the
//!   history popover with [`ConversationSummary`]s grouped by connection
//!   label, and [`ChatPanel::status`] is the pill: `listo`, `pensando…`,
//!   `esperando permiso`, `desconectado`, `autenticación requerida`.
//! - **Conversations**: one connection, one conversation. Picking a
//!   connection emits [`ChatEvent::ConnectionSelected`], the `+` emits
//!   [`ChatEvent::NewConversation`], and a history row emits
//!   [`ChatEvent::OpenConversation`] / [`ChatEvent::DeleteConversation`]; the
//!   workspace owns `conversations/<id>.json` and answers with
//!   [`ChatPanel::start_new_conversation`] / [`ChatPanel::load_conversation`].
//!   While a `session/load` replays history,
//!   [`ChatPanel::begin_replay`] drops the updates already on screen.
//! - **Transcript**: [`Entry`] is exactly the list of `chat.md` — `UserMessage`,
//!   `AgentText` (streaming markdown), `AgentThought`, `ToolCall`, `Plan`,
//!   `Permission`, `AuthRequired`, `Notice`, `TurnSeparator`. A user message is
//!   an accent-tinted bubble against the right edge (`text.accent` 14 % fill,
//!   35 % border) whose text is Markdown rendered like the answers (a paragraph
//!   with mentions keeps its inline chips), and an agent answer is plain text
//!   on the panel, with its
//!   code on `bg.editor` under a language / "Copiar" header; there are no
//!   "Vos" / agent-name labels, the look alone tells the two voices apart.
//!   Sizes follow one type scale ([`settings::TEXT_BODY`] 13 px,
//!   [`settings::TEXT_CODE`] 12.5 px, [`settings::TEXT_SMALL`] 12 px,
//!   [`settings::TEXT_LABEL`] 11 px, headings at most
//!   [`settings::TEXT_HEADING_MAX`] 15 px). The workspace
//!   feeds the `+N −M` of an edit card with
//!   [`ChatPanel::set_tool_stats`]`(tool_call_id, added, removed, cx)` once
//!   `ReviewStore` has counted the hunks, and tells the panel which project the
//!   mentions belong to with [`ChatPanel::set_project_root`] so a `@` shows
//!   `main.rs · src` instead of `/home/…/src/main.rs`.
//! - **Theme and settings**: [`ChatTheme`] and [`ChatSettings`] are plain
//!   `Default` structs (like `cincel_editor::EditorTheme`), hot-reloaded with
//!   [`ChatPanel::set_theme`] and [`ChatPanel::set_settings`].
//! - **Key bindings**: [`default_key_bindings`] returns the Linux defaults of
//!   `02-visual.md` §8 for the chat; install them with [`bind_default_keys`]
//!   before the user's keymap. Actions: `chat::send`, `chat::newline`,
//!   `chat::cancel_turn`, `chat::focus_input`, `chat::new_session`, plus the
//!   popover and permission ones. Key contexts: `Chat`, `Chat && permission`
//!   and `Chat > Composer`. [`bind_default_keys`] installs the code editor's
//!   defaults first (the composer is an editor) and the chat's after them.
//!   Install them **after** `gpui_kit::init`: at equal depth GPUI keeps the
//!   binding added last.
//! - **Composer**: an `cincel_editor::EditorView` configured as a Markdown
//!   text field (minimal chrome, soft wrap, one to eight rows then scrolling,
//!   prose in the interface font and fenced code in the mono one), reachable
//!   with [`ChatPanel::composer`]. Its [`composer`] decorator paints the
//!   Markdown structure with the twelve syntax tokens and gives fenced blocks a
//!   `bg.editor` background and their language's highlighting. The composer
//!   box takes the editor's own actions in the capture phase: `Enter`
//!   (`editor::insert_newline`) answers a pending permission, confirms the
//!   open popover or sends; the arrows, `Tab` and `Esc` drive the popover
//!   while it lists something; `Shift+Enter` is `chat::newline`.
//! - **Zoom**: [`ChatSettings::scale`] multiplies every pixel size of the
//!   panel (type scale, rows, radii, the composer's font); the workspace sets
//!   it from `workspace::zoom_*`.
//! - **Selection**: every text view of the chat (answers, bubbles, code
//!   blocks, the composer) selects with `text.accent` at 35 %
//!   ([`ChatTheme::text_selection`]).
//! - **Persistence**: [`ChatPanel::export_transcript`] returns a serde
//!   [`TranscriptDump`] and [`ChatPanel::import_transcript`] restores it, so the
//!   workspace can keep one transcript per project in its XDG state directory.
//!
//! # What the chat never does
//!
//! It never renders a full diff. An edit tool card shows the path, `+N −M` and
//! at most [`ChatSettings::diff_preview_lines`] lines of context with the note
//! "Revisá en el editor" and a button that emits
//! [`ChatEvent::OpenFileAtHunk`] (`chat.md`, acceptance criteria).
//!
//! # Streaming
//!
//! An `AgentText` entry owns one `gpui-kit` `TextViewState` per segment
//! (prose, or a top-level code block of more than 20 lines, which is painted
//! folded: [`code_folds`]); every `session/update` chunk is appended to the
//! last segment with `push_str`, which reparses only the tail, so the answer
//! grows without the transcript flickering. Fenced code blocks are
//! highlighted by `cincel-syntax` (see [`markdown`]) and carry a "Copiar"
//! button that emits [`ChatEvent::CopyToClipboard`] with the whole block.
//!
//! # Deviations from the spec
//!
//! - There is no autonomy selector (removed in Etapa 3): the permission
//!   policy is fixed on the workspace side (`cincel_acp::PermissionPolicy`)
//!   and the only selectors of the composer are the agent's own ACP ones.
//! - Inline code inside an answer is painted at 0.875 × the body size
//!   (≈ 11.4 px), not at [`settings::TEXT_CODE`]: `gpui-kit` 0.6.1 hard-codes
//!   that scale for inline code spans and exposes no setting for it. Fenced
//!   blocks, commands and outputs do use 12.5 px.
//! - The `@` mention is an **inline token** in the draft (`@calculadora.py`),
//!   not an inline pill: the composer is a text editor with no inline widgets,
//!   so the token is real text (painted in the `function` colour) the user can
//!   type around and delete by hand. On send [`split_mentions`] cuts the text around the
//!   tokens into ordered `PromptBlock::Text` / `ResourceLink` blocks with the
//!   absolute path; in the bubble the mention is painted as an inline chip
//!   with the file name and the relative path as a tooltip. Two files with the
//!   same name get `@dir/name` tokens ([`mention_token`]).
//! - A paragraph of a user bubble that holds a mention is laid out as plain
//!   text and inline chips, not through Markdown (the renderer has no inline
//!   widgets to put the chip in); the other paragraphs of the same message are
//!   Markdown. A mention inside a list item or a code block therefore turns
//!   that block into plain text.
//! - In the composer's prose rows (proportional font) `↑` / `↓` keep the
//!   character column, not the pixel x, so the caret may drift sideways a
//!   little between lines of very different letters; soft wrap and clicks do
//!   measure real glyph advances.
//! - A turn ends with 12 px of air instead of the "turno terminado · 12 s" rule
//!   of §7: the line said nothing the transcript did not already show and it
//!   read as a section heading in the middle of an answer. `TurnSeparator` is
//!   still in the model and in the dump, so the duration is not lost.
//! - The popovers (`@`, `/`, connections, sessions, selectors) are painted by the
//!   panel itself instead of `gpui-kit`'s `Popover`, which anchors to an entity
//!   trigger and cannot be driven from the keyboard the way `chat.md` needs.
//! - Dragging a file from the tree (`chat.md`) is the workspace's half of the
//!   feature; the panel exposes [`ChatPanel::insert_mention`] for it and,
//!   since Etapa 7, [`ChatPanel::mention_or_attach`] (an image is attached as
//!   an image, anything else is still a mention).
//! - On Wayland a file manager hands copied files over as `file://` text,
//!   not as `ClipboardEntry::ExternalPaths`: `Ctrl+V` reads such a text made
//!   only of `file://` lines as files ([`attachments::file_uri_paths`]).
//!
//! # Images and comments (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5, §6.6)
//!
//! - **Attaching**: `Ctrl+V` with an image in the clipboard, a drop of files
//!   from the system file manager on the composer box and the paperclip button
//!   ([`ChatEvent::PickImages`] → the workspace's file dialog →
//!   [`ChatPanel::attach_paths`]) all end in [`ChatPanel::attach_images`]. The
//!   capability comes from `AgentEvent::Connected` or
//!   [`ChatPanel::set_image_support`]; without it nothing is attached and a
//!   [`ChatEvent::Notify`] says why. [`attachments::prepare_image`] reduces an
//!   image above 2 000 px and refuses one above 10 MB, at most 10 per message.
//! - **Sending**: the prompt is the text and mentions, then one
//!   `PromptBlock::Image` per image; the message keeps a
//!   [`MessageBlock::Image`] ([`ImageRef`], bytes in memory until the workspace
//!   stores them as `conversations/<id>/images/<sha256>.<ext>`,
//!   [`Conversation::images_to_store`]). A click on a sent thumbnail emits
//!   [`ChatEvent::OpenImage`]; [`ChatPanel::set_conversation_dir`] says where
//!   stored images are.
//! - **Comments**: [`ChatPanel::set_pending_comments`] paints one tag per
//!   unsent comment inside the box (its `×` emits [`ChatEvent::RemoveComment`],
//!   a click [`ChatEvent::OpenLocation`]); [`ChatPanel::attach_sent_comments`]
//!   adds [`MessageBlock::Comment`] cards to the last message and
//!   [`ChatPanel::mark_comments_not_sent`] flags them when it never left.
//! - [`CONVERSATION_VERSION`] is 2; [`Conversation::migrate`] reads a version 1
//!   conversation as it is.
//! - The history popover does load a conversation now; whether its ACP session
//!   comes back is the agent's call (`agentCapabilities.loadSession` /
//!   `sessionCapabilities.resume`). When it cannot,
//!   [`READ_ONLY_HISTORY_NOTICE`] is shown and the next message opens a fresh
//!   session with the history still visible.

#![deny(missing_docs)]

pub mod actions;
pub mod attachments;
pub mod code_folds;
pub mod comments;
pub mod composer;
pub mod events;
pub mod markdown;
pub mod model;
pub mod panel;
pub mod panel_media;
pub mod render;
pub mod render_media;
pub mod settings;
pub mod theme;

#[cfg(all(test, feature = "test-support"))]
mod tests;

#[cfg(all(test, feature = "test-support"))]
mod activity_row_tests;
#[cfg(all(test, feature = "test-support"))]
mod attachments_tests;
#[cfg(all(test, feature = "test-support"))]
mod code_fold_tests;
#[cfg(all(test, feature = "test-support"))]
mod comment_cards_tests;
#[cfg(all(test, feature = "test-support"))]
mod connection_rows_tests;
#[cfg(all(test, feature = "test-support"))]
mod connections_view_tests;
#[cfg(all(test, feature = "test-support"))]
mod header_badge_tests;
#[cfg(all(test, feature = "test-support"))]
mod menu_dismiss_tests;
#[cfg(all(test, feature = "test-support"))]
mod message_media_order_tests;
#[cfg(all(test, feature = "test-support"))]
mod transcript_follow_tests;

pub use actions::{
    AcceptPermission, AttachImage, CancelTurn, ClosePopover, FocusInput, NewSession, Newline,
    PopoverConfirm, PopoverNext, PopoverPrev, RejectPermission, Send, bind_default_keys,
    default_key_bindings,
};
pub use attachments::{
    IMAGE_EXTENSIONS, ImageError, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, MAX_IMAGES_PER_MESSAGE,
    PICK_DIALOG_FILTER, PICK_DIALOG_TITLE, PreparedImage, prepare_image,
};
pub use comments::{
    COMMENT_STATE_ACCEPTED, COMMENT_STATE_MIXED, COMMENT_STATE_NO_AGENT_CHANGE,
    COMMENT_STATE_PENDING, COMMENT_STATE_REJECTED, PendingComment, SentCommentCard,
    SentCommentKind, TagSource, comment_tag_labels, comment_tag_tooltip,
};
pub use events::ChatEvent;
pub use markdown::CodeHighlighter;
pub use model::{
    AgentStatus, AgentText, AgentThought, AuthChoice, AuthEntry, CONVERSATION_VERSION,
    ChatConnection, ChatImageSource, ConnectionBadge, ConnectionBanner, Conversation,
    ConversationSummary, Entry, ImageRef, LEGACY_CONNECTION_GROUP, LEGACY_HISTORY_NOTICE, Mention,
    MessageBlock, NO_CONNECTION_TEXT, Notice, NoticeLevel, PermissionChoice, PermissionEntry,
    PlanEntry, PlanItem, READ_ONLY_HISTORY_NOTICE, TITLE_MAX_CHARS, TRANSCRIPT_VERSION,
    TextSegment, ToolCallEntry, ToolContent, TranscriptDump, TurnSeparator, UNTITLED_CONVERSATION,
    UserMessage, chip_label, conversation_title, exit_code_in, file_name_label, mention_token,
    parent_label, provider_monogram, provider_name, relative_label, split_mentions,
    stop_reason_label, strip_code_fences, tool_kind_icon, tool_kind_label, tool_status_label,
    visible_agent_type,
};
pub use panel::{ChatPanel, Popover};
pub use panel_media::{AttachmentSource, AttachmentState, PendingAttachment};
pub use render::provider_icon;
pub use settings::ChatSettings;
pub use theme::ChatTheme;

/// Installs the chat's key bindings.
///
/// Call it after `gpui_kit::init`, which installs the `Input` bindings the
/// composer relies on, and before loading the user's `keymap.json`.
pub fn init(cx: &mut gpui::App) {
    bind_default_keys(cx);
}
