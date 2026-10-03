//! [`ChatPanel`]: the entity, its state and everything that is not painting.
//!
//! Painting lives in [`crate::render`], which is a second `impl ChatPanel`
//! block so this file stays about the model.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use cincel_acp::acp::schema::v1::{
    AuthMethod, AvailableCommand, ContentBlock, PermissionOption, SessionConfigId,
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigOptionValue,
    SessionConfigSelectOption, SessionConfigSelectOptions, SessionConfigValueId, SessionId,
    SessionModeId, SessionModeState, SessionUpdate, ToolCallContent, ToolCallId, ToolCallStatus,
    ToolCallUpdate, ToolKind,
};
use cincel_acp::protocol::PromptBlock;
use cincel_acp::{AgentCommand, AgentEvent, PermissionOutcome, PermissionRequestId};
use cincel_editor::{AutoHeight, EditorChrome, EditorEvent, EditorSettings, EditorView};
use cincel_syntax::LanguageRegistry;
use cincel_text::Buffer;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FollowMode,
    KeyContext, ListAlignment, ListState, SharedString, Subscription, Window, px,
};
use gpui_kit::base::text::TextViewState;

use crate::actions;
use crate::code_folds::{CodeBlockKey, split_markdown};
use crate::events::ChatEvent;
use crate::markdown::{CodeHighlighter, append_chunk, markdown_state, set_text};
use crate::model::*;
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

/// Which overlay is open over the panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Popover {
    /// None.
    #[default]
    Closed,
    /// The "Conectar" popover of the header: one row per connection, and
    /// "Conectar nuevo agente…" / "Eliminar conexión…" at the foot
    /// (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4 F1).
    Connections,
    /// The context menu of one row of [`Popover::Connections`] ("Renombrar"),
    /// by connection id.
    ConnectionMenu(String),
    /// The conversation history of the header, grouped by agent.
    Conversations,
    /// The `@` file picker; `start` is the byte offset of the `@` in the input.
    Files {
        /// Byte offset of the `@` that opened it.
        start: usize,
        /// What the user has typed after the `@`.
        query: String,
        /// Row the arrows have moved to.
        selected: usize,
    },
    /// The `/` command list; `start` is the byte offset of the `/`.
    Commands {
        /// Byte offset of the `/` that opened it.
        start: usize,
        /// What the user has typed after the `/`.
        query: String,
        /// Row the arrows have moved to.
        selected: usize,
    },
    /// A `ConfigOption` selector of the footer, by index in `config_options`.
    Config(usize),
    /// The legacy `availableModes` selector of the footer.
    Modes,
}

impl Popover {
    /// Whether the overlay takes the keyboard focus while it is open.
    ///
    /// The header menus and the footer selectors do (they are menus the user
    /// navigates on their own); `@` and `/` do not, because the user keeps
    /// typing in the composer while they list something
    /// (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §4.2).
    #[must_use]
    pub fn takes_focus(&self) -> bool {
        matches!(
            self,
            Popover::Connections
                | Popover::ConnectionMenu(_)
                | Popover::Conversations
                | Popover::Config(_)
                | Popover::Modes
        )
    }
}

/// How many stderr lines are kept for the "agente caído" card (`02-visual.md` §9).
const STDERR_KEPT: usize = 8;

/// The placeholder while no connection is active.
const DEFAULT_PLACEHOLDER: &str = "Conectá un agente para empezar…";

/// The line under the input that names the four things the composer does.
pub const COMPOSER_HINT: &str =
    "@ archivos · / comandos · Enter envía · Shift+Enter salto de línea";

/// Interface families of the composer's prose rows, after the one gpui-kit's
/// theme resolved (02-visual §3: Inter, then the system sans).
const PROSE_FALLBACKS: &[&str] = &["Inter", "Cantarell", "Noto Sans", "DejaVu Sans"];
/// Code families of the composer's fenced rows, after gpui-kit's mono.
const CODE_FALLBACKS: &[&str] = &[
    "JetBrains Mono",
    "JetBrainsMono Nerd Font Mono",
    "Zed Mono",
    "DejaVu Sans Mono",
    "monospace",
];

/// The composer's editor settings: a Markdown text field (no gutter, soft
/// wrap, 1 to 8 rows then scrolling, no auto-closed brackets) in the body size
/// of the type scale, prose in the interface font and code in the mono one.
pub(crate) fn composer_settings(settings: &ChatSettings, cx: &App) -> EditorSettings {
    let typography = &gpui_kit::base::Theme::global(cx).tokens.typography;
    let mut prose = vec![typography.sans.to_string()];
    prose.extend(PROSE_FALLBACKS.iter().map(|family| family.to_string()));
    let mut code = vec![typography.mono.to_string()];
    code.extend(CODE_FALLBACKS.iter().map(|family| family.to_string()));
    EditorSettings {
        soft_wrap: true,
        auto_close_pairs: false,
        font_family: code,
        font_size: settings.text_body(),
        line_height: 1.5,
        chrome: EditorChrome::Minimal,
        auto_height: Some(AutoHeight {
            min_rows: settings.input_min_rows.max(1) as u32,
            max_rows: settings.input_max_rows.max(1) as u32,
        }),
        prose_font_family: Some(prose),
        ..EditorSettings::default()
    }
}

/// Markdown states of the user bubbles: `(entry, piece, segment)` → (text,
/// state); a piece is cut into segments like an answer (§10.3).
pub(crate) type BubbleViews = HashMap<(usize, usize, usize), (String, Entity<TextViewState>)>;

/// The chat panel (`docs/specs/modulos/chat.md`).
pub struct ChatPanel {
    theme: ChatTheme,
    settings: ChatSettings,
    focus_handle: FocusHandle,
    /// The composer: an `cincel_editor::EditorView` configured as a Markdown
    /// text field ([`composer_settings`], [`crate::composer`]).
    pub(crate) input: Entity<EditorView>,
    /// The mention tokens the composer's decorator was last built with.
    decorated_mentions: Vec<String>,
    /// Markdown states of the user bubbles, per `(entry, piece, segment)`, with the
    /// text they were built from (rebuilt when it differs).
    pub(crate) bubble_views: RefCell<BubbleViews>,
    /// The transcript's `gpui::list`: one item per [`Entry`], following the
    /// tail. A list and not a scrolling `div`, because the list lays its items
    /// out inside the panel's text style: `gpui-kit` measures a paragraph with
    /// inline code from `window.text_style()` at layout time, which outside a
    /// list is the window default (16 px) instead of the chat's 13 px.
    pub(crate) list: ListState,
    pub(crate) entries: Vec<Entry>,
    /// The rows of the "Conectar" popover, as the workspace last set them.
    pub(crate) connections: Vec<ChatConnection>,
    /// Id of the connection whose process is running, if any.
    pub(crate) active_connection: Option<String>,
    /// Row highlighted when the popover opens with no active connection
    /// (`connections.default_label`; it never connects by itself).
    pub(crate) preselected_connection: Option<String>,
    /// "La sesión de «X» venció" / "«X» no está disponible" above the
    /// transcript.
    pub(crate) banner: Option<ConnectionBanner>,
    pub(crate) conversations: Vec<ConversationSummary>,
    pub(crate) active_conversation: Option<String>,
    /// The muted line above the transcript when the loaded conversation
    /// cannot be resumed ([`crate::model::READ_ONLY_HISTORY_NOTICE`]).
    pub(crate) history_notice: Option<String>,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) status: AgentStatus,
    pub(crate) config_options: Vec<SessionConfigOption>,
    pub(crate) modes: Option<SessionModeState>,
    pub(crate) commands: Vec<AvailableCommand>,
    /// `@` mentions of the current draft: token → absolute path.
    pub(crate) mentions: Vec<Mention>,
    pub(crate) popover: Popover,
    /// Focus of the open header menu or footer selector: the menu's own focus
    /// lets "the focus went somewhere else" close it, whatever took it (the
    /// editor, the tree, `Ctrl+L`, another window).
    pub(crate) popover_focus: FocusHandle,
    /// Whoever had the focus before the menu took it; `Esc` and choosing a row
    /// give it back.
    restore_focus: Option<FocusHandle>,
    /// A menu that takes the focus was opened and has not been given it yet
    /// (the focus can only move while painting, where the window is at hand).
    focus_popover_pending: bool,
    /// The menu a click outside just closed, and the press that did it. A
    /// click on the button that had opened the menu comes right after and
    /// must leave it closed instead of opening it again.
    just_dismissed: Option<(Popover, u64)>,
    /// Counts the mouse presses over the panel, to tell `just_dismissed`'s
    /// press from an old one.
    pub(crate) press_serial: u64,
    pub(crate) file_candidates: Vec<PathBuf>,
    pub(crate) project_root: Option<PathBuf>,
    pub(crate) auth_methods: Vec<AuthMethod>,
    pub(crate) stderr: Vec<String>,
    pub(crate) registry: Arc<LanguageRegistry>,
    tool_index: HashMap<ToolCallId, usize>,
    pending_permission: Option<usize>,
    /// While a `session/load` replays the conversation, an update that is
    /// already on screen is dropped instead of appended
    /// ([`ChatPanel::begin_replay`]).
    replaying: bool,
    placeholder_dirty: bool,
    turn_started: Option<Instant>,
    pub(crate) thought_started: Option<Instant>,
    /// The last kind of signal the agent sent in the running turn: what the
    /// activity row says ([`ChatPanel::activity`]). Never stored.
    activity_signal: AgentActivity,
    /// The images of the draft, in the order they were attached
    /// (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.3.1).
    pub(crate) attachments: Vec<crate::panel_media::PendingAttachment>,
    /// Id of the next [`crate::panel_media::PendingAttachment`].
    pub(crate) next_attachment_id: u64,
    /// Whether the active connection accepts images: `None` without one (or
    /// before its `Connected`), D6.
    pub(crate) image_support: Option<bool>,
    /// Folder of the conversation on screen; sent images whose bytes are not
    /// in memory are read from its `images/` (D7).
    pub(crate) conversation_dir: Option<PathBuf>,
    /// The unsent review comments, one tag each inside the composer box.
    pub(crate) pending_comments: Vec<crate::comments::PendingComment>,
    /// Decoded-once thumbnails and the "is the file there" answers.
    pub(crate) media_cache: RefCell<crate::panel_media::MediaCache>,
    /// Comment cards whose code is unfolded, by `(entry, block)`.
    pub(crate) expanded_cards: std::collections::HashSet<(usize, usize)>,
    /// Long code blocks the user unfolded ("Ver más"), for as long as the
    /// conversation stays open (`docs/specs/10-etapa7-ronda2.md` §10.1).
    pub(crate) expanded_code_blocks: std::collections::HashSet<CodeBlockKey>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChatEvent> for ChatPanel {}

