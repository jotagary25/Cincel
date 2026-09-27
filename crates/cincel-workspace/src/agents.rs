//! Agent lifecycle: from a saved connection to a running
//! [`AgentConnection`] and back to the chat and the project
//! (`docs/specs/06-etapa4-conexiones-y-cincel.md` §1, §4, §6).
//!
//! [`Agents`] is the glue `docs/specs/03-arquitectura.md` §3 asks for: it
//! owns the connections engine ([`cincel_connections::Connections`]), the ACP
//! worker thread's handle of the **active** connection, drains its
//! `AgentEvent`s on the GPUI main thread and answers the project-side ones
//! (`fs/read_text_file`, `fs/write_text_file`, permission requests under the
//! fixed [`PermissionPolicy`](cincel_acp::PermissionPolicy)) itself;
//! everything else is forwarded to [`cincel_chat::ChatPanel::handle_event`].
//! It also drains [`cincel_chat::ChatEvent`]s the other way: `Command`
//! becomes an [`AgentCommand`] on the worker's channel, the connection
//! gestures of the header open [`ConnectionsModal`] or switch the active
//! connection, `RequestFileList` is answered from the worktree,
//! `OpenTerminalWithCommand`/`CopyToClipboard` reach the desktop.
//!
//! # Connections
//!
//! Nothing connects by itself (§1.2): on startup the index is read to fill
//! the "Conectar" popover and nothing is spawned; `settings.connections.
//! default_label` only preselects a row. Choosing a row
//! ([`Agents::activate_connection`]) stops the previous connection's process
//! (nothing is deleted), launches the agent with that connection's profile
//! (`Connections::agent_launch` → `AgentConnection::start_with_env`) and
//! opens a new, empty conversation bound to it. A connection whose agent says
//! it needs authentication (`auth_required`, a failed `authenticate`, an
//! `_auth/status_update` of kind `none`) or whose profile lost its
//! credentials is "Sesión vencida": amber badge, banner "La sesión de «X»
//! venció" and "Volver a conectar" (F4). One whose runtime or adapter is
//! missing is "No disponible" with "Reparar" (F5).
//!
//! # Conversations
//!
//! One connection, one conversation. This module saves whatever was on
//! screen into `~/.local/state/cincel/workspaces/<hash>/conversations/<id>.json`
//! ([`crate::conversations::ConversationStore`]) with its `connection_id`,
//! and drives the ACP side of the move: a new conversation is a
//! `session/new`, an opened one is a `session/load` (or `session/resume`,
//! or a read-only notice when the agent announces neither). Conversations
//! saved before connections existed have no `connection_id`: they are
//! listed under "(conexión anterior)" and open read-only.
//!
//! # Borrowed events
//!
//! GPUI delivers `cx.subscribe`/`cx.observe` events by reference
//! (`&ChatEvent`), but `ChatEvent::Command` wraps a non-`Clone`
//! [`AgentCommand`] and `ChatEvent::RequestFileList` a `Box<dyn FnOnce>` that
//! cannot be called through a shared reference. [`Agents::forward_command`]
//! works around the first by rebuilding the command field by field (every
//! field type involved is `Clone`); [`Agents::answer_file_list`] works around
//! the second by ignoring the closure entirely and calling
//! [`cincel_chat::ChatPanel::set_file_candidates`] directly, since this
//! module already holds a strong handle to the panel.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cincel_acp::acp::schema::v1::{
    AgentCapabilities, AuthMethod, PermissionOption, SessionId, SessionUpdate, ToolCallContent,
    ToolCallStatus, ToolCallUpdate, ToolKind,
};
use cincel_acp::{
    AgentCommand, AgentConnection, AgentEvent, AuthStatusKind, AutoAnswer, McpServerSpec,
    PermissionOutcome, PermissionPolicy, PermissionRequestId, pick_allow_option,
};
use cincel_chat::{
    AgentStatus, ChatConnection, ChatEvent, ChatPanel, ConnectionBadge, ConnectionBanner,
    Conversation, LEGACY_CONNECTION_GROUP, LEGACY_HISTORY_NOTICE, READ_ONLY_HISTORY_NOTICE,
    conversation_title,
};
use cincel_connections::{
    CincelPaths, Connection, ConnectionStatus, Connections, Identity, NodeVersion, Runtime,
};
use cincel_project::{EntryKind, OpenError};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task, Window};
use uuid::Uuid;

use crate::connection_modal::{ConnectionsModal, ModalEvent};
use crate::conversations::{ConversationStore, format_when, new_conversation_id, now_seconds};
use crate::project::Project;
use crate::review::Review;

/// How often the open conversation is written to disk when it changed
/// (`docs/etapas/etapa-2.md`, "Transcript persistence").
const TRANSCRIPT_AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// What this module has to remember about the conversation on screen; its
/// entries live in the panel and are only pulled out when it is saved.
#[derive(Clone, Debug)]
struct ActiveConversation {
    id: String,
    agent_id: String,
    /// The connection's label when it was created (the history's group
    /// header if the connection is renamed or deleted later).
    agent_name: String,
    /// The connection it belongs to; `None` only for a read-only old one.
    connection_id: Option<Uuid>,
    created_at: u64,
    /// An old conversation (no connection, or a connection that no longer
    /// exists): shown, never written back.
    read_only: bool,
}

/// The connection whose agent runs (or should run) in the chat.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveConnection {
    id: Uuid,
    agent_id: String,
    label: String,
}

/// Agent lifecycle and ACP glue, one per workspace window.
pub struct Agents {
    chat: Entity<ChatPanel>,
    /// The review of agent edits: every `fs/*` request, edit tool call and
    /// turn boundary goes through it (`crate::review`).
    review: Entity<Review>,
    project: Option<Entity<Project>>,

    /// The connections engine: index, profiles, private runtime, adapters.
    connections: Arc<Connections>,
    /// "Conectar nuevo agente", "Eliminar conexión", "Renombrar", "Volver a
    /// conectar", "Reparar".
    modal: Entity<ConnectionsModal>,
    /// `settings.connections.default_label`: preselects a row, never
    /// connects.
    default_label: Option<String>,
    mcp_servers: Vec<McpServerSpec>,

    connection: Option<AgentConnection>,
    /// The connection chosen by the user; `None` is "Sin conexión".
    active: Option<ActiveConnection>,
    /// Connections whose agent said they are logged out even though the
    /// profile still has a credentials file (a revoked token): shown as
    /// "Sesión vencida" until a login succeeds.
    expired: HashSet<Uuid>,
    /// What the running agent announced in `initialize`; decides between
    /// `session/load`, `session/resume` and a read-only notice.
    capabilities: Option<Box<AgentCapabilities>>,
    /// Conversations of the open project.
    store: Option<ConversationStore>,
    /// The conversation on screen.
    conversation: Option<ActiveConversation>,
    /// The ACP session a freshly opened conversation wants back, consumed by
    /// the next `Connected`.
    pending_load: Option<SessionId>,
    sandbox_root: Option<PathBuf>,
    /// Kept alive only while there is no project open; dropping it deletes
    /// the sandbox directory (`docs/etapas/etapa-2.md`, "Empty/edge states").
    temp_dir: Option<tempfile::TempDir>,
    tool_call_kinds: HashMap<cincel_acp::acp::schema::v1::ToolCallId, ToolKind>,
    /// Every path each tool call named so far: a completion update usually
    /// carries only its status.
    tool_call_paths: HashMap<cincel_acp::acp::schema::v1::ToolCallId, Vec<PathBuf>>,

    /// Which permission requests are answered without asking.
    policy: PermissionPolicy,
    /// A `session/prompt` went out and its `TurnEnded` has not come back.
    prompt_in_flight: bool,
    /// While that prompt runs, another request that can fail on its own
    /// (mode, config option, authentication, session) went out too, so an
    /// `AgentEvent::Error` is no longer known to be the prompt's.
    side_request_in_turn: bool,

    last_saved_conversation: Option<String>,

    /// `false` in tests (mirrors `ProjectOptions::inert`): a real
    /// `AgentConnection` still spawns a real process and a real OS thread,
    /// but the automatic event-draining task is a `cx.spawn` foreground task
    /// woken by that foreign thread, which GPUI's deterministic test executor
    /// does not tolerate (`docs/etapas/etapa-1.md`, the same reason
    /// `Watcher`/`GitStatusWatcher` are disabled under `ProjectOptions::inert`).
    /// Tests drive events by calling [`Agents::handle_agent_event`] directly
    /// after a blocking `recv` on the connection's own channel, off any GPUI
    /// task.
    background: bool,

    event_task: Option<Task<()>>,
    transcript_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,

    /// Last lines of the active connection's stderr (bounded), so an
    /// `AuthRequired` with no message of its own (`cincel_acp::AgentEvent`
    /// carries none) can still be logged with some diagnostic (spec 06,
    /// 2026-09-26 Gemini "sesión vencida" investigation: gemini-cli writes
    /// the real reason — e.g. a Code Assist eligibility rejection — to its
    /// own stderr via `debugLogger.error`, never into the ACP error). Reset
    /// on every [`Agents::spawn_active`] so it never mixes agents.
    recent_stderr: std::collections::VecDeque<String>,
}

/// How many stderr lines [`Agents::recent_stderr`] keeps.
const RECENT_STDERR_LINES: usize = 20;

/// The connections engine over the XDG directories, with the Node version
/// `settings.connections.runtime.node_version` asks for.
fn default_connections(settings: &cincel_settings::Settings) -> Connections {
    let paths = CincelPaths::from_xdg();
    Connections::new(paths.clone()).with_runtime(Runtime::new(paths).with_version(
        NodeVersion::parse(&settings.connections.runtime.node_version),
    ))
}

impl Agents {
    /// Builds the controller. `background` gates every piece of real
    /// concurrency (event draining, the 30 s transcript timer, the modal's
    /// threads) so tests can drive everything by hand.
    ///
    /// Nothing is spawned here (`docs/specs/06-etapa4-conexiones-y-cincel.md`
    /// §1): the index of connections is read to fill the popover, and that
    /// is all.
    ///
    /// The subscription to the chat panel lives here rather than in
    /// [`crate::Workspace`] so that a window built by hand (the tests) gets
    /// the same wiring as the real one.
    pub fn new(
        chat: Entity<ChatPanel>,
        review: Entity<Review>,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = crate::settings::settings(cx);
        let policy = build_policy(&settings);
        let connections = Arc::new(default_connections(&settings));
        let modal = cx.new(|cx| ConnectionsModal::new(connections.clone(), background, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&chat, window, |this, _chat, event, window, cx| {
                this.handle_chat_event(event, window, cx);
            }),
            cx.subscribe_in(&modal, window, |this, _modal, event, window, cx| {
                this.handle_modal_event(event, window, cx);
            }),
        ];

        let mut agents = Self {
            chat,
            review,
            project: None,
            connections,
            modal,
            default_label: settings.connections.default_label.clone(),
            mcp_servers: map_mcp_servers(&settings.connections.mcp_servers),
            connection: None,
            active: None,
            expired: HashSet::new(),
            capabilities: None,
            store: None,
            conversation: None,
            pending_load: None,
            sandbox_root: None,
            temp_dir: None,
            tool_call_kinds: HashMap::new(),
            tool_call_paths: HashMap::new(),
            policy,
            prompt_in_flight: false,
            side_request_in_turn: false,
            last_saved_conversation: None,
            background,
            event_task: None,
            transcript_task: None,
            _subscriptions: subscriptions,
            recent_stderr: std::collections::VecDeque::new(),
        };
        agents.refresh_connections(cx);
        tracing::info!(
            conexiones = agents.chat.read(cx).connections().len(),
            "conexiones leídas; no se lanza ningún agente hasta que el usuario elija una"
        );
        agents
    }

    // ------------------------------------------------------------ getters

    /// The connection modals, which the workspace paints while they are open.
    pub fn modal(&self) -> &Entity<ConnectionsModal> {
        &self.modal
    }

    /// The connections engine.
    pub fn connections(&self) -> &Arc<Connections> {
        &self.connections
    }

    /// The id of the active connection, if any.
    pub fn active_connection_id(&self) -> Option<Uuid> {
        self.active.as_ref().map(|active| active.id)
    }

    /// Whether an agent process is running.
    pub fn is_agent_running(&self) -> bool {
        self.connection.is_some()
    }

    /// Replaces the connections engine (the tests point it at a temp
    /// directory with a fake runtime) and refreshes the popover.
    pub fn set_connections(&mut self, connections: Arc<Connections>, cx: &mut Context<Self>) {
        self.connections = connections.clone();
        self.modal
            .update(cx, |modal, _cx| modal.set_connections(connections));
        self.refresh_connections(cx);
    }

    // ------------------------------------------------------------ project

    /// Tracks the open project: the sandbox root for future connections, and
    /// where the transcript is stored. Also the single entry point for
    /// *switching* projects (Ctrl+O, recientes o el path de la CLI, todos a
    /// través de `Workspace::open_project`) — `docs/etapas/etapa-2.md`,
    /// "Ciclo de vida del agente": a turn in progress is cancelled first,
    /// the outgoing project's conversation is saved, the running process is
    /// torn down, the incoming project's conversation list is read (migrating
    /// its old `chat.json` if it still has one) and, if the user had chosen a
    /// connection, it is relaunched with the new `project_root` in a fresh
    /// conversation. With no connection chosen (startup) nothing is spawned.
    pub fn set_project(
        &mut self,
        project: Option<Entity<Project>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_turn_in_progress(cx);
        self.save_conversation_now(cx);
        self.stop_agent(cx);

        self.project = project;
        // The review follows the project: it saves the one being left and
        // loads the one persisted for the new root.
        let for_review = self.project.clone();
        self.review
            .update(cx, |review, cx| review.set_project(for_review, cx));
        self.transcript_task = None;
        self.sandbox_root = None;
        self.conversation = None;
        self.pending_load = None;
        self.capabilities = None;
        self.last_saved_conversation = None;
        if self.project.is_some() {
            self.temp_dir = None;
        }
        // The composer relativizes mention labels against the project root.
        let root = self
            .project
            .as_ref()
            .map(|project| project.read(cx).root().to_path_buf());
        self.chat
            .update(cx, |chat, cx| chat.set_project_root(root.clone(), cx));

        // The previous project's conversation must not leak into this one.
        self.chat
            .update(cx, |chat, cx| chat.start_new_conversation(window, cx));
        self.store = root.as_deref().and_then(ConversationStore::for_project);
        if let Some(store) = &self.store {
            store.migrate_legacy();
        }
        self.refresh_conversations(cx);

        if self.project.is_some() && self.background {
            self.start_transcript_autosave(cx);
        }

        if self.active.is_some() {
            self.begin_conversation(window, cx);
            self.launch_active(cx);
        }
    }

