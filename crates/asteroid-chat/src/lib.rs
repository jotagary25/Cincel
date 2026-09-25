//! `asteroid-chat`: the chat panel of Asteroid, on GPUI.
//! See `docs/specs/modulos/chat.md` and `docs/specs/02-visual.md` §7–§9.
//!
//! # Public API (stable for the workspace)
//!
//! ```no_run
//! use asteroid_chat::{ChatAgent, ChatEvent, ChatPanel, ChatSettings, ChatTheme};
//! # use gpui::{App, AppContext, Window};
//! # fn demo(window: &mut Window, cx: &mut App) {
//! let panel = cx.new(|cx| {
//!     ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
//! });
//! panel.update(cx, |panel, cx| {
//!     panel.set_agents(vec![ChatAgent::installed("claude-acp", "Claude")], cx);
//! });
//! # let _ = panel;
//! # }
//! ```
//!
//! - **Entity**: [`ChatPanel::new(theme, settings, window, cx)`](ChatPanel::new).
//!   The workspace owns the two `async-channel`s of
//!   `03-arquitectura.md` §3 and forwards every `AgentEvent` with
//!   [`ChatPanel::handle_event`]; the panel never talks to `asteroid-acp`
//!   itself.
//! - **Events**: the panel is an `EventEmitter<`[`ChatEvent`]`>`:
//!   `Command(AgentCommand)` (prompt, cancel, new session, set mode / config
//!   option, respond permission, authenticate), `OpenFileAtHunk { path }` (the
//!   "Ver en el editor" of an edit card), `AutonomyChanged(AutonomyMode)`,
//!   `RequestFileList { query, reply }` (the `@` picker; the workspace answers
//!   from the worktree, or calls [`ChatPanel::set_file_candidates`] directly),
//!   `OpenTerminalWithCommand(String)` and `CopyToClipboard(String)`.
//! - **Header**: [`ChatPanel::set_agents`] fills the agent selector with
//!   [`ChatAgent`]s (`installed: false` dims the row and shows its `hint`),
//!   [`ChatPanel::set_conversations`] fills the history popover with
//!   [`ConversationSummary`]s, and [`ChatPanel::status`] is the pill: `listo`,
//!   `pensando…`, `esperando permiso`, `desconectado`,
//!   `autenticación requerida`.
//! - **Conversations**: one agent, one conversation. Picking an agent emits
//!   [`ChatEvent::AgentSelected`], the `+` emits
//!   [`ChatEvent::NewConversation`], and a history row emits
//!   [`ChatEvent::OpenConversation`] / [`ChatEvent::DeleteConversation`]; the
//!   workspace owns `conversations/<id>.json` and answers with
//!   [`ChatPanel::start_new_conversation`] / [`ChatPanel::load_conversation`].
//!   While a `session/load` replays history,
//!   [`ChatPanel::begin_replay`] drops the updates already on screen.
//! - **Transcript**: [`Entry`] is exactly the list of `chat.md` — `UserMessage`,
//!   `AgentText` (streaming markdown), `AgentThought`, `ToolCall`, `Plan`,
//!   `Permission`, `AuthRequired`, `Notice`, `TurnSeparator`. A user message is
//!   a bubble against the right edge and an agent answer is full width with a
//!   2 px accent rule, so the two voices never blur together. The workspace
//!   feeds the `+N −M` of an edit card with
//!   [`ChatPanel::set_tool_stats`]`(tool_call_id, added, removed, cx)` once
//!   `ReviewStore` has counted the hunks, and tells the panel which project the
//!   mentions belong to with [`ChatPanel::set_project_root`] so a `@` shows
//!   `main.rs · src` instead of `/home/…/src/main.rs`.
//! - **Theme and settings**: [`ChatTheme`] and [`ChatSettings`] are plain
//!   `Default` structs (like `asteroid_editor::EditorTheme`), hot-reloaded with
//!   [`ChatPanel::set_theme`] and [`ChatPanel::set_settings`].
//! - **Key bindings**: [`default_key_bindings`] returns the Linux defaults of
//!   `02-visual.md` §8 for the chat; install them with [`bind_default_keys`]
//!   before the user's keymap. Actions: `chat::send`, `chat::newline`,
//!   `chat::cancel_turn`, `chat::focus_input`, `chat::new_session`, plus the
//!   popover and permission ones. Key contexts: `Chat`, `Chat && permission`
//!   and `Chat > Input` — the last one is the composer, whose `Input` node sits
//!   deeper than the panel's and would otherwise keep `Enter`, `Tab` and the
//!   arrows away from an open `@` / `/` popover. Install them **after**
//!   `gpui_kit::init`: at equal depth GPUI keeps the binding added last.
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
//! An `AgentText` entry owns a `gpui-kit` `TextViewState`; every
//! `session/update` chunk is appended with `push_str`, which reparses only the
//! tail, so the answer grows without the transcript flickering. Fenced code
//! blocks are highlighted by `asteroid-syntax` (see [`markdown`]) and carry a
//! "Copiar" button that emits [`ChatEvent::CopyToClipboard`].
//!
//! # Deviations from the spec
//!
//! - `AutonomyChanged` carries `asteroid_acp::AutonomyMode`, not `Autonomy`:
//!   since E2 the latter is a policy object holding the sensitive-path globs,
//!   which belong to the ACP side, and only the mode is a UI choice.
//! - The `@` mention is an **inline token** in the draft (`@calculadora.py`),
//!   not an inline pill: `gpui-kit`'s `Textarea` holds plain text and has no
//!   inline-widget API, so the token is real text the user can type around and
//!   delete by hand. On send [`split_mentions`] cuts the text around the
//!   tokens into ordered `PromptBlock::Text` / `ResourceLink` blocks with the
//!   absolute path; in the bubble the mention is painted as an inline chip
//!   with the file name and the relative path as a tooltip. Two files with the
//!   same name get `@dir/name` tokens ([`mention_token`]).
//! - A turn ends with 12 px of air instead of the "turno terminado · 12 s" rule
//!   of §7: the line said nothing the transcript did not already show and it
//!   read as a section heading in the middle of an answer. `TurnSeparator` is
//!   still in the model and in the dump, so the duration is not lost.
//! - The popovers (`@`, `/`, agent, sessions, selectors) are painted by the
//!   panel itself instead of `gpui-kit`'s `Popover`, which anchors to an entity
//!   trigger and cannot be driven from the keyboard the way `chat.md` needs.
//! - Dragging a file from the tree (`chat.md`) is the workspace's half of the
//!   feature; the panel exposes [`ChatPanel::insert_mention`] for it.
//! - Pasting an image is not supported in v1 and there is no notice yet.
//! - The history popover does load a conversation now; whether its ACP session
//!   comes back is the agent's call (`agentCapabilities.loadSession` /
//!   `sessionCapabilities.resume`). When it cannot,
//!   [`READ_ONLY_HISTORY_NOTICE`] is shown and the next message opens a fresh
//!   session with the history still visible.