impl Focusable for ChatPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ChatPanel {
    /// Builds the panel. The workspace owns the ACP channels and forwards every
    /// [`AgentEvent`] with [`ChatPanel::handle_event`].
    pub fn new(
        theme: ChatTheme,
        settings: ChatSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let registry = Arc::new(LanguageRegistry::new());
        let editor_settings = composer_settings(&settings, cx);
        let composer_theme = theme.composer_theme();
        let input = cx.new(|cx| {
            let buffer = cincel_editor::shared(Buffer::new(""));
            let markdown = registry.language("markdown");
            let mut editor = EditorView::new(
                buffer,
                markdown,
                registry.clone(),
                editor_settings,
                composer_theme,
                window,
                cx,
            );
            editor.set_placeholder(Some(SharedString::from(DEFAULT_PLACEHOLDER)), cx);
            editor.set_decorator(
                Some(crate::composer::decorator(
                    registry.clone(),
                    theme.bg_editor.into(),
                    Vec::new(),
                )),
                cx,
            );
            editor
        });
        let subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        let popover_focus = cx.focus_handle();
        let composer_focus = input.read(cx).focus_handle(cx);
        // The menus close by themselves (`docs/specs/09-etapa7-…` §4): the
        // focus leaving the menu's own handle (a click on the editor or the
        // tree, `Ctrl+L`, another window), the composer losing it while `@` or
        // `/` list something, and the window going to the background.
        let menu_focus_out = cx.on_focus_out(&popover_focus, window, |this, _, _, cx| {
            this.dismiss_popover(cx);
        });
        let composer_focus_out = cx.on_focus_out(&composer_focus, window, |this, _, _, cx| {
            if matches!(
                this.popover,
                Popover::Files { .. } | Popover::Commands { .. }
            ) {
                this.dismiss_popover(cx);
            }
        });
        let window_activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.dismiss_popover(cx);
            }
        });
        Self {
            theme,
            settings,
            focus_handle: cx.focus_handle(),
            input,
            decorated_mentions: Vec::new(),
            bubble_views: RefCell::new(HashMap::new()),
            list: {
                let list = ListState::new(0, ListAlignment::Top, px(1024.));
                list.set_follow_mode(FollowMode::Tail);
                list
            },
            entries: Vec::new(),
            connections: Vec::new(),
            active_connection: None,
            preselected_connection: None,
            banner: None,
            conversations: Vec::new(),
            active_conversation: None,
            history_notice: None,
            session_id: None,
            status: AgentStatus::Disconnected,
            config_options: Vec::new(),
            modes: None,
            commands: Vec::new(),
            mentions: Vec::new(),
            popover: Popover::Closed,
            popover_focus,
            restore_focus: None,
            focus_popover_pending: false,
            just_dismissed: None,
            press_serial: 0,
            file_candidates: Vec::new(),
            project_root: None,
            auth_methods: Vec::new(),
            stderr: Vec::new(),
            registry,
            tool_index: HashMap::new(),
            pending_permission: None,
            replaying: false,
            placeholder_dirty: false,
            turn_started: None,
            thought_started: None,
            activity_signal: AgentActivity::Thinking,
            attachments: Vec::new(),
            next_attachment_id: 0,
            image_support: None,
            conversation_dir: None,
            pending_comments: Vec::new(),
            media_cache: RefCell::new(crate::panel_media::MediaCache::default()),
            expanded_cards: std::collections::HashSet::new(),
            expanded_code_blocks: std::collections::HashSet::new(),
            _subscriptions: vec![
                subscription,
                menu_focus_out,
                composer_focus_out,
                window_activation,
            ],
        }
    }

    // ---------------------------------------------------------------- getters

    /// The transcript, in order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// What the header pill says.
    #[must_use]
    pub fn status(&self) -> AgentStatus {
        self.status
    }

    /// The rows of the "Conectar" popover.
    #[must_use]
    pub fn connections(&self) -> &[ChatConnection] {
        &self.connections
    }

    /// The connection whose agent is running, when there is one. The header
    /// shows its label; without one it shows the "Conectar" button.
    #[must_use]
    pub fn active_connection(&self) -> Option<&ChatConnection> {
        let id = self.active_connection.as_deref()?;
        self.connections
            .iter()
            .find(|connection| connection.id == id)
    }

    /// The badge of the header: the state of the active connection only, by
    /// priority (expired session > unavailable > authentication required >
    /// disconnected > connected). `None` without an active connection. The
    /// agent's activity (thinking, working, waiting for a permission) never
    /// changes it (`docs/specs/10-etapa7-ronda2.md` §3.1).
    #[must_use]
    pub fn header_badge(&self) -> Option<HeaderBadge> {
        let connection = self.active_connection()?;
        Some(match (&connection.badge, self.status) {
            (ConnectionBadge::SessionExpired, _) => HeaderBadge::SessionExpired,
            (ConnectionBadge::Unavailable { reason }, _) => HeaderBadge::Unavailable {
                reason: reason.clone(),
            },
            (_, AgentStatus::AuthRequired) => HeaderBadge::AuthRequired,
            (_, AgentStatus::Disconnected) => HeaderBadge::Disconnected,
            _ => HeaderBadge::Connected,
        })
    }

    /// The row the popover highlights when nothing is active.
    #[must_use]
    pub fn preselected_connection(&self) -> Option<&str> {
        self.preselected_connection.as_deref()
    }

    /// The banner above the transcript, if any.
    #[must_use]
    pub fn banner(&self) -> Option<&ConnectionBanner> {
        self.banner.as_ref()
    }

    /// The rows of the history popover, as the workspace last set them.
    #[must_use]
    pub fn conversations(&self) -> &[ConversationSummary] {
        &self.conversations
    }

    /// Id of the conversation on screen, when it came from the history.
    #[must_use]
    pub fn active_conversation(&self) -> Option<&str> {
        self.active_conversation.as_deref()
    }

    /// The muted line above the transcript, when there is one.
    #[must_use]
    pub fn history_notice(&self) -> Option<&str> {
        self.history_notice.as_deref()
    }

    /// The config options the footer paints a selector for.
    #[must_use]
    pub fn config_options(&self) -> &[SessionConfigOption] {
        &self.config_options
    }

    /// The slash commands the agent announced.
    #[must_use]
    pub fn commands(&self) -> &[AvailableCommand] {
        &self.commands
    }

    /// The `@` mentions of the current draft, in insertion order.
    #[must_use]
    pub fn mentions(&self) -> &[Mention] {
        &self.mentions
    }

    /// The blocks [`ChatPanel::send`] would emit for the current draft: the
    /// text split around the mention tokens that are still there.
    #[must_use]
    pub fn draft_blocks(&self, cx: &App) -> Vec<MessageBlock> {
        split_mentions(&self.input_text(cx), &self.mentions)
    }

    /// The session the panel is talking to.
    #[must_use]
    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    /// Which overlay is open.
    #[must_use]
    pub fn popover(&self) -> &Popover {
        &self.popover
    }

    /// The text currently in the composer.
    #[must_use]
    pub fn input_text(&self, cx: &App) -> String {
        self.input.read(cx).text()
    }

    /// The composer's editor, for the workspace and the tests.
    #[must_use]
    pub fn composer(&self) -> &Entity<EditorView> {
        &self.input
    }

    /// The theme in use.
    #[must_use]
    pub fn theme(&self) -> &ChatTheme {
        &self.theme
    }

    /// The settings in use.
    #[must_use]
    pub fn settings(&self) -> &ChatSettings {
        &self.settings
    }

    /// Whether a permission request is waiting for an answer. While it is,
    /// sending is blocked (`chat.md`, acceptance criteria).
    #[must_use]
    pub fn is_awaiting_permission(&self) -> bool {
        self.pending_permission.is_some()
    }

    /// What the activity row at the foot of the transcript says
    /// (`docs/specs/10-etapa7-ronda2.md` §8): `None` outside a turn, while a
    /// `session/load` replays, and when the live thought row ("Pensando…
    /// (N s)", with its own spinner) is the last item, so it is not doubled.
    pub(crate) fn activity(&self) -> Option<AgentActivity> {
        if self.replaying {
            return None;
        }
        if self.pending_permission.is_some() && self.status == AgentStatus::WaitingPermission {
            return Some(AgentActivity::WaitingPermission);
        }
        if self.status != AgentStatus::Thinking {
            return None;
        }
        let live_thought = self.thought_started.is_some()
            && matches!(self.entries.last(), Some(Entry::AgentThought(_)));
        (!live_thought).then_some(self.activity_signal)
    }

    /// Whether the transcript keeps its view at the end as things arrive
    /// (`docs/specs/10-etapa7-ronda2.md` §9, R6).
    pub(crate) fn is_following_tail(&self) -> bool {
        self.list.is_following_tail()
    }

    /// Whether the "Ir al final" arrow is on screen: the user scrolled away
    /// from the end and there is somewhere to go.
    ///
    /// `ListState::is_scrolled_to_end` answers `None` while any item has no
    /// height yet (the top of a long conversation nobody scrolled to, or the
    /// entries that arrived below the view after the user scrolled up), so it
    /// is completed with the same comparison over the heights the list does
    /// know: the items from the scroll top down to the end of the trailing
    /// overdraw are always measured, which is all the answer needs.
    pub(crate) fn shows_jump_to_end(&self) -> bool {
        if self.is_following_tail() {
            return false;
        }
        match self.list.is_scrolled_to_end() {
            Some(at_end) => !at_end,
            None => {
                let max = self.list.max_offset_for_scrollbar().y;
                let offset = -self.list.scroll_px_offset_for_scrollbar().y;
                max > px(0.) && offset < max - px(1.)
            }
        }
    }

    /// The arrow: back to the end, following again.
    pub fn jump_to_end(&mut self, cx: &mut Context<Self>) {
        self.list.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    /// Whether the long code block `key` is unfolded ("Ver menos" at its foot).
    #[must_use]
    pub fn is_code_block_expanded(&self, key: CodeBlockKey) -> bool {
        self.expanded_code_blocks.contains(&key)
    }

    /// "Ver más" / "Ver menos" of a long code block (§10.1). The list
    /// remeasures the entry on the next frame: following, the tail stays in
    /// view; scrolled up, the top of the view stays where it was.
    pub fn toggle_code_block(&mut self, key: CodeBlockKey, cx: &mut Context<Self>) {
        if !self.expanded_code_blocks.remove(&key) {
            self.expanded_code_blocks.insert(key);
        }
        cx.notify();
    }

    /// What segment `key` holds, cut the way the transcript paints it: an
    /// answer's own segments, or a user bubble's Markdown piece cut again.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn code_block_kind(&self, key: CodeBlockKey) -> Option<crate::code_folds::SegmentKind> {
        match self.entries.get(key.entry)? {
            Entry::AgentText(text) if key.piece == 0 => {
                text.segments.get(key.segment).map(|segment| segment.kind)
            }
            Entry::UserMessage(message) => {
                match crate::render::user_pieces(&message.blocks).get(key.piece)? {
                    crate::render::UserPiece::Markdown(text) => split_markdown(text)
                        .get(key.segment)
                        .map(|segment| segment.kind),
                    crate::render::UserPiece::Flow(_) => None,
                }
            }
            _ => None,
        }
    }

    /// The label of the button at the foot of block `key`, as painted.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn code_block_toggle_label(&self, key: CodeBlockKey) -> Option<String> {
        self.code_block_kind(key)?
            .toggle_label(self.is_code_block_expanded(key))
    }

    /// The transcript's scroll position, in items (`ListState::logical_scroll_top`).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn transcript_scroll_offset(&self) -> gpui::ListOffset {
        self.list.logical_scroll_top()
    }

    /// The permission request on screen, if any.
    #[must_use]
    pub fn pending_permission(&self) -> Option<&PermissionEntry> {
        let index = self.pending_permission?;
        match self.entries.get(index) {
            Some(Entry::Permission(permission)) => Some(permission),
            _ => None,
        }
    }

    /// The key context of the panel (`actions.rs`).
    #[must_use]
    pub fn key_context(&self) -> KeyContext {
        let mut context = KeyContext::new_with_defaults();
        context.add(actions::CONTEXT_CHAT);
        if self.is_awaiting_permission() {
            context.add("permission");
        }
        context
    }

    /// The code-block highlighter of the current theme.
    pub(crate) fn highlighter(&self) -> CodeHighlighter {
        CodeHighlighter::new(self.registry.clone(), &self.theme)
    }

    // ---------------------------------------------------------------- setters

    /// Hot-reloads the colours.
    pub fn set_theme(&mut self, theme: ChatTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        let composer_theme = theme.composer_theme();
        self.input
            .update(cx, |input, cx| input.set_theme(composer_theme, cx));
        self.refresh_decorator(cx);
        self.bubble_views.borrow_mut().clear();
        cx.notify();
    }

    /// Hot-reloads the settings (the zoom among them: every size of the panel
    /// and the composer's font follow [`ChatSettings::scale`]).
    pub fn set_settings(&mut self, settings: ChatSettings, cx: &mut Context<Self>) {
        self.settings = settings;
        let editor_settings = composer_settings(&settings, cx);
        self.input
            .update(cx, |input, cx| input.set_settings(editor_settings, cx));
        cx.notify();
    }

    /// Re-installs the composer's Markdown decorator (theme or mentions
    /// changed).
    fn refresh_decorator(&mut self, cx: &mut Context<Self>) {
        let mentions: Vec<String> = self
            .mentions
            .iter()
            .map(|mention| mention.token.clone())
            .collect();
        self.decorated_mentions = mentions.clone();
        let decorator = crate::composer::decorator(
            self.registry.clone(),
            self.theme.bg_editor.into(),
            mentions,
        );
        self.input
            .update(cx, |input, cx| input.set_decorator(Some(decorator), cx));
    }

    /// Keeps the composer in step with the panel: read-only while a
    /// permission waits, and its decorator aware of the draft's mentions.
    /// Runs from `render`; both setters are no-ops when nothing changed.
    pub(crate) fn sync_composer(&mut self, cx: &mut Context<Self>) {
        let blocked = self.is_awaiting_permission();
        self.input
            .update(cx, |input, cx| input.set_read_only(blocked, cx));
        let current: Vec<&str> = self
            .mentions
            .iter()
            .map(|mention| mention.token.as_str())
            .collect();
        if current
            != self
                .decorated_mentions
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        {
            self.refresh_decorator(cx);
        }
    }

    /// Replaces the rows of the "Conectar" popover. Nothing is selected or
    /// connected by this: the workspace says which one is running with
    /// [`ChatPanel::set_active_connection`].
    pub fn set_connections(&mut self, connections: Vec<ChatConnection>, cx: &mut Context<Self>) {
        self.connections = connections;
        if let Popover::ConnectionMenu(id) = &self.popover
            && !self
                .connections
                .iter()
                .any(|connection| &connection.id == id)
        {
            self.popover = Popover::Closed;
        }
        self.placeholder_dirty = true;
        cx.notify();
    }

    /// Points the header at the running connection (or back at "Conectar").
    /// Display only: it emits nothing.
    pub fn set_active_connection(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.active_connection != id {
            self.active_connection = id;
            self.placeholder_dirty = true;
            cx.notify();
        }
    }

    /// Highlights a row of the popover (`connections.default_label`). It never
    /// connects by itself (`docs/specs/06-etapa4-conexiones-y-cincel.md` §1).
    pub fn set_preselected_connection(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.preselected_connection = id;
        cx.notify();
    }

    /// Shows (or clears) the banner above the transcript.
    pub fn set_banner(&mut self, banner: Option<ConnectionBanner>, cx: &mut Context<Self>) {
        if self.banner != banner {
            self.banner = banner;
            cx.notify();
        }
    }

    /// Overrides the header pill (the workspace uses it for
    /// `autenticación requerida`, which no longer arrives as a card).
    pub fn set_status(&mut self, status: AgentStatus, cx: &mut Context<Self>) {
        self.status = status;
        cx.notify();
    }

    /// A row of the "Conectar" popover was chosen: the workspace stops the
    /// previous connection, launches this one and opens a new conversation.
    ///
    /// Choosing the connection that is already active is not a no-op either:
    /// it is the "empezar de cero con esta misma conexión" gesture.
    pub fn select_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::ConnectionSelected { id: id.to_string() });
        cx.notify();
    }

    /// Opens the "Conectar" popover (the header button, the empty state and
    /// the status bar chip).
    pub fn open_connections(&mut self, cx: &mut Context<Self>) {
        self.open_popover(Popover::Connections, cx);
    }

    /// "Conectar nuevo agente…".
    pub fn request_new_connection(&mut self, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::NewConnection);
        cx.notify();
    }

    /// "Eliminar conexión…".
    pub fn request_delete_connection(&mut self, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::DeleteConnections);
        cx.notify();
    }

    /// Opens the context menu of a row (right click).
    pub fn open_connection_menu(&mut self, id: &str, cx: &mut Context<Self>) {
        self.open_popover(Popover::ConnectionMenu(id.to_string()), cx);
    }

    /// "Renombrar" of a row's context menu.
    pub fn request_rename_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::RenameConnection { id: id.to_string() });
        cx.notify();
    }

    /// "Volver a conectar" (the banner or an expired row).
    pub fn request_reconnect(&mut self, id: &str, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::Reconnect { id: id.to_string() });
        cx.notify();
    }

    /// "Reparar" (the banner or an unavailable row).
    pub fn request_repair(&mut self, id: &str, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::Repair { id: id.to_string() });
        cx.notify();
    }

    /// Replaces the history popover's rows (`02-visual.md` §7).
    pub fn set_conversations(
        &mut self,
        conversations: Vec<ConversationSummary>,
        cx: &mut Context<Self>,
    ) {
        self.conversations = conversations;
        cx.notify();
    }

    /// Shows (or clears) the muted line above the transcript.
    pub fn set_history_notice(&mut self, notice: Option<String>, cx: &mut Context<Self>) {
        self.history_notice = notice;
        cx.notify();
    }

    /// Sets the `+N −M` of an edit tool card, from `ReviewStore::stats`.
    pub fn set_tool_stats(
        &mut self,
        tool_call_id: &ToolCallId,
        added: u32,
        removed: u32,
        cx: &mut Context<Self>,
    ) {
        if let Some(&index) = self.tool_index.get(tool_call_id)
            && let Some(Entry::ToolCall(call)) = self.entries.get_mut(index)
        {
            call.stats = Some((added, removed));
            cx.notify();
        }
    }

    /// Answers a [`ChatEvent::RequestFileList`] with the worktree's paths.
    pub fn set_file_candidates(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.file_candidates = files;
        if let Popover::Files { selected, .. } = &mut self.popover {
            *selected = 0;
        }
        cx.notify();
    }

    /// The project the paths of the `@` picker are shown relative to.
    ///
    /// Only the **display** changes: a mention still travels to the agent as a
    /// `resource_link` with the absolute `file://` URI (`chat.md`, acceptance
    /// criteria).
    pub fn set_project_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.project_root = root;
        cx.notify();
    }

    /// The project root the mentions are shown relative to, when the workspace
    /// told the panel about it.
    #[must_use]
    pub fn project_root(&self) -> Option<&std::path::Path> {
        self.project_root.as_deref()
    }

    /// Replaces the composer's text and leaves the caret at the end.
    ///
    /// The popover is recomputed here too, so the caller sees it right away
    /// instead of after the composer's `CursorMoved` comes back.
    pub fn set_input_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_input_text_with_caret(text, text.len(), window, cx);
    }

    /// [`ChatPanel::set_input_text`] with the caret left at `caret` instead of
    /// at the end, which is what inserting a mention mid-sentence needs.
    pub fn set_input_text_with_caret(
        &mut self,
        text: &str,
        caret: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        let caret = caret.min(text.len());
        // A read-only composer (a permission waits) refuses the edit, and so
        // does this: the draft is not the panel's to throw away meanwhile.
        self.input.update(cx, |input, cx| {
            let was_read_only = input.is_read_only();
            input.set_read_only(false, cx);
            input.set_text(text, caret, cx);
            input.set_read_only(was_read_only, cx);
        });
        self.refresh_trigger(cx);
        cx.notify();
    }

    /// Wipes the transcript and the composer, keeping agents and settings.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entries.clear();
        self.bubble_views.borrow_mut().clear();
        self.tool_index.clear();
        self.pending_permission = None;
        self.mentions.clear();
        // The draft's images go with the draft; the unsent comments do not:
        // they belong to the project, not to the conversation (§6.7).
        self.attachments.clear();
        self.expanded_cards.clear();
        self.expanded_code_blocks.clear();
        self.media_cache.borrow_mut().clear();
        self.history_notice = None;
        self.replaying = false;
        self.activity_signal = AgentActivity::Thinking;
        self.popover = Popover::Closed;
        // A new (or newly opened) conversation starts at its end, following.
        self.list.set_follow_mode(FollowMode::Tail);
        self.set_input_text("", window, cx);
        cx.notify();
    }

    /// Empties the panel for a brand new conversation: no entries, no session
    /// and no leftover notice. The workspace calls it after saving whatever
    /// was on screen.
    pub fn start_new_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear(window, cx);
        self.session_id = None;
        self.active_conversation = None;
        self.status = AgentStatus::Disconnected;
        cx.notify();
    }

    /// Puts a stored conversation on screen: its entries. The ACP session
    /// is **not** set and neither is the connection — reopening them (or
    /// deciding they cannot be reopened) is the workspace's half of the job.
    pub fn load_conversation(
        &mut self,
        conversation: Conversation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear(window, cx);
        self.bubble_views.borrow_mut().clear();
        // Reopened, its long blocks start folded again (§10.1).
        self.expanded_code_blocks.clear();
        self.session_id = None;
        self.active_conversation = Some(conversation.id);
        self.entries = conversation.entries;
        // Builds before 0.2.0 stored the replayed mention links (above) as a
        // lone user message: nothing a user can type produces that shape.
        self.entries.retain(|entry| !is_stray_mention_bubble(entry));
        self.rebuild_views(cx);
        self.sync_list();
        // Opening a conversation shows its end and follows it (§9, R6).
        self.list.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    /// Starts dropping the `session/load` replay of what is already on screen.
    ///
    /// The comparison is by content, not by position: an agent replays a
    /// message as one chunk where the live stream sent twenty, so an update
    /// whose text is already contained in an entry of the same kind is taken
    /// for the same thing. Anything the transcript does not have (a turn that
    /// never reached disk) is appended as usual. Cleared by `SessionCreated`,
    /// which is what closes the replay in `cincel-acp`.
    pub fn begin_replay(&mut self, cx: &mut Context<Self>) {
        self.replaying = true;
        cx.notify();
    }

    /// Whether a `session/load` replay is being filtered right now.
    #[must_use]
    pub fn is_replaying(&self) -> bool {
        self.replaying
    }

    /// Whether `update` is already on screen, and so must not be replayed.
    fn is_already_shown(&self, update: &SessionUpdate) -> bool {
        match update {
            SessionUpdate::UserMessageChunk(chunk)
                if matches!(chunk.content, ContentBlock::Image(_)) =>
            {
                let Some(identity) = crate::panel_media::replayed_image(&chunk.content)
                    .and_then(|image| crate::panel_media::image_identity(&image))
                else {
                    return false;
                };
                self.entries.iter().any(|entry| match entry {
                    Entry::UserMessage(message) => message.blocks.iter().any(|block| {
                        matches!(block, MessageBlock::Image(stored)
                            if crate::panel_media::image_identity(stored).as_deref() == Some(identity.as_str()))
                    }),
                    _ => false,
                })
            }
            // `@archivo` mentions travel to the agent as resource links while the
            // stored user message keeps them as text; a replayed link is never
            // something new to show (it used to come back as a lone "@a@b@c"
            // bubble on every `session/load`).
            SessionUpdate::UserMessageChunk(chunk)
                if matches!(
                    chunk.content,
                    ContentBlock::ResourceLink(_) | ContentBlock::Resource(_)
                ) =>
            {
                true
            }
            SessionUpdate::UserMessageChunk(chunk) => {
                let text = block_text(&chunk.content);
                if text.is_empty() {
                    return false;
                }
                // The Claude adapter turns every mention into the text
                // `[@name](file://…)` before storing the prompt, and replays it
                // as such; the stored message keeps the mention as a `File`
                // block, so the text alone is never something new to show.
                if let Some(names) = mention_links_only(&text) {
                    let mentioned = |name: &str| {
                        self.entries.iter().any(|entry| match entry {
                            Entry::UserMessage(message) => message.blocks.iter().any(|block| {
                                matches!(block, MessageBlock::File(path)
                                    if &*file_name_label(path) == name)
                            }),
                            _ => false,
                        })
                    };
                    if names.iter().all(|name| mentioned(name)) {
                        return true;
                    }
                }
                self.entries.iter().any(|entry| {
                    match entry {
                    Entry::UserMessage(message) => message.blocks.iter().any(|block| {
                        matches!(block, MessageBlock::Text(stored) if stored.contains(&text))
                    }),
                    _ => false,
                }
                })
            }
            SessionUpdate::AgentMessageChunk(chunk) => {
                let text = block_text(&chunk.content);
                !text.is_empty()
                    && self.entries.iter().any(
                        |entry| matches!(entry, Entry::AgentText(stored) if stored.markdown.contains(&text)),
                    )
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = block_text(&chunk.content);
                !text.is_empty()
                    && self.entries.iter().any(
                        |entry| matches!(entry, Entry::AgentThought(stored) if stored.text.contains(&text)),
                    )
            }
            SessionUpdate::ToolCall(call) => self.tool_index.contains_key(&call.tool_call_id),
            SessionUpdate::ToolCallUpdate(update) => {
                self.tool_index.contains_key(&update.tool_call_id)
            }
            SessionUpdate::Plan(_) => self
                .entries
                .iter()
                .any(|entry| matches!(entry, Entry::Plan(_))),
            _ => false,
        }
    }

    /// Rebuilds the `gpui-kit` markdown states of the entries in place.
    fn rebuild_views(&mut self, cx: &mut Context<Self>) {
        self.tool_index.clear();
        self.pending_permission = None;
        for (index, entry) in self.entries.iter_mut().enumerate() {
            match entry {
                Entry::AgentText(text) => {
                    text.streaming = false;
                    rebuild_segments(text, cx);
                }
                Entry::AgentThought(thought) => match thought.view.clone() {
                    Some(view) => set_text(&view, &thought.text, cx),
                    None => thought.view = Some(markdown_state(&thought.text, cx)),
                },
                Entry::ToolCall(call) => {
                    self.tool_index.insert(call.id.clone(), index);
                }
                // A request nobody answered before the editor closed is dead:
                // the agent is not waiting for it any more.
                Entry::Permission(permission) if permission.answered.is_none() => {
                    permission.answered = Some("Sin responder".to_string());
                }
                _ => {}
            }
        }
    }

    /// The placeholder of `02-visual.md` §7, with the active connection's
    /// label.
    ///
    /// What `@` and `/` do lives under the input now ([`COMPOSER_HINT`]),
    /// where it stays readable once the user starts typing.
    #[must_use]
    pub fn placeholder(&self) -> String {
        match self.active_connection() {
            Some(connection) => format!("Escribí un mensaje para {}…", connection.label),
            None => DEFAULT_PLACEHOLDER.to_string(),
        }
    }

    /// Pushes the placeholder into the composer; runs from `render` whenever
    /// the active connection changed.
    pub(crate) fn sync_placeholder(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.placeholder_dirty {
            return;
        }
        self.placeholder_dirty = false;
        let placeholder = SharedString::from(self.placeholder());
        self.input
            .update(cx, |input, cx| input.set_placeholder(Some(placeholder), cx));
    }

    // ------------------------------------------------------------ agent input

    /// Consumes one [`AgentEvent`]. The workspace owns the channel and forwards
    /// everything it reads; unknown variants are ignored on purpose, so the ACP
    /// crate can grow without breaking the UI.
    pub fn handle_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::Connected {
                auth_methods,
                capabilities,
                ..
            } => {
                self.auth_methods = auth_methods;
                self.status = AgentStatus::Ready;
                // D6: what the agent announced decides whether images can be
                // attached; the workspace may still override it with
                // `set_image_support`.
                self.image_support = Some(cincel_acp::agent_supports_images(&capabilities));
            }
            AgentEvent::AuthRequired { methods, .. } => {
                self.auth_methods = methods.clone();
                let entry = AuthEntry {
                    methods: methods.iter().map(auth_choice).collect(),
                };
                self.push(Entry::AuthRequired(entry));
                self.status = AgentStatus::AuthRequired;
            }
            AgentEvent::SessionCreated {
                session_id,
                modes,
                config_options,
                commands,
            } => {
                self.session_id = Some(session_id);
                self.modes = modes;
                self.config_options = config_options;
                self.commands = commands;
                self.status = AgentStatus::Ready;
                // `session/load` finishes with `SessionCreated`, so the replay
                // filter is over by definition.
                self.replaying = false;
            }
            AgentEvent::Update { update, .. } => {
                if self.replaying && self.is_already_shown(&update) {
                    return;
                }
                self.apply_update(*update, cx);
            }
            AgentEvent::PermissionRequest {
                id,
                tool_call,
                options,
                ..
            } => self.push_permission(id.0, &tool_call, options),
            AgentEvent::TurnEnded { stop_reason, .. } => {
                self.finish_streaming();
                let duration_secs = self
                    .turn_started
                    .take()
                    .map(|start| start.elapsed().as_secs())
                    .unwrap_or(0);
                self.push(Entry::TurnSeparator(TurnSeparator {
                    stop_reason: stop_reason_label(&stop_reason).to_string(),
                    duration_secs,
                }));
                if !self.is_awaiting_permission() {
                    self.status = AgentStatus::Ready;
                }
            }
            AgentEvent::ToolCallsCancelled { ids, .. } => {
                for id in ids {
                    if let Some(&index) = self.tool_index.get(&id)
                        && let Some(Entry::ToolCall(call)) = self.entries.get_mut(index)
                    {
                        call.status = ToolCallStatus::Failed;
                    }
                }
            }
            AgentEvent::Stderr(line) => {
                self.stderr.push(line);
                if self.stderr.len() > STDERR_KEPT {
                    self.stderr.remove(0);
                }
                return;
            }
            AgentEvent::Exited { code, stderr_tail } => {
                self.status = AgentStatus::Disconnected;
                // No process, no capability: attaching says "Conectá un
                // agente" until the next `Connected`.
                self.image_support = None;
                self.finish_streaming();
                let code = code.map_or_else(|| "sin código".to_string(), |code| code.to_string());
                let detail = self.error_detail(&stderr_tail);
                self.push(Entry::Notice(Notice {
                    level: NoticeLevel::Error,
                    text: format!("El agente se cerró ({code}).{detail}"),
                }));
            }
            AgentEvent::Error {
                message,
                stderr_tail,
            } => {
                let detail = self.error_detail(&stderr_tail);
                self.push(Entry::Notice(Notice {
                    level: NoticeLevel::Error,
                    text: format!("{message}{detail}"),
                }));
            }
            // A failed `authenticate` is its own event since E4: its message
            // is shown, never swallowed.
            AgentEvent::AuthFailed { message, .. } => {
                self.push(Entry::Notice(Notice {
                    level: NoticeLevel::Error,
                    text: format!("No se pudo autenticar: {message}"),
                }));
                self.status = AgentStatus::AuthRequired;
            }
            AgentEvent::AuthSucceeded { .. } => {
                self.push(Entry::Notice(Notice {
                    level: NoticeLevel::Info,
                    text: "Autenticación completada.".to_string(),
                }));
                if self.status == AgentStatus::AuthRequired {
                    self.status = AgentStatus::Ready;
                }
            }
            AgentEvent::LoggedOut { ok } => {
                self.push(Entry::Notice(Notice {
                    level: if ok {
                        NoticeLevel::Info
                    } else {
                        NoticeLevel::Warning
                    },
                    text: if ok {
                        "Se cerró la sesión del agente.".to_string()
                    } else {
                        "El agente no pudo cerrar la sesión.".to_string()
                    },
                }));
            }
            AgentEvent::ElicitationCompleted { .. } => {
                self.push(Entry::Notice(Notice {
                    level: NoticeLevel::Info,
                    text: "El agente confirmó que terminaste el paso en el navegador.".to_string(),
                }));
            }
            // `cincel-acp` keeps growing (file change reports, elicitation,
            // …); anything this version does not know about is not an error.
            _ => return,
        }
        cx.notify();
    }

    fn apply_update(&mut self, update: SessionUpdate, cx: &mut Context<Self>) {
        match update {
            SessionUpdate::UserMessageChunk(chunk)
                if matches!(chunk.content, ContentBlock::Image(_)) =>
            {
                // A replayed image is shown from its data, not stored apart
                // (§5.1, §5.3.2).
                let Some(image) = crate::panel_media::replayed_image(&chunk.content) else {
                    return;
                };
                match self.entries.last_mut() {
                    Some(Entry::UserMessage(message)) => {
                        message.blocks.push(MessageBlock::Image(image));
                    }
                    _ => self.push(Entry::UserMessage(UserMessage {
                        blocks: vec![MessageBlock::Image(image)],
                    })),
                }
            }
            SessionUpdate::UserMessageChunk(chunk) => {
                let text = block_text(&chunk.content);
                if text.is_empty() {
                    return;
                }
                match self.entries.last_mut() {
                    Some(Entry::UserMessage(message)) => match message.blocks.last_mut() {
                        Some(MessageBlock::Text(existing)) => existing.push_str(&text),
                        _ => message.blocks.push(MessageBlock::Text(text)),
                    },
                    _ => self.push(Entry::UserMessage(UserMessage {
                        blocks: vec![MessageBlock::Text(text)],
                    })),
                }
            }
            SessionUpdate::AgentMessageChunk(chunk) => {
                let text = block_text(&chunk.content);
                if text.is_empty() {
                    return;
                }
                self.status = AgentStatus::Thinking;
                self.activity_signal = AgentActivity::Writing;
                self.thought_started = None;
                if let Some(Entry::AgentText(entry)) = self.entries.last_mut()
                    && entry.streaming
                {
                    append_to_segments(entry, &text, cx);
                    return;
                }
                let mut entry = AgentText {
                    markdown: String::new(),
                    streaming: true,
                    segments: Vec::new(),
                };
                append_to_segments(&mut entry, &text, cx);
                self.push(Entry::AgentText(entry));
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = block_text(&chunk.content);
                // Logged before the early return: an agent that sends empty
                // thought chunks looks exactly like one that sends none, and
                // the workspace log is where that gets answered.
                tracing::debug!(
                    bytes = text.len(),
                    "llegó un fragmento de pensamiento del agente"
                );
                if text.is_empty() {
                    return;
                }
                self.status = AgentStatus::Thinking;
                self.activity_signal = AgentActivity::Thinking;
                let started = *self.thought_started.get_or_insert_with(Instant::now);
                if let Some(Entry::AgentThought(entry)) = self.entries.last_mut() {
                    entry.text.push_str(&text);
                    entry.duration_secs = started.elapsed().as_secs();
                    if let Some(view) = entry.view.clone() {
                        append_chunk(&view, &text, cx);
                    }
                    return;
                }
                let view = markdown_state(&text, cx);
                let collapsed = self.settings.collapse_thoughts;
                self.push(Entry::AgentThought(AgentThought {
                    text,
                    collapsed,
                    duration_secs: 0,
                    view: Some(view),
                }));
            }
            SessionUpdate::ToolCall(call) => {
                let preview_lines = self.settings.diff_preview_lines;
                let entry = ToolCallEntry {
                    id: call.tool_call_id.clone(),
                    kind: call.kind,
                    title: call.title.clone(),
                    status: call.status,
                    content: tool_contents(&call.content, call.kind, &call.title, preview_lines),
                    locations: call
                        .locations
                        .iter()
                        .map(|location| location.path.clone())
                        .collect(),
                    stats: None,
                    expanded: expands_by_default(call.status),
                    output_expanded: false,
                };
                self.tool_index
                    .insert(call.tool_call_id.clone(), self.entries.len());
                self.push(Entry::ToolCall(entry));
                self.activity_signal = self.tool_activity();
            }
            SessionUpdate::ToolCallUpdate(update) => {
                self.apply_tool_update(&update);
                self.activity_signal = self.tool_activity();
            }
            SessionUpdate::Plan(plan) => {
                let items: Vec<PlanItem> = plan
                    .entries
                    .iter()
                    .map(|entry| PlanItem {
                        text: entry.content.clone(),
                        status: entry.status.clone(),
                    })
                    .collect();
                if let Some(Entry::Plan(existing)) = self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|entry| matches!(entry, Entry::Plan(_)))
                {
                    existing.items = items;
                } else {
                    self.push(Entry::Plan(PlanEntry {
                        items,
                        collapsed: false,
                    }));
                }
            }
            SessionUpdate::AvailableCommandsUpdate(update) => {
                self.commands = update.available_commands;
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                if let Some(modes) = self.modes.as_mut() {
                    modes.current_mode_id = update.current_mode_id;
                }
            }
            SessionUpdate::ConfigOptionUpdate(update) => {
                self.config_options = update.config_options;
            }
            _ => {}
        }
    }

    /// The activity after a tool signal: "Trabajando…" while a tool call of
    /// the current turn (the entries after the last user message) is pending
    /// or running, "Pensando…" once they all finished (the agent decides what
    /// comes next).
    fn tool_activity(&self) -> AgentActivity {
        let running = self
            .entries
            .iter()
            .rev()
            .take_while(|entry| !matches!(entry, Entry::UserMessage(_)))
            .any(|entry| {
                matches!(entry, Entry::ToolCall(call)
                    if matches!(call.status, ToolCallStatus::Pending | ToolCallStatus::InProgress))
            });
        if running {
            AgentActivity::Working
        } else {
            AgentActivity::Thinking
        }
    }

    fn apply_tool_update(&mut self, update: &ToolCallUpdate) {
        let preview_lines = self.settings.diff_preview_lines;
        let Some(&index) = self.tool_index.get(&update.tool_call_id) else {
            return;
        };
        let Some(Entry::ToolCall(call)) = self.entries.get_mut(index) else {
            return;
        };
        if let Some(kind) = update.fields.kind {
            call.kind = kind;
        }
        if let Some(status) = update.fields.status {
            // A card that fails opens itself: the error is the one thing the
            // user has to read. A card that succeeds folds back up, which is
            // what keeps a turn with twenty `execute`s readable (`02-visual`
            // §7, density).
            if status != call.status {
                call.expanded = expands_by_default(status);
            }
            call.status = status;
        }
        if let Some(title) = update.fields.title.clone() {
            call.title = title;
        }
        if let Some(content) = update.fields.content.as_ref() {
            call.content = tool_contents(content, call.kind, &call.title, preview_lines);
        }
        if let Some(locations) = update.fields.locations.as_ref() {
            call.locations = locations
                .iter()
                .map(|location| location.path.clone())
                .collect();
        }
    }

    /// Shows a permission request that did not arrive as an [`AgentEvent`].
    ///
    /// `AgentEvent::PermissionRequest` carries a `PermissionResponder` only
    /// `cincel-acp` can build, so this is the seam the demo and the tests use;
    /// [`ChatPanel::handle_event`] funnels the real event through the same code.
    /// `request_id` is the `u64` inside `cincel_acp::PermissionRequestId`, and
    /// it comes back in the `RespondPermission` command.
    pub fn request_permission(
        &mut self,
        request_id: u64,
        tool_call: &ToolCallUpdate,
        options: Vec<PermissionOption>,
        cx: &mut Context<Self>,
    ) {
        self.push_permission(request_id, tool_call, options);
        cx.notify();
    }

    fn push_permission(
        &mut self,
        id: u64,
        tool_call: &ToolCallUpdate,
        options: Vec<PermissionOption>,
    ) {
        let title = tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| "El agente pide permiso".to_string());
        let detail = tool_call.fields.content.as_ref().and_then(|content| {
            let text = content
                .iter()
                .filter_map(|item| match item {
                    ToolCallContent::Content(inner) => Some(block_text(&inner.content)),
                    ToolCallContent::Diff(diff) => Some(diff.path.display().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        });
        let entry = PermissionEntry {
            request_id: id,
            title,
            detail,
            options: options
                .into_iter()
                .map(|option| PermissionChoice {
                    id: option.option_id,
                    name: option.name,
                    kind: option.kind,
                })
                .collect(),
            answered: None,
            allowed: false,
            expanded: false,
        };
        self.pending_permission = Some(self.entries.len());
        self.push(Entry::Permission(entry));
        self.status = AgentStatus::WaitingPermission;
    }

    /// Keeps the list's item count equal to the transcript's: new entries are
    /// spliced at the end, anything else (a clear, a loaded conversation)
    /// resets it. In-place changes are remeasured on every render.
    pub(crate) fn sync_list(&self) {
        let count = self.entries.len();
        let listed = self.list.item_count();
        if count > listed {
            self.list.splice(listed..listed, count - listed);
        } else if count < listed {
            self.list.reset(count);
        }
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        if self.entries.len() > self.settings.max_entries {
            let excess = self.entries.len() - self.settings.max_entries;
            self.entries.drain(0..excess);
            self.pending_permission = self
                .pending_permission
                .and_then(|index| index.checked_sub(excess));
            for index in self.tool_index.values_mut() {
                *index = index.saturating_sub(excess);
            }
            // The unfolded blocks move with their entries; those of the
            // dropped ones go.
            self.expanded_code_blocks = self
                .expanded_code_blocks
                .drain()
                .filter_map(|key| {
                    key.entry
                        .checked_sub(excess)
                        .map(|entry| CodeBlockKey { entry, ..key })
                })
                .collect();
        }
        // No `scroll_to_end` here: following the tail is the list's own
        // `FollowMode::Tail`, and a pushed entry (the end-of-turn separator
        // among them) must not drag back a user who scrolled up (§9, R6).
        self.sync_list();
    }

    /// The last stderr lines shown under an error notice (`02-visual.md` §9).
    fn error_detail(&self, stderr_tail: &str) -> String {
        let tail: Vec<&str> = stderr_tail
            .lines()
            .rev()
            .take(STDERR_KEPT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !tail.is_empty() {
            return format!("\n{}", tail.join("\n"));
        }
        if self.stderr.is_empty() {
            String::new()
        } else {
            format!("\n{}", self.stderr.join("\n"))
        }
    }

    fn finish_streaming(&mut self) {
        for entry in self.entries.iter_mut().rev() {
            if let Entry::AgentText(text) = entry {
                text.streaming = false;
                break;
            }
        }
        self.thought_started = None;
    }

    // ------------------------------------------------------------- user input

    /// What the composer reports back: every edit and every caret move ends
    /// in `CursorMoved`, which recomputes the `@` / `/` popover (moving the
    /// caret out of an `@query` closes it). `Enter` never reaches the editor's
    /// `insert_newline` here: the composer box takes it in the capture phase
    /// (`render.rs`), so the keystroke has a single owner.
    fn on_input_event(
        &mut self,
        _editor: &Entity<EditorView>,
        event: &EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, EditorEvent::CursorMoved { .. }) {
            self.refresh_trigger(cx);
        }
    }

    /// Recomputes the `@` / `/` popover from the text before the caret.
    fn refresh_trigger(&mut self, cx: &mut Context<Self>) {
        let (text, cursor) = self
            .input
            .read_with(cx, |input, _cx| (input.text(), input.cursor_offset()));
        let cursor = cursor.min(text.len());
        match trigger_at(&text, cursor) {
            Some(('@', start, query)) => {
                let selected = match &self.popover {
                    Popover::Files { selected, .. } => *selected,
                    _ => 0,
                };
                self.popover = Popover::Files {
                    start,
                    query: query.clone(),
                    selected,
                };
                self.request_files(query, cx);
            }
            Some(('/', start, query)) => {
                let selected = match &self.popover {
                    Popover::Commands { selected, .. } => *selected,
                    _ => 0,
                };
                self.popover = Popover::Commands {
                    start,
                    query,
                    selected,
                };
            }
            _ => {
                if matches!(
                    self.popover,
                    Popover::Files { .. } | Popover::Commands { .. }
                ) {
                    self.popover = Popover::Closed;
                }
            }
        }
        cx.notify();
    }

    fn request_files(&mut self, query: String, cx: &mut Context<Self>) {
        let panel = cx.entity().downgrade();
        let mut async_cx = cx.to_async();
        cx.emit(ChatEvent::RequestFileList {
            query,
            reply: Box::new(move |files| {
                panel
                    .update(&mut async_cx, |panel, cx| {
                        panel.set_file_candidates(files, cx);
                    })
                    .ok();
            }),
        });
    }

    /// Sends the composed message (`chat::send`).
    ///
    /// Does nothing while a permission request is pending, which is the "un
    /// pedido de permiso bloquea el envío" of `chat.md`.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_awaiting_permission() {
            return;
        }
        // Images still being prepared, or a connection that does not take
        // the ones attached, hold the message back (§5.1).
        if self.is_preparing_images() || self.images_block_send() {
            return;
        }
        // The mentions are inline tokens in the text, so the order the user
        // wrote is the order the agent sees (`docs/etapas/etapa-2.md`).
        let mut blocks = split_mentions(&self.input_text(cx), &self.mentions);
        // The images go after the text and the mentions, in the order they
        // were attached (D4); a message may be images alone.
        blocks.extend(
            self.ready_images()
                .map(|image| MessageBlock::Image(ImageRef::from_prepared(image))),
        );
        if blocks.is_empty() {
            return;
        }
        let content: Vec<PromptBlock> = blocks.iter().filter_map(message_block_to_prompt).collect();
        self.attachments.clear();
        self.push(Entry::UserMessage(UserMessage {
            blocks: blocks.clone(),
        }));
        // Sending is the user asking to see what they sent: back to the end,
        // following again (§9, R6).
        self.list.set_follow_mode(FollowMode::Tail);
        self.mentions.clear();
        self.popover = Popover::Closed;
        // The "solo lectura" line is only true until the first message: from
        // here on there is a live session again.
        self.history_notice = None;
        self.set_input_text("", window, cx);

        match self.session_id.clone() {
            Some(session_id) => {
                self.status = AgentStatus::Thinking;
                self.activity_signal = AgentActivity::Thinking;
                self.turn_started = Some(Instant::now());
                cx.emit(ChatEvent::Command(AgentCommand::Prompt {
                    session_id,
                    blocks: content,
                    // The review feedback is the workspace's to add: only it
                    // knows the `ReviewStore` (`03-arquitectura.md` §2).
                    feedback: None,
                }));
            }
            None => self.push(Entry::Notice(Notice {
                level: NoticeLevel::Warning,
                text: "No hay una sesión abierta: conectá un agente arriba.".to_string(),
            })),
        }
        cx.notify();
    }

    /// Closes a turn that will never get its `TurnEnded`: its prompt request
    /// failed (the workspace saw an `AgentEvent::Error` for it) or its agent
    /// process was replaced. Stops the streaming and the "pensando…" pill so
    /// the composer is usable again; unlike `TurnEnded` it adds no separator.
    pub fn abort_turn(&mut self, cx: &mut Context<Self>) {
        self.finish_streaming();
        self.turn_started = None;
        self.thought_started = None;
        if self.status == AgentStatus::Thinking {
            self.status = AgentStatus::Ready;
        }
        cx.notify();
    }

    /// Cancels the running turn (`chat::cancel_turn`, `02-visual.md` §8).
    pub fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        if self.popover != Popover::Closed {
            self.popover = Popover::Closed;
            cx.notify();
            return;
        }
        if self.is_awaiting_permission() {
            self.reject_permission(cx);
            return;
        }
        let Some(session_id) = self.session_id.clone() else {
            return;
        };
        cx.emit(ChatEvent::Command(AgentCommand::Cancel { session_id }));
        self.finish_streaming();
        self.status = AgentStatus::Ready;
        cx.notify();
    }

    /// "Nueva conversación" (`chat::new_session`): asks the workspace to save
    /// what is on screen and start an empty conversation with the same agent.
    ///
    /// The panel does **not** clear itself here: it does not know whether the
    /// current conversation was persisted yet, and the workspace — which owns
    /// `conversations/` — drives the whole move
    /// ([`ChatPanel::start_new_conversation`]).
    pub fn new_session(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::NewConversation);
        cx.notify();
    }

    /// A row of the history popover was chosen.
    pub fn open_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::OpenConversation { id: id.to_string() });
        cx.notify();
    }

    /// The ✕ of a history row.
    pub fn delete_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
        self.conversations
            .retain(|conversation| conversation.id != id);
        cx.emit(ChatEvent::DeleteConversation { id: id.to_string() });
        cx.notify();
    }

    /// Moves the keyboard focus to the composer (`workspace::toggle_chat`,
    /// the focus wheel and `chat::focus_input`).
    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Whether the keyboard focus is anywhere inside the panel: the composer,
    /// the transcript, the header and every popover (`@`, `/`, "Conectar",
    /// the conversation list, the context menus), which are all painted
    /// inside the panel's element tree
    /// (`docs/specs/07-etapa5-productividad.md` §7.1).
    ///
    /// Answers from the last rendered frame, like every GPUI focus query.
    #[must_use]
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    /// `Shift+Enter`: a line break in the composer, focusing it first if
    /// needed.
    pub fn insert_newline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.input.read(cx).focus_handle(cx);
        if !handle.is_focused(window) {
            window.focus(&handle, cx);
        }
        self.input
            .update(cx, |input, cx| input.insert_text("\n", cx));
    }

    /// Answers the permission on screen with `option_id`.
    pub fn answer_permission(
        &mut self,
        option_id: &cincel_acp::acp::schema::v1::PermissionOptionId,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.pending_permission else {
            return;
        };
        let Some(Entry::Permission(permission)) = self.entries.get_mut(index) else {
            return;
        };
        let Some(choice) = permission
            .options
            .iter()
            .find(|option| &option.id == option_id)
            .cloned()
        else {
            return;
        };
        permission.answered = Some(choice.name.clone());
        permission.allowed = matches!(
            choice.kind,
            cincel_acp::acp::schema::v1::PermissionOptionKind::AllowOnce
                | cincel_acp::acp::schema::v1::PermissionOptionKind::AllowAlways
        );
        let request_id = PermissionRequestId(permission.request_id);
        self.pending_permission = None;
        self.status = AgentStatus::Thinking;
        cx.emit(ChatEvent::Command(AgentCommand::RespondPermission {
            id: request_id,
            outcome: PermissionOutcome::Selected(choice.id),
        }));
        cx.notify();
    }

    /// `Enter` on a permission card: the agent's first option (`chat.md`).
    pub fn accept_permission(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .pending_permission()
            .and_then(PermissionEntry::default_option)
            .map(|choice| choice.id.clone())
        else {
            return;
        };
        self.answer_permission(&id, cx);
    }

    /// `Esc` on a permission card: its first `reject_*` option (`chat.md`).
    pub fn reject_permission(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .pending_permission()
            .and_then(PermissionEntry::reject_option)
            .map(|choice| choice.id.clone())
        else {
            return;
        };
        self.answer_permission(&id, cx);
    }

    /// Asks the workspace to authenticate with `method_id`.
    pub fn authenticate(&mut self, method_id: String, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::Command(AgentCommand::Authenticate { method_id }));
    }

    /// Writes `@archivo` into the draft **at the caret** and remembers which
    /// path it stands for.
    ///
    /// The token is plain text inside the composer (painted in the `function`
    /// colour by its decorator), so the user can keep typing around it —
    /// `mira este archivo @calculadora.py, ¿qué te parece?` — and
    /// [`split_mentions`] turns it back into ordered
    /// `PromptBlock::Text`/`ResourceLink` on send. Called both by the `@`
    /// picker (which replaces the `@query` it was typed over) and by the
    /// tree's "Mencionar en el chat" (which inserts wherever the caret is).
    pub fn insert_mention(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input_text(cx);
        let caret = self.input.read(cx).cursor_offset().min(text.len());
        let (start, end) = match self.popover.clone() {
            Popover::Files { start, query, .. } => (
                start.min(text.len()),
                (start + 1 + query.len()).min(text.len()),
            ),
            _ => (caret, caret),
        };
        let (token, rewrites) = self.allocate_token(&path);

        // The disambiguation rewrites are applied to each side of the
        // insertion point separately, so the offsets above stay valid whatever
        // the replacements do to the length of the text.
        let mut head = text[..start].to_string();
        let mut tail = text[end..].to_string();
        for (previous, next) in &rewrites {
            head = head.replace(previous.as_str(), next);
            tail = tail.replace(previous.as_str(), next);
        }

        let mut next = String::with_capacity(head.len() + tail.len() + token.len() + 2);
        next.push_str(&head);
        if !head.is_empty() && !head.ends_with(char::is_whitespace) {
            next.push(' ');
        }
        next.push_str(&token);
        let caret = next.len() + 1;
        next.push(' ');
        next.push_str(tail.trim_start_matches(' '));
        self.set_input_text_with_caret(&next, caret, window, cx);
        self.popover = Popover::Closed;
        cx.notify();
    }

    /// The token `path` gets in the draft, registering the mention.
    ///
    /// A file already mentioned keeps its token. A **name** already taken by
    /// another file disambiguates both sides (`@src/main.rs` and
    /// `@tests/main.rs`); the returned rewrites are the tokens already written
    /// that the caller has to replace, so the user never ends up with two
    /// identical `@main.rs`.
    fn allocate_token(&mut self, path: &PathBuf) -> (String, Vec<(String, String)>) {
        if let Some(existing) = self.mentions.iter().find(|mention| &mention.path == path) {
            return (existing.token.clone(), Vec::new());
        }
        let root = self.project_root.clone();
        let plain = mention_token(root.as_deref(), path, false, false);
        let clashes: Vec<usize> = self
            .mentions
            .iter()
            .enumerate()
            .filter(|(_, mention)| mention.token == plain)
            .map(|(index, _)| index)
            .collect();

        let mut rewrites = Vec::new();
        let mut token = if clashes.is_empty() {
            plain
        } else {
            for index in clashes {
                let upgraded =
                    mention_token(root.as_deref(), &self.mentions[index].path, true, false);
                let previous = self.mentions[index].token.clone();
                if upgraded != previous {
                    rewrites.push((previous, upgraded.clone()));
                    self.mentions[index].token = upgraded;
                }
            }
            mention_token(root.as_deref(), path, true, false)
        };
        if self.mentions.iter().any(|mention| mention.token == token) {
            token = mention_token(root.as_deref(), path, true, true);
        }
        self.mentions.push(Mention {
            token: token.clone(),
            path: path.clone(),
        });
        (token, rewrites)
    }

    /// Forgets the mention whose token is `token` (the user removed it).
    pub fn forget_mention(&mut self, token: &str, cx: &mut Context<Self>) {
        self.mentions.retain(|mention| mention.token != token);
        cx.notify();
    }

    /// Replaces the `/query` with `/name ` (`02-visual.md` §7, "Comandos slash").
    pub fn insert_command(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Popover::Commands { start, query, .. } = self.popover.clone() {
            let text = self.input_text(cx);
            let end = (start + 1 + query.len()).min(text.len());
            let mut next = String::with_capacity(text.len() + name.len());
            next.push_str(&text[..start.min(text.len())]);
            next.push('/');
            next.push_str(name);
            next.push(' ');
            next.push_str(&text[end..]);
            self.set_input_text(&next, window, cx);
        }
        self.popover = Popover::Closed;
        cx.notify();
    }

    /// Changes a `ConfigOption` and tells the agent (`chat.md`, "Selectores").
    pub fn choose_config_value(
        &mut self,
        config_id: &SessionConfigId,
        value: SessionConfigValueId,
        cx: &mut Context<Self>,
    ) {
        // Optimistic: the agent's `configOptions` answer overwrites it.
        if let Some(option) = self
            .config_options
            .iter_mut()
            .find(|option| &option.id == config_id)
            && let SessionConfigKind::Select(select) = &mut option.kind
        {
            select.current_value = value.clone();
        }
        self.popover = Popover::Closed;
        let Some(session_id) = self.session_id.clone() else {
            cx.notify();
            return;
        };
        cx.emit(ChatEvent::Command(AgentCommand::SetConfigOption {
            session_id,
            config_id: config_id.clone(),
            value: SessionConfigOptionValue::ValueId { value },
        }));
        cx.notify();
    }

    /// Changes the legacy session mode (`session/set_mode`).
    pub fn choose_mode(&mut self, mode_id: SessionModeId, cx: &mut Context<Self>) {
        if let Some(modes) = self.modes.as_mut() {
            modes.current_mode_id = mode_id.clone();
        }
        self.popover = Popover::Closed;
        let Some(session_id) = self.session_id.clone() else {
            cx.notify();
            return;
        };
        cx.emit(ChatEvent::Command(AgentCommand::SetMode {
            session_id,
            mode_id,
        }));
        cx.notify();
    }

    // ------------------------------------------------------------- popovers

    /// Whether `Enter` belongs to the open popover rather than to sending.
    #[must_use]
    pub fn popover_confirms(&self) -> bool {
        self.popover_rows() > 0
    }

    /// How many rows the open `@` / `/` popover is showing.
    #[must_use]
    pub fn popover_rows(&self) -> usize {
        match &self.popover {
            Popover::Files { .. } => self.filtered_files().len(),
            Popover::Commands { .. } => self.filtered_commands().len(),
            _ => 0,
        }
    }

    /// The row the arrows have moved to in the open `@` / `/` popover.
    #[must_use]
    pub fn popover_selection(&self) -> Option<usize> {
        match &self.popover {
            Popover::Files { selected, .. } | Popover::Commands { selected, .. } => Some(*selected),
            _ => None,
        }
    }

    /// Opens (or closes) a header/footer popover.
    ///
    /// A click on the very button that had opened the menu arrives right after
    /// the click outside the menu closed it: that second gesture must leave it
    /// closed instead of opening it again (`just_dismissed`).
    pub fn toggle_popover(&mut self, popover: Popover, cx: &mut Context<Self>) {
        let just_closed = self
            .just_dismissed
            .take()
            .is_some_and(|(closed, press)| closed == popover && press == self.press_serial);
        if just_closed {
            cx.notify();
            return;
        }
        if self.popover == popover {
            self.popover = Popover::Closed;
            cx.notify();
        } else {
            self.open_popover(popover, cx);
        }
    }

    /// Shows `popover`; one that takes the focus gets it on the next paint.
    fn open_popover(&mut self, popover: Popover, cx: &mut Context<Self>) {
        self.focus_popover_pending = popover.takes_focus();
        self.popover = popover;
        cx.notify();
    }

    /// Closes whatever is open (`Esc`, choosing a row). The focus goes back to
    /// whoever had it before the menu took it, on the next paint
    /// ([`ChatPanel::sync_popover_focus`]).
    pub fn close_popover(&mut self, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.notify();
    }

    /// Closes whatever is open because the user went elsewhere: a click
    /// outside the menu, the focus moving to another element or the window
    /// losing it. Unlike [`ChatPanel::close_popover`] it remembers what it
    /// closed, so the click on the button that opened the menu does not
    /// reopen it, and it does not take the focus back from where it went.
    pub(crate) fn dismiss_popover(&mut self, cx: &mut Context<Self>) {
        if self.popover == Popover::Closed {
            return;
        }
        self.just_dismissed = Some((
            std::mem::replace(&mut self.popover, Popover::Closed),
            self.press_serial,
        ));
        self.restore_focus = None;
        cx.notify();
    }

    /// Moves the keyboard focus for the menus, while painting (the only place
    /// the panel has the window at hand without changing its public API):
    /// a menu that takes the focus gets it, remembering who had it; a menu
    /// that closed while it still held it gives it back.
    pub(crate) fn sync_popover_focus(
        &mut self,
        overlay_shown: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let holds_focus = self.popover_focus.is_focused(window);
        if self.popover.takes_focus() && overlay_shown {
            if self.focus_popover_pending && !holds_focus {
                self.restore_focus = window.focused(cx).filter(|previous| {
                    // The panel's own root is not a place worth returning to:
                    // the composer is.
                    *previous != self.focus_handle && *previous != self.popover_focus
                });
                window.focus(&self.popover_focus, cx);
            }
            self.focus_popover_pending = false;
            return;
        }
        self.focus_popover_pending = false;
        if holds_focus && !self.popover.takes_focus() {
            let target = self
                .restore_focus
                .take()
                .unwrap_or_else(|| self.input.read(cx).focus_handle(cx));
            window.focus(&target, cx);
        }
        if !self.popover.takes_focus() {
            self.restore_focus = None;
        }
    }

    /// Moves the selection of the open popover.
    pub fn move_popover_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.popover_rows();
        if count == 0 {
            return;
        }
        let selected = match &mut self.popover {
            Popover::Files { selected, .. } | Popover::Commands { selected, .. } => selected,
            _ => return,
        };
        let next = (*selected as isize + delta).rem_euclid(count as isize);
        *selected = next as usize;
        cx.notify();
    }

    /// Confirms the selected row of the open popover.
    pub fn confirm_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.popover.clone() {
            Popover::Files { selected, .. } => {
                if let Some(path) = self.filtered_files().get(selected).cloned() {
                    self.insert_mention(path, window, cx);
                }
            }
            Popover::Commands { selected, .. } => {
                if let Some(name) = self
                    .filtered_commands()
                    .get(selected)
                    .map(|command| command.name.clone())
                {
                    self.insert_command(&name, window, cx);
                }
            }
            _ => {}
        }
    }

    /// The paths the `@` popover lists: substring filter, v1 without fuzzy
    /// matching (`chat.md`), ordered by how well the **name** matches.
    ///
    /// Typing `mai` puts `main.rs` before `src/domain.rs`: a prefix of the file
    /// name first, then a substring of the name, then a match anywhere in the
    /// path. Ties keep the order the workspace sent, which is the worktree's.
    #[must_use]
    pub fn filtered_files(&self) -> Vec<PathBuf> {
        let Popover::Files { query, .. } = &self.popover else {
            return Vec::new();
        };
        let needle = query.to_lowercase();
        let mut scored: Vec<(u8, usize, &PathBuf)> = self
            .file_candidates
            .iter()
            .enumerate()
            .filter_map(|(order, path)| {
                let rank = match_rank(path, &needle)?;
                Some((rank, order, path))
            })
            .collect();
        scored.sort_by_key(|(rank, order, _)| (*rank, *order));
        scored
            .into_iter()
            .take(crate::settings::POPOVER_MAX_ROWS)
            .map(|(_, _, path)| path.clone())
            .collect()
    }

    /// The commands the `/` popover lists.
    #[must_use]
    pub fn filtered_commands(&self) -> Vec<AvailableCommand> {
        let Popover::Commands { query, .. } = &self.popover else {
            return Vec::new();
        };
        let needle = query.to_lowercase();
        self.commands
            .iter()
            .filter(|command| needle.is_empty() || command.name.to_lowercase().contains(&needle))
            .take(crate::settings::POPOVER_MAX_ROWS)
            .cloned()
            .collect()
    }

    /// Shows the rest of a tool card's output ("ver más") or folds it back.
    pub fn toggle_tool_output(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(Entry::ToolCall(call)) = self.entries.get_mut(index) {
            call.output_expanded = !call.output_expanded;
            cx.notify();
        }
    }

    /// Folds or unfolds an entry (thought, tool card, plan, permission detail).
    pub fn toggle_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.entries.get_mut(index) {
            Some(Entry::AgentThought(thought)) => thought.collapsed = !thought.collapsed,
            Some(Entry::ToolCall(call)) => call.expanded = !call.expanded,
            Some(Entry::Plan(plan)) => plan.collapsed = !plan.collapsed,
            Some(Entry::Permission(permission)) => permission.expanded = !permission.expanded,
            _ => return,
        }
        cx.notify();
    }

    /// "Ver en el editor" of an edit tool card.
    pub fn open_file_at_hunk(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::OpenFileAtHunk { path });
    }

    /// Puts `text` in the clipboard through the workspace.
    pub fn copy(&mut self, text: String, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::CopyToClipboard(text));
    }

    /// "Abrir terminal" of the authentication card.
    pub fn open_terminal(&mut self, command: String, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::OpenTerminalWithCommand(command));
    }

    // ----------------------------------------------------------- persistence

    /// The transcript as the workspace stores it (`chat.md`, "Persistencia").
    #[must_use]
    pub fn export_transcript(&self) -> TranscriptDump {
        TranscriptDump {
            version: TRANSCRIPT_VERSION,
            agent_id: self
                .active_connection()
                .map(|connection| connection.agent_id.clone()),
            session_id: self.session_id.as_ref().map(|id| id.0.to_string()),
            entries: self.entries.clone(),
        }
    }

    /// Restores a transcript, rebuilding the markdown states.
    ///
    /// Read-only history: it does not touch the session, so nothing is sent to
    /// the agent (`chat.md`: "solo lectura hasta que el agente soporte
    /// `session/load`").
    pub fn import_transcript(&mut self, dump: TranscriptDump, cx: &mut Context<Self>) {
        self.entries = dump.entries;
        self.bubble_views.borrow_mut().clear();
        self.expanded_code_blocks.clear();
        self.rebuild_views(cx);
        cx.notify();
    }
}

