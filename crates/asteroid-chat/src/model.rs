//! The transcript model of `docs/specs/modulos/chat.md` ("Estructura").
//!
//! Everything here is plain data and serialisable, so the workspace can persist
//! the session transcript per project ([`TranscriptDump`]). The only non-data
//! members are the `gpui-kit` markdown states, which are `#[serde(skip)]` and
//! rebuilt on import.

use std::path::PathBuf;

use asteroid_acp::AutonomyMode;
use asteroid_acp::acp::schema::v1::{
    PermissionOptionId, PermissionOptionKind, PlanEntryStatus, StopReason, ToolCallId,
    ToolCallStatus, ToolKind,
};
use gpui::{Entity, SharedString};
use gpui_kit::component::text::TextViewState;
use serde::{Deserialize, Serialize};

/// An agent the user can pick in the header (`01-producto.md` §F3.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatAgent {
    /// Registry id (`claude-acp`, `gemini`, …).
    pub id: String,
    /// Name shown in the selector.
    pub name: String,
    /// Whether its executable is available; the row is dimmed when it is not.
    pub installed: bool,
    /// What to do about it when it is missing (`npx …`), shown as a hint.
    pub hint: Option<String>,
}

impl ChatAgent {
    /// An installed agent with no hint.
    #[must_use]
    pub fn installed(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            installed: true,
            hint: None,
        }
    }
}

/// One row of the history popover: a stored conversation, as the workspace
/// summarised it out of `index.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// Conversation id (the file name under `conversations/`).
    pub id: String,
    /// Registry id of the agent that produced it; the popover groups by it.
    pub agent_id: String,
    /// Name of that agent, which is what the group header shows.
    pub agent_name: String,
    /// One-line title (the first user message).
    pub title: String,
    /// Already-formatted relative date ("hace 5 min", "ayer"), so this crate
    /// needs no clock.
    pub when: String,
    /// ACP session id, when the conversation ever had one.
    pub session_id: Option<String>,
}

/// What the header pill says (`02-visual.md` §7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    /// Connected and idle.
    #[default]
    Ready,
    /// A turn is running.
    Thinking,
    /// A permission request is on screen.
    WaitingPermission,
    /// No agent process.
    Disconnected,
    /// The agent answered `auth_required`.
    AuthRequired,
}

impl AgentStatus {
    /// The Spanish label of `02-visual.md` §7.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Ready => "listo",
            AgentStatus::Thinking => "pensando…",
            AgentStatus::WaitingPermission => "esperando permiso",
            AgentStatus::Disconnected => "desconectado",
            AgentStatus::AuthRequired => "autenticación requerida",
        }
    }
}

/// A block of a user message, before it becomes an ACP `ContentBlock`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageBlock {
    /// Typed text.
    Text(String),
    /// A file chip inserted with `@`; becomes a `resource_link` with the
    /// absolute path (`chat.md`, acceptance criteria).
    File(PathBuf),
}

/// One `@archivo` the user inserted in the draft, as the composer tracks it.
///
/// The token is the literal text that sits **inside** the textarea
/// (`@calculadora.py`), because `gpui-kit`'s `Textarea` holds plain text and
/// has no inline-widget API; the path is what actually travels to the agent.
/// Editing or deleting the token by hand simply makes the mention stop
/// existing: [`split_mentions`] only emits a link for a token it still finds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    /// The literal text in the draft, `@` included.
    pub token: String,
    /// Absolute path of the mentioned file.
    pub path: PathBuf,
}

/// Severity of a [`Notice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeLevel {
    /// Neutral information.
    Info,
    /// Something to look at.
    Warning,
    /// The agent died, a command failed, …
    Error,
}

/// The message the user sent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserMessage {
    /// Text and file chips, in order.
    pub blocks: Vec<MessageBlock>,
}

/// A streamed markdown answer (`02-visual.md` §7).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentText {
    /// The markdown received so far; chunks are appended to it.
    pub markdown: String,
    /// Whether more chunks are still expected.
    pub streaming: bool,
    /// The `gpui-kit` streaming state; rebuilt on import.
    #[serde(skip)]
    pub view: Option<Entity<TextViewState>>,
}