    /// Edge case of `docs/etapas/etapa-2.md`, "Ciclo de vida del agente":
    /// switching projects while a turn is running asks the agent to stop
    /// first (`AgentCommand::Cancel`) instead of yanking the process out
    /// from under it, and tells the user why their conversation is about to
    /// disappear. Best-effort: the connection is torn down right after this
    /// returns regardless of whether the agent acknowledges the cancel.
    fn cancel_turn_in_progress(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_none() {
            return;
        }
        let in_progress = matches!(
            self.chat.read(cx).status(),
            AgentStatus::Thinking | AgentStatus::WaitingPermission
        );
        if !in_progress {
            return;
        }
        if let Some(session_id) = self.chat.read(cx).session_id().cloned() {
            self.forward_command(&AgentCommand::Cancel { session_id }, cx);
        }
        crate::toast::warn("Se cerró la sesión del agente al cambiar de proyecto.", cx);
    }

    /// Tears the running process down: dropping [`AgentConnection`] sends
    /// `AgentCommand::Shutdown` and blocks for the worker thread, which
    /// kills the whole process group before returning
    /// (`cincel_acp::connection::run_agent`). Reflected in the chat's pill
    /// as "desconectado"; a deliberate stop is not a crash, so no notice is
    /// added to the conversation. Nothing on disk is touched.
    fn stop_agent(&mut self, cx: &mut Context<Self>) {
        self.event_task = None;
        self.capabilities = None;
        if self.connection.take().is_none() {
            return;
        }
        tracing::info!("se detuvo el proceso del agente de la conexión anterior");
        self.end_orphaned_turn(cx);
        self.chat.update(cx, |chat, cx| {
            chat.set_status(AgentStatus::Disconnected, cx)
        });
    }

    /// The sandbox root a new connection should use: the project root, or a
    /// fresh temp directory with a one-time notice
    /// (`docs/etapas/etapa-2.md`, "Empty/edge states").
    fn resolve_sandbox_root(&mut self, cx: &mut Context<Self>) -> PathBuf {
        if let Some(project) = &self.project {
            return project.read(cx).root().to_path_buf();
        }
        if let Some(dir) = &self.temp_dir {
            return dir.path().to_path_buf();
        }
        match tempfile::Builder::new().prefix("cincel-agent-").tempdir() {
            Ok(dir) => {
                let path = dir.path().to_path_buf();
                self.temp_dir = Some(dir);
                crate::toast::info("Abrí una carpeta para que el agente vea tus archivos.", cx);
                path
            }
            Err(error) => {
                tracing::warn!(%error, "no se pudo crear un directorio temporal para el agente");
                std::env::temp_dir()
            }
        }
    }

    // --------------------------------------------------------- connections

    /// Rebuilds the "Conectar" popover from the index: label, identity,
    /// "Usado hace…" and the status badge of every connection
    /// (`Connections::list_with_status`, never the network), plus the active
    /// one and the `default_label` preselection. Also refreshes the history,
    /// whose group headers are the connections' labels.
    pub fn refresh_connections(&mut self, cx: &mut Context<Self>) {
        let listed = self.connections.list_with_status().unwrap_or_else(|error| {
            tracing::warn!(%error, "no se pudo leer el índice de conexiones");
            Vec::new()
        });
        let rows: Vec<ChatConnection> = listed
            .iter()
            .map(|(connection, status)| ChatConnection {
                id: connection.id.to_string(),
                agent_id: connection.agent_id.clone(),
                label: connection.label.clone(),
                identity: connection.identity.as_ref().and_then(Identity::summary),
                last_used: used_label(connection.last_used_at),
                badge: self.badge(connection, status),
            })
            .collect();
        if let Some(active) = &mut self.active
            && let Some((connection, _)) = listed
                .iter()
                .find(|(connection, _)| connection.id == active.id)
        {
            active.label = connection.label.clone();
        }
        let preselected = self.default_label.as_ref().and_then(|label| {
            listed
                .iter()
                .find(|(connection, _)| &connection.label == label)
                .map(|(connection, _)| connection.id.to_string())
        });
        let active = self.active.as_ref().map(|active| active.id.to_string());
        self.chat.update(cx, |chat, cx| {
            chat.set_connections(rows, cx);
            chat.set_active_connection(active, cx);
            chat.set_preselected_connection(preselected, cx);
        });
        self.refresh_conversations(cx);
    }

    /// The badge of one connection: what the profile says, overridden by
    /// what its agent said.
    fn badge(&self, connection: &Connection, status: &ConnectionStatus) -> ConnectionBadge {
        match status {
            ConnectionStatus::Connected if self.expired.contains(&connection.id) => {
                ConnectionBadge::SessionExpired
            }
            ConnectionStatus::Connected => ConnectionBadge::Connected,
            ConnectionStatus::SessionExpired => ConnectionBadge::SessionExpired,
            ConnectionStatus::Unavailable { reason } => ConnectionBadge::Unavailable {
                reason: reason.clone(),
            },
        }
    }

    /// F1: the user picked a connection. Stops the previous connection's
    /// process (nothing is deleted), launches this one with its profile and
    /// opens a new, empty conversation bound to it. An expired or unavailable
    /// connection shows its banner instead of launching.
    pub fn activate_connection(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(connection) = self.connections.store().get(id) else {
            crate::toast::warn("Esa conexión ya no existe.", cx);
            self.refresh_connections(cx);
            return;
        };
        self.save_conversation_now(cx);
        self.stop_agent(cx);
        // Choosing it again is a fresh attempt: the agent decides whether
        // the session is still good.
        self.expired.remove(&id);
        self.active = Some(ActiveConnection {
            id,
            agent_id: connection.agent_id.clone(),
            label: connection.label.clone(),
        });
        if let Err(error) = self.connections.store().touch(id) {
            tracing::warn!(%error, "no se pudo marcar el uso de la conexión");
        }
        self.begin_conversation(window, cx);
        self.launch_active(cx);
        self.refresh_connections(cx);
    }

    /// Launches the active connection's agent if it can run; otherwise shows
    /// why not (the F4/F5 banner).
    fn launch_active(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        let Ok(connection) = self.connections.store().get(active.id) else {
            self.deactivate(cx);
            return;
        };
        match self.connections.status(&connection) {
            ConnectionStatus::Connected if !self.expired.contains(&active.id) => {
                self.chat.update(cx, |chat, cx| chat.set_banner(None, cx));
                self.spawn_active(&connection, cx);
            }
            ConnectionStatus::Connected | ConnectionStatus::SessionExpired => {
                self.show_expired(cx);
            }
            ConnectionStatus::Unavailable { reason } => self.show_unavailable(reason, cx),
        }
    }

    /// Spawns the agent of `connection` with its profile environment and
    /// replaces whatever process was running (dropping it shuts it down).
    fn spawn_active(&mut self, connection: &Connection, cx: &mut Context<Self>) {
        let launch = match self.connections.agent_launch(connection) {
            Ok(launch) => launch,
            Err(error) => {
                self.show_unavailable(error.to_string(), cx);
                return;
            }
        };
        let root = self.resolve_sandbox_root(cx);
        self.sandbox_root = Some(root.clone());
        // The previous process (if any) dies with its connection below, and
        // its `TurnEnded` with it.
        if self.connection.is_some() {
            self.end_orphaned_turn(cx);
        }
        // A new process means a new `initialize`; what the previous one
        // announced says nothing about this one.
        self.capabilities = None;
        self.tool_call_kinds.clear();
        self.tool_call_paths.clear();
        // A new agent's stderr must never be blamed on the previous one.
        self.recent_stderr.clear();
        // Dropping the previous connection tears its process down.
        self.event_task = None;
        self.connection = None;
        tracing::info!(
            agente = %connection.agent_id,
            "se lanza el agente de la conexión elegida por el usuario"
        );
        let agent = AgentConnection::start_with_env(root.clone(), launch.env);
        let _ = agent.commands().send_blocking(AgentCommand::Spawn {
            launch: launch.launch,
            cwd: root,
        });

        if self.background {
            let events = agent.events().clone();
            self.event_task = Some(cx.spawn(async move |this, cx| {
                while let Ok(event) = events.recv().await {
                    let alive = this
                        .update(cx, |agents, cx| agents.handle_agent_event(event, cx))
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            }));
        }
        self.connection = Some(agent);
    }

    /// Relaunches the active connection's agent (the "Reiniciar" of a crash
    /// toast, a finished "Volver a conectar" or "Reparar"), keeping the
    /// conversation on screen: its session is reopened if the agent can.
    fn restart_active(&mut self, cx: &mut Context<Self>) {
        if self.active.is_none() {
            return;
        }
        self.stop_agent(cx);
        self.pending_load = self.chat.read(cx).session_id().cloned();
        self.launch_active(cx);
    }

    /// "Sin conexión": stops the process and forgets which connection was
    /// active (nothing on disk is touched).
    fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.stop_agent(cx);
        self.active = None;
        self.chat.update(cx, |chat, cx| {
            chat.set_banner(None, cx);
            chat.set_active_connection(None, cx);
            chat.set_status(AgentStatus::Disconnected, cx);
        });
    }

    /// F4: "La sesión de «X» venció" for the active connection.
    fn show_expired(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        self.expired.insert(active.id);
        self.chat.update(cx, |chat, cx| {
            chat.set_banner(
                Some(ConnectionBanner::Expired {
                    id: active.id.to_string(),
                    label: active.label.clone(),
                }),
                cx,
            );
            chat.set_status(AgentStatus::AuthRequired, cx);
        });
        self.refresh_connections(cx);
    }

    /// F5: "«X» no está disponible" for the active connection.
    fn show_unavailable(&mut self, reason: String, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        self.chat.update(cx, |chat, cx| {
            chat.set_banner(
                Some(ConnectionBanner::Unavailable {
                    id: active.id.to_string(),
                    label: active.label.clone(),
                    reason,
                }),
                cx,
            );
            chat.set_status(AgentStatus::Disconnected, cx);
        });
    }