// -------------------------------------------------------------------- helpers

/// Appends a streamed chunk to an answer and keeps its segments in step
/// (`docs/specs/10-etapa7-ronda2.md` §10.3).
///
/// Only the tail is cut again, from the start of the last segment: what came
/// before it is final (a prose run ends where a long block opens, and a long
/// block ends at its closing fence). When the tail is still one segment of
/// the same kind that reached the end of the text, the chunk is appended to
/// its state without reparsing (the fast path of every chunk of an ordinary
/// answer); otherwise the last state gets its new text and the segments that
/// appeared get states of their own. The states before the tail are never
/// touched, so the text above a block that starts folding keeps its entity.
pub(crate) fn append_to_segments(text: &mut AgentText, chunk: &str, cx: &mut App) {
    let old_len = text.markdown.len();
    text.markdown.push_str(chunk);
    let tail_start = text.segments.last().map_or(0, |last| last.range.start);
    let mut fresh = split_markdown(&text.markdown[tail_start..]);
    for segment in &mut fresh {
        segment.range = segment.range.start + tail_start..segment.range.end + tail_start;
    }
    let markdown = &text.markdown;
    let mut fresh = fresh.into_iter();
    if let Some(last) = text.segments.last_mut() {
        let Some(first) = fresh.next() else {
            // The tail is whitespace only: nothing to show yet.
            return;
        };
        let fast = fresh.len() == 0
            && first.kind.same_variant(last.kind)
            && first.range.start == last.range.start
            && last.range.end == old_len
            && first.range.end == markdown.len();
        if fast {
            append_chunk(&last.view, chunk, cx);
        } else if first.range != last.range {
            // With the same range the state already holds these bytes.
            set_text(&last.view, &markdown[first.range.clone()], cx);
        }
        last.range = first.range;
        last.kind = first.kind;
    }
    for segment in fresh {
        let view = markdown_state(&markdown[segment.range.clone()], cx);
        text.segments.push(TextSegment {
            range: segment.range,
            kind: segment.kind,
            view,
        });
    }
}