/// The agent's internal reasoning, collapsed by default.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentThought {
    /// The reasoning received so far.
    pub text: String,
    /// Whether the one-line summary is shown instead of the text.
    pub collapsed: bool,
    /// Seconds spent thinking, as shown in "Pensando… (12 s)".
    pub duration_secs: u64,
    /// The `gpui-kit` streaming state; rebuilt on import.
    #[serde(skip)]
    pub view: Option<Entity<TextViewState>>,
}

/// The detail a tool card shows when expanded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolContent {
    /// Plain output or a message.
    Text(String),
    /// A shell command and what it printed (`kind = execute`).
    Command {
        /// The command line.
        command: String,
        /// Its output so far.
        output: String,
    },
    /// An edit. Only the first lines of context are kept: the chat never shows
    /// a full diff (`chat.md`).
    Edit {
        /// Absolute path of the edited file.
        path: PathBuf,
        /// At most `ChatSettings::diff_preview_lines` lines of context.
        preview: Vec<String>,
    },
}

/// A tool call card (`02-visual.md` §7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallEntry {
    /// Id the agent uses in its updates.
    pub id: ToolCallId,
    /// Which icon to paint.
    pub kind: ToolKind,
    /// The title the agent sent.
    pub title: String,
    /// Spinner / ✓ / ✗.
    pub status: ToolCallStatus,
    /// Detail shown when expanded.
    pub content: Vec<ToolContent>,
    /// Files this call touches.
    pub locations: Vec<PathBuf>,
    /// `+N −M`, set by the workspace from `ReviewStore::stats`.
    pub stats: Option<(u32, u32)>,
    /// Whether the detail is on screen.
    pub expanded: bool,
    /// Whether the whole output is on screen instead of its first
    /// [`crate::settings::MAX_OUTPUT_LINES`] lines ("ver más").
    #[serde(default)]
    pub output_expanded: bool,
}

/// One task of the agent's plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    /// What the task says.
    pub text: String,
    /// Pending / in progress / done.
    pub status: PlanEntryStatus,
}

/// The agent's plan, collapsible.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlanEntry {
    /// The checklist.
    pub items: Vec<PlanItem>,
    /// Whether the checklist is hidden.
    pub collapsed: bool,
}

/// One button of a permission card, in the order the agent sent it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionChoice {
    /// Id to send back.
    pub id: PermissionOptionId,
    /// Label to paint.
    pub name: String,
    /// Allow / reject, once / always.
    pub kind: PermissionOptionKind,
}

/// A permission request (`02-visual.md` §7, `chat.md` "Permisos").
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionEntry {
    /// The request id of `asteroid_acp::PermissionRequestId`.
    pub request_id: u64,
    /// Title of the tool call about to run.
    pub title: String,
    /// Optional detail, collapsed.
    pub detail: Option<String>,
    /// The buttons, in the agent's order.
    pub options: Vec<PermissionChoice>,
    /// The label of the chosen option once answered; the card then compacts to
    /// a single "Permitido / Rechazado" line.
    pub answered: Option<String>,
    /// Whether the answer was an `allow_*` one.
    pub allowed: bool,
    /// Whether the detail is on screen.
    pub expanded: bool,
}

impl PermissionEntry {
    /// Index of the option `Enter` chooses: the first one the agent sent.
    #[must_use]
    pub fn default_option(&self) -> Option<&PermissionChoice> {
        self.options.first()
    }

    /// Index of the option `Esc` chooses: the first `reject_*` one, falling
    /// back to the last option when the agent offers no rejection.
    #[must_use]
    pub fn reject_option(&self) -> Option<&PermissionChoice> {
        self.options
            .iter()
            .find(|option| {
                matches!(
                    option.kind,
                    PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                )
            })
            .or_else(|| self.options.last())
    }
}

/// One way of authenticating, as `initialize` advertised it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthChoice {
    /// Method id to send with `authenticate`.
    pub id: String,
    /// Label shown on the button.
    pub name: String,
    /// The command the user has to run in a terminal, when there is one.
    pub command: Option<String>,
}

/// The "hace falta autenticarse" card (`02-visual.md` §7).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthEntry {
    /// The methods the agent offers.
    pub methods: Vec<AuthChoice>,
}

impl AuthEntry {
    /// The command shown next to "Copiar"; the first method that has one.
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        self.methods
            .iter()
            .find_map(|method| method.command.as_deref())
    }
}

