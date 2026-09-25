//! [`ChatPanel`]: the entity, its state and everything that is not painting.
//!
//! Painting lives in [`crate::render`], which is a second `impl ChatPanel`
//! block so this file stays about the model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use asteroid_acp::acp::schema::v1::{
    AuthMethod, AvailableCommand, ContentBlock, PermissionOption, SessionConfigId,
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigOptionValue,
    SessionConfigSelectOption, SessionConfigSelectOptions, SessionConfigValueId, SessionId,
    SessionModeId, SessionModeState, SessionUpdate, ToolCallContent, ToolCallId, ToolCallStatus,
    ToolCallUpdate, ToolKind,
};
use asteroid_acp::protocol::PromptBlock;
use asteroid_acp::{
    AgentCommand, AgentEvent, AutonomyMode, PermissionOutcome, PermissionRequestId,
};
use asteroid_syntax::LanguageRegistry;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyContext,
    ScrollHandle, Subscription, Window,
};
use gpui_kit::component::input::{InputEvent, TextareaState};

use crate::actions;
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
    /// The agent selector of the header.
    Agents,
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
    /// The autonomy selector of the footer.
    AutonomySelect,
    /// A `ConfigOption` selector of the footer, by index in `config_options`.
    Config(usize),
    /// The legacy `availableModes` selector of the footer.
    Modes,
}

/// How many stderr lines are kept for the "agente caído" card (`02-visual.md` §9).
const STDERR_KEPT: usize = 8;

/// The placeholder before an agent is chosen.
const DEFAULT_PLACEHOLDER: &str = "Escribí un mensaje para el agente…";

/// The line under the input that names the four things the composer does.
pub const COMPOSER_HINT: &str =
    "@ archivos · / comandos · Enter envía · Shift+Enter salto de línea";