    /// What the modal reports back.
    fn handle_modal_event(
        &mut self,
        event: &ModalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ModalEvent::Saved { id } => {
                self.refresh_connections(cx);
                self.activate_connection(*id, window, cx);
            }
            ModalEvent::Relogged { id } => {
                self.expired.remove(id);
                if self.active_connection_id() == Some(*id) {
                    self.chat.update(cx, |chat, cx| chat.set_banner(None, cx));
                    self.restart_active(cx);
                }
                self.refresh_connections(cx);
            }
            ModalEvent::Repaired { id } => {
                if self.active_connection_id() == Some(*id) {
                    self.restart_active(cx);
                }
                self.refresh_connections(cx);
            }
            ModalEvent::WillDelete { id } => {
                // Its agent must not be running while the logout runs on the
                // same profile.
                if self.active_connection_id() == Some(*id) {
                    self.deactivate(cx);
                }
                self.modal
                    .update(cx, |modal, cx| modal.run_delete(window, cx));
            }
            ModalEvent::Deleted { id, report, .. } => {
                self.expired.remove(id);
                let forgotten = report.as_ref().is_some_and(|report| report.index_removed);
                if forgotten {
                    let key = id.to_string();
                    let removed = crate::conversations::delete_for_connection_everywhere(&key);
                    tracing::info!(
                        conversaciones = removed,
                        "se borraron las conversaciones de la conexión eliminada"
                    );
                    let on_screen = self
                        .conversation
                        .as_ref()
                        .is_some_and(|conversation| conversation.connection_id == Some(*id));
                    if on_screen {
                        self.conversation = None;
                        self.last_saved_conversation = None;
                        self.chat
                            .update(cx, |chat, cx| chat.start_new_conversation(window, cx));
                    }
                }
                self.refresh_connections(cx);
            }
            ModalEvent::Renamed { .. } => self.refresh_connections(cx),
            ModalEvent::Closed => {
                self.chat
                    .update(cx, |chat, cx| chat.focus_input(window, cx));
            }
        }
    }

    /// The agent reported its identity (`_auth/status_update`): persisted in
    /// the index (`ConnectionStore::set_identity`) so the popover and the
    /// header show it.
    fn update_identity(&mut self, identity: Identity, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        let current = self
            .connections
            .store()
            .get(active.id)
            .ok()
            .and_then(|connection| connection.identity);
        if current.as_ref() == Some(&identity) {
            return;
        }
        if let Err(error) = self
            .connections
            .store()
            .set_identity(active.id, Some(identity))
        {
            tracing::warn!(%error, "no se pudo guardar la identidad de la conexión");
        }
        self.refresh_connections(cx);
    }

    // ------------------------------------------------------- conversations

    /// Saves whatever is on screen and opens an empty conversation bound to
    /// the active connection, reusing its process when it is running.
    fn begin_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_conversation_now(cx);
        self.chat
            .update(cx, |chat, cx| chat.start_new_conversation(window, cx));
        self.conversation = self.active.as_ref().map(|active| ActiveConversation {
            id: new_conversation_id(),
            agent_id: active.agent_id.clone(),
            agent_name: active.label.clone(),
            connection_id: Some(active.id),
            created_at: now_seconds(),
            read_only: false,
        });
        self.last_saved_conversation = None;
        self.pending_load = None;
        if self.connection.is_some() && self.capabilities.is_some() {
            // The process is fine and already initialized; only the session
            // has to start over.
            self.start_session();
        } else if self.connection.is_none()
            && self
                .active
                .as_ref()
                .is_some_and(|active| self.expired.contains(&active.id))
        {
            self.chat.update(cx, |chat, cx| {
                chat.set_status(AgentStatus::AuthRequired, cx)
            });
        }
        // Otherwise `initialize` is still in flight (or about to be) and its
        // `Connected` will open the session
        // ([`Agents::resume_or_start_session`]).
        self.refresh_conversations(cx);
    }

    /// Loads a stored conversation and tries to reopen its ACP session with
    /// its own connection (switching to it if another one is active). One
    /// without a connection, or whose connection is gone, opens read-only.
    fn open_conversation(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.save_conversation_now(cx);
        let Some(conversation) = self.store.as_ref().and_then(|store| store.load(id)) else {
            crate::toast::warn("No se pudo abrir esa conversación.", cx);
            self.refresh_conversations(cx);
            return;
        };
        let owner = conversation
            .connection_id
            .as_deref()
            .and_then(|raw| Uuid::parse_str(raw).ok())
            .and_then(|uuid| self.connections.store().get(uuid).ok());
        let reusable = owner.as_ref().is_some_and(|owner| {
            self.active_connection_id() == Some(owner.id) && self.connection.is_some()
        });
        if owner.is_some() && !reusable {
            // Another connection's process goes before the history is on
            // screen, so nothing it says lands in it.
            self.stop_agent(cx);
        }
        self.conversation = Some(ActiveConversation {
            id: conversation.id.clone(),
            agent_id: conversation.agent_id.clone(),
            agent_name: conversation.agent_name.clone(),
            connection_id: owner.as_ref().map(|connection| connection.id),
            created_at: conversation.created_at,
            read_only: owner.is_none(),
        });
        self.pending_load = conversation
            .session_id
            .clone()
            .filter(|id| !id.is_empty() && owner.is_some())
            .map(SessionId::new);
        self.last_saved_conversation = None;
        self.chat.update(cx, |chat, cx| {
            chat.load_conversation(conversation, window, cx)
        });

        let Some(owner) = owner else {
            self.chat.update(cx, |chat, cx| {
                chat.set_history_notice(Some(LEGACY_HISTORY_NOTICE.to_string()), cx);
            });
            self.refresh_conversations(cx);
            return;
        };
        if reusable {
            if self.capabilities.is_some() {
                // Already initialized: what the agent announced is known, so
                // the decision can be taken right away.
                self.resume_or_start_session(cx);
            }
        } else {
            self.active = Some(ActiveConnection {
                id: owner.id,
                agent_id: owner.agent_id.clone(),
                label: owner.label.clone(),
            });
            if let Err(error) = self.connections.store().touch(owner.id) {
                tracing::warn!(%error, "no se pudo marcar el uso de la conexión");
            }
            self.launch_active(cx);
        }
        self.refresh_connections(cx);
    }

    /// Removes a conversation, offering 5 s of undo
    /// (`docs/etapas/etapa-2.md` § correcciones).
    fn delete_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Some(removed) = store.delete(id) else {
            self.refresh_conversations(cx);
            return;
        };
        // Deleting the conversation on screen detaches it: the autosave must
        // not write the file back a second later.
        if self
            .conversation
            .as_ref()
            .is_some_and(|active| active.id == id)
        {
            self.conversation = None;
            self.last_saved_conversation = None;
        }
        self.refresh_conversations(cx);

        let weak = cx.entity().downgrade();
        let title = removed.title.clone();
        crate::toast::undo(
            format!("Conversación eliminada: {title}"),
            move |cx: &mut App| {
                if let Err(error) = store.save(&removed) {
                    tracing::warn!(%error, "no se pudo restaurar la conversación");
                }
                if let Some(agents) = weak.upgrade() {
                    agents.update(cx, |agents, cx| agents.refresh_conversations(cx));
                }
            },
            cx,
        );
    }

    /// The conversation on screen as it goes to disk, or `None` when there is
    /// nothing worth saving (no project, a read-only old conversation, or an
    /// empty conversation nobody has written in yet).
    fn current_conversation(&self, cx: &Context<Self>) -> Option<Conversation> {
        let active = self.conversation.as_ref()?;
        if active.read_only {
            return None;
        }
        let project = self.project.as_ref()?;
        let dump = self.chat.read(cx).export_transcript();
        if dump.entries.is_empty() {
            return None;
        }
        Some(Conversation {
            version: cincel_chat::CONVERSATION_VERSION,
            id: active.id.clone(),
            agent_id: active.agent_id.clone(),
            agent_name: active.agent_name.clone(),
            connection_id: active.connection_id.map(|id| id.to_string()),
            session_id: dump.session_id.clone(),
            cwd: project.read(cx).root().to_path_buf(),
            created_at: active.created_at,
            updated_at: now_seconds(),
            title: conversation_title(&dump.entries),
            entries: dump.entries,
        })
    }

    /// The event channel of the running connection, for tests that drain it
    /// by hand (`background == false`).
    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn connection_events(&self) -> Option<async_channel::Receiver<AgentEvent>> {
        self.connection
            .as_ref()
            .map(|connection| connection.events().clone())
    }

    /// The id of the conversation on screen and its connection (tests).
    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn conversation_binding(&self) -> Option<(String, Option<Uuid>)> {
        self.conversation
            .as_ref()
            .map(|conversation| (conversation.id.clone(), conversation.connection_id))
    }

    // ---------------------------------------------------------- chat -> acp

    /// Everything [`cincel_chat::ChatPanel`] emits, dispatched to the ACP
    /// worker, the connection modals, the project or the desktop. Called from
    /// a `cx.subscribe_in` on the chat panel (needs `Window` for
    /// `OpenFileAtHunk` and the modals).
    pub fn handle_chat_event(
        &mut self,
        event: &ChatEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ChatEvent::Command(command) => self.forward_command(command, cx),
            ChatEvent::OpenFileAtHunk { path } => {
                // "Ver en el editor": the first pending change of the file.
                self.review.update(cx, |review, cx| {
                    review.open_file_at_first_hunk(path, window, cx)
                });
            }
            ChatEvent::ConnectionSelected { id } => match Uuid::parse_str(id) {
                Ok(id) => self.activate_connection(id, window, cx),
                Err(error) => tracing::warn!(%error, "id de conexión inválido"),
            },
            ChatEvent::NewConnection => {
                self.modal
                    .update(cx, |modal, cx| modal.open_connect(window, cx));
            }
            ChatEvent::DeleteConnections => {
                self.modal
                    .update(cx, |modal, cx| modal.open_delete(window, cx));
            }
            ChatEvent::RenameConnection { id } => {
                if let Ok(id) = Uuid::parse_str(id) {
                    self.modal
                        .update(cx, |modal, cx| modal.open_rename(id, window, cx));
                }
            }
            ChatEvent::Reconnect { id } => {
                if let Ok(id) = Uuid::parse_str(id) {
                    self.modal
                        .update(cx, |modal, cx| modal.open_relogin(id, window, cx));
                }
            }
            ChatEvent::Repair { id } => {
                if let Ok(id) = Uuid::parse_str(id) {
                    self.modal
                        .update(cx, |modal, cx| modal.open_repair(id, window, cx));
                }
            }
            ChatEvent::NewConversation => {
                if self.active.is_some() {
                    self.begin_conversation(window, cx);
                } else {
                    crate::toast::warn("Conectá un agente para empezar una conversación.", cx);
                }
            }
            ChatEvent::OpenConversation { id } => {
                let id = id.clone();
                self.open_conversation(&id, window, cx);
            }
            ChatEvent::DeleteConversation { id } => {
                let id = id.clone();
                self.delete_conversation(&id, cx);
            }
            ChatEvent::RequestFileList { query, .. } => self.answer_file_list(query.clone(), cx),
            ChatEvent::OpenTerminalWithCommand(command) => self.open_terminal(command.clone(), cx),
            ChatEvent::CopyToClipboard(text) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
            }
            _ => tracing::debug!("ChatEvent no reconocido, se ignora"),
        }
    }

    /// Rebuilds `command` field by field (every field type involved is
    /// `Clone`) and pushes it onto the worker's command channel — see the
    /// "Borrowed events" note at the top of this module. `NewSession` /
    /// `LoadSession` / `ResumeSession` get their `cwd` overwritten with the
    /// connection's own sandbox root: `cincel-chat` has no notion of a
    /// project and its own `new_session()` fills `cwd` with
    /// `std::env::current_dir()`, which is almost never the right answer
    /// (wish: let the caller supply `cwd`, or drop the field from the
    /// command and let the glue layer fill it in, exactly as done here).
    pub(crate) fn forward_command(&mut self, command: &AgentCommand, cx: &mut Context<Self>) {
        if self.connection.is_none() {
            crate::toast::warn("No hay un agente conectado todavía.", cx);
            return;
        }
        let root = self.sandbox_root.clone().unwrap_or_else(std::env::temp_dir);
        let owned = match command {
            AgentCommand::NewSession { mcp_servers, .. } => {
                self.tool_call_kinds.clear();
                self.tool_call_paths.clear();
                AgentCommand::NewSession {
                    cwd: root,
                    mcp_servers: mcp_servers.clone(),
                }
            }
            AgentCommand::LoadSession {
                session_id,
                mcp_servers,
                ..
            } => AgentCommand::LoadSession {
                session_id: session_id.clone(),
                cwd: root,
                mcp_servers: mcp_servers.clone(),
            },
            AgentCommand::ResumeSession {
                session_id,
                mcp_servers,
                ..
            } => AgentCommand::ResumeSession {
                session_id: session_id.clone(),
                cwd: root,
                mcp_servers: mcp_servers.clone(),
            },
            AgentCommand::Prompt {
                session_id,
                blocks,
                feedback,
            } => {
                let feedback = self.prepare_prompt(feedback.clone(), cx);
                AgentCommand::Prompt {
                    session_id: session_id.clone(),
                    blocks: blocks.clone(),
                    feedback,
                }
            }
            AgentCommand::Cancel { session_id } => AgentCommand::Cancel {
                session_id: session_id.clone(),
            },
            AgentCommand::SetMode {
                session_id,
                mode_id,
            } => AgentCommand::SetMode {
                session_id: session_id.clone(),
                mode_id: mode_id.clone(),
            },
            AgentCommand::SetConfigOption {
                session_id,
                config_id,
                value,
            } => AgentCommand::SetConfigOption {
                session_id: session_id.clone(),
                config_id: config_id.clone(),
                value: value.clone(),
            },
            AgentCommand::RespondPermission { id, outcome } => AgentCommand::RespondPermission {
                id: *id,
                outcome: outcome.clone(),
            },
            AgentCommand::Authenticate { method_id } => AgentCommand::Authenticate {
                method_id: method_id.clone(),
            },
            other => {
                tracing::debug!(?other, "comando del chat sin manejar todavía");
                return;
            }
        };
        match &owned {
            AgentCommand::Prompt { .. } => {
                self.prompt_in_flight = true;
                self.side_request_in_turn = false;
            }
            AgentCommand::SetMode { .. }
            | AgentCommand::SetConfigOption { .. }
            | AgentCommand::Authenticate { .. }
            | AgentCommand::NewSession { .. }
            | AgentCommand::LoadSession { .. }
            | AgentCommand::ResumeSession { .. }
                if self.prompt_in_flight =>
            {
                self.side_request_in_turn = true;
            }
            _ => {}
        }
        if let Some(connection) = &self.connection {
            let _ = connection.commands().send_blocking(owned);
        }
    }

    /// A turn whose `TurnEnded` will never arrive: its prompt failed, or the
    /// process that ran it is being replaced. Ends the review turn (so the
    /// editors stop showing their controls disabled) and frees the chat.
    fn end_orphaned_turn(&mut self, cx: &mut Context<Self>) {
        self.prompt_in_flight = false;
        self.side_request_in_turn = false;
        self.tool_call_paths.clear();
        if self.review.read(cx).is_turn_active() {
            self.review.update(cx, |review, cx| review.agent_gone(cx));
        }
        self.chat.update(cx, |chat, cx| chat.abort_turn(cx));
    }

    /// A prompt is about to go out: starts its review turn and attaches what
    /// the user did to the previous turn's edits (`report_for_agent`, which
    /// `cincel-acp` wraps in `<user_review_feedback>`), plus whatever
    /// feedback the chat already had.
    pub(crate) fn prepare_prompt(
        &mut self,
        from_chat: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let (_turn, report) = self.review.update(cx, |review, cx| review.begin_prompt(cx));
        match (from_chat, report) {
            (Some(chat), Some(report)) => Some(format!("{report}\n{chat}")),
            (chat, report) => report.or(chat),
        }
    }

    /// Answers `ChatEvent::RequestFileList` directly with
    /// `ChatPanel::set_file_candidates` instead of the event's own `reply`
    /// closure, which cannot be called through the `&ChatEvent` a GPUI
    /// subscription hands out (see the module doc's "Borrowed events" note).
    fn answer_file_list(&mut self, query: String, cx: &mut Context<Self>) {
        let files = match &self.project {
            Some(project) => {
                let needle = query.to_lowercase();
                let root = project.read(cx).root().to_path_buf();
                project
                    .read(cx)
                    .worktree()
                    .entries()
                    .filter(|entry| entry.kind == EntryKind::File)
                    .filter(|entry| {
                        needle.is_empty()
                            || entry
                                .path
                                .to_string_lossy()
                                .to_lowercase()
                                .contains(&needle)
                    })
                    .take(50)
                    .map(|entry| root.join(&entry.path))
                    .collect()
            }
            None => Vec::new(),
        };
        self.chat
            .update(cx, |chat, cx| chat.set_file_candidates(files, cx));
    }

    /// `settings.json` changed: the policy takes the new
    /// `review.sensitive_paths`, new sessions get the new
    /// `connections.mcp_servers`, and `connections.default_label` moves the
    /// popover's preselection (it never connects anything).
    pub fn reload_settings(&mut self, cx: &mut Context<Self>) {
        let settings = crate::settings::settings(cx);
        self.policy = build_policy(&settings);
        self.mcp_servers = map_mcp_servers(&settings.connections.mcp_servers);
        if self.default_label != settings.connections.default_label {
            self.default_label = settings.connections.default_label.clone();
            self.refresh_connections(cx);
        }
    }

    /// "Abrir terminal" of the authentication card: tries a short list of
    /// terminal emulators in order and stops at the first one that starts.
    fn open_terminal(&mut self, command: String, cx: &mut Context<Self>) {
        let full = format!("{command}; exec $SHELL");
        let candidates: [(&str, &[&str]); 4] = [
            ("cosmic-term", &["--", "sh", "-c"]),
            ("x-terminal-emulator", &["-e", "sh", "-c"]),
            ("gnome-terminal", &["--", "sh", "-c"]),
            ("xterm", &["-e", "sh", "-c"]),
        ];
        for (program, prefix_args) in candidates {
            let mut command = std::process::Command::new(program);
            command.args(prefix_args).arg(&full);
            if command.spawn().is_ok() {
                return;
            }
        }
        crate::toast::error(
            "No se encontró una terminal para abrir (probé cosmic-term, x-terminal-emulator, gnome-terminal y xterm).",
            cx,
        );
    }

    // ---------------------------------------------------------- acp -> ui

    /// Consumes one [`AgentEvent`] read off the worker's channel: the
    /// project-side requests are answered here, the authentication ones
    /// update the connection (badge, banner, identity), everything else
    /// reaches [`cincel_chat::ChatPanel::handle_event`] unchanged.
    pub(crate) fn handle_agent_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::Connected {
                agent_info,
                auth_methods,
                capabilities,
            } => {
                self.capabilities = Some(capabilities.clone());
                self.resume_or_start_session(cx);
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(
                        AgentEvent::Connected {
                            agent_info,
                            auth_methods,
                            capabilities,
                        },
                        cx,
                    );
                });
            }
            AgentEvent::AuthRequired { ref methods } => {
                // F4: the connection's session is gone. The chat's old
                // "Hace falta autenticarse" card is not used any more: the
                // banner's "Volver a conectar" repeats the login on the same
                // profile.
                //
                // `AgentEvent::AuthRequired` carries no message of its own
                // (the ACP error text that triggered it is discarded when
                // `cincel-acp` maps `ErrorCode::AuthRequired` to this
                // variant), so the announced methods and whatever the agent
                // wrote to its own stderr are the only diagnostic left:
                // gemini-cli, for one, logs the real rejection reason (e.g.
                // a Code Assist eligibility error) through
                // `debugLogger.error`, never in the RPC error.
                let method_ids: Vec<&str> = methods
                    .iter()
                    .map(|method| match method {
                        AuthMethod::Terminal(terminal) => terminal.id.0.as_ref(),
                        AuthMethod::Agent(agent) => agent.id.0.as_ref(),
                        // `AuthMethod` is `#[non_exhaustive]`: a future
                        // variant this build does not know about yet.
                        _ => "?",
                    })
                    .collect();
                tracing::warn!(
                    metodos = ?method_ids,
                    stderr_reciente = %self.recent_stderr_tail(),
                    "el agente pide autenticación: sesión vencida"
                );
                self.show_expired(cx);
            }
            AgentEvent::AuthFailed { method_id, message } => {
                tracing::warn!(metodo = %method_id, %message, "falló la autenticación del agente");
                let label = self
                    .active
                    .as_ref()
                    .map(|active| active.label.clone())
                    .unwrap_or_default();
                crate::toast::error(format!("No se pudo autenticar «{label}»: {message}"), cx);
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(
                        AgentEvent::AuthFailed {
                            method_id: method_id.clone(),
                            message: message.clone(),
                        },
                        cx,
                    );
                });
                if self.prompt_in_flight && !self.side_request_in_turn {
                    self.end_orphaned_turn(cx);
                }
                self.show_expired(cx);
            }
            AgentEvent::AuthSucceeded { method_id } => {
                if let Some(active) = &self.active {
                    self.expired.remove(&active.id);
                }
                self.chat.update(cx, |chat, cx| {
                    chat.set_banner(None, cx);
                    chat.handle_event(AgentEvent::AuthSucceeded { method_id }, cx);
                });
                self.refresh_connections(cx);
            }
            AgentEvent::LoggedOut { ok } => {
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(AgentEvent::LoggedOut { ok }, cx)
                });
                if ok {
                    self.show_expired(cx);
                }
            }
            AgentEvent::AuthStatus {
                kind,
                label,
                detail,
                account,
            } => {
                // Elevated from `debug!` during the Gemini "sesión vencida"
                // investigation (2026-09-26): at the default log level this
                // was invisible, so the only signal the author ever saw was
                // the generic "sesión vencida" banner.
                tracing::warn!(
                    ?kind,
                    %label,
                    ?detail,
                    hay_cuenta = account.is_some(),
                    "estado de autenticación del agente"
                );
                match (&kind, account) {
                    (AuthStatusKind::None, _) => self.show_expired(cx),
                    (_, Some(account)) => {
                        let identity = Identity {
                            email: account.email,
                            plan: account.plan,
                            organization: account.organization,
                        };
                        if !identity.is_empty() {
                            self.update_identity(identity, cx);
                        }
                    }
                    _ => {}
                }
            }
            AgentEvent::ElicitationCompleted { id, elicitation_id } => {
                tracing::debug!(%elicitation_id, "el agente completó una elicitación");
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(AgentEvent::ElicitationCompleted { id, elicitation_id }, cx);
                });
            }
            AgentEvent::PermissionRequest {
                id,
                session_id,
                tool_call,
                options,
                reply,
            } => self.handle_permission_request(id, session_id, tool_call, options, reply, cx),
            AgentEvent::FsRead {
                path,
                line,
                limit,
                reply,
                ..
            } => self.review.update(cx, |review, cx| {
                review.handle_fs_read(path, line, limit, reply, cx)
            }),
            AgentEvent::FsWrite {
                path,
                content,
                reply,
                ..
            } => self.review.update(cx, |review, cx| {
                review.handle_fs_write(path, content, reply, cx)
            }),
            AgentEvent::Update { session_id, update } => {
                self.track_tool_call_kind(&update);
                self.review_tool_call(&update, cx);
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(AgentEvent::Update { session_id, update }, cx);
                });
            }
            AgentEvent::FileChangeReport { report, .. } => {
                // The agent's own list of what it touched in the turn: re-read
                // every one of them from disk.
                self.review
                    .update(cx, |review, cx| review.reread_paths(&report.paths, cx));
            }
            AgentEvent::TurnEnded {
                session_id,
                stop_reason,
            } => {
                // Every stop reason ends the review turn: `end_turn`,
                // `max_tokens`, `max_turn_requests`, `refusal` and
                // `cancelled` alike. Nothing is accepted automatically.
                self.prompt_in_flight = false;
                self.side_request_in_turn = false;
                self.review.update(cx, |review, cx| review.end_turn(cx));
                self.tool_call_paths.clear();
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(
                        AgentEvent::TurnEnded {
                            session_id,
                            stop_reason,
                        },
                        cx,
                    );
                });
            }
            AgentEvent::Exited { code, stderr_tail } => {
                self.prompt_in_flight = false;
                self.side_request_in_turn = false;
                self.review.update(cx, |review, cx| review.agent_gone(cx));
                self.connection = None;
                self.event_task = None;
                self.capabilities = None;
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(
                        AgentEvent::Exited {
                            code,
                            stderr_tail: stderr_tail.clone(),
                        },
                        cx,
                    );
                });
                self.offer_restart(&stderr_tail, cx);
            }
            AgentEvent::Error {
                message,
                stderr_tail,
            } => {
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(
                        AgentEvent::Error {
                            message,
                            stderr_tail: stderr_tail.clone(),
                        },
                        cx,
                    );
                });
                // `cincel-acp` reports a failed `session/prompt` as an
                // `Error` and never sends its `TurnEnded`: without this the
                // review turn stays open and every editor keeps its review
                // controls disabled. Only when no other request is in the air
                // is the error known to be the prompt's.
                if self.prompt_in_flight && !self.side_request_in_turn {
                    self.end_orphaned_turn(cx);
                }
                self.offer_restart(&stderr_tail, cx);
            }
            AgentEvent::Stderr(line) => {
                // Kept only so `AuthRequired` (which carries no message of
                // its own) can still be logged with some diagnostic. Every
                // URL query is masked first: Antigravity prints a Google
                // login link (with `state`/`code_challenge`) when its saved
                // token stopped working and it wants a new browser login in
                // the middle of `session/new`; that is a "sesión vencida"
                // too, and the banner's "Volver a conectar" runs the login
                // in the modal instead.
                let asks_for_login =
                    !cincel_connections::find_login_urls(&cincel_connections::strip_ansi(&line))
                        .is_empty();
                let line = cincel_connections::redact_line(&line, &[]);
                self.push_recent_stderr(line.clone());
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(AgentEvent::Stderr(line), cx)
                });
                if asks_for_login {
                    tracing::warn!("el agente pide iniciar sesión en el navegador: sesión vencida");
                    self.show_expired(cx);
                }
            }
            other => {
                self.chat
                    .update(cx, |chat, cx| chat.handle_event(other, cx));
            }
        }
    }

    /// `01-producto.md` §F5: ask [`PermissionPolicy::decide`] first; auto-answer with
    /// the agent's own `allow_once`/`allow_always` option when it says so,
    /// otherwise drop `reply` (harmless: `AgentCommand::RespondPermission`
    /// answers the very same slot by id) and let the chat card handle it.
    fn handle_permission_request(
        &mut self,
        id: PermissionRequestId,
        _session_id: SessionId,
        tool_call: Box<ToolCallUpdate>,
        options: Vec<PermissionOption>,
        reply: cincel_acp::PermissionResponder,
        cx: &mut Context<Self>,
    ) {
        let kind = tool_call
            .fields
            .kind
            .or_else(|| self.tool_call_kinds.get(&tool_call.tool_call_id).copied())
            .unwrap_or(ToolKind::Other);
        let paths = cincel_acp::connection::tool_call_paths(&tool_call);
        if self.policy.decide(kind, &paths) == AutoAnswer::Allow
            && let Some(option) = pick_allow_option(&options)
        {
            reply.respond(PermissionOutcome::Selected(option.option_id.clone()));
            return;
        }
        self.chat.update(cx, |chat, cx| {
            chat.request_permission(id.0, &tool_call, options, cx)
        });
    }

    /// Remembers the latest declared `kind` of each tool call: a completion
    /// update often carries only `status`, so this is what lets
    /// [`Agents::maybe_reload_after_tool_call`] and
    /// [`Agents::handle_permission_request`] still know it was an edit.
    fn track_tool_call_kind(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::ToolCall(call) => {
                self.tool_call_kinds
                    .insert(call.tool_call_id.clone(), call.kind);
            }
            SessionUpdate::ToolCallUpdate(update) => {
                if let Some(kind) = update.fields.kind {
                    self.tool_call_kinds
                        .insert(update.tool_call_id.clone(), kind);
                }
            }
            _ => {}
        }
    }

    /// `docs/specs/03-arquitectura.md` §4: an edit tool call (`edit`,
    /// `delete`, `move`) is the first contact with the paths it names, so
    /// the review captures their base; when it completes (or fails, which may
    /// still have written something) the files are re-read from disk. The
    /// `diff` the agent sends is only a UI signal and is never applied.
    fn review_tool_call(&mut self, update: &SessionUpdate, cx: &mut Context<Self>) {
        let as_update = match update {
            SessionUpdate::ToolCall(call) => ToolCallUpdate::from(call.clone()),
            SessionUpdate::ToolCallUpdate(call) => call.clone(),
            _ => return,
        };
        let id = as_update.tool_call_id.clone();
        let kind = as_update.fields.kind;
        let status = as_update.fields.status;
        let named = cincel_acp::connection::tool_call_paths(&as_update);
        let kind = kind.or_else(|| self.tool_call_kinds.get(&id).copied());
        if !matches!(
            kind,
            Some(ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
        ) {
            return;
        }
        let kind = kind.unwrap_or(ToolKind::Edit);
        let known = self.tool_call_paths.entry(id.clone()).or_default();
        let fresh: Vec<PathBuf> = named
            .into_iter()
            .filter(|path| !known.contains(path))
            .collect();
        known.extend(fresh.iter().cloned());
        let finished = matches!(
            status,
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
        );
        if !fresh.is_empty() {
            let auto_granted = self.policy.decide(kind, &fresh) == AutoAnswer::Allow;
            // First heard of at its completion: the disk already holds the
            // agent's text, so the diff's `oldText` is the only base left.
            let late_diffs: HashMap<PathBuf, Option<String>> = if finished {
                as_update
                    .fields
                    .content
                    .iter()
                    .flatten()
                    .filter_map(|item| match item {
                        ToolCallContent::Diff(diff) if fresh.contains(&diff.path) => {
                            Some((diff.path.clone(), diff.old_text.clone()))
                        }
                        _ => None,
                    })
                    .collect()
            } else {
                HashMap::new()
            };
            self.review.update(cx, |review, cx| {
                review.tool_call_started(&fresh, &late_diffs, auto_granted, cx)
            });
        }
        if finished {
            let paths = self.tool_call_paths.remove(&id).unwrap_or_default();
            self.review
                .update(cx, |review, cx| review.tool_call_finished(&paths, cx));
        }
    }

    /// `session/new` right after `initialize` succeeds, so the chat has a
    /// session the moment the user can type (`01-producto.md` §F3).
    fn start_session(&mut self) {
        let Some(connection) = &self.connection else {
            return;
        };
        let cwd = self.sandbox_root.clone().unwrap_or_else(std::env::temp_dir);
        let _ = connection
            .commands()
            .send_blocking(AgentCommand::NewSession {
                cwd,
                mcp_servers: self.mcp_servers.clone(),
            });
    }

    /// What to do with the session once the agent is connected.
    ///
    /// With a conversation waiting to be reopened, the agent's own
    /// `initialize` answer decides: `agentCapabilities.loadSession` means
    /// `session/load` (history is replayed, and the chat drops what it already
    /// shows), `sessionCapabilities.resume` means `session/resume` (no
    /// replay), and neither means the history stays on screen read-only while
    /// a brand new session is opened underneath
    /// (`docs/etapas/etapa-2.md` § correcciones).
    fn resume_or_start_session(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.pending_load.take() else {
            self.start_session();
            return;
        };
        let cwd = self.sandbox_root.clone().unwrap_or_else(std::env::temp_dir);
        let mcp_servers = self.mcp_servers.clone();
        let capabilities = self.capabilities.clone().unwrap_or_default();
        let command = if capabilities.load_session {
            self.chat.update(cx, |chat, cx| chat.begin_replay(cx));
            Some(AgentCommand::LoadSession {
                session_id,
                cwd,
                mcp_servers,
            })
        } else if capabilities.session_capabilities.resume.is_some() {
            Some(AgentCommand::ResumeSession {
                session_id,
                cwd,
                mcp_servers,
            })
        } else {
            None
        };
        match command {
            Some(command) => {
                if let Some(connection) = &self.connection {
                    let _ = connection.commands().send_blocking(command);
                }
            }
            None => {
                self.chat.update(cx, |chat, cx| {
                    chat.set_history_notice(Some(READ_ONLY_HISTORY_NOTICE.to_string()), cx);
                });
                self.start_session();
            }
        }
    }

    /// Records one stderr line from the active connection, keeping at most
    /// [`RECENT_STDERR_LINES`] (see [`Agents::recent_stderr`]).
    fn push_recent_stderr(&mut self, line: String) {
        if self.recent_stderr.len() >= RECENT_STDERR_LINES {
            self.recent_stderr.pop_front();
        }
        self.recent_stderr.push_back(line);
    }

    /// The buffered stderr lines joined for a log field, newest last.
    fn recent_stderr_tail(&self) -> String {
        self.recent_stderr
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Toast with the stderr tail and a "Reiniciar" action
    /// (`docs/etapas/etapa-2.md`), which relaunches the active connection.
    fn offer_restart(&mut self, stderr_tail: &str, cx: &mut Context<Self>) {
        if self.active.is_none() {
            return;
        }
        let tail: Vec<&str> = stderr_tail.lines().rev().take(4).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        let message = if tail.is_empty() {
            "El agente se detuvo.".to_string()
        } else {
            format!("El agente se detuvo.\n{}", tail.join("\n"))
        };
        let weak = cx.entity().downgrade();
        crate::toast::ask(
            message,
            [(
                "Reiniciar",
                Box::new(move |cx: &mut App| {
                    if let Some(agents) = weak.upgrade() {
                        agents.update(cx, |agents, cx| agents.restart_active(cx));
                    }
                }) as crate::toast::ToastAction,
            )],
            cx,
        );
    }

    // ----------------------------------------------------------- transcript

    /// Writes the open conversation if it changed since the last save. Called
    /// every [`TRANSCRIPT_AUTOSAVE_INTERVAL`], before every conversation
    /// switch and once more on window close.
    pub fn save_conversation_now(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Some(conversation) = self.current_conversation(cx) else {
            return;
        };
        // `updated_at` moves on every call, so the comparison ignores it.
        let fingerprint = serde_json::to_string(&(
            &conversation.id,
            &conversation.agent_id,
            &conversation.session_id,
            &conversation.title,
            &conversation.entries,
        ))
        .unwrap_or_default();
        if self.last_saved_conversation.as_deref() == Some(fingerprint.as_str()) {
            return;
        }
        match store.save(&conversation) {
            Ok(()) => self.last_saved_conversation = Some(fingerprint),
            Err(error) => {
                tracing::warn!(%error, "no se pudo guardar la conversación");
                return;
            }
        }
        self.refresh_conversations(cx);
    }

    fn start_transcript_autosave(&mut self, cx: &mut Context<Self>) {
        self.transcript_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(TRANSCRIPT_AUTOSAVE_INTERVAL)
                    .await;
                let alive = this
                    .update(cx, |agents, cx| agents.save_conversation_now(cx))
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }

    /// Rebuilds the history popover out of `index.json`, newest first,
    /// grouped by the connections' current labels ("(conexión anterior)" for
    /// the old ones). With no project open the list is cleared — otherwise a
    /// project switch would leave the previous project's conversations on
    /// screen.
    fn refresh_conversations(&mut self, cx: &mut Context<Self>) {
        let labels: HashMap<String, String> = self
            .connections
            .store()
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|connection| (connection.id.to_string(), connection.label))
            .collect();
        let summaries = self
            .store
            .as_ref()
            .map(|store| {
                store
                    .index()
                    .into_iter()
                    .map(|entry| {
                        let when = format_when(entry.updated_at);
                        let mut summary = entry.summary(when);
                        summary.group = match &summary.connection_id {
                            Some(id) => labels
                                .get(id)
                                .cloned()
                                .unwrap_or_else(|| summary.agent_name.clone()),
                            None => LEGACY_CONNECTION_GROUP.to_string(),
                        };
                        summary
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.chat
            .update(cx, |chat, cx| chat.set_conversations(summaries, cx));
    }
}

/// "Usado hace 2 h" for the popover.
fn used_label(last_used_at: u64) -> String {
    if last_used_at == 0 {
        return "Sin usar todavía".to_string();
    }
    format!("Usado {}", format_when(last_used_at))
}

fn map_mcp_servers(servers: &[cincel_settings::McpServerSettings]) -> Vec<McpServerSpec> {
    servers
        .iter()
        .map(|server| McpServerSpec {
            name: server.name.clone(),
            command: PathBuf::from(&server.command),
            args: server.args.clone(),
            env: server.env.clone(),
        })
        .collect()
}

/// The fixed [`PermissionPolicy`] with `review.sensitive_paths` as its
/// sensitive globs (the built-in ones when the list does not parse).
fn build_policy(settings: &cincel_settings::Settings) -> PermissionPolicy {
    PermissionPolicy::default()
        .with_sensitive_globs(settings.review.sensitive_paths.clone())
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "review.sensitive_paths inválido, se usan los patrones por defecto");
            PermissionPolicy::default()
        })
}

/// Maps an [`OpenError`] onto the JSON-RPC-shaped [`cincel_acp::FsError`]
/// `fs/read_text_file` answers with.
pub(crate) fn map_open_error(error: &OpenError) -> cincel_acp::FsError {
    match error {
        OpenError::NotUtf8(path) => {
            cincel_acp::FsError::InvalidParams(format!("«{}» no es UTF-8", path.display()))
        }
        OpenError::NotAFile(path) => {
            cincel_acp::FsError::InvalidParams(format!("«{}» no es un archivo", path.display()))
        }
        OpenError::Io { path, source } if source.kind() == std::io::ErrorKind::NotFound => {
            cincel_acp::FsError::NotFound(format!("«{}» no existe", path.display()))
        }
        other => cincel_acp::FsError::Internal(other.to_string()),
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use std::path::Path;

    use cincel_acp::acp::schema::v1::PermissionOptionId;
    use cincel_acp::{AuthAccount, PromptBlock};
    use cincel_chat::{ChatSettings, ChatTheme, Entry, NoticeLevel};
    use cincel_connections::{AgentKind, LoginEvent};
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::center::CenterPanel;
    use crate::connection_modal::{DeleteStep, Mode, Step, delete_question};
    use crate::project::ProjectOptions;
    use crate::test_support::{
        FAKE_AGY_URL, FAKE_EMAIL, FAKE_LOGIN_CODE, FAKE_LOGIN_URL, FakeEnv, isolate_state,
    };

    fn init_test(cx: &mut TestAppContext) {
        isolate_state();
        cx.update(|cx| crate::init(cincel_settings::Config::default(), cx));
    }

    /// Everything a test needs: the chat, the tab area, the controller, the
    /// project, the fake connections engine (keep it alive: it owns the
    /// profile directory) and the id of its one saved connection, "Fake".
    struct Harness {
        chat: Entity<ChatPanel>,
        center: Entity<CenterPanel>,
        agents: Entity<Agents>,
        env: FakeEnv,
        id: Uuid,
    }

    impl Harness {
        fn modal(&self, cx: &mut VisualTestContext) -> Entity<ConnectionsModal> {
            self.agents
                .read_with(cx, |agents, _| agents.modal().clone())
        }

        fn toasts(&self, cx: &mut VisualTestContext) -> Entity<crate::toast::Toasts> {
            let toasts = cx.update(|_, cx| cx.new(|_| crate::toast::Toasts::new()));
            cx.update(|_, cx| crate::toast::set_global(&toasts, cx));
            toasts
        }
    }

    /// Builds a chat panel, a center panel and an `Agents` (background
    /// draining off, per the module doc) in one window, the project, and a
    /// fake engine with one Claude connection called "Fake". Nothing is
    /// activated.
    fn harness<'a>(
        project_dir: &Path,
        cx: &'a mut TestAppContext,
    ) -> (Harness, &'a mut VisualTestContext) {
        init_test(cx);
        let settings = cx.update(|cx| crate::settings::settings(cx));
        let project = cx.update(|cx| {
            crate::project::open_with(project_dir, &settings, ProjectOptions::inert(), cx)
                .expect("el proyecto se abre")
        });
        let center = cx.update(CenterPanel::new);
        let (chat, cx) = cx.add_window_view(|window, cx| {
            ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
        });
        center.update(cx, |center, cx| {
            center.set_project(Some(project.clone()), cx)
        });
        let agents = cx.update(|window, cx| {
            let review = cx.new(|cx| Review::new(&center, window, cx));
            cx.new(|cx| Agents::new(chat.clone(), review, false, window, cx))
        });
        let env = FakeEnv::new();
        let id = env.add_connection("claude-acp", "Fake");
        cx.update(|window, cx| {
            agents.update(cx, |agents, cx| {
                agents.set_connections(env.connections.clone(), cx);
                agents.set_project(Some(project.clone()), window, cx);
            });
        });
        cx.run_until_parked();
        (
            Harness {
                chat,
                center,
                agents,
                env,
                id,
            },
            cx,
        )
    }

    /// Picks `id` in the popover, the way the user does.
    fn select(h: &Harness, id: Uuid, cx: &mut VisualTestContext) {
        h.chat
            .update(cx, |chat, cx| chat.select_connection(&id.to_string(), cx));
        cx.run_until_parked();
    }

    /// Activates the harness' connection and drains events by hand (see the
    /// module doc's `background` note) until `SessionCreated`. Returns the
    /// session id.
    fn spawn_and_connect(h: &Harness, cx: &mut VisualTestContext) -> SessionId {
        select(h, h.id, cx);
        drain_until_session_created(h, cx)
    }

    fn drain_until(h: &Harness, cx: &mut VisualTestContext, stop: impl Fn(&AgentEvent) -> bool) {
        let events = h
            .agents
            .read_with(cx, |agents, _| agents.connection_events())
            .expect("la conexión existe");
        loop {
            let event = events.recv_blocking().expect("evento del agente falso");
            let done = stop(&event);
            h.agents
                .update(cx, |agents, cx| agents.handle_agent_event(event, cx));
            cx.run_until_parked();
            if done {
                break;
            }
        }
    }

    /// Drains events by hand until `SessionCreated`, without activating
    /// anything first — for when it was already launched. Returns the session.
    fn drain_until_session_created(h: &Harness, cx: &mut VisualTestContext) -> SessionId {
        drain_until(h, cx, |event| {
            matches!(event, AgentEvent::SessionCreated { .. })
        });
        h.chat.read_with(cx, |chat, _| {
            chat.session_id().cloned().expect("hay sesión")
        })
    }

    fn drain_until_turn_ended(h: &Harness, cx: &mut VisualTestContext) {
        drain_until(h, cx, |event| matches!(event, AgentEvent::TurnEnded { .. }));
    }

    /// Feeds the modal's login events back to it until `stop` matches.
    fn drain_login_until(
        h: &Harness,
        cx: &mut VisualTestContext,
        stop: impl Fn(&LoginEvent) -> bool,
    ) -> Vec<LoginEvent> {
        let modal = h.modal(cx);
        let events = modal
            .read_with(cx, |modal, _| modal.login_events())
            .expect("hay un inicio de sesión en curso");
        let mut seen = Vec::new();
        loop {
            let event = events.recv_blocking().expect("evento de inicio de sesión");
            let done = stop(&event);
            seen.push(event.clone());
            cx.update(|window, cx| {
                modal.update(cx, |modal, cx| modal.handle_login_event(event, window, cx))
            });
            cx.run_until_parked();
            if done {
                break;
            }
        }
        seen
    }

    fn with_modal<R>(
        h: &Harness,
        cx: &mut VisualTestContext,
        f: impl FnOnce(&mut ConnectionsModal, &mut Window, &mut Context<ConnectionsModal>) -> R,
    ) -> R {
        let modal = h.modal(cx);
        let result = cx.update(|window, cx| modal.update(cx, |modal, cx| f(modal, window, cx)));
        cx.run_until_parked();
        result
    }

    /// Types `text` in the composer and sends it, which is what puts a
    /// `UserMessage` in the transcript (and therefore a title on the
    /// conversation) before the command reaches the worker.
    fn send_from_chat(chat: &Entity<ChatPanel>, text: &str, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            chat.update(cx, |chat, cx| {
                chat.set_input_text(text, window, cx);
                chat.send(window, cx);
            });
        });
        cx.run_until_parked();
    }

    fn prompt(h: &Harness, session_id: SessionId, text: &str, cx: &mut VisualTestContext) {
        h.agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id,
                    blocks: vec![PromptBlock::Text(text.to_string())],
                    feedback: None,
                },
                cx,
            );
        });
    }

    fn sample_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("demo.txt"), "alfa\nbeta\n").unwrap();
        dir
    }

    fn live_store(h: &Harness, cx: &mut VisualTestContext) -> ConversationStore {
        h.agents
            .read_with(cx, |agents, _| agents.store.clone())
            .expect("hay directorio de estado")
    }

    fn open_second_project(cx: &mut VisualTestContext, dir: &Path) -> Entity<Project> {
        cx.update(|_window, cx| {
            let settings = crate::settings::settings(cx);
            crate::project::open_with(dir, &settings, ProjectOptions::inert(), cx)
                .expect("el proyecto B se abre")
        })
    }

    fn toast_messages(
        toasts: &Entity<crate::toast::Toasts>,
        cx: &mut VisualTestContext,
    ) -> Vec<String> {
        toasts.read_with(cx, |toasts, _| {
            toasts
                .items()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect()
        })
    }

    fn badge_of(h: &Harness, id: Uuid, cx: &mut VisualTestContext) -> ConnectionBadge {
        h.chat.read_with(cx, |chat, _| {
            chat.connections()
                .iter()
                .find(|row| row.id == id.to_string())
                .map(|row| row.badge.clone())
                .expect("la fila existe")
        })
    }

    // ------------------------------------------------- arranque y popover

    #[gpui::test]
    fn startup_spawns_nothing_and_says_sin_conexion(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        assert!(
            !h.agents
                .read_with(cx, |agents, _| agents.is_agent_running()),
            "al arrancar no se lanza ningún agente"
        );
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.active_connection_id()),
            None
        );
        assert!(
            h.chat
                .read_with(cx, |chat, _| chat.active_connection().is_none())
        );
        // The saved connection is listed, but only listed.
        assert_eq!(h.chat.read_with(cx, |chat, _| chat.connections().len()), 1);
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat.status()),
            AgentStatus::Disconnected
        );
    }

    #[gpui::test]
    fn the_default_label_preselects_a_row_and_connects_nothing(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        h.agents.update(cx, |agents, cx| {
            agents.default_label = Some("Fake".to_string());
            agents.refresh_connections(cx);
        });
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat
                .preselected_connection()
                .map(str::to_string)),
            Some(h.id.to_string())
        );
        assert!(
            !h.agents
                .read_with(cx, |agents, _| agents.is_agent_running())
        );
    }

    #[gpui::test]
    fn the_popover_lists_connections_with_their_status(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let expired = h.env.add_connection("claude-acp", "Claude · vencida");
        let broken = h.env.add_connection("codex-acp", "Codex · trabajo");
        h.env.expire(expired);
        // Without its adapter, Codex cannot run.
        std::fs::remove_file(h.env.paths.agents_dir().join("codex-acp").join("current")).unwrap();
        h.agents
            .update(cx, |agents, cx| agents.refresh_connections(cx));

        assert_eq!(badge_of(&h, h.id, cx), ConnectionBadge::Connected);
        assert_eq!(badge_of(&h, expired, cx), ConnectionBadge::SessionExpired);
        assert!(matches!(
            badge_of(&h, broken, cx),
            ConnectionBadge::Unavailable { reason } if reason.contains("Codex")
        ));
        let row = h.chat.read_with(cx, |chat, _| {
            chat.connections()
                .iter()
                .find(|row| row.id == h.id.to_string())
                .cloned()
                .unwrap()
        });
        assert_eq!(row.identity.as_deref(), Some("gary@example.com · max"));
        assert!(row.last_used.starts_with("Usado"), "{}", row.last_used);
    }

    // ------------------------------------------------------ activación

    #[gpui::test]
    fn choosing_a_connection_launches_it_with_its_profile(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let session = spawn_and_connect(&h, cx);
        assert!(session.0.to_string().starts_with("fake-session-"));
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat.status()),
            AgentStatus::Ready
        );
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat
                .active_connection()
                .map(|c| c.label.clone())),
            Some("Fake".to_string())
        );
        // The empty conversation opened with it belongs to it.
        let binding = h
            .agents
            .read_with(cx, |agents, _| agents.conversation_binding());
        assert_eq!(binding.and_then(|(_, connection)| connection), Some(h.id));

        // The process runs with the profile variable of that connection.
        let session_id = session;
        h.agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id,
                    blocks: vec![
                        PromptBlock::Text("echo-env".to_string()),
                        PromptBlock::Text("CLAUDE_CONFIG_DIR".to_string()),
                    ],
                    feedback: None,
                },
                cx,
            );
        });
        drain_until_turn_ended(&h, cx);
        let profile = h.env.profile_dir(h.id).display().to_string();
        let said = h.chat.read_with(cx, |chat, _| {
            chat.entries()
                .iter()
                .filter_map(|entry| match entry {
                    Entry::AgentText(text) => Some(text.markdown.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
        assert!(
            said.contains(&format!("CLAUDE_CONFIG_DIR={profile}")),
            "el agente corre con el perfil de la conexión: {said}"
        );
    }

    #[gpui::test]
    fn switching_connections_stops_the_old_process_and_keeps_its_conversations(
        cx: &mut TestAppContext,
    ) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let other = h.env.add_connection("codex-acp", "Codex · trabajo");
        h.agents
            .update(cx, |agents, cx| agents.refresh_connections(cx));
        let _session = spawn_and_connect(&h, cx);
        send_from_chat(&h.chat, "primera conversación", cx);
        drain_until_turn_ended(&h, cx);
        let old_commands = h.agents.read_with(cx, |agents, _| {
            agents
                .connection
                .as_ref()
                .expect("hay conexión")
                .commands()
                .clone()
        });

        select(&h, other, cx);
        assert!(
            old_commands.send_blocking(AgentCommand::Shutdown).is_err(),
            "el proceso de la conexión anterior se detuvo"
        );
        assert!(
            h.chat.read_with(cx, |chat, _| chat.entries().is_empty()),
            "cambiar de conexión abre una conversación vacía"
        );
        let _session = drain_until_session_created(&h, cx);
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.active_connection_id()),
            Some(other)
        );
        // Nothing was deleted: the old conversation is saved, bound to the
        // first connection and grouped under its label.
        let index = live_store(&h, cx).index();
        assert_eq!(index.len(), 1, "{index:?}");
        assert_eq!(index[0].connection_id, Some(h.id.to_string()));
        assert_eq!(index[0].title, "primera conversación");
        let rows = h
            .chat
            .read_with(cx, |chat, _| chat.conversations().to_vec());
        assert_eq!(rows[0].group, "Fake");
        assert!(
            h.env.profile_dir(h.id).is_dir(),
            "el perfil anterior sigue ahí"
        );
    }

    #[gpui::test]
    fn picking_a_past_conversation_restores_its_entries_and_its_connection(
        cx: &mut TestAppContext,
    ) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let other = h.env.add_connection("codex-acp", "Codex · trabajo");
        h.agents
            .update(cx, |agents, cx| agents.refresh_connections(cx));
        let _session = spawn_and_connect(&h, cx);
        send_from_chat(&h.chat, "primera conversación", cx);
        drain_until_turn_ended(&h, cx);
        let entries_before = h.chat.read_with(cx, |chat, _| chat.entries().len());

        select(&h, other, cx);
        let _session = drain_until_session_created(&h, cx);
        let stored = live_store(&h, cx).index()[0].id.clone();

        h.chat
            .update(cx, |chat, cx| chat.open_conversation(&stored, cx));
        cx.run_until_parked();
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat.entries().len()),
            entries_before,
            "vuelven las entradas guardadas"
        );
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.active_connection_id()),
            Some(h.id),
            "y vuelve su conexión"
        );
        assert_eq!(
            h.chat
                .read_with(cx, |chat, _| chat.active_conversation().map(str::to_string)),
            Some(stored)
        );
        let session = drain_until_session_created(&h, cx);
        assert!(!session.0.is_empty());
    }

    #[gpui::test]
    fn an_old_conversation_without_connection_opens_read_only(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let store = live_store(&h, cx);
        let mut old = Conversation::new(
            "c-antes",
            "claude-acp",
            "Claude",
            dir.path().to_path_buf(),
            now_seconds(),
        );
        old.entries = vec![cincel_chat::Entry::UserMessage(cincel_chat::UserMessage {
            blocks: vec![cincel_chat::MessageBlock::Text("de antes".to_string())],
        })];
        old.title = "de antes".to_string();
        store.save(&old).expect("se guarda");
        h.agents
            .update(cx, |agents, cx| agents.refresh_conversations(cx));
        let rows = h
            .chat
            .read_with(cx, |chat, _| chat.conversations().to_vec());
        assert_eq!(rows[0].group, LEGACY_CONNECTION_GROUP);

        h.chat
            .update(cx, |chat, cx| chat.open_conversation("c-antes", cx));
        cx.run_until_parked();
        assert_eq!(h.chat.read_with(cx, |chat, _| chat.entries().len()), 1);
        assert_eq!(
            h.chat
                .read_with(cx, |chat, _| chat.history_notice().map(str::to_string)),
            Some(LEGACY_HISTORY_NOTICE.to_string())
        );
        assert!(
            !h.agents
                .read_with(cx, |agents, _| agents.is_agent_running()),
            "una conversación vieja no lanza nada"
        );
        // Read-only: saving leaves the file as it was.
        h.agents
            .update(cx, |agents, cx| agents.save_conversation_now(cx));
        assert_eq!(store.load("c-antes").unwrap().connection_id, None);
    }

    // ---------------------------------------------------- nuevo agente (F2)

    #[gpui::test]
    fn a_new_connection_goes_through_the_modal_end_to_end(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        h.chat
            .update(cx, |chat, cx| chat.request_new_connection(cx));
        cx.run_until_parked();
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Choose)
        );

        with_modal(&h, cx, |modal, window, cx| {
            modal.choose(AgentKind::Claude, window, cx)
        });
        // Preparing ran (everything was installed) and the login started.
        let (runtime_done, adapter_done) = h.modal(cx).read_with(cx, |modal, _| {
            let flow = modal.connect_flow().unwrap();
            (flow.runtime.done, flow.adapter.done)
        });
        assert!(runtime_done && adapter_done);

        let seen = drain_login_until(&h, cx, |event| matches!(event, LoginEvent::NeedsPastedCode));
        assert!(
            seen.iter()
                .all(|event| !matches!(event, LoginEvent::Output(line) if line.contains("s3cr3t"))),
            "los detalles técnicos llegan redactados"
        );
        let (step, url) = h.modal(cx).read_with(cx, |modal, _| {
            let flow = modal.connect_flow().unwrap();
            (flow.step.clone(), flow.url.clone())
        });
        assert_eq!(step, Step::Link);
        assert_eq!(url.as_deref(), Some(FAKE_LOGIN_URL), "el enlace se muestra");

        with_modal(&h, cx, |modal, window, cx| {
            modal.set_code_text(FAKE_LOGIN_CODE, window, cx);
            modal.submit_code(window, cx);
        });
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Waiting)
        );
        drain_login_until(&h, cx, |event| {
            matches!(
                event,
                LoginEvent::Completed { .. } | LoginEvent::Failed { .. }
            )
        });
        let (step, identity) = h.modal(cx).read_with(cx, |modal, _| {
            let flow = modal.connect_flow().unwrap();
            (flow.step.clone(), flow.identity.clone())
        });
        assert_eq!(step, Step::Done);
        assert_eq!(
            identity.and_then(|identity| identity.email).as_deref(),
            Some(FAKE_EMAIL)
        );
        let suggested = h.modal(cx).read_with(cx, |modal, cx| modal.label_text(cx));
        assert_eq!(suggested, "Claude · personal");

        with_modal(&h, cx, |modal, window, cx| {
            modal.set_label_text("Claude · casa", window, cx);
            modal.save(window, cx);
        });
        assert!(!h.modal(cx).read_with(cx, |modal, _| modal.is_open()));
        let saved = h
            .env
            .connections
            .store()
            .list()
            .unwrap()
            .into_iter()
            .find(|connection| connection.label == "Claude · casa")
            .expect("la conexión quedó guardada");
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.active_connection_id()),
            Some(saved.id),
            "Guardar activa la conexión"
        );
        let _session = drain_until_session_created(&h, cx);
        let binding = h
            .agents
            .read_with(cx, |agents, _| agents.conversation_binding());
        assert_eq!(
            binding.and_then(|(_, connection)| connection),
            Some(saved.id)
        );
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat
                .active_connection()
                .map(|c| c.label.clone())),
            Some("Claude · casa".to_string())
        );
    }

    #[gpui::test]
    fn a_codex_login_finishes_without_a_pasted_code(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        with_modal(&h, cx, |modal, window, cx| {
            modal.open_connect(window, cx);
            modal.choose(AgentKind::Codex, window, cx);
        });
        let seen = drain_login_until(&h, cx, |event| {
            matches!(
                event,
                LoginEvent::Completed { .. } | LoginEvent::Failed { .. }
            )
        });
        assert!(
            !seen
                .iter()
                .any(|event| matches!(event, LoginEvent::NeedsPastedCode))
        );
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Done)
        );
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, cx| modal.label_text(cx)),
            "Codex · personal"
        );
    }

    #[gpui::test]
    fn an_antigravity_connection_logs_in_over_acp_without_a_code(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        with_modal(&h, cx, |modal, window, cx| {
            modal.open_connect(window, cx);
            modal.choose(AgentKind::Antigravity, window, cx);
        });
        // Preparing needed no Node: only the official binary.
        let (adapter_done, runtime_text) = h.modal(cx).read_with(cx, |modal, _| {
            let flow = modal.connect_flow().unwrap();
            (flow.adapter.done, flow.runtime.text.clone())
        });
        assert!(adapter_done);
        assert_eq!(runtime_text, "No hace falta");

        let seen = drain_login_until(&h, cx, |event| matches!(event, LoginEvent::UrlDetected(_)));
        assert!(
            seen.iter()
                .all(|event| !matches!(event, LoginEvent::Output(line) if line.contains("s3cr3t")))
        );
        let (step, url, needs_code) = h.modal(cx).read_with(cx, |modal, _| {
            let flow = modal.connect_flow().unwrap();
            (flow.step.clone(), flow.url.clone(), flow.needs_code)
        });
        assert_eq!(step, Step::Link);
        assert_eq!(url.as_deref(), Some(FAKE_AGY_URL), "el enlace se muestra");
        assert!(!needs_code, "no hay campo de código");

        let seen = drain_login_until(&h, cx, |event| {
            matches!(
                event,
                LoginEvent::Completed { .. } | LoginEvent::Failed { .. }
            )
        });
        assert!(
            !seen
                .iter()
                .any(|event| matches!(event, LoginEvent::NeedsPastedCode))
        );
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Done)
        );
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, cx| modal.label_text(cx)),
            "Antigravity · personal"
        );
        with_modal(&h, cx, |modal, window, cx| modal.save(window, cx));
        let saved = h
            .env
            .connections
            .store()
            .list()
            .unwrap()
            .into_iter()
            .find(|connection| connection.agent_id == "antigravity-acp")
            .expect("la conexión quedó guardada");
        assert!(
            h.env
                .profile_dir(saved.id)
                .join("antigravity-acp/acp_token.json")
                .is_file(),
            "el token quedó en el perfil propio"
        );
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.active_connection_id()),
            Some(saved.id)
        );
        let _session = drain_until_session_created(&h, cx);
    }

    #[gpui::test]
    fn cancelling_a_login_removes_the_half_created_profile(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        with_modal(&h, cx, |modal, window, cx| {
            modal.open_connect(window, cx);
            modal.choose(AgentKind::Claude, window, cx);
        });
        drain_login_until(&h, cx, |event| matches!(event, LoginEvent::NeedsPastedCode));
        let profiles = || {
            std::fs::read_dir(h.env.paths.connections_dir())
                .unwrap()
                .count()
        };
        assert_eq!(profiles(), 2, "el guardado y el pendiente");

        with_modal(&h, cx, |modal, window, cx| modal.cancel_login(window, cx));
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Choose),
            "vuelve a «Elegí un agente»"
        );
        assert_eq!(profiles(), 1, "el perfil a medio crear se borró");
        assert_eq!(h.env.connections.store().list().unwrap().len(), 1);
    }

    #[gpui::test]
    fn esc_during_a_login_asks_before_closing(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        with_modal(&h, cx, |modal, window, cx| {
            modal.open_connect(window, cx);
            // With nothing in flight, Esc just closes.
            modal.request_close(window, cx);
        });
        assert!(!h.modal(cx).read_with(cx, |modal, _| modal.is_open()));

        with_modal(&h, cx, |modal, window, cx| {
            modal.open_connect(window, cx);
            modal.choose(AgentKind::Claude, window, cx);
        });
        drain_login_until(&h, cx, |event| matches!(event, LoginEvent::NeedsPastedCode));
        with_modal(&h, cx, |modal, window, cx| modal.request_close(window, cx));
        assert!(
            h.modal(cx)
                .read_with(cx, |modal, _| modal.is_confirming_close())
        );
        with_modal(&h, cx, |modal, _window, cx| modal.keep_open(cx));
        assert!(h.modal(cx).read_with(cx, |modal, _| modal.is_open()));
        with_modal(&h, cx, |modal, window, cx| {
            modal.request_close(window, cx);
            modal.close(window, cx);
        });
        assert!(!h.modal(cx).read_with(cx, |modal, _| modal.is_open()));
        assert_eq!(
            std::fs::read_dir(h.env.paths.connections_dir())
                .unwrap()
                .count(),
            1,
            "cerrar a mitad de camino no deja perfiles huérfanos"
        );
    }

    #[gpui::test]
    fn a_wrong_code_fails_with_a_message_and_offers_retry(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        with_modal(&h, cx, |modal, window, cx| {
            modal.open_connect(window, cx);
            modal.choose(AgentKind::Claude, window, cx);
        });
        drain_login_until(&h, cx, |event| matches!(event, LoginEvent::NeedsPastedCode));
        with_modal(&h, cx, |modal, window, cx| {
            modal.set_code_text("OTRO-CODIGO", window, cx);
            modal.submit_code(window, cx);
        });
        drain_login_until(&h, cx, |event| {
            matches!(
                event,
                LoginEvent::Completed { .. } | LoginEvent::Failed { .. }
            )
        });
        let step = h.modal(cx).read_with(cx, |modal, _| modal.step().cloned());
        match step {
            Some(Step::Failed { message }) => {
                assert!(message.contains("código 1"), "{message}");
            }
            other => panic!("se esperaba un error, llegó {other:?}"),
        }
        with_modal(&h, cx, |modal, window, cx| modal.retry(window, cx));
        drain_login_until(&h, cx, |event| matches!(event, LoginEvent::NeedsPastedCode));
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Link),
            "Reintentar vuelve a abrir el inicio de sesión"
        );
    }

    #[gpui::test]
    fn without_network_nor_runtime_the_modal_explains_what_is_missing(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let empty = FakeEnv::empty();
        h.agents.update(cx, |agents, cx| {
            agents.set_connections(empty.connections.clone(), cx)
        });
        with_modal(&h, cx, |modal, window, cx| {
            modal.open_connect(window, cx);
            modal.choose(AgentKind::Antigravity, window, cx);
        });
        match h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()) {
            Some(Step::Failed { message }) => assert!(message.contains("internet"), "{message}"),
            other => panic!("se esperaba un error, llegó {other:?}"),
        }
    }

    // ------------------------------------------------------ eliminar (F3)

    #[gpui::test]
    fn deleting_the_active_connection_forgets_everything_and_says_sin_conexion(
        cx: &mut TestAppContext,
    ) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let keep = h.env.add_connection("codex-acp", "Codex · trabajo");
        let _session = spawn_and_connect(&h, cx);
        send_from_chat(&h.chat, "a borrar", cx);
        drain_until_turn_ended(&h, cx);
        h.agents
            .update(cx, |agents, cx| agents.save_conversation_now(cx));
        let store = live_store(&h, cx);
        // A conversation of the other connection must survive.
        let mut survivor = Conversation::new(
            "c-codex",
            "codex-acp",
            "Codex · trabajo",
            dir.path().to_path_buf(),
            now_seconds(),
        )
        .with_connection(keep.to_string());
        survivor.entries = vec![cincel_chat::Entry::UserMessage(cincel_chat::UserMessage {
            blocks: vec![cincel_chat::MessageBlock::Text("queda".to_string())],
        })];
        store.save(&survivor).unwrap();
        let profile = h.env.profile_dir(h.id);
        assert!(profile.is_dir());

        h.chat
            .update(cx, |chat, cx| chat.request_delete_connection(cx));
        cx.run_until_parked();
        with_modal(&h, cx, |modal, _window, cx| {
            modal.pick_for_deletion(h.id, cx)
        });
        let question = h.modal(cx).read_with(cx, |modal, _| match modal.mode() {
            Mode::Delete(DeleteStep::Confirm {
                label,
                conversations,
                ..
            }) => delete_question(label, *conversations),
            other => panic!("se esperaba la confirmación, llegó {other:?}"),
        });
        assert_eq!(
            question,
            "¿Eliminar «Fake»? Se cerrará la sesión, se borrarán sus credenciales de este \
             equipo y su 1 conversación."
        );
        with_modal(&h, cx, |modal, window, cx| modal.confirm_delete(window, cx));
        cx.run_until_parked();

        assert!(
            !h.modal(cx).read_with(cx, |modal, _| modal.is_open()),
            "salió limpio"
        );
        assert!(!profile.exists(), "el perfil se borró");
        assert!(
            h.env.connections.store().get(h.id).is_err(),
            "y salió del índice"
        );
        let index = store.index();
        assert_eq!(
            index.len(),
            1,
            "solo queda la de la otra conexión: {index:?}"
        );
        assert_eq!(index[0].id, "c-codex");
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.active_connection_id()),
            None
        );
        assert!(
            !h.agents
                .read_with(cx, |agents, _| agents.is_agent_running())
        );
        assert!(
            h.chat
                .read_with(cx, |chat, _| chat.active_connection().is_none())
        );
        assert_eq!(h.chat.read_with(cx, |chat, _| chat.connections().len()), 1);
        assert!(
            h.env.profile_dir(keep).is_dir(),
            "la otra conexión no se tocó"
        );
    }

    // ---------------------------------------------- sesión vencida (F4)

    #[gpui::test]
    fn an_expired_session_shows_the_banner_and_relogin_keeps_the_label(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        // A conversation of this connection, which must survive the relogin.
        let _session = spawn_and_connect(&h, cx);
        send_from_chat(&h.chat, "antes de vencer", cx);
        drain_until_turn_ended(&h, cx);
        h.agents
            .update(cx, |agents, cx| agents.save_conversation_now(cx));
        let conversations_before = live_store(&h, cx).count_for_connection(&h.id.to_string());
        assert_eq!(conversations_before, 1);

        // "Borrar el archivo de credenciales del perfil" (spec §10).
        h.env.expire(h.id);
        select(&h, h.id, cx);
        assert_eq!(badge_of(&h, h.id, cx), ConnectionBadge::SessionExpired);
        let banner = h.chat.read_with(cx, |chat, _| chat.banner().cloned());
        assert_eq!(
            banner.as_ref().map(ConnectionBanner::text).as_deref(),
            Some("La sesión de «Fake» venció")
        );
        assert!(
            !h.agents
                .read_with(cx, |agents, _| agents.is_agent_running()),
            "con la sesión vencida no se lanza el agente"
        );

        // "Volver a conectar": F2 from step 3 on the same profile.
        h.chat
            .update(cx, |chat, cx| chat.request_reconnect(&h.id.to_string(), cx));
        cx.run_until_parked();
        drain_login_until(&h, cx, |event| matches!(event, LoginEvent::NeedsPastedCode));
        with_modal(&h, cx, |modal, window, cx| {
            modal.set_code_text(FAKE_LOGIN_CODE, window, cx);
            modal.submit_code(window, cx);
        });
        drain_login_until(&h, cx, |event| {
            matches!(
                event,
                LoginEvent::Completed { .. } | LoginEvent::Failed { .. }
            )
        });
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Done)
        );
        with_modal(&h, cx, |modal, window, cx| modal.save(window, cx));
        let _session = drain_until_session_created(&h, cx);

        assert!(h.chat.read_with(cx, |chat, _| chat.banner().is_none()));
        assert_eq!(badge_of(&h, h.id, cx), ConnectionBadge::Connected);
        let connection = h.env.connections.store().get(h.id).unwrap();
        assert_eq!(connection.label, "Fake", "conserva la etiqueta");
        assert_eq!(
            h.env.connections.store().list().unwrap().len(),
            1,
            "mismo perfil"
        );
        assert_eq!(
            live_store(&h, cx).count_for_connection(&h.id.to_string()),
            conversations_before,
            "y sus conversaciones"
        );
    }

    #[gpui::test]
    fn a_login_link_on_stderr_means_the_session_expired_and_is_never_kept(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        h.agents.update(cx, |agents, cx| {
            agents.handle_agent_event(
                AgentEvent::Stderr(format!(
                    "Open the following link to authenticate the ACP server: {FAKE_AGY_URL}"
                )),
                cx,
            );
        });
        cx.run_until_parked();
        h.agents.read_with(cx, |agents, _| {
            let kept = agents.recent_stderr.back().cloned().unwrap_or_default();
            assert!(kept.contains("[REDACTED]"), "{kept}");
            assert!(!kept.contains("s3cr3t"), "{kept}");
        });
        assert!(matches!(
            h.chat.read_with(cx, |chat, _| chat.banner().cloned()),
            Some(ConnectionBanner::Expired { .. })
        ));
    }

    #[gpui::test]
    fn stderr_lines_are_kept_bounded_for_the_auth_required_log(cx: &mut TestAppContext) {
        // `AgentEvent::AuthRequired` carries no message of its own (see the
        // 2026-09-26 Gemini "sesión vencida" investigation), so
        // `Agents::recent_stderr` is the only diagnostic left to log; this
        // checks its bookkeeping directly rather than the log line itself
        // (this codebase has no tracing-capture harness).
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        h.agents.update(cx, |agents, cx| {
            for i in 0..(RECENT_STDERR_LINES + 5) {
                agents.handle_agent_event(AgentEvent::Stderr(format!("line {i}")), cx);
            }
        });
        cx.run_until_parked();
        h.agents.read_with(cx, |agents, _| {
            assert_eq!(agents.recent_stderr.len(), RECENT_STDERR_LINES);
            assert_eq!(
                agents.recent_stderr.front().cloned(),
                Some("line 5".to_string()),
                "las líneas más viejas se descartan primero"
            );
            assert_eq!(
                agents.recent_stderr.back().cloned(),
                Some(format!("line {}", RECENT_STDERR_LINES + 4))
            );
        });
    }

    #[gpui::test]
    fn relaunching_the_active_connection_clears_the_previous_agents_stderr(
        cx: &mut TestAppContext,
    ) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        h.agents.update(cx, |agents, cx| {
            agents.handle_agent_event(
                AgentEvent::Stderr("de la conexión anterior".to_string()),
                cx,
            );
        });
        cx.run_until_parked();
        // Re-choosing the same connection relaunches its agent
        // (`Agents::spawn_active`), exactly as "Volver a conectar" does.
        select(&h, h.id, cx);
        let _session = drain_until_session_created(&h, cx);
        h.agents.read_with(cx, |agents, _| {
            assert!(
                agents.recent_stderr.is_empty(),
                "el stderr de la conexión anterior no debe confundirse con el de la nueva"
            );
        });
    }

    #[gpui::test]
    fn auth_required_from_the_agent_marks_the_session_expired(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        h.agents.update(cx, |agents, cx| {
            agents.handle_agent_event(
                AgentEvent::AuthRequired {
                    methods: Vec::new(),
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(
            badge_of(&h, h.id, cx),
            ConnectionBadge::SessionExpired,
            "el agente manda aunque el archivo de credenciales siga ahí"
        );
        assert!(matches!(
            h.chat.read_with(cx, |chat, _| chat.banner().cloned()),
            Some(ConnectionBanner::Expired { .. })
        ));
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat.status()),
            AgentStatus::AuthRequired
        );
    }

    #[gpui::test]
    fn a_failed_authentication_is_surfaced(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let toasts = h.toasts(cx);
        let _session = spawn_and_connect(&h, cx);
        h.agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Authenticate {
                    method_id: "fail".to_string(),
                },
                cx,
            )
        });
        drain_until(&h, cx, |event| {
            matches!(event, AgentEvent::AuthFailed { .. })
        });
        let notice = h.chat.read_with(cx, |chat, _| {
            chat.entries().iter().rev().find_map(|entry| match entry {
                Entry::Notice(notice) if notice.level == NoticeLevel::Error => {
                    Some(notice.text.clone())
                }
                _ => None,
            })
        });
        let notice = notice.expect("el chat muestra el error");
        assert!(notice.contains("No se pudo autenticar"), "{notice}");
        let messages = toast_messages(&toasts, cx);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("No se pudo autenticar «Fake»")),
            "{messages:?}"
        );
        assert_eq!(badge_of(&h, h.id, cx), ConnectionBadge::SessionExpired);
    }

    #[gpui::test]
    fn the_agent_reported_identity_updates_the_connection(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        h.agents.update(cx, |agents, cx| {
            agents.handle_agent_event(
                AgentEvent::AuthStatus {
                    kind: AuthStatusKind::Account,
                    label: "Claude Max".to_string(),
                    detail: None,
                    account: Some(AuthAccount {
                        email: Some("otra@example.com".to_string()),
                        organization: None,
                        plan: Some("Claude Max".to_string()),
                    }),
                },
                cx,
            )
        });
        cx.run_until_parked();
        let stored = h
            .env
            .connections
            .store()
            .get(h.id)
            .unwrap()
            .identity
            .unwrap();
        assert_eq!(stored.email.as_deref(), Some("otra@example.com"));
        let shown = h.chat.read_with(cx, |chat, _| {
            chat.active_connection().and_then(|c| c.identity.clone())
        });
        assert_eq!(shown.as_deref(), Some("otra@example.com · Claude Max"));

        // `kind: none` is a logged-out profile.
        h.agents.update(cx, |agents, cx| {
            agents.handle_agent_event(
                AgentEvent::AuthStatus {
                    kind: AuthStatusKind::None,
                    label: "Not logged in".to_string(),
                    detail: None,
                    account: None,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(badge_of(&h, h.id, cx), ConnectionBadge::SessionExpired);
    }

    #[gpui::test]
    fn logout_and_elicitation_completion_reach_the_chat(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        h.agents.update(cx, |agents, cx| {
            agents.handle_agent_event(
                AgentEvent::ElicitationCompleted {
                    id: PermissionRequestId(3),
                    elicitation_id: "elic-1".to_string(),
                },
                cx,
            );
            agents.handle_agent_event(AgentEvent::LoggedOut { ok: true }, cx);
        });
        cx.run_until_parked();
        let notices: Vec<String> = h.chat.read_with(cx, |chat, _| {
            chat.entries()
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Notice(notice) => Some(notice.text.clone()),
                    _ => None,
                })
                .collect()
        });
        assert!(
            notices.iter().any(|text| text.contains("navegador")),
            "{notices:?}"
        );
        assert!(
            notices
                .iter()
                .any(|text| text.contains("Se cerró la sesión")),
            "{notices:?}"
        );
        assert_eq!(badge_of(&h, h.id, cx), ConnectionBadge::SessionExpired);
    }

    // ------------------------------------------------ no disponible (F5)

    #[gpui::test]
    fn an_unavailable_connection_is_repaired_without_touching_credentials(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let current = h.env.paths.agents_dir().join("claude-acp").join("current");
        let version = std::fs::read_to_string(&current).unwrap();
        std::fs::remove_file(&current).unwrap();
        select(&h, h.id, cx);
        assert!(matches!(
            h.chat.read_with(cx, |chat, _| chat.banner().cloned()),
            Some(ConnectionBanner::Unavailable { .. })
        ));
        assert!(
            !h.agents
                .read_with(cx, |agents, _| agents.is_agent_running())
        );

        // What "Reparar" would download comes back (offline, the tests
        // restore it by hand) and the repair finds it.
        std::fs::write(&current, version).unwrap();
        let credentials = h
            .env
            .connections
            .store()
            .profile(&h.env.connections.store().get(h.id).unwrap())
            .unwrap()
            .credentials_file();
        let before = std::fs::read(&credentials).unwrap();
        h.chat
            .update(cx, |chat, cx| chat.request_repair(&h.id.to_string(), cx));
        cx.run_until_parked();
        assert_eq!(
            h.modal(cx).read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Repaired)
        );
        with_modal(&h, cx, |modal, window, cx| modal.save(window, cx));
        let _session = drain_until_session_created(&h, cx);
        assert!(h.chat.read_with(cx, |chat, _| chat.banner().is_none()));
        assert_eq!(
            std::fs::read(&credentials).unwrap(),
            before,
            "credenciales intactas"
        );
    }

    // ------------------------------------------------------ renombrar

    #[gpui::test]
    fn renaming_updates_the_popover_and_the_history(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        h.chat.update(cx, |chat, cx| {
            chat.request_rename_connection(&h.id.to_string(), cx)
        });
        cx.run_until_parked();
        with_modal(&h, cx, |modal, window, cx| {
            modal.set_rename_text("   ", window, cx);
            modal.save_rename(window, cx);
        });
        assert!(matches!(
            h.modal(cx).read_with(cx, |modal, _| match modal.mode() {
                Mode::Rename { error, .. } => error.clone(),
                _ => None,
            }),
            Some(message) if message.contains("vacío")
        ));
        with_modal(&h, cx, |modal, window, cx| {
            modal.set_rename_text("Claude · trabajo", window, cx);
            modal.save_rename(window, cx);
        });
        assert!(!h.modal(cx).read_with(cx, |modal, _| modal.is_open()));
        assert_eq!(
            h.env.connections.store().get(h.id).unwrap().label,
            "Claude · trabajo"
        );
        let label = h
            .chat
            .read_with(cx, |chat, _| chat.connections()[0].label.clone());
        assert_eq!(label, "Claude · trabajo");
    }

    // ------------------------------------------- el pegamento ACP de siempre

    #[gpui::test]
    fn a_prompt_makes_entries_appear_in_the_transcript(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&h, cx);
        prompt(&h, session_id, "hola", cx);
        drain_until_turn_ended(&h, cx);
        let entries = h.chat.read_with(cx, |chat, _| chat.entries().len());
        assert!(entries > 0, "el transcript debería tener entradas");
    }

    #[gpui::test]
    fn a_completed_edit_reloads_the_buffer_and_marks_the_tab(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&h, cx);
        let target = dir.path().join("demo.txt");
        cx.update(|window, cx| {
            h.center
                .update(cx, |center, cx| center.open_file(&target, true, window, cx));
        });
        cx.run_until_parked();
        prompt(&h, session_id, "hola", cx);
        drain_until_turn_ended(&h, cx);
        let hunks = h.center.read_with(cx, |center, cx| {
            center
                .tabs()
                .iter()
                .find(|tab| tab.path.ends_with("demo.txt"))
                .map(|tab| tab.editor().read(cx).review().hunks.len())
                .unwrap_or(0)
        });
        assert_eq!(
            hunks, 1,
            "la pestaña debería mostrar el segmento del agente"
        );
        let on_disk = std::fs::read_to_string(&target).unwrap();
        assert!(
            on_disk.ends_with("escrito por el agente falso\n"),
            "{on_disk:?}"
        );
    }

    #[gpui::test]
    fn an_edit_permission_is_auto_answered_and_an_execute_one_reaches_the_chat(
        cx: &mut TestAppContext,
    ) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&h, cx);
        prompt(&h, session_id.clone(), "hola", cx);
        drain_until_turn_ended(&h, cx);
        assert!(
            !h.chat
                .read_with(cx, |chat, _| chat.is_awaiting_permission()),
            "las ediciones se permiten solas"
        );

        prompt(&h, session_id, "execute-permission", cx);
        drain_until(&h, cx, |event| {
            matches!(event, AgentEvent::PermissionRequest { .. })
        });
        assert!(
            h.chat
                .read_with(cx, |chat, _| chat.is_awaiting_permission()),
            "un execute tiene que mostrar la tarjeta de permiso"
        );
        h.chat.update(cx, |chat, cx| chat.accept_permission(cx));
        cx.run_until_parked();
        let request_id = h.chat.read_with(cx, |chat, _| {
            chat.entries()
                .iter()
                .find_map(|entry| match entry {
                    Entry::Permission(permission) => Some(permission.request_id),
                    _ => None,
                })
                .expect("hay un permiso en el transcript")
        });
        h.agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::RespondPermission {
                    id: PermissionRequestId(request_id),
                    outcome: PermissionOutcome::Selected(PermissionOptionId::new("allow-once")),
                },
                cx,
            );
        });
        drain_until_turn_ended(&h, cx);
        assert!(
            !h.chat
                .read_with(cx, |chat, _| chat.is_awaiting_permission())
        );
    }

    #[gpui::test]
    fn the_policy_asks_for_sensitive_paths_from_settings(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        h.agents.update(cx, |agents, _cx| {
            assert!(agents.policy.is_sensitive(Path::new("/proj/.env")));
            assert!(agents.policy.is_sensitive(Path::new("/proj/Cargo.lock")));
            let lock = [PathBuf::from("/proj/Cargo.lock")];
            let code = [PathBuf::from("/proj/src/main.rs")];
            assert_eq!(agents.policy.decide(ToolKind::Edit, &lock), AutoAnswer::Ask);
            assert_eq!(
                agents.policy.decide(ToolKind::Edit, &code),
                AutoAnswer::Allow
            );
            assert_eq!(
                agents.policy.decide(ToolKind::Execute, &code),
                AutoAnswer::Ask
            );
        });
    }

    #[gpui::test]
    fn request_file_list_returns_worktree_files(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let (h, cx) = harness(dir.path(), cx);
        h.agents.update(cx, |agents, cx| {
            agents.answer_file_list("main".to_string(), cx)
        });
        cx.update(|window, cx| {
            h.chat
                .update(cx, |chat, cx| chat.set_input_text("@main", window, cx));
        });
        cx.run_until_parked();
        let candidates = h.chat.read_with(cx, |chat, _| chat.filtered_files());
        assert!(
            candidates.iter().any(|path| path.ends_with("src/main.rs")),
            "{candidates:?}"
        );
    }

    #[gpui::test]
    fn a_conversation_round_trips_through_disk_with_its_connection(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&h, cx);
        send_from_chat(&h.chat, "hola", cx);
        drain_until_turn_ended(&h, cx);
        h.agents
            .update(cx, |agents, cx| agents.save_conversation_now(cx));
        let store = live_store(&h, cx);
        let index = store.index();
        assert_eq!(index.len(), 1, "{index:?}");
        let before = h.chat.read_with(cx, |chat, _| chat.entries().len());
        let stored = store.load(&index[0].id).expect("la conversación se lee");
        assert_eq!(stored.entries.len(), before);
        assert_eq!(stored.agent_id, "claude-acp");
        assert_eq!(stored.agent_name, "Fake");
        assert_eq!(stored.connection_id, Some(h.id.to_string()));
        assert_eq!(stored.title, "hola");
        assert_eq!(stored.session_id.as_deref(), Some(session_id.0.as_ref()));
    }

    #[gpui::test]
    fn restarting_mid_turn_ends_its_turn(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&h, cx);
        prompt(&h, session_id, "never-end", cx);
        cx.run_until_parked();
        let review = h.agents.read_with(cx, |agents, _| agents.review.clone());
        assert!(review.read_with(cx, |review, _| review.is_turn_active()));
        h.agents.update(cx, |agents, cx| agents.restart_active(cx));
        cx.run_until_parked();
        assert!(!review.read_with(cx, |review, _| review.is_turn_active()));
        assert_ne!(
            h.chat.read_with(cx, |chat, _| chat.status()),
            AgentStatus::Thinking
        );
    }

    #[gpui::test]
    fn switching_projects_restarts_the_chosen_connection_in_the_new_root(cx: &mut TestAppContext) {
        let dir_a = sample_project();
        let (h, cx) = harness(dir_a.path(), cx);
        let session_a = spawn_and_connect(&h, cx);
        prompt(&h, session_a, "hola", cx);
        drain_until_turn_ended(&h, cx);
        let store_a = live_store(&h, cx);
        let old_commands = h.agents.read_with(cx, |agents, _| {
            agents.connection.as_ref().unwrap().commands().clone()
        });

        let dir_b = sample_project();
        let project_b = open_second_project(cx, dir_b.path());
        cx.update(|window, cx| {
            h.agents.update(cx, |agents, cx| {
                agents.set_project(Some(project_b.clone()), window, cx);
            });
        });
        cx.run_until_parked();
        assert!(old_commands.send_blocking(AgentCommand::Shutdown).is_err());
        assert_eq!(h.chat.read_with(cx, |chat, _| chat.entries().len()), 0);
        let _session_b = drain_until_session_created(&h, cx);
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.sandbox_root.clone()),
            Some(dir_b.path().to_path_buf())
        );
        assert_eq!(
            h.agents
                .read_with(cx, |agents, _| agents.active_connection_id()),
            Some(h.id),
            "la conexión elegida sigue siendo la misma"
        );
        // A's conversation was flushed before the switch, bound to the
        // connection.
        let index_a = store_a.index();
        assert_eq!(index_a.len(), 1, "{index_a:?}");
        assert_eq!(index_a[0].connection_id, Some(h.id.to_string()));
    }

    #[gpui::test]
    fn switching_projects_mid_turn_cancels_it_and_warns_the_user(cx: &mut TestAppContext) {
        let dir_a = sample_project();
        let (h, cx) = harness(dir_a.path(), cx);
        let toasts = h.toasts(cx);
        let session_a = spawn_and_connect(&h, cx);
        prompt(&h, session_a, "hola", cx);
        let events = h
            .agents
            .read_with(cx, |agents, _| agents.connection_events())
            .unwrap();
        let event = events.recv_blocking().expect("evento del agente falso");
        h.agents
            .update(cx, |agents, cx| agents.handle_agent_event(event, cx));
        cx.run_until_parked();
        assert_eq!(
            h.chat.read_with(cx, |chat, _| chat.status()),
            AgentStatus::Thinking
        );
        let dir_b = sample_project();
        let project_b = open_second_project(cx, dir_b.path());
        cx.update(|window, cx| {
            h.agents.update(cx, |agents, cx| {
                agents.set_project(Some(project_b.clone()), window, cx);
            });
        });
        cx.run_until_parked();
        let messages = toast_messages(&toasts, cx);
        assert!(
            messages
                .iter()
                .any(|message| message
                    .contains("Se cerró la sesión del agente al cambiar de proyecto")),
            "{messages:?}"
        );
        let _session_b = drain_until_session_created(&h, cx);
    }

    #[gpui::test]
    fn no_project_uses_a_temp_sandbox_with_a_notice(cx: &mut TestAppContext) {
        init_test(cx);
        let center = cx.update(CenterPanel::new);
        let (chat, cx) = cx.add_window_view(|window, cx| {
            ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
        });
        let toasts = cx.update(|_, cx| cx.new(|_| crate::toast::Toasts::new()));
        cx.update(|_, cx| crate::toast::set_global(&toasts, cx));
        let agents = cx.update(|window, cx| {
            let review = cx.new(|cx| Review::new(&center, window, cx));
            cx.new(|cx| Agents::new(chat.clone(), review, false, window, cx))
        });
        let root = agents.update(cx, |agents, cx| agents.resolve_sandbox_root(cx));
        assert!(root.is_dir(), "el sandbox temporal debería existir");
        let messages = toast_messages(&toasts, cx);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("Abrí una carpeta")),
            "{messages:?}"
        );
    }

    #[gpui::test]
    fn the_old_chat_json_becomes_a_read_only_old_conversation(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let store = live_store(&h, cx);
        let base = store.dir().parent().expect("tiene padre").to_path_buf();
        std::fs::create_dir_all(&base).unwrap();
        let legacy = serde_json::json!({
            "version": 1,
            "agent_id": "fake",
            "session_id": "s-vieja",
            "autonomy": "review_after",
            "entries": [
                { "UserMessage": { "blocks": [{ "Text": "lo de antes" }] } }
            ]
        });
        std::fs::write(
            base.join("chat.json"),
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();
        let project = open_second_project(cx, dir.path());
        cx.update(|window, cx| {
            h.agents.update(cx, |agents, cx| {
                agents.set_project(Some(project), window, cx);
            });
        });
        cx.run_until_parked();
        let index = live_store(&h, cx).index();
        assert_eq!(index.len(), 1, "{index:?}");
        assert_eq!(index[0].title, "lo de antes");
        assert_eq!(index[0].connection_id, None);
        assert!(base.join("chat.json.migrado").is_file());
        let rows = h
            .chat
            .read_with(cx, |chat, _| chat.conversations().to_vec());
        assert_eq!(rows[0].group, LEGACY_CONNECTION_GROUP);
    }

    #[gpui::test]
    fn reopening_a_conversation_loads_the_session_without_duplicating_it(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        let capabilities = h
            .agents
            .read_with(cx, |agents, _| agents.capabilities.clone());
        assert!(capabilities.expect("hubo Connected").load_session);

        let store = live_store(&h, cx);
        let mut conversation = Conversation::new(
            "c-vieja",
            "claude-acp",
            "Fake",
            dir.path().to_path_buf(),
            now_seconds(),
        )
        .with_connection(h.id.to_string());
        conversation.session_id = Some("loaded-1".to_string());
        conversation.title = "vieja".to_string();
        conversation.entries = vec![
            cincel_chat::Entry::UserMessage(cincel_chat::UserMessage {
                blocks: vec![cincel_chat::MessageBlock::Text("vieja".to_string())],
            }),
            cincel_chat::Entry::AgentText(cincel_chat::AgentText {
                markdown: "primer mensaje replayadosegundo mensaje replayado".to_string(),
                streaming: false,
                view: None,
            }),
        ];
        store.save(&conversation).expect("se guarda");
        h.agents
            .update(cx, |agents, cx| agents.refresh_conversations(cx));

        h.chat
            .update(cx, |chat, cx| chat.open_conversation("c-vieja", cx));
        cx.run_until_parked();
        assert_eq!(h.chat.read_with(cx, |chat, _| chat.entries().len()), 2);
        assert!(h.chat.read_with(cx, |chat, _| chat.is_replaying()));
        let session = drain_until_session_created(&h, cx);
        assert_eq!(session.0.as_ref(), "loaded-1", "volvió la misma sesión");
        assert_eq!(h.chat.read_with(cx, |chat, _| chat.entries().len()), 2);
        assert!(
            h.chat
                .read_with(cx, |chat, _| chat.history_notice().is_none())
        );
    }

    #[gpui::test]
    fn an_agent_without_load_shows_the_read_only_notice(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (h, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&h, cx);
        let store = live_store(&h, cx);
        let mut conversation = Conversation::new(
            "c-vieja",
            "claude-acp",
            "Fake",
            dir.path().to_path_buf(),
            now_seconds(),
        )
        .with_connection(h.id.to_string());
        conversation.session_id = Some("loaded-1".to_string());
        conversation.entries = vec![cincel_chat::Entry::UserMessage(cincel_chat::UserMessage {
            blocks: vec![cincel_chat::MessageBlock::Text("vieja".to_string())],
        })];
        store.save(&conversation).expect("se guarda");
        h.agents.update(cx, |agents, _cx| {
            agents.capabilities = Some(Box::default());
        });
        cx.update(|window, cx| {
            h.agents.update(cx, |agents, cx| {
                agents.open_conversation("c-vieja", window, cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(
            h.chat
                .read_with(cx, |chat, _| chat.history_notice().map(str::to_string)),
            Some(READ_ONLY_HISTORY_NOTICE.to_string())
        );
        let session = drain_until_session_created(&h, cx);
        assert!(session.0.to_string().starts_with("fake-session-"));
    }
}