/// Builds the segments of an answer from its whole Markdown, reusing the
/// states it already had (an imported transcript, a loaded conversation).
pub(crate) fn rebuild_segments(text: &mut AgentText, cx: &mut App) {
    let fresh = split_markdown(&text.markdown);
    let mut old = std::mem::take(&mut text.segments).into_iter();
    for segment in fresh {
        let source = &text.markdown[segment.range.clone()];
        let view = match old.next() {
            Some(reused) => {
                set_text(&reused.view, source, cx);
                reused.view
            }
            None => markdown_state(source, cx),
        };
        text.segments.push(TextSegment {
            range: segment.range,
            kind: segment.kind,
            view,
        });
    }
}

/// Whether a tool card shows its detail without being asked.
///
/// Only a failure does: everything else is noise while the turn runs
/// (`02-visual.md` §7).
pub(crate) fn expands_by_default(status: ToolCallStatus) -> bool {
    status == ToolCallStatus::Failed
}

/// If `text` is nothing but mention links as the Claude adapter writes them
/// (`[@name](file://…)`, possibly several, with whitespace between), the
/// mentioned file names; `None` for any other text.
pub(crate) fn mention_links_only(text: &str) -> Option<Vec<String>> {
    let mut rest = text.trim();
    let mut names = Vec::new();
    while !rest.is_empty() {
        let after_open = rest.strip_prefix("[@")?;
        let close = after_open.find("](")?;
        let name = &after_open[..close];
        let after_name = &after_open[close + 2..];
        let end = after_name.find(')')?;
        let uri = &after_name[..end];
        if name.is_empty() || name.contains('\n') || !uri.starts_with("file://") {
            return None;
        }
        names.push(name.to_string());
        rest = after_name[end + 1..].trim_start();
    }
    (!names.is_empty()).then_some(names)
}