#![deny(missing_docs)]

pub mod actions;
pub mod events;
pub mod markdown;
pub mod model;
pub mod panel;
pub mod render;
pub mod settings;
pub mod theme;

#[cfg(all(test, feature = "test-support"))]
mod tests;

pub use actions::{
    AcceptPermission, CancelTurn, ClosePopover, FocusInput, NewSession, Newline, PopoverConfirm,
    PopoverNext, PopoverPrev, RejectPermission, Send, bind_default_keys, default_key_bindings,
};
pub use events::ChatEvent;
pub use markdown::CodeHighlighter;
pub use model::{
    AUTONOMY_VALUES, AgentStatus, AgentText, AgentThought, AuthChoice, AuthEntry,
    CONVERSATION_VERSION, ChatAgent, Conversation, ConversationSummary, Entry, Mention,
    MessageBlock, Notice, NoticeLevel, PermissionChoice, PermissionEntry, PlanEntry, PlanItem,
    READ_ONLY_HISTORY_NOTICE, TITLE_MAX_CHARS, TRANSCRIPT_VERSION, ToolCallEntry, ToolContent,
    TranscriptDump, TurnSeparator, UNTITLED_CONVERSATION, UserMessage, autonomy_label,
    autonomy_tooltip, chip_label, conversation_title, exit_code_in, file_name_label, mention_token,
    parent_label, relative_label, split_mentions, stop_reason_label, strip_code_fences,
    tool_kind_icon, tool_kind_label, tool_status_label,
};
pub use panel::{ChatPanel, Popover};
pub use settings::ChatSettings;
pub use theme::ChatTheme;

/// Installs the chat's key bindings.
///
/// Call it after `gpui_kit::init`, which installs the `Input` bindings the
/// composer relies on, and before loading the user's `keymap.json`.
pub fn init(cx: &mut gpui::App) {
    bind_default_keys(cx);
}