/// A one-line message from Asteroid itself (`02-visual.md` §9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// How loud it is.
    pub level: NoticeLevel,
    /// What it says.
    pub text: String,
}

/// The line that closes a turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnSeparator {
    /// Why the turn stopped, already translated.
    pub stop_reason: String,
    /// How long the turn took.
    pub duration_secs: u64,
}

/// Translates a [`StopReason`] into the words the separator shows.
#[must_use]
pub fn stop_reason_label(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "turno terminado",
        StopReason::MaxTokens => "límite de tokens",
        StopReason::MaxTurnRequests => "límite de pedidos",
        StopReason::Refusal => "el agente se negó",
        StopReason::Cancelled => "cancelado",
        _ => "turno terminado",
    }
}

/// One row of the transcript (`chat.md`, "Estructura").
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Entry {
    /// What the user sent.
    UserMessage(UserMessage),
    /// A streamed markdown answer.
    AgentText(AgentText),
    /// The agent's reasoning.
    AgentThought(AgentThought),
    /// A tool call card.
    ToolCall(ToolCallEntry),
    /// The agent's plan.
    Plan(PlanEntry),
    /// A permission request.
    Permission(PermissionEntry),
    /// The authentication card.
    AuthRequired(AuthEntry),
    /// A message from Asteroid.
    Notice(Notice),
    /// The end of a turn.
    TurnSeparator(TurnSeparator),
}

/// AutonomyMode as it travels through `serde` (`AutonomyMode` lives in `asteroid-acp`,
/// which has no `serde` dependency).
pub(crate) mod autonomy_serde {
    use super::AutonomyMode;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(crate) fn serialize<S: Serializer>(
        value: &AutonomyMode,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        id(*value).serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<AutonomyMode, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(from_id(&raw))
    }

    /// The stable id of an autonomy value.
    #[must_use]
    pub(crate) fn id(value: AutonomyMode) -> &'static str {
        match value {
            AutonomyMode::ReviewAfter => "review_after",
            AutonomyMode::AskBefore => "ask_before",
            AutonomyMode::AlwaysApply => "always_apply",
        }
    }

    /// The autonomy value of an id, defaulting to the spec's default.
    #[must_use]
    pub(crate) fn from_id(raw: &str) -> AutonomyMode {
        match raw {
            "ask_before" => AutonomyMode::AskBefore,
            "always_apply" => AutonomyMode::AlwaysApply,
            _ => AutonomyMode::ReviewAfter,
        }
    }
}

/// The label of an autonomy value in the composer selector (`01-producto.md` §F5).
///
/// The words say what happens, not what the setting is called: "Pedir antes"
/// left the user guessing what was being asked and when.
#[must_use]
pub fn autonomy_label(value: AutonomyMode) -> &'static str {
    match value {
        AutonomyMode::ReviewAfter => "Aplicar y revisar después",
        AutonomyMode::AskBefore => "Preguntar antes de cada cambio",
        AutonomyMode::AlwaysApply => "Aplicar sin revisar",
    }
}

/// The one-sentence tooltip of an autonomy value, from `01-producto.md` §F5.
#[must_use]
pub fn autonomy_tooltip(value: AutonomyMode) -> &'static str {
    match value {
        AutonomyMode::ReviewAfter => {
            "El agente escribe sin pedir permiso; todo queda pendiente de aprobación en el editor."
        }
        AutonomyMode::AskBefore => {
            "Cada edición pide permiso antes de escribirse y además queda pendiente en el editor."
        }
        AutonomyMode::AlwaysApply => {
            "El agente escribe y acepta solo, sin pedir permiso: para tareas mecánicas."
        }
    }
}

/// Every autonomy value, in the order the selector lists them.
pub const AUTONOMY_VALUES: [AutonomyMode; 3] = [
    AutonomyMode::ReviewAfter,
    AutonomyMode::AskBefore,
    AutonomyMode::AlwaysApply,
];

/// A transcript as it is written to disk (`chat.md`, "Persistencia").
///
/// `ChatPanel::export_transcript` produces it and `ChatPanel::import_transcript`
/// restores it; the workspace stores it in the XDG state directory per project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptDump {
    /// Format version, so a future change can migrate instead of failing.
    pub version: u32,
    /// Id of the agent that produced it, when known.
    pub agent_id: Option<String>,
    /// Id of the session, when known.
    pub session_id: Option<String>,
    /// The autonomy the session was running with.
    #[serde(with = "autonomy_serde")]
    pub autonomy: AutonomyMode,
    /// The rows, in order.
    pub entries: Vec<Entry>,
}