/// A user message that is only the replayed mention links of §11 (spec 10):
/// what builds before the fix stored on every `session/load`.
fn is_stray_mention_bubble(entry: &Entry) -> bool {
    match entry {
        Entry::UserMessage(message) => match message.blocks.as_slice() {
            [MessageBlock::Text(text)] => mention_links_only(text).is_some(),
            _ => false,
        },
        _ => false,
    }
}

/// The text of a content block, as the transcript shows it.
pub(crate) fn block_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        ContentBlock::ResourceLink(link) => format!("[{}]({})", link.name, link.uri),
        ContentBlock::Resource(_) => String::new(),
        ContentBlock::Image(_) => "«imagen»".to_string(),
        ContentBlock::Audio(_) => "«audio»".to_string(),
        _ => String::new(),
    }
}

/// Turns a composed block into the ACP prompt block.
///
/// A file chip becomes `PromptBlock::ResourceLink` with the **absolute** path,
/// which is what `chat.md`'s acceptance criterion asks for; an image becomes
/// `PromptBlock::Image` with its bytes (base64 is `cincel-acp`'s job, D4). A
/// comment card is not the user's prompt (the workspace sends the comments
/// inside the review feedback) and an image whose bytes are gone cannot
/// travel: both give `None`.
pub(crate) fn message_block_to_prompt(block: &MessageBlock) -> Option<PromptBlock> {
    match block {
        MessageBlock::Text(text) => Some(PromptBlock::Text(text.clone())),
        MessageBlock::File(path) => Some(PromptBlock::mention(path)),
        MessageBlock::Image(image) => {
            let data = image.data.clone()?;
            match PromptBlock::image(&image.mime_type, data) {
                Ok(block) => Some(block),
                Err(error) => {
                    tracing::warn!(%error, name = %image.name, "una imagen adjunta no se pudo enviar");
                    None
                }
            }
        }
        MessageBlock::Comment(_) => None,
    }
}