/// The chat panel (`docs/specs/modulos/chat.md`).
pub struct ChatPanel {
    theme: ChatTheme,
    settings: ChatSettings,
    focus_handle: FocusHandle,
    /// The composer.
    pub(crate) input: Entity<TextareaState>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) entries: Vec<Entry>,
    pub(crate) agents: Vec<ChatAgent>,
    pub(crate) active_agent: Option<usize>,
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
    pub(crate) autonomy: AutonomyMode,
    /// `@` mentions of the current draft: token → absolute path.
    pub(crate) mentions: Vec<Mention>,
    pub(crate) popover: Popover,
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
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(settings.input_min_rows, settings.input_max_rows)
                .submit_on_enter(true)
                .placeholder(DEFAULT_PLACEHOLDER)
        });
        let subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        Self {
            theme,
            settings,
            focus_handle: cx.focus_handle(),
            input,
            scroll: ScrollHandle::new(),
            entries: Vec::new(),
            agents: Vec::new(),
            active_agent: None,
            conversations: Vec::new(),
            active_conversation: None,
            history_notice: None,
            session_id: None,
            status: AgentStatus::Disconnected,
            config_options: Vec::new(),
            modes: None,
            commands: Vec::new(),
            autonomy: AutonomyMode::default(),
            mentions: Vec::new(),
            popover: Popover::Closed,
            file_candidates: Vec::new(),
            project_root: None,
            auth_methods: Vec::new(),
            stderr: Vec::new(),
            registry: Arc::new(LanguageRegistry::new()),
            tool_index: HashMap::new(),
            pending_permission: None,
            replaying: false,
            placeholder_dirty: false,
            turn_started: None,
            thought_started: None,
            _subscriptions: vec![subscription],
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

    /// The autonomy of the session (`01-producto.md` §F5).
    #[must_use]
    pub fn autonomy(&self) -> AutonomyMode {
        self.autonomy
    }

    /// The agents the header offers.
    #[must_use]
    pub fn agents(&self) -> &[ChatAgent] {
        &self.agents
    }

    /// The agent the header shows, when one is selected.
    #[must_use]
    pub fn active_agent(&self) -> Option<&ChatAgent> {
        self.active_agent.and_then(|index| self.agents.get(index))
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
        self.input.read(cx).value().to_string()
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
        cx.notify();
    }

    /// Hot-reloads the settings.
    pub fn set_settings(&mut self, settings: ChatSettings, cx: &mut Context<Self>) {
        self.settings = settings;
        self.input.update(cx, |input, cx| {
            input.set_auto_grow(settings.input_min_rows, settings.input_max_rows, cx);
        });
        cx.notify();
    }

    /// Replaces the list the agent selector shows (`01-producto.md` §F3.1).
    pub fn set_agents(&mut self, agents: Vec<ChatAgent>, cx: &mut Context<Self>) {
        let active = self
            .active_agent()
            .map(|agent| agent.id.clone())
            .and_then(|id| agents.iter().position(|agent| agent.id == id));
        self.agents = agents;
        let auto = active.is_none();
        self.active_agent = active.or_else(|| {
            self.agents
                .iter()
                .position(|agent| agent.installed)
                .filter(|_| self.agents.len() == 1)
        });
        self.placeholder_dirty = true;
        // Auto-selecting the only installed agent is a selection like any
        // other: the workspace has to hear about it to open a conversation.
        if auto && let Some(agent) = self.active_agent() {
            let agent_id = agent.id.clone();
            cx.emit(ChatEvent::AgentSelected { agent_id });
        }
        cx.notify();
    }

    /// Selects an agent by id, which always opens a **new** conversation for
    /// it: the workspace saves the previous one and spawns the process
    /// (`docs/etapas/etapa-2.md` § correcciones).
    ///
    /// Selecting the agent that is already active is not a no-op either — it
    /// is the "empezar de cero con este mismo agente" gesture.
    pub fn set_active_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.agents.iter().position(|agent| agent.id == id) else {
            return;
        };
        self.active_agent = Some(index);
        self.popover = Popover::Closed;
        self.placeholder_dirty = true;
        cx.emit(ChatEvent::AgentSelected {
            agent_id: id.to_string(),
        });
        cx.notify();
    }

    /// Points the header at `id` **without** starting a conversation: the
    /// workspace uses it while it loads a stored one, where the agent is part
    /// of what is being restored.
    pub fn show_active_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.agents.iter().position(|agent| agent.id == id) {
            self.active_agent = Some(index);
            self.placeholder_dirty = true;
            cx.notify();
        }
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

    /// Changes the autonomy and tells the workspace (`01-producto.md` §F5).
    pub fn set_autonomy(&mut self, autonomy: AutonomyMode, cx: &mut Context<Self>) {
        self.autonomy = autonomy;
        self.popover = Popover::Closed;
        cx.emit(ChatEvent::AutonomyChanged(autonomy));
        cx.notify();
    }

    /// Replaces the composer's text and leaves the caret at the end.
    ///
    /// `gpui-kit`'s `set_value` is silent on purpose (it resets the undo stack
    /// and emits no `InputEvent::Change`), so the `@` / `/` popover is
    /// recomputed here instead of waiting for an event that never comes.
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
        let caret = caret.min(text.len());
        self.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(caret..caret, cx);
        });
        self.refresh_trigger(cx);
        cx.notify();
    }

    /// Wipes the transcript and the composer, keeping agents and settings.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entries.clear();
        self.tool_index.clear();
        self.pending_permission = None;
        self.mentions.clear();
        self.history_notice = None;
        self.replaying = false;
        self.popover = Popover::Closed;
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

    /// Puts a stored conversation on screen: its entries, its autonomy and its
    /// agent. The ACP session is **not** set — reopening it (or deciding it
    /// cannot be reopened) is the workspace's half of the job.
    pub fn load_conversation(
        &mut self,
        conversation: Conversation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear(window, cx);
        self.session_id = None;
        self.active_conversation = Some(conversation.id);
        self.autonomy = conversation.autonomy;
        self.entries = conversation.entries;
        self.rebuild_views(cx);
        if let Some(index) = self
            .agents
            .iter()
            .position(|agent| agent.id == conversation.agent_id)
        {
            self.active_agent = Some(index);
            self.placeholder_dirty = true;
        }
        self.scroll
            .scroll_to_item(self.entries.len().saturating_sub(1));
        cx.notify();
    }

    /// Starts dropping the `session/load` replay of what is already on screen.
    ///
    /// The comparison is by content, not by position: an agent replays a
    /// message as one chunk where the live stream sent twenty, so an update
    /// whose text is already contained in an entry of the same kind is taken
    /// for the same thing. Anything the transcript does not have (a turn that
    /// never reached disk) is appended as usual. Cleared by `SessionCreated`,
    /// which is what closes the replay in `asteroid-acp`.
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
            SessionUpdate::UserMessageChunk(chunk) => {
                let text = block_text(&chunk.content);
                !text.is_empty()
                    && self.entries.iter().any(|entry| {
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
                    match text.view.clone() {
                        Some(view) => set_text(&view, &text.markdown, cx),
                        None => text.view = Some(markdown_state(&text.markdown, cx)),
                    }
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

    /// The placeholder of `02-visual.md` §7, with the active agent's name.
    ///
    /// What `@` and `/` do lives under the input now ([`COMPOSER_HINT`]),
    /// where it stays readable once the user starts typing.
    #[must_use]
    pub fn placeholder(&self) -> String {
        match self.active_agent() {
            Some(agent) => format!("Escribí un mensaje para {}…", agent.name),
            None => DEFAULT_PLACEHOLDER.to_string(),
        }
    }

    /// Pushes the placeholder into the textarea; needs a `Window`, so it runs
    /// from `render` whenever the active agent changed.
    pub(crate) fn sync_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.placeholder_dirty {
            return;
        }
        self.placeholder_dirty = false;
        let placeholder = self.placeholder();
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
    }

    // ------------------------------------------------------------ agent input

    /// Consumes one [`AgentEvent`]. The workspace owns the channel and forwards
    /// everything it reads; unknown variants are ignored on purpose, so the ACP
    /// crate can grow without breaking the UI.
    pub fn handle_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::Connected {
                agent_info,
                auth_methods,
                ..
            } => {
                self.auth_methods = auth_methods;
                if let Some(info) = agent_info
                    && let Some(index) = self.active_agent
                    && let Some(agent) = self.agents.get_mut(index)
                {
                    agent.name = info.title.unwrap_or(info.name);
                }
                self.status = AgentStatus::Ready;
            }
            AgentEvent::AuthRequired { methods } => {
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
            // `asteroid-acp` keeps growing (file change reports, elicitation,
            // …); anything this version does not know about is not an error.
            _ => return,
        }
        cx.notify();
    }

    fn apply_update(&mut self, update: SessionUpdate, cx: &mut Context<Self>) {
        match update {
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
                self.thought_started = None;
                if let Some(Entry::AgentText(entry)) = self.entries.last_mut()
                    && entry.streaming
                {
                    entry.markdown.push_str(&text);
                    if let Some(view) = entry.view.clone() {
                        append_chunk(&view, &text, cx);
                    }
                    return;
                }
                let view = markdown_state(&text, cx);
                self.push(Entry::AgentText(AgentText {
                    markdown: text,
                    streaming: true,
                    view: Some(view),
                }));
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
            }
            SessionUpdate::ToolCallUpdate(update) => self.apply_tool_update(&update),
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
    /// `asteroid-acp` can build, so this is the seam the demo and the tests use;
    /// [`ChatPanel::handle_event`] funnels the real event through the same code.
    /// `request_id` is the `u64` inside `asteroid_acp::PermissionRequestId`, and
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
        }
        self.scroll
            .scroll_to_item(self.entries.len().saturating_sub(1));
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

    /// What the composer reports back.
    ///
    /// `InputEvent::PressEnter` is deliberately **not** handled: `gpui-kit`
    /// emits it through `cx.emit`, which is delivered at the end of the effect
    /// cycle, long after the same keystroke has already propagated to the
    /// chat's `enter` binding. Acting on both is what made `Enter` insert the
    /// highlighted mention *and then send the message*. The keystroke has a
    /// single owner now: the actions of `actions.rs`.
    fn on_input_event(
        &mut self,
        _state: &Entity<TextareaState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.refresh_trigger(cx);
        }
    }

    /// Recomputes the `@` / `/` popover from the text before the caret.
    fn refresh_trigger(&mut self, cx: &mut Context<Self>) {
        let (text, cursor) = self
            .input
            .read_with(cx, |input, _cx| (input.value().to_string(), input.cursor()));
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
        // The mentions are inline tokens in the text, so the order the user
        // wrote is the order the agent sees (`docs/etapas/etapa-2.md`).
        let blocks = split_mentions(&self.input_text(cx), &self.mentions);
        if blocks.is_empty() {
            return;
        }
        let content: Vec<PromptBlock> = blocks.iter().map(message_block_to_prompt).collect();
        self.push(Entry::UserMessage(UserMessage {
            blocks: blocks.clone(),
        }));
        self.mentions.clear();
        self.popover = Popover::Closed;
        // The "solo lectura" line is only true until the first message: from
        // here on there is a live session again.
        self.history_notice = None;
        self.set_input_text("", window, cx);

        match self.session_id.clone() {
            Some(session_id) => {
                self.status = AgentStatus::Thinking;
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
                text: "No hay una sesión abierta: elegí un agente arriba.".to_string(),
            })),
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

    /// Moves the keyboard focus to the composer (`Ctrl+L`).
    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.input.focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Answers the permission on screen with `option_id`.
    pub fn answer_permission(
        &mut self,
        option_id: &asteroid_acp::acp::schema::v1::PermissionOptionId,
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
            asteroid_acp::acp::schema::v1::PermissionOptionKind::AllowOnce
                | asteroid_acp::acp::schema::v1::PermissionOptionKind::AllowAlways
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
    /// The token is plain text inside the textarea (`gpui-kit` has no inline
    /// widgets), so the user can keep typing around it —
    /// `mira este archivo @calculadora.py, ¿qué te parece?` — and
    /// [`split_mentions`] turns it back into ordered
    /// `PromptBlock::Text`/`ResourceLink` on send. Called both by the `@`
    /// picker (which replaces the `@query` it was typed over) and by the
    /// tree's "Mencionar en el chat" (which inserts wherever the caret is).
    pub fn insert_mention(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input_text(cx);
        let caret = self.input.read(cx).cursor().min(text.len());
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
    pub fn toggle_popover(&mut self, popover: Popover, cx: &mut Context<Self>) {
        self.popover = if self.popover == popover {
            Popover::Closed
        } else {
            popover
        };
        cx.notify();
    }

    /// Closes whatever is open.
    pub fn close_popover(&mut self, cx: &mut Context<Self>) {
        self.popover = Popover::Closed;
        cx.notify();
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
            agent_id: self.active_agent().map(|agent| agent.id.clone()),
            session_id: self.session_id.as_ref().map(|id| id.0.to_string()),
            autonomy: self.autonomy,
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
        self.autonomy = dump.autonomy;
        self.rebuild_views(cx);
        if let Some(agent_id) = dump.agent_id {
            self.active_agent = self.agents.iter().position(|agent| agent.id == agent_id);
        }
        cx.notify();
    }
}

// -------------------------------------------------------------------- helpers

/// Whether a tool card shows its detail without being asked.
///
/// Only a failure does: everything else is noise while the turn runs
/// (`02-visual.md` §7).
pub(crate) fn expands_by_default(status: ToolCallStatus) -> bool {
    status == ToolCallStatus::Failed
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
/// which is what `chat.md`'s acceptance criterion asks for.
pub(crate) fn message_block_to_prompt(block: &MessageBlock) -> PromptBlock {
    match block {
        MessageBlock::Text(text) => PromptBlock::Text(text.clone()),
        MessageBlock::File(path) => PromptBlock::mention(path),
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
/// `asteroid_acp::protocol::AuthMethodView::shell_command`.
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