/// The version [`TranscriptDump`] is written with.
pub const TRANSCRIPT_VERSION: u32 = 1;

/// A conversation: one agent, one thread, one file on disk.
///
/// The Antigravity model of `docs/etapas/etapa-2.md` § correcciones: choosing
/// another agent in the header does not continue the thread, it opens a new
/// one. The workspace stores these under
/// `~/.local/state/asteroid/workspaces/<hash>/conversations/<id>.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    /// Format version, so a future change can migrate instead of failing.
    pub version: u32,
    /// Opaque identifier; also the file name.
    pub id: String,
    /// Registry id of the agent this conversation belongs to.
    pub agent_id: String,
    /// Name of that agent at the time, for the history popover.
    pub agent_name: String,
    /// The ACP session id, when the agent gave one.
    pub session_id: Option<String>,
    /// Project root the conversation was held in.
    pub cwd: PathBuf,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    /// Seconds since the Unix epoch.
    pub updated_at: u64,
    /// First user message, at most [`TITLE_MAX_CHARS`] characters.
    pub title: String,
    /// The autonomy the conversation was running with.
    #[serde(with = "autonomy_serde")]
    pub autonomy: AutonomyMode,
    /// The rows, in order.
    pub entries: Vec<Entry>,
}

/// The version [`Conversation`] is written with.
pub const CONVERSATION_VERSION: u32 = 1;

/// How long a conversation title gets before it is cut.
pub const TITLE_MAX_CHARS: usize = 60;

/// What the history popover shows when a conversation has no user message yet.
pub const UNTITLED_CONVERSATION: &str = "Conversación sin título";

/// The notice shown at the top of a conversation whose agent cannot reopen its
/// ACP session (`docs/etapas/etapa-2.md` § correcciones).
pub const READ_ONLY_HISTORY_NOTICE: &str = "Historial de solo lectura: el agente no puede retomar \
     esta sesión; tu próximo mensaje abre una sesión nueva";

impl Conversation {
    /// An empty conversation for `agent_id`, created now.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        agent_id: impl Into<String>,
        agent_name: impl Into<String>,
        cwd: PathBuf,
        now: u64,
    ) -> Self {
        Self {
            version: CONVERSATION_VERSION,
            id: id.into(),
            agent_id: agent_id.into(),
            agent_name: agent_name.into(),
            session_id: None,
            cwd,
            created_at: now,
            updated_at: now,
            title: UNTITLED_CONVERSATION.to_string(),
            autonomy: AutonomyMode::default(),
            entries: Vec::new(),
        }
    }

    /// The row the history popover paints for it; `when` is the already
    /// formatted relative date, which only the workspace can produce.
    #[must_use]
    pub fn summary(&self, when: impl Into<String>) -> ConversationSummary {
        ConversationSummary {
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            agent_name: self.agent_name.clone(),
            title: self.title.clone(),
            when: when.into(),
            session_id: self.session_id.clone(),
        }
    }
}