/// Maps a tool call's content onto what the card shows, never a full diff.
pub(crate) fn tool_contents(
    content: &[ToolCallContent],
    kind: ToolKind,
    title: &str,
    preview_lines: usize,
) -> Vec<ToolContent> {
    let mut out = Vec::new();
    let mut output = String::new();
    for item in content {
        match item {
            ToolCallContent::Content(inner) => {
                let text = block_text(&inner.content);
                if text.is_empty() {
                    continue;
                }
                if kind == ToolKind::Execute {
                    // Agents wrap `execute` output in a ```console fence; the
                    // card already is a terminal (`etapa-2.md` § correcciones).
                    let text = strip_code_fences(&text);
                    if text.is_empty() {
                        continue;
                    }
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&text);
                } else {
                    out.push(ToolContent::Text(text));
                }
            }
            ToolCallContent::Diff(diff) => out.push(ToolContent::Edit {
                path: diff.path.clone(),
                preview: diff
                    .new_text
                    .lines()
                    .take(preview_lines)
                    .map(str::to_string)
                    .collect(),
            }),
            ToolCallContent::Terminal(terminal) => {
                out.push(ToolContent::Text(format!(
                    "terminal {}",
                    terminal.terminal_id.0
                )));
            }
            _ => {}
        }
    }
    if kind == ToolKind::Execute {
        out.insert(
            0,
            ToolContent::Command {
                command: title.to_string(),
                output,
            },
        );
    }
    out
}

/// One authentication method, as the card shows it.
///
/// A `Terminal` method is the one the user has to run themselves; its command
/// is the agent's own launch line plus the method's arguments, which only the
/// workspace knows (it owns the `LaunchSpec`), so the card shows the arguments
/// and the workspace may replace them with
/// `cincel_acp::protocol::AuthMethodView::shell_command`.
fn auth_choice(method: &AuthMethod) -> AuthChoice {
    let command = match method {
        AuthMethod::Terminal(terminal) => Some(terminal.args.join(" ")),
        _ => None,
    };
    AuthChoice {
        id: method.id().0.to_string(),
        name: method.name().to_string(),
        command: command.filter(|command| !command.is_empty()),
    }
}

/// How well `path` matches `needle`: lower is better, `None` is no match.
///
/// 0 = the file name starts with it, 1 = the file name contains it, 2 = the
/// rest of the path contains it. An empty needle matches everything at 0.
pub(crate) fn match_rank(path: &std::path::Path, needle: &str) -> Option<u8> {
    if needle.is_empty() {
        return Some(0);
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.starts_with(needle) {
        return Some(0);
    }
    if name.contains(needle) {
        return Some(1);
    }
    path.to_string_lossy()
        .to_lowercase()
        .contains(needle)
        .then_some(2)
}

/// Finds the `@` or `/` the caret is inside.
///
/// Returns `(trigger, byte offset of the trigger, what follows it)`. A trigger
/// only counts at the start of a word, so `foo@bar` and `a/b` are plain text.
pub(crate) fn trigger_at(text: &str, cursor: usize) -> Option<(char, usize, String)> {
    let head = text.get(..cursor)?;
    let start = head
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let token = &head[start..];
    let mut chars = token.chars();
    let trigger = chars.next()?;
    if trigger != '@' && trigger != '/' {
        return None;
    }
    let query = chars.as_str().to_string();
    if query.contains(['@', '/']) {
        return None;
    }
    Some((trigger, start, query))
}

/// The select options of a config option, flattened out of its groups.
pub(crate) fn select_options(option: &SessionConfigOption) -> Vec<SessionConfigSelectOption> {
    match &option.kind {
        SessionConfigKind::Select(select) => match &select.options {
            SessionConfigSelectOptions::Ungrouped(options) => options.clone(),
            SessionConfigSelectOptions::Grouped(groups) => groups
                .iter()
                .flat_map(|group| group.options.iter().cloned())
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The label of the selected value of a config option.
pub(crate) fn current_label(option: &SessionConfigOption) -> String {
    match &option.kind {
        SessionConfigKind::Select(select) => select_options(option)
            .into_iter()
            .find(|candidate| candidate.value == select.current_value)
            .map_or_else(|| select.current_value.0.to_string(), |value| value.name),
        SessionConfigKind::Boolean(boolean) => {
            if boolean.current_value { "Sí" } else { "No" }.to_string()
        }
        _ => String::new(),
    }
}

/// Whether the footer paints a selector for this option (`chat.md`).
pub(crate) fn is_footer_option(option: &SessionConfigOption) -> bool {
    matches!(
        option.category,
        Some(SessionConfigOptionCategory::Mode)
            | Some(SessionConfigOptionCategory::Model)
            | Some(SessionConfigOptionCategory::ThoughtLevel)
            | None
    ) && matches!(option.kind, SessionConfigKind::Select(_))
}