/// The title of a conversation: its first user message, one line, at most
/// [`TITLE_MAX_CHARS`] characters (an ellipsis replaces what is cut).
#[must_use]
pub fn conversation_title(entries: &[Entry]) -> String {
    let first = entries.iter().find_map(|entry| match entry {
        Entry::UserMessage(message) => {
            let text: String = message
                .blocks
                .iter()
                .map(|block| match block {
                    MessageBlock::Text(text) => text.clone(),
                    MessageBlock::File(path) => format!("@{}", file_name_label(path)),
                })
                .collect::<Vec<_>>()
                .join(" ");
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    });
    let Some(text) = first else {
        return UNTITLED_CONVERSATION.to_string();
    };
    if text.chars().count() <= TITLE_MAX_CHARS {
        return text;
    }
    let cut: String = text.chars().take(TITLE_MAX_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The token an `@` mention takes in the draft.
///
/// `qualified` adds the file's own directory so two files with the same name
/// stay apart (`@src/main.rs` vs `@tests/main.rs`); `full` falls back to the
/// whole project-relative path when even that collides.
#[must_use]
pub fn mention_token(
    root: Option<&std::path::Path>,
    path: &std::path::Path,
    qualified: bool,
    full: bool,
) -> String {
    let name = file_name_label(path);
    if full {
        return format!("@{}", relative_label(root, path));
    }
    if !qualified {
        return format!("@{name}");
    }
    let parent = parent_label(root, path);
    match std::path::Path::new(&*parent)
        .file_name()
        .and_then(|last| last.to_str())
    {
        Some(last) if !last.is_empty() => format!("@{last}/{name}"),
        _ => format!("@{name}"),
    }
}

/// Splits `text` around the mention tokens, keeping the order the user wrote.
///
/// A token only counts at the start of a word and when what follows it is not
/// part of the same word, so `mira @calculadora.py, ¿qué te parece?` becomes
/// text · link · text while `@calculadora.pyx` stays plain text. A token the
/// user edited or deleted is simply not found, which is what makes a mention
/// stop being one (`docs/etapas/etapa-2.md` § correcciones).
#[must_use]
pub fn split_mentions(text: &str, mentions: &[Mention]) -> Vec<MessageBlock> {
    let mut sorted: Vec<&Mention> = mentions
        .iter()
        .filter(|mention| !mention.token.is_empty())
        .collect();
    // Longest first, so `@src/main.rs` wins over a bare `@src`.
    sorted.sort_by_key(|mention| std::cmp::Reverse(mention.token.len()));

    let mut blocks: Vec<MessageBlock> = Vec::new();
    let mut pending = String::new();
    let mut index = 0usize;
    'scan: while index < text.len() {
        for mention in &sorted {
            let token = mention.token.as_str();
            if text[index..].starts_with(token) && token_boundaries(text, index, token.len()) {
                if !pending.is_empty() {
                    blocks.push(MessageBlock::Text(std::mem::take(&mut pending)));
                }
                blocks.push(MessageBlock::File(mention.path.clone()));
                index += token.len();
                continue 'scan;
            }
        }
        let ch = text[index..].chars().next().unwrap_or(' ');
        pending.push(ch);
        index += ch.len_utf8();
    }
    if !pending.is_empty() {
        blocks.push(MessageBlock::Text(pending));
    }

    // Only the message's own edges are trimmed: the spacing around a mention
    // is what the user typed and the agent should see it.
    if let Some(MessageBlock::Text(first)) = blocks.first_mut() {
        *first = first.trim_start().to_string();
    }
    if let Some(MessageBlock::Text(last)) = blocks.last_mut() {
        *last = last.trim_end().to_string();
    }
    blocks.retain(|block| !matches!(block, MessageBlock::Text(text) if text.is_empty()));
    blocks
}

/// Whether a token found at `start` is a whole word: preceded by the start of
/// the text or whitespace, and not glued to a word character on its right.
fn token_boundaries(text: &str, start: usize, len: usize) -> bool {
    let before_ok = text[..start]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    let after_ok = text[start + len..]
        .chars()
        .next()
        .is_none_or(|ch| !ch.is_alphanumeric() && !matches!(ch, '_' | '-' | '/'));
    before_ok && after_ok
}

/// Strips the markdown fences an agent wraps tool output in.
///
/// Agents hand `execute` output back as ```` ```console … ``` ````; the card
/// already paints a terminal, so the fence is noise (`docs/etapas/etapa-2.md`
/// § correcciones). Only a fence that wraps the **whole** block is removed, and
/// nested fences are peeled one at a time.
#[must_use]
pub fn strip_code_fences(text: &str) -> String {
    let mut current = text.trim_matches('\n').to_string();
    loop {
        let trimmed = current.trim_matches('\n');
        let mut lines = trimmed.lines();
        let Some(first) = lines.next() else {
            return trimmed.to_string();
        };
        if !is_fence(first, true) {
            return trimmed.to_string();
        }
        let rest: Vec<&str> = lines.collect();
        let Some(last) = rest.last() else {
            return trimmed.to_string();
        };
        if !is_fence(last, false) {
            return trimmed.to_string();
        }
        current = rest[..rest.len() - 1].join("\n");
    }
}

/// Whether `line` is a ``` fence; an opening one may carry a language.
fn is_fence(line: &str, opening: bool) -> bool {
    let line = line.trim();
    let Some(rest) = line.strip_prefix("```") else {
        return false;
    };
    if opening {
        rest.chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '+' | '.'))
    } else {
        rest.is_empty()
    }
}

/// The exit code a failed command left in its output, when it says so.
///
/// Agents word it differently (`exit code: 1`, `exit status 127`, `salió con
/// código 2`), so a small list of markers is scanned instead of one format.
#[must_use]
pub fn exit_code_in(output: &str) -> Option<i32> {
    const MARKERS: [&str; 6] = [
        "exit code",
        "exit status",
        "exited with code",
        "exited with status",
        "código de salida",
        "codigo de salida",
    ];
    let lower = output.to_lowercase();
    for marker in MARKERS {
        let mut from = 0usize;
        while let Some(position) = lower[from..].find(marker) {
            let after = from + position + marker.len();
            let rest = lower[after..].trim_start_matches([' ', ':', '=', '\t']);
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty()
                && let Ok(code) = digits.parse::<i32>()
            {
                return Some(code);
            }
            from = after;
        }
    }
    None
}

impl Default for TranscriptDump {
    fn default() -> Self {
        Self {
            version: TRANSCRIPT_VERSION,
            agent_id: None,
            session_id: None,
            autonomy: AutonomyMode::default(),
            entries: Vec::new(),
        }
    }
}

/// The glyph a tool kind is drawn with while the Lucide icons are not loaded.
#[must_use]
pub fn tool_kind_label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "leer",
        ToolKind::Edit => "editar",
        ToolKind::Delete => "borrar",
        ToolKind::Move => "mover",
        ToolKind::Search => "buscar",
        ToolKind::Execute => "ejecutar",
        ToolKind::Think => "pensar",
        ToolKind::Fetch => "traer",
        ToolKind::SwitchMode => "modo",
        _ => "otro",
    }
}

/// The name of the Lucide icon a tool kind uses (`02-visual.md` §7).
#[must_use]
pub fn tool_kind_icon(kind: ToolKind) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName;
    match kind {
        ToolKind::Read => IconName::FileText,
        ToolKind::Edit => IconName::FilePen,
        ToolKind::Delete => IconName::Trash,
        ToolKind::Move => IconName::FolderSymlink,
        ToolKind::Search => IconName::Search,
        ToolKind::Execute => IconName::SquareTerminal,
        ToolKind::Think => IconName::Brain,
        ToolKind::Fetch => IconName::Globe,
        ToolKind::SwitchMode => IconName::Shuffle,
        _ => IconName::Wrench,
    }
}

/// The short label of a tool status, used by the tests and the accessibility
/// name of the row.
#[must_use]
pub fn tool_status_label(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "en cola",
        ToolCallStatus::InProgress => "ejecutando",
        ToolCallStatus::Completed => "listo",
        ToolCallStatus::Failed => "falló",
        _ => "desconocido",
    }
}

/// A file chip's label: the file name, with its parent for context.
#[must_use]
pub fn chip_label(path: &std::path::Path) -> SharedString {
    SharedString::from(format!("@{}", file_name_label(path)))
}

/// The file name with its extension, which is all a mention ever shows.
///
/// An absolute path is unreadable in a 380 px panel: the name identifies the
/// file and the directory is secondary information ([`parent_label`]) or a
/// tooltip ([`relative_label`]).
#[must_use]
pub fn file_name_label(path: &std::path::Path) -> SharedString {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => SharedString::from(name.to_string()),
        None => SharedString::from(path.display().to_string()),
    }
}

/// The path as the user reads it: relative to the project root when it is
/// inside it, absolute when it is not.
#[must_use]
pub fn relative_label(root: Option<&std::path::Path>, path: &std::path::Path) -> SharedString {
    let relative = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    SharedString::from(relative.display().to_string())
}

/// The directory of [`relative_label`], shown as muted secondary text.
///
/// Empty for a file that sits at the root of the project, so the row shows the
/// name alone instead of a stray separator.
#[must_use]
pub fn parent_label(root: Option<&std::path::Path>, path: &std::path::Path) -> SharedString {
    let relative = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    match relative.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            SharedString::from(parent.display().to_string())
        }
        _ => SharedString::default(),
    }
}
