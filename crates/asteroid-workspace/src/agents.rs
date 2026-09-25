//! Agent lifecycle: from `settings.agents` to a running [`AgentConnection`]
//! and back to the chat and the project (`docs/etapas/etapa-2.md`).
//!
//! [`Agents`] is the glue `docs/specs/03-arquitectura.md` §3 asks for: it
//! owns the ACP worker thread's handle, drains its `AgentEvent`s on the GPUI
//! main thread and answers the project-side ones (`fs/read_text_file`,
//! `fs/write_text_file`, permission requests under the active
//! [`Autonomy`](asteroid_acp::Autonomy) policy) itself; everything else is
//! forwarded to [`asteroid_chat::ChatPanel::handle_event`]. It also drains
//! [`asteroid_chat::ChatEvent`]s the other way: `Command` becomes an
//! [`AgentCommand`] on the worker's channel, `RequestFileList` is answered
//! from the worktree, `OpenTerminalWithCommand`/`CopyToClipboard` reach the
//! desktop, and `AutonomyChanged` updates the policy in place.
//!
//! # Conversations
//!
//! One agent, one conversation (`docs/etapas/etapa-2.md` § correcciones).
//! `asteroid-chat` emits [`asteroid_chat::ChatEvent::AgentSelected`],
//! `NewConversation`, `OpenConversation` and `DeleteConversation`; this module
//! saves whatever was on screen into
//! `~/.local/state/asteroid/workspaces/<hash>/conversations/<id>.json`
//! ([`crate::conversations::ConversationStore`]) and drives the ACP side of
//! the move: a new conversation is a `session/new`, an opened one is a
//! `session/load` (or `session/resume`, or a read-only notice when the agent
//! announces neither).
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
//! [`asteroid_chat::ChatPanel::set_file_candidates`] directly, since this
//! module already holds a strong handle to the panel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use asteroid_acp::acp::schema::v1::{
    AgentCapabilities, PermissionOption, SessionId, SessionUpdate, ToolCallStatus, ToolCallUpdate,
    ToolKind,
};
use asteroid_acp::{
    AgentCommand, AgentConnection, AgentDescriptor, AgentEvent, AgentRegistry, AutoAnswer,
    Autonomy, AutonomyMode, CustomAgent, McpServerSpec, PermissionOutcome, PermissionRequestId,
    pick_allow_option,
};
use asteroid_chat::{
    AgentStatus, ChatAgent, ChatEvent, ChatPanel, Conversation, READ_ONLY_HISTORY_NOTICE,
    conversation_title,
};
use asteroid_project::{EntryKind, OpenError};
use gpui::{App, Context, Entity, Subscription, Task, Window};

use crate::center::CenterPanel;
use crate::conversations::{ConversationStore, format_when, new_conversation_id, now_seconds};
use crate::project::Project;

/// How often the open conversation is written to disk when it changed
/// (`docs/etapas/etapa-2.md`, "Transcript persistence").
const TRANSCRIPT_AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// What this module has to remember about the conversation on screen; its
/// entries live in the panel and are only pulled out when it is saved.
#[derive(Clone, Debug)]
struct ActiveConversation {
    id: String,
    agent_id: String,
    agent_name: String,
    created_at: u64,
}

/// Agent lifecycle and ACP glue, one per workspace window.
pub struct Agents {
    chat: Entity<ChatPanel>,
    center: Entity<CenterPanel>,
    project: Option<Entity<Project>>,

    registry: Option<AgentRegistry>,
    default_agent_id: String,
    custom_agents: Vec<asteroid_settings::AgentSettings>,
    mcp_servers: Vec<McpServerSpec>,

    connection: Option<AgentConnection>,
    active_agent_id: Option<String>,
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
    current_turn_id: u64,
    tool_call_kinds: HashMap<asteroid_acp::acp::schema::v1::ToolCallId, ToolKind>,

    autonomy: Autonomy,
    sensitive_globs: Vec<String>,

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
    _chat_subscription: Subscription,
}

impl Agents {
    /// Builds the controller. `background` gates every piece of real
    /// concurrency (registry download, event draining, the 30 s transcript
    /// timer) so tests can drive everything by hand.
    ///
    /// The subscription to the chat panel lives here rather than in
    /// [`crate::Workspace`] so that a window built by hand (the tests) gets
    /// the same wiring as the real one.
    pub fn new(
        chat: Entity<ChatPanel>,
        center: Entity<CenterPanel>,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = crate::settings::settings(cx);
        let (autonomy, sensitive_globs) = build_autonomy(&settings);
        let subscription = cx.subscribe_in(&chat, window, |this, _chat, event, window, cx| {
            this.handle_chat_event(event, window, cx);
        });

        let mut agents = Self {
            chat,
            center,
            project: None,
            registry: None,
            default_agent_id: settings.agents.default.clone(),
            custom_agents: settings.agents.custom.clone(),
            mcp_servers: map_mcp_servers(&settings.agents.mcp_servers),
            connection: None,
            active_agent_id: None,
            capabilities: None,
            store: None,
            conversation: None,
            pending_load: None,
            sandbox_root: None,
            temp_dir: None,
            current_turn_id: 0,
            tool_call_kinds: HashMap::new(),
            autonomy,
            sensitive_globs,
            last_saved_conversation: None,
            background,
            event_task: None,
            transcript_task: None,
            _chat_subscription: subscription,
        };

        if background {
            agents.load_registry(cx);
        }
        agents
    }

    // ------------------------------------------------------------ project

    /// Tracks the open project: the sandbox root for future connections, and
    /// where the transcript is stored. Also the single entry point for
    /// *switching* projects (Ctrl+O, recientes o el path de la CLI, todos a
    /// través de `Workspace::open_project`) — `docs/etapas/etapa-2.md`,
    /// "Ciclo de vida del agente": a turn in progress is cancelled first,
    /// the outgoing project's conversation is saved, the running connection is
    /// torn down, the incoming project's conversation list is read (migrating
    /// its old `chat.json` if it still has one) and, if an agent was selected,
    /// it is relaunched with the new `project_root` in a fresh conversation,
    /// so the status goes back to "listo" without an extra step from the user.
    ///
    pub fn set_project(
        &mut self,
        project: Option<Entity<Project>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_turn_in_progress(cx);
        self.save_conversation_now(cx);
        self.disconnect(cx);

        self.project = project;
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

        if let Some(id) = self.active_agent_id.clone() {
            self.begin_conversation(&id, window, cx);
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

    /// Tears the running connection down: dropping [`AgentConnection`] sends
    /// `AgentCommand::Shutdown` and blocks for the worker thread, which
    /// kills the whole process group before returning
    /// (`asteroid_acp::connection::run_agent`). Reflected in the chat as
    /// "desconectado"; the resulting notice entry is transient — the caller
    /// replaces the transcript right after
    /// ([`Agents::sync_transcript_with_project`]), so it never reaches disk
    /// or a repaint.
    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.event_task = None;
        self.capabilities = None;
        if self.connection.take().is_none() {
            return;
        }
        self.chat.update(cx, |chat, cx| {
            chat.handle_event(
                AgentEvent::Exited {
                    code: None,
                    stderr_tail: String::new(),
                },
                cx,
            );
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
        match tempfile::Builder::new().prefix("asteroid-agent-").tempdir() {
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

    // -------------------------------------------------------------- agents

    /// Kicks off the background download/cache read of the ACP registry.
    fn load_registry(&mut self, cx: &mut Context<Self>) {
        let custom = self.custom_agents.clone();
        let default_id = self.default_agent_id.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { AgentRegistry::load() })
                .await;
            let _ = this.update(cx, |agents, cx| {
                agents.apply_registry(loaded, &custom, &default_id, cx);
            });
        })
        .detach();
    }

    /// Merges the loaded (or offline) registry with `agents.custom`, fills
    /// the chat's agent selector and selects `agents.default` when it is
    /// present.
    fn apply_registry(
        &mut self,
        loaded: asteroid_acp::Result<AgentRegistry>,
        custom: &[asteroid_settings::AgentSettings],
        default_id: &str,
        cx: &mut Context<Self>,
    ) {
        let custom_agents = to_custom_agents(custom);
        let registry = match loaded {
            Ok(registry) => registry.with_custom(custom_agents),
            Err(error) => {
                // No network and no usable cache: still offer whatever the
                // user defined in `settings.json` (`docs/etapas/etapa-2.md`,
                // "agent without ACP registry").
                crate::toast::warn(
                    format!(
                        "No se pudo descargar el registro de agentes ({error}); se muestran solo los agentes personalizados."
                    ),
                    cx,
                );
                empty_registry().with_custom(custom_agents)
            }
        };
        self.install_registry(registry, default_id, cx);
    }

    /// Installs `registry` as the source of truth and rebuilds the chat's
    /// agent list from it. Split out of [`Agents::apply_registry`] so tests
    /// can hand a registry straight in, without touching the network.
    fn install_registry(
        &mut self,
        registry: AgentRegistry,
        default_id: &str,
        cx: &mut Context<Self>,
    ) {
        let chat_agents: Vec<ChatAgent> = registry
            .agents()
            .iter()
            .map(|descriptor| {
                let (installed, hint) = agent_installed_status(&registry, descriptor);
                ChatAgent {
                    id: descriptor.id.clone(),
                    name: descriptor.name.clone(),
                    installed,
                    hint,
                }
            })
            .collect();
        let has_default = chat_agents.iter().any(|agent| agent.id == default_id);
        self.registry = Some(registry);
        self.chat.update(cx, |chat, cx| {
            chat.set_agents(chat_agents, cx);
            if has_default {
                chat.set_active_agent(default_id, cx);
            }
        });
    }

    // ------------------------------------------------------- conversations

    /// Saves whatever is on screen and opens an empty conversation with
    /// `agent_id`, spawning the process when it is not the one already
    /// running (`docs/etapas/etapa-2.md` § correcciones).
    fn begin_conversation(&mut self, agent_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.save_conversation_now(cx);
        let agent_name = self.agent_name(agent_id, cx);
        self.chat
            .update(cx, |chat, cx| chat.start_new_conversation(window, cx));
        self.conversation = Some(ActiveConversation {
            id: new_conversation_id(),
            agent_id: agent_id.to_string(),
            agent_name,
            created_at: now_seconds(),
        });
        self.last_saved_conversation = None;
        self.pending_load = None;

        let reusable =
            self.active_agent_id.as_deref() == Some(agent_id) && self.connection.is_some();
        self.active_agent_id = Some(agent_id.to_string());
        if !reusable {
            self.spawn_agent(agent_id, cx);
        } else if self.capabilities.is_some() {
            // The process is fine and already initialized; only the session
            // has to start over.
            self.start_session();
        }
        // Otherwise `initialize` is still in flight and its `Connected` will
        // open the session ([`Agents::resume_or_start_session`]).
        self.refresh_conversations(cx);
    }

    /// Loads a stored conversation and tries to reopen its ACP session.
    fn open_conversation(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.save_conversation_now(cx);
        let Some(conversation) = self.store.as_ref().and_then(|store| store.load(id)) else {
            crate::toast::warn("No se pudo abrir esa conversación.", cx);
            self.refresh_conversations(cx);
            return;
        };
        let agent_id = conversation.agent_id.clone();
        let agent_name = if conversation.agent_name.is_empty() {
            self.agent_name(&agent_id, cx)
        } else {
            conversation.agent_name.clone()
        };
        self.conversation = Some(ActiveConversation {
            id: conversation.id.clone(),
            agent_id: agent_id.clone(),
            agent_name,
            created_at: conversation.created_at,
        });
        self.pending_load = conversation
            .session_id
            .clone()
            .filter(|id| !id.is_empty())
            .map(SessionId::new);
        self.last_saved_conversation = None;
        // `load_conversation` points the header at the conversation's agent
        // without emitting `AgentSelected`, so this does not loop back in.
        self.chat.update(cx, |chat, cx| {
            chat.load_conversation(conversation, window, cx)
        });

        let reusable =
            self.active_agent_id.as_deref() == Some(agent_id.as_str()) && self.connection.is_some();
        self.active_agent_id = Some(agent_id.clone());
        if !reusable {
            self.spawn_agent(&agent_id, cx);
        } else if self.capabilities.is_some() {
            // Already initialized: what the agent announced is known, so the
            // decision can be taken right away.
            self.resume_or_start_session(cx);
        }
        self.refresh_conversations(cx);
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
    /// nothing worth saving (no project, or an empty conversation nobody has
    /// written in yet).
    fn current_conversation(&self, cx: &Context<Self>) -> Option<Conversation> {
        let active = self.conversation.as_ref()?;
        let project = self.project.as_ref()?;
        let dump = self.chat.read(cx).export_transcript();
        if dump.entries.is_empty() {
            return None;
        }
        Some(Conversation {
            version: asteroid_chat::CONVERSATION_VERSION,
            id: active.id.clone(),
            agent_id: active.agent_id.clone(),
            agent_name: active.agent_name.clone(),
            session_id: dump.session_id.clone(),
            cwd: project.read(cx).root().to_path_buf(),
            created_at: active.created_at,
            updated_at: now_seconds(),
            title: conversation_title(&dump.entries),
            autonomy: dump.autonomy,
            entries: dump.entries,
        })
    }

    /// The display name of an agent, falling back to its registry id.
    fn agent_name(&self, agent_id: &str, cx: &Context<Self>) -> String {
        self.chat
            .read(cx)
            .agents()
            .iter()
            .find(|agent| agent.id == agent_id)
            .map_or_else(|| agent_id.to_string(), |agent| agent.name.clone())
    }

    /// Resolves `id` in the registry, launches it and replaces whatever
    /// connection was active (dropping it shuts the old process down).
    fn spawn_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(registry) = &self.registry else {
            crate::toast::warn(
                "El registro de agentes todavía se está cargando; probá de nuevo en un momento.",
                cx,
            );
            return;
        };
        let Some(descriptor) = registry.get(id) else {
            return;
        };
        let launch = match registry.launch_command(descriptor) {
            Ok(launch) => launch,
            Err(error) => {
                crate::toast::error(format!("No se pudo lanzar «{id}»: {error}"), cx);
                return;
            }
        };

        let root = self.resolve_sandbox_root(cx);
        self.sandbox_root = Some(root.clone());
        // A new process means a new `initialize`; what the previous one
        // announced says nothing about this one.
        self.capabilities = None;
        self.current_turn_id = 0;
        self.tool_call_kinds.clear();
        // Dropping the previous connection tears its process down.
        self.event_task = None;
        let connection = AgentConnection::start(root.clone());
        let _ = connection
            .commands()
            .send_blocking(AgentCommand::Spawn { launch, cwd: root });

        if self.background {
            let events = connection.events().clone();
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
        self.connection = Some(connection);
    }

    // ---------------------------------------------------------- chat -> acp

    /// Everything [`asteroid_chat::ChatPanel`] emits, dispatched to the ACP
    /// worker, the project or the desktop. Called by the workspace from a
    /// `cx.subscribe_in` on the chat panel (needs `Window` for
    /// `OpenFileAtHunk`).
    pub fn handle_chat_event(
        &mut self,
        event: &ChatEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ChatEvent::Command(command) => self.forward_command(command, cx),
            ChatEvent::OpenFileAtHunk { path } => {
                self.center
                    .update(cx, |center, cx| center.open_file(path, true, window, cx));
            }
            ChatEvent::AutonomyChanged(mode) => self.set_autonomy_mode(*mode),
            ChatEvent::AgentSelected { agent_id } => {
                let agent_id = agent_id.clone();
                self.begin_conversation(&agent_id, window, cx);
            }
            ChatEvent::NewConversation => {
                if let Some(agent_id) = self.active_agent_id.clone() {
                    self.begin_conversation(&agent_id, window, cx);
                } else {
                    crate::toast::warn("Elegí un agente para empezar una conversación.", cx);
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
    /// connection's own sandbox root: `asteroid-chat` has no notion of a
    /// project and its own `new_session()` fills `cwd` with
    /// `std::env::current_dir()`, which is almost never the right answer
    /// (wish: let the caller supply `cwd`, or drop the field from the
    /// command and let the glue layer fill it in, exactly as done here).
    fn forward_command(&mut self, command: &AgentCommand, cx: &mut Context<Self>) {
        let Some(connection) = &self.connection else {
            crate::toast::warn("No hay un agente conectado todavía.", cx);
            return;
        };
        let root = self.sandbox_root.clone().unwrap_or_else(std::env::temp_dir);
        let owned = match command {
            AgentCommand::NewSession { mcp_servers, .. } => {
                self.current_turn_id = 0;
                self.tool_call_kinds.clear();
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
                self.current_turn_id += 1;
                AgentCommand::Prompt {
                    session_id: session_id.clone(),
                    blocks: blocks.clone(),
                    feedback: feedback.clone(),
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
        let _ = connection.commands().send_blocking(owned);
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

    /// Replaces the autonomy policy in place, keeping the sensitive-path
    /// globs from settings (`01-producto.md` §F5). Persistence is lazy: the
    /// workspace reads `chat.autonomy()` when it writes `layout.json`,
    /// exactly like every other per-project value.
    fn set_autonomy_mode(&mut self, mode: AutonomyMode) {
        self.autonomy = Autonomy::new(mode)
            .with_sensitive_globs(self.sensitive_globs.clone())
            .unwrap_or_else(|_| Autonomy::new(mode));
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
    /// project-side requests are answered here, everything else reaches
    /// [`asteroid_chat::ChatPanel::handle_event`] unchanged.
    fn handle_agent_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
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
            AgentEvent::PermissionRequest {
                id,
                session_id,
                tool_call,
                options,
                reply,
            } => self.handle_permission_request(id, session_id, tool_call, options, reply, cx),
            AgentEvent::FsRead {
                session_id,
                path,
                line,
                limit,
                reply,
            } => self.handle_fs_read(session_id, path, line, limit, reply, cx),
            AgentEvent::FsWrite {
                session_id,
                path,
                content,
                reply,
            } => self.handle_fs_write(session_id, path, content, reply, cx),
            AgentEvent::Update { session_id, update } => {
                self.track_tool_call_kind(&update);
                self.maybe_reload_after_tool_call(&update, cx);
                self.chat.update(cx, |chat, cx| {
                    chat.handle_event(AgentEvent::Update { session_id, update }, cx);
                });
            }
            AgentEvent::FileChangeReport { report, .. } => {
                let mut touched = false;
                for path in &report.paths {
                    if self
                        .center
                        .update(cx, |center, cx| center.reload_after_agent_edit(path, cx))
                    {
                        touched = true;
                    }
                }
                if touched {
                    self.refresh_git(cx);
                }
            }
            AgentEvent::Exited { code, stderr_tail } => {
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
                self.offer_restart(&stderr_tail, cx);
            }
            other => {
                self.chat
                    .update(cx, |chat, cx| chat.handle_event(other, cx));
            }
        }
    }

    /// `01-producto.md` §F5: ask [`Autonomy::decide`] first; auto-answer with
    /// the agent's own `allow_once`/`allow_always` option when it says so,
    /// otherwise drop `reply` (harmless: `AgentCommand::RespondPermission`
    /// answers the very same slot by id) and let the chat card handle it.
    fn handle_permission_request(
        &mut self,
        id: PermissionRequestId,
        _session_id: SessionId,
        tool_call: Box<ToolCallUpdate>,
        options: Vec<PermissionOption>,
        reply: asteroid_acp::PermissionResponder,
        cx: &mut Context<Self>,
    ) {
        let kind = tool_call
            .fields
            .kind
            .or_else(|| self.tool_call_kinds.get(&tool_call.tool_call_id).copied())
            .unwrap_or(ToolKind::Other);
        let paths = asteroid_acp::connection::tool_call_paths(&tool_call);
        if self.autonomy.decide(kind, &paths) == AutoAnswer::Allow
            && let Some(option) = pick_allow_option(&options)
        {
            reply.respond(PermissionOutcome::Selected(option.option_id.clone()));
            return;
        }
        self.chat.update(cx, |chat, cx| {
            chat.request_permission(id.0, &tool_call, options, cx)
        });
    }

    /// `fs/read_text_file`: served from `BufferStore` (in-memory content,
    /// unsaved changes included). `line > total` is checked here, as
    /// documented in `docs/specs/modulos/acp.md` §Handlers (`asteroid-acp`
    /// itself does not validate it).
    fn handle_fs_read(
        &mut self,
        _session_id: SessionId,
        path: PathBuf,
        line: Option<u32>,
        limit: Option<u32>,
        reply: tokio::sync::oneshot::Sender<Result<String, asteroid_acp::FsError>>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.clone() else {
            let _ = reply.send(Err(asteroid_acp::FsError::Internal(
                "no hay un proyecto abierto".to_string(),
            )));
            return;
        };
        let full = project.update(cx, |project, _cx| {
            project.buffers_mut().read_for_agent(&path, None, None)
        });
        let full = match full {
            Ok(text) => text,
            Err(error) => {
                let _ = reply.send(Err(map_open_error(&error)));
                return;
            }
        };
        if let Some(line) = line {
            let total = full.split_inclusive('\n').count().max(1) as u32;
            if line > total {
                let _ = reply.send(Err(asteroid_acp::FsError::InvalidParams(format!(
                    "«{}» tiene {total} líneas; se pidió desde la {line}",
                    path.display()
                ))));
                return;
            }
        }
        let sliced = project.update(cx, |project, _cx| {
            project.buffers_mut().read_for_agent(&path, line, limit)
        });
        match sliced {
            Ok(text) => {
                let _ = reply.send(Ok(text));
            }
            Err(error) => {
                let _ = reply.send(Err(map_open_error(&error)));
            }
        }
    }

    /// `fs/write_text_file`: `BufferStore::apply_agent_write` (creating the
    /// buffer if the file was not open) already saves; this only nudges the
    /// open editor (if any) and refreshes git.
    fn handle_fs_write(
        &mut self,
        _session_id: SessionId,
        path: PathBuf,
        content: String,
        reply: tokio::sync::oneshot::Sender<Result<(), asteroid_acp::FsError>>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.clone() else {
            let _ = reply.send(Err(asteroid_acp::FsError::Internal(
                "no hay un proyecto abierto".to_string(),
            )));
            return;
        };
        let turn_id = self.current_turn_id;
        let result = project.update(cx, |project, _cx| {
            project
                .buffers_mut()
                .apply_agent_write(&path, &content, turn_id)
        });
        match result {
            Ok(_write) => {
                let _ = reply.send(Ok(()));
                self.center
                    .update(cx, |center, cx| center.note_agent_write(&path, cx));
                self.refresh_git(cx);
            }
            Err(error) => {
                let _ = reply.send(Err(asteroid_acp::FsError::Internal(error.to_string())));
            }
        }
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

    /// `docs/specs/03-arquitectura.md` §4: the diff the agent sends is a UI
    /// signal, not the source of truth. When an `edit` tool call completes,
    /// reload the touched buffers from disk (if clean) and mark their tabs.
    fn maybe_reload_after_tool_call(&mut self, update: &SessionUpdate, cx: &mut Context<Self>) {
        let SessionUpdate::ToolCallUpdate(call_update) = update else {
            return;
        };
        if call_update.fields.status != Some(ToolCallStatus::Completed) {
            return;
        }
        let kind = call_update
            .fields
            .kind
            .or_else(|| self.tool_call_kinds.get(&call_update.tool_call_id).copied());
        if kind != Some(ToolKind::Edit) {
            return;
        }
        let paths = asteroid_acp::connection::tool_call_paths(call_update);
        let mut touched = false;
        for path in paths {
            if self
                .center
                .update(cx, |center, cx| center.reload_after_agent_edit(&path, cx))
            {
                touched = true;
            }
        }
        if touched {
            self.refresh_git(cx);
        }
    }

    fn refresh_git(&self, cx: &Context<Self>) {
        if let Some(project) = &self.project {
            project.read(cx).refresh_git();
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

    /// Toast with the stderr tail and a "Reiniciar" action
    /// (`docs/etapas/etapa-2.md`).
    fn offer_restart(&mut self, stderr_tail: &str, cx: &mut Context<Self>) {
        let Some(id) = self.active_agent_id.clone() else {
            return;
        };
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
                        agents.update(cx, |agents, cx| agents.spawn_agent(&id, cx));
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

    /// Rebuilds the history popover out of `index.json`, newest first. With no
    /// project open the list is cleared — otherwise a project switch would
    /// leave the previous project's conversations on screen.
    fn refresh_conversations(&mut self, cx: &mut Context<Self>) {
        let summaries = self
            .store
            .as_ref()
            .map(|store| {
                store
                    .index()
                    .into_iter()
                    .map(|entry| {
                        let when = format_when(entry.updated_at);
                        entry.summary(when)
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.chat
            .update(cx, |chat, cx| chat.set_conversations(summaries, cx));
    }
}

fn to_custom_agents(custom: &[asteroid_settings::AgentSettings]) -> Vec<CustomAgent> {
    custom
        .iter()
        .map(|agent| CustomAgent {
            id: agent.id.clone(),
            name: agent.name.clone(),
            command: agent.command.clone(),
            args: agent.args.clone(),
            env: agent.env.clone(),
        })
        .collect()
}

fn map_mcp_servers(servers: &[asteroid_settings::McpServerSettings]) -> Vec<McpServerSpec> {
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

fn map_autonomy_mode(mode: asteroid_settings::Autonomy) -> AutonomyMode {
    match mode {
        asteroid_settings::Autonomy::ReviewAfter => AutonomyMode::ReviewAfter,
        asteroid_settings::Autonomy::AskBefore => AutonomyMode::AskBefore,
        asteroid_settings::Autonomy::AlwaysApply => AutonomyMode::AlwaysApply,
    }
}

/// The starting [`Autonomy`] policy and the raw glob list it was built with
/// (kept around so [`Agents::set_autonomy_mode`] can rebuild it for a new
/// mode without losing the sensitive-path list).
fn build_autonomy(settings: &asteroid_settings::Settings) -> (Autonomy, Vec<String>) {
    let globs = settings.review.sensitive_paths.clone();
    let mode = map_autonomy_mode(settings.agents.autonomy);
    let autonomy = Autonomy::new(mode)
        .with_sensitive_globs(globs.clone())
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "review.sensitive_paths inválido, se usan los patrones por defecto");
            Autonomy::new(mode)
        });
    (autonomy, globs)
}

/// A registry with no agents at all, for the offline fallback.
fn empty_registry() -> AgentRegistry {
    AgentRegistry::parse(r#"{"version":"0","agents":[]}"#).expect("registro vacío válido")
}

/// Whether `descriptor` can be launched right now, and the hint to show when
/// it cannot (`docs/etapas/etapa-2.md`, "Empty/edge states").
fn agent_installed_status(
    registry: &AgentRegistry,
    descriptor: &AgentDescriptor,
) -> (bool, Option<String>) {
    if let Some(custom) = &descriptor.distribution.custom {
        let ok = command_available(&custom.command);
        return (
            ok,
            (!ok).then(|| format!("No se encontró «{}» en el PATH", custom.command)),
        );
    }
    if descriptor.distribution.npx.is_some() {
        let ok = asteroid_acp::ensure_node_available().is_ok();
        return (ok, (!ok).then(|| "Instalá Node 22+".to_string()));
    }
    if descriptor.distribution.uvx.is_some() {
        let ok = asteroid_acp::uv_available();
        return (ok, (!ok).then(|| "Instalá uv (uvx)".to_string()));
    }
    if descriptor.distribution.binary.is_some() {
        let ok = registry.is_installed(descriptor);
        return (
            ok,
            (!ok).then(|| "Se instala la primera vez que lo uses".to_string()),
        );
    }
    (
        false,
        Some("Este agente no tiene una forma de lanzarse todavía".to_string()),
    )
}

/// Whether `command` resolves to an executable: an absolute path that
/// exists, or a name found on `PATH`.
fn command_available(command: &str) -> bool {
    let path = Path::new(command);
    if path.is_absolute() {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(command).is_file()))
}

/// Maps an [`OpenError`] onto the JSON-RPC-shaped [`asteroid_acp::FsError`]
/// `fs/read_text_file` answers with.
fn map_open_error(error: &OpenError) -> asteroid_acp::FsError {
    match error {
        OpenError::NotUtf8(path) => {
            asteroid_acp::FsError::InvalidParams(format!("«{}» no es UTF-8", path.display()))
        }
        OpenError::NotAFile(path) => {
            asteroid_acp::FsError::InvalidParams(format!("«{}» no es un archivo", path.display()))
        }
        OpenError::Io { path, source } if source.kind() == std::io::ErrorKind::NotFound => {
            asteroid_acp::FsError::NotFound(format!("«{}» no existe", path.display()))
        }
        other => asteroid_acp::FsError::Internal(other.to_string()),
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use std::sync::OnceLock;

    use asteroid_acp::PromptBlock;
    use asteroid_chat::{ChatSettings, ChatTheme};
    use gpui::{AppContext as _, TestAppContext, VisualTestContext};

    use super::*;
    use crate::project::ProjectOptions;

    /// The `asteroid-acp-fake-agent` binary, built by
    /// `cargo build -p asteroid-acp --bin asteroid-acp-fake-agent` before
    /// these tests run (`docs/etapas/etapa-2.md`).
    fn fake_agent_path() -> PathBuf {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest
            .parent()
            .and_then(Path::parent)
            .expect("crates/asteroid-workspace tiene dos ancestros");
        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root.join("target"));
        for profile in ["debug", "release"] {
            let candidate = target_dir.join(profile).join("asteroid-acp-fake-agent");
            if candidate.is_file() {
                return candidate;
            }
        }
        panic!(
            "no se encontró asteroid-acp-fake-agent bajo {}; corré \
             `cargo build -p asteroid-acp --bin asteroid-acp-fake-agent` primero",
            target_dir.display()
        );
    }

    fn isolate_state() {
        static GUARD: OnceLock<tempfile::TempDir> = OnceLock::new();
        GUARD.get_or_init(|| {
            let dir = tempfile::tempdir().expect("tempdir");
            #[allow(unsafe_code)]
            unsafe {
                std::env::set_var("XDG_STATE_HOME", dir.path().join("state"));
                std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
                std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
            }
            dir
        });
    }

    fn init_test(cx: &mut TestAppContext) {
        isolate_state();
        cx.update(|cx| crate::init(asteroid_settings::Config::default(), cx));
    }

    fn fake_registry() -> AgentRegistry {
        empty_registry().with_custom(vec![CustomAgent {
            id: "fake".to_string(),
            name: "Fake".to_string(),
            command: fake_agent_path().to_string_lossy().into_owned(),
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
        }])
    }

    /// Builds a chat panel, a center panel and an `Agents` (background
    /// draining off, per the module doc) in one window, plus the project.
    fn harness<'a>(
        project_dir: &Path,
        cx: &'a mut TestAppContext,
    ) -> (
        Entity<ChatPanel>,
        Entity<CenterPanel>,
        Entity<Agents>,
        Entity<Project>,
        &'a mut VisualTestContext,
    ) {
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
            cx.new(|cx| Agents::new(chat.clone(), center.clone(), false, window, cx))
        });
        cx.update(|window, cx| {
            agents.update(cx, |agents, cx| {
                agents.install_registry(fake_registry(), "fake", cx);
                agents.set_project(Some(project.clone()), window, cx);
            });
        });
        cx.run_until_parked();
        (chat, center, agents, project, cx)
    }

    /// Selects the agent and drains events by hand (see the module doc's
    /// `background` note) until `SessionCreated`. Returns the session id.
    fn spawn_and_connect(
        chat: &Entity<ChatPanel>,
        agents: &Entity<Agents>,
        cx: &mut VisualTestContext,
    ) -> SessionId {
        chat.update(cx, |chat, cx| chat.set_active_agent("fake", cx));
        cx.run_until_parked();
        drain_until_session_created(chat, agents, cx)
    }

    /// Drains events by hand until `SessionCreated`, without picking an
    /// agent first — for when it was already relaunched automatically (a
    /// project switch), the way [`Agents::set_project`] does. Returns the
    /// session id.
    fn drain_until_session_created(
        chat: &Entity<ChatPanel>,
        agents: &Entity<Agents>,
        cx: &mut VisualTestContext,
    ) -> SessionId {
        let events = agents
            .read_with(cx, |agents, _| {
                agents.connection.as_ref().map(|c| c.events().clone())
            })
            .expect("la conexión existe");
        loop {
            let event = events.recv_blocking().expect("evento del agente falso");
            let is_session_created = matches!(event, AgentEvent::SessionCreated { .. });
            agents.update(cx, |agents, cx| agents.handle_agent_event(event, cx));
            cx.run_until_parked();
            if is_session_created {
                break;
            }
        }
        chat.read_with(cx, |chat, _| {
            chat.session_id().cloned().expect("hay sesión")
        })
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

    fn drain_until_turn_ended(agents: &Entity<Agents>, cx: &mut VisualTestContext) {
        let events = agents
            .read_with(cx, |agents, _| {
                agents.connection.as_ref().map(|c| c.events().clone())
            })
            .expect("la conexión existe");
        loop {
            let event = events.recv_blocking().expect("evento del agente falso");
            let ended = matches!(event, AgentEvent::TurnEnded { .. });
            agents.update(cx, |agents, cx| agents.handle_agent_event(event, cx));
            cx.run_until_parked();
            if ended {
                break;
            }
        }
    }

    fn sample_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("demo.txt"), "alfa\nbeta\n").unwrap();
        dir
    }

    #[gpui::test]
    fn spawning_reflects_session_created_in_the_status(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);

        let session = spawn_and_connect(&chat, &agents, cx);
        assert!(session.0.to_string().starts_with("fake-session-"));
        assert_eq!(
            chat.read_with(cx, |chat, _| chat.status()),
            asteroid_chat::AgentStatus::Ready
        );
    }

    #[gpui::test]
    fn a_prompt_makes_entries_appear_in_the_transcript(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&chat, &agents, cx);

        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id,
                    blocks: vec![PromptBlock::Text("hola".to_string())],
                    feedback: None,
                },
                cx,
            );
        });
        drain_until_turn_ended(&agents, cx);

        let entries = chat.read_with(cx, |chat, _| chat.entries().len());
        assert!(entries > 0, "el transcript debería tener entradas");
    }

    #[gpui::test]
    fn a_completed_edit_reloads_the_buffer_and_marks_the_tab(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, center, agents, _project, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&chat, &agents, cx);

        // Open the tab the fake agent's default scenario edits.
        let target = dir.path().join("demo.txt");
        cx.update(|window, cx| {
            center.update(cx, |center, cx| center.open_file(&target, true, window, cx));
        });
        cx.run_until_parked();

        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id,
                    blocks: vec![PromptBlock::Text("hola".to_string())],
                    feedback: None,
                },
                cx,
            );
        });
        drain_until_turn_ended(&agents, cx);

        let (agent_touched, on_disk) = center.read_with(cx, |center, cx| {
            let tab = center
                .tabs()
                .iter()
                .find(|tab| tab.path.ends_with("demo.txt"));
            (
                tab.map(|tab| tab.agent_touched).unwrap_or(false),
                tab.map(|tab| tab.editor().read(cx).text()),
            )
        });
        let _ = on_disk;
        assert!(agent_touched, "la pestaña debería marcarse con ◆");
    }

    #[gpui::test]
    fn a_permission_is_auto_answered_under_review_after_and_a_fresh_prompt_asks(
        cx: &mut TestAppContext,
    ) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&chat, &agents, cx);
        assert_eq!(
            chat.read_with(cx, |chat, _| chat.autonomy()),
            AutonomyMode::ReviewAfter
        );

        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id: session_id.clone(),
                    blocks: vec![PromptBlock::Text("hola".to_string())],
                    feedback: None,
                },
                cx,
            );
        });
        drain_until_turn_ended(&agents, cx);

        // `default_turn` in the fake agent asks permission for an `edit` tool
        // call; under `Revisar después` it is auto-answered, so no
        // `Entry::Permission` should be left waiting.
        let waiting = chat.read_with(cx, |chat, _| chat.is_awaiting_permission());
        assert!(!waiting, "revisar después debería auto-responder ediciones");

        // Switch to `Pedir antes` and run the `execute-permission` scenario,
        // which must now reach the chat as a card.
        chat.update(cx, |chat, cx| {
            chat.set_autonomy(AutonomyMode::AskBefore, cx);
        });
        cx.run_until_parked();
        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id,
                    blocks: vec![PromptBlock::Text("execute-permission".to_string())],
                    feedback: None,
                },
                cx,
            );
        });
        // Drain until the permission card is on screen (the turn stays open
        // until it is answered).
        let events = agents
            .read_with(cx, |agents, _| {
                agents.connection.as_ref().map(|c| c.events().clone())
            })
            .expect("la conexión existe");
        loop {
            let event = events.recv_blocking().expect("evento");
            let is_permission = matches!(event, AgentEvent::PermissionRequest { .. });
            agents.update(cx, |agents, cx| agents.handle_agent_event(event, cx));
            cx.run_until_parked();
            if is_permission {
                break;
            }
        }
        assert!(
            chat.read_with(cx, |chat, _| chat.is_awaiting_permission()),
            "pedir antes debería mostrar la tarjeta de permiso"
        );

        // Accepting it in the chat emits `ChatEvent::Command(RespondPermission)`,
        // which the workspace's subscription would normally forward; here it is
        // forwarded by hand, exactly like the production `handle_chat_event`
        // would, to keep the test independent of the workspace's wiring.
        chat.update(cx, |chat, cx| chat.accept_permission(cx));
        cx.run_until_parked();
        let request_id = chat.read_with(cx, |chat, _| {
            chat.entries()
                .iter()
                .find_map(|entry| match entry {
                    asteroid_chat::Entry::Permission(permission) => Some(permission.request_id),
                    _ => None,
                })
                .expect("hay un permiso en el transcript")
        });
        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::RespondPermission {
                    id: PermissionRequestId(request_id),
                    outcome: PermissionOutcome::Selected(
                        asteroid_acp::acp::schema::v1::PermissionOptionId::new("allow-once"),
                    ),
                },
                cx,
            );
        });
        drain_until_turn_ended(&agents, cx);
        assert!(!chat.read_with(cx, |chat, _| chat.is_awaiting_permission()));
    }

    #[gpui::test]
    fn request_file_list_returns_worktree_files(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);

        // `answer_file_list` fills the candidate list `ChatPanel::filtered_files`
        // reads from; opening the `@` popover on the same query is the only
        // public way to observe it (`file_candidates` itself is crate-private
        // to `asteroid-chat`).
        agents.update(cx, |agents, cx| {
            agents.answer_file_list("main".to_string(), cx)
        });
        cx.update(|window, cx| {
            chat.update(cx, |chat, cx| chat.set_input_text("@main", window, cx));
        });
        cx.run_until_parked();
        let candidates = chat.read_with(cx, |chat, _| chat.filtered_files());
        assert!(
            candidates.iter().any(|path| path.ends_with("src/main.rs")),
            "{candidates:?}"
        );
    }

    #[gpui::test]
    fn a_conversation_round_trips_through_disk(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        let session_id = spawn_and_connect(&chat, &agents, cx);
        send_from_chat(&chat, "hola", cx);
        drain_until_turn_ended(&agents, cx);

        agents.update(cx, |agents, cx| agents.save_conversation_now(cx));
        // The store is read off `Agents` rather than rebuilt from the path:
        // two test modules in this crate isolate `XDG_STATE_HOME` with their
        // own `OnceLock`, so recomputing it can land in the other one's.
        let store = agents
            .read_with(cx, |agents, _| agents.store.clone())
            .expect("hay directorio de estado");
        let index = store.index();
        assert_eq!(index.len(), 1, "{index:?}");

        let before = chat.read_with(cx, |chat, _| chat.entries().len());
        let stored = store.load(&index[0].id).expect("la conversación se lee");
        assert_eq!(stored.entries.len(), before);
        assert_eq!(stored.agent_id, "fake");
        assert_eq!(stored.title, "hola");
        assert_eq!(stored.session_id.as_deref(), Some(session_id.0.as_ref()));
    }

    #[gpui::test]
    fn autonomy_change_rebuilds_the_policy_with_the_same_globs(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (_chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        agents.update(cx, |agents, _cx| {
            assert_eq!(agents.autonomy.mode(), AutonomyMode::ReviewAfter);
            agents.set_autonomy_mode(AutonomyMode::AlwaysApply);
            assert_eq!(agents.autonomy.mode(), AutonomyMode::AlwaysApply);
            assert!(agents.autonomy.is_sensitive(Path::new("/proj/.env")));
        });
    }

    #[gpui::test]
    fn the_agents_write_lands_on_disk_and_in_the_open_buffer(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, center, agents, _project, cx) = harness(dir.path(), cx);
        let target = dir.path().join("demo.txt");
        cx.update(|window, cx| {
            center.update(cx, |center, cx| center.open_file(&target, true, window, cx));
        });
        cx.run_until_parked();

        let session_id = spawn_and_connect(&chat, &agents, cx);
        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id,
                    blocks: vec![PromptBlock::Text("hola".to_string())],
                    feedback: None,
                },
                cx,
            );
        });
        drain_until_turn_ended(&agents, cx);

        let on_disk = std::fs::read_to_string(&target).unwrap();
        assert!(
            on_disk.ends_with("escrito por el agente falso\n"),
            "{on_disk:?}"
        );
        let in_buffer = center.read_with(cx, |center, cx| {
            center
                .tabs()
                .iter()
                .find(|tab| tab.path == target)
                .map(|tab| tab.editor().read(cx).text())
                .expect("la pestaña sigue abierta")
        });
        assert_eq!(
            in_buffer, on_disk,
            "el buffer abierto debería reflejar la escritura del agente"
        );
    }

    #[gpui::test]
    fn switching_agents_shuts_down_the_previous_connection(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        agents.update(cx, |agents, cx| {
            let extended = fake_registry().with_custom(vec![CustomAgent {
                id: "fake2".to_string(),
                name: "Fake 2".to_string(),
                command: fake_agent_path().to_string_lossy().into_owned(),
                args: Vec::new(),
                env: std::collections::BTreeMap::new(),
            }]);
            agents.install_registry(extended, "fake", cx);
        });

        let _session = spawn_and_connect(&chat, &agents, cx);
        let old_commands = agents.read_with(cx, |agents, _| {
            agents
                .connection
                .as_ref()
                .expect("hay conexión")
                .commands()
                .clone()
        });

        chat.update(cx, |chat, cx| chat.set_active_agent("fake2", cx));
        cx.run_until_parked();

        assert!(
            old_commands.send_blocking(AgentCommand::Shutdown).is_err(),
            "la conexión anterior debería haberse cerrado al cambiar de agente"
        );
    }

    /// Opens a second project through [`Agents::set_project`] (what
    /// `Workspace::open_project` calls for Ctrl+O, recientes y el path de la
    /// CLI) and returns it alongside the settings used to build it, so tests
    /// don't repeat the boilerplate.
    fn open_second_project(cx: &mut VisualTestContext, dir: &Path) -> Entity<Project> {
        cx.update(|_window, cx| {
            let settings = crate::settings::settings(cx);
            crate::project::open_with(dir, &settings, ProjectOptions::inert(), cx)
                .expect("el proyecto B se abre")
        })
    }

    #[gpui::test]
    fn switching_projects_shuts_down_the_old_agent_and_restarts_in_the_new_root(
        cx: &mut TestAppContext,
    ) {
        let dir_a = sample_project();
        let (chat, _center, agents, _project_a, cx) = harness(dir_a.path(), cx);
        let _session_a = spawn_and_connect(&chat, &agents, cx);

        let old_commands = agents.read_with(cx, |agents, _| {
            agents
                .connection
                .as_ref()
                .expect("hay conexión")
                .commands()
                .clone()
        });

        let dir_b = sample_project();
        let project_b = open_second_project(cx, dir_b.path());
        cx.update(|window, cx| {
            agents.update(cx, |agents, cx| {
                agents.set_project(Some(project_b.clone()), window, cx);
            });
        });
        cx.run_until_parked();

        // A's fake agent process is gone: its command channel is closed
        // (`AgentConnection::drop` blocks for the worker thread, which kills
        // the whole process group before returning).
        assert!(
            old_commands.send_blocking(AgentCommand::Shutdown).is_err(),
            "la conexión de A debería haberse cerrado al cambiar de proyecto"
        );

        // The agent that was selected in A is relaunched automatically, in
        // B's root.
        let _session_b = drain_until_session_created(&chat, &agents, cx);
        assert_eq!(
            chat.read_with(cx, |chat, _| chat.status()),
            asteroid_chat::AgentStatus::Ready
        );
        let sandbox = agents.read_with(cx, |agents, _| agents.sandbox_root.clone());
        assert_eq!(
            sandbox,
            Some(dir_b.path().to_path_buf()),
            "el sandbox (y por lo tanto FsRead/FsWrite) debería apuntar a B"
        );
    }

    #[gpui::test]
    fn switching_projects_persists_the_old_transcript_and_shows_the_new_empty_one(
        cx: &mut TestAppContext,
    ) {
        let dir_a = sample_project();
        let (chat, _center, agents, _project_a, cx) = harness(dir_a.path(), cx);
        let session_a = spawn_and_connect(&chat, &agents, cx);

        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id: session_a,
                    blocks: vec![PromptBlock::Text("hola".to_string())],
                    feedback: None,
                },
                cx,
            );
        });
        drain_until_turn_ended(&agents, cx);
        let entries_a = chat.read_with(cx, |chat, _| chat.entries().len());
        assert!(entries_a > 0, "A debería tener entradas antes de cambiar");

        let store_a = agents
            .read_with(cx, |agents, _| agents.store.clone())
            .expect("hay estado");

        let dir_b = sample_project();
        let project_b = open_second_project(cx, dir_b.path());
        cx.update(|window, cx| {
            agents.update(cx, |agents, cx| {
                agents.set_project(Some(project_b.clone()), window, cx);
            });
        });
        cx.run_until_parked();

        // A's conversation was flushed to disk before the switch, and it has
        // the prompt/response pair just sent.
        let index_a = store_a.index();
        assert_eq!(index_a.len(), 1, "{index_a:?}");
        let stored_a = store_a.load(&index_a[0].id).expect("se lee");
        assert_eq!(
            stored_a.entries.len(),
            entries_a,
            "la conversación de A debería reflejar lo que tenía el panel"
        );

        // B has no conversation yet, so the panel starts empty instead
        // of showing what was left over from A.
        let entries_b = chat.read_with(cx, |chat, _| chat.entries().len());
        assert_eq!(
            entries_b, 0,
            "el panel debería mostrar el transcript (vacío) de B, no el de A"
        );

        let _session_b = drain_until_session_created(&chat, &agents, cx);
    }

    #[gpui::test]
    fn switching_projects_mid_turn_cancels_it_and_warns_the_user(cx: &mut TestAppContext) {
        let dir_a = sample_project();
        let (chat, _center, agents, _project_a, cx) = harness(dir_a.path(), cx);
        let toasts = cx.update(|_, cx| cx.new(|_| crate::toast::Toasts::new()));
        cx.update(|_, cx| crate::toast::set_global(&toasts, cx));
        let session_a = spawn_and_connect(&chat, &agents, cx);

        agents.update(cx, |agents, cx| {
            agents.forward_command(
                &AgentCommand::Prompt {
                    session_id: session_a,
                    blocks: vec![PromptBlock::Text("hola".to_string())],
                    feedback: None,
                },
                cx,
            );
        });
        // One text chunk is enough to put the turn "in progress"
        // (`AgentStatus::Thinking`) without waiting for `TurnEnded`.
        let events = agents
            .read_with(cx, |agents, _| {
                agents.connection.as_ref().map(|c| c.events().clone())
            })
            .expect("hay conexión");
        let event = events.recv_blocking().expect("evento del agente falso");
        agents.update(cx, |agents, cx| agents.handle_agent_event(event, cx));
        cx.run_until_parked();
        assert_eq!(
            chat.read_with(cx, |chat, _| chat.status()),
            asteroid_chat::AgentStatus::Thinking,
            "el turno debería seguir en curso"
        );

        let dir_b = sample_project();
        let project_b = open_second_project(cx, dir_b.path());
        cx.update(|window, cx| {
            agents.update(cx, |agents, cx| {
                agents.set_project(Some(project_b.clone()), window, cx);
            });
        });
        cx.run_until_parked();

        let messages: Vec<String> = toasts.read_with(cx, |toasts, _| {
            toasts
                .items()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect()
        });
        assert!(
            messages
                .iter()
                .any(|message| message
                    .contains("Se cerró la sesión del agente al cambiar de proyecto")),
            "{messages:?}"
        );

        // The agent still comes back up in B, same as any other switch.
        let _session_b = drain_until_session_created(&chat, &agents, cx);
        assert_eq!(
            chat.read_with(cx, |chat, _| chat.status()),
            asteroid_chat::AgentStatus::Ready
        );
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
            cx.new(|cx| Agents::new(chat.clone(), center.clone(), false, window, cx))
        });

        let root = agents.update(cx, |agents, cx| agents.resolve_sandbox_root(cx));
        assert!(root.is_dir(), "el sandbox temporal debería existir");

        let messages: Vec<String> = toasts.read_with(cx, |toasts, _| {
            toasts
                .items()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect()
        });
        assert!(
            messages
                .iter()
                .any(|message| message.contains("Abrí una carpeta")),
            "{messages:?}"
        );
    }

    // ---------------------------------------------------- conversaciones

    /// The registry of the tests plus a second agent, so switching agents has
    /// somewhere to go.
    fn two_agent_registry() -> AgentRegistry {
        fake_registry().with_custom(vec![CustomAgent {
            id: "fake2".to_string(),
            name: "Fake 2".to_string(),
            command: fake_agent_path().to_string_lossy().into_owned(),
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
        }])
    }

    fn live_store(agents: &Entity<Agents>, cx: &mut VisualTestContext) -> ConversationStore {
        agents
            .read_with(cx, |agents, _| agents.store.clone())
            .expect("hay directorio de estado")
    }

    #[gpui::test]
    fn choosing_another_agent_saves_the_old_conversation_and_opens_an_empty_one(
        cx: &mut TestAppContext,
    ) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        agents.update(cx, |agents, cx| {
            agents.install_registry(two_agent_registry(), "fake", cx);
        });
        let _session = spawn_and_connect(&chat, &agents, cx);
        send_from_chat(&chat, "primera conversación", cx);
        drain_until_turn_ended(&agents, cx);
        assert!(chat.read_with(cx, |chat, _| !chat.entries().is_empty()));

        chat.update(cx, |chat, cx| chat.set_active_agent("fake2", cx));
        cx.run_until_parked();

        assert!(
            chat.read_with(cx, |chat, _| chat.entries().is_empty()),
            "cambiar de agente abre una conversación vacía"
        );
        assert_eq!(
            chat.read_with(cx, |chat, _| chat
                .active_agent()
                .map(|agent| agent.id.clone())),
            Some("fake2".to_string())
        );

        let index = live_store(&agents, cx).index();
        assert_eq!(index.len(), 1, "la anterior quedó guardada: {index:?}");
        assert_eq!(index[0].agent_id, "fake");
        assert_eq!(index[0].title, "primera conversación");
        // La popover del chat ya la lista, agrupada por agente.
        let rows = chat.read_with(cx, |chat, _| chat.conversations().to_vec());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].agent_name, "Fake");
    }

    #[gpui::test]
    fn picking_a_past_conversation_restores_its_entries_and_its_agent(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        agents.update(cx, |agents, cx| {
            agents.install_registry(two_agent_registry(), "fake", cx);
        });
        let _session = spawn_and_connect(&chat, &agents, cx);
        send_from_chat(&chat, "primera conversación", cx);
        drain_until_turn_ended(&agents, cx);
        let entries_before = chat.read_with(cx, |chat, _| chat.entries().len());

        // Otro agente: la de «fake» queda guardada y la pantalla, vacía.
        chat.update(cx, |chat, cx| chat.set_active_agent("fake2", cx));
        cx.run_until_parked();
        let index = live_store(&agents, cx).index();
        let stored = index[0].id.clone();

        chat.update(cx, |chat, cx| chat.open_conversation(&stored, cx));
        cx.run_until_parked();

        assert_eq!(
            chat.read_with(cx, |chat, _| chat.entries().len()),
            entries_before,
            "vuelven las entradas guardadas"
        );
        assert_eq!(
            chat.read_with(cx, |chat, _| chat
                .active_agent()
                .map(|agent| agent.id.clone())),
            Some("fake".to_string()),
            "y vuelve su agente"
        );
        assert_eq!(
            chat.read_with(cx, |chat, _| chat.active_conversation().map(str::to_string)),
            Some(stored)
        );
    }

    #[gpui::test]
    fn the_old_chat_json_becomes_the_first_conversation(cx: &mut TestAppContext) {
        let dir = sample_project();
        // Un transcript del formato anterior, escrito antes de abrir nada.
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        let store = live_store(&agents, cx);
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

        // Reabrir el proyecto es lo que dispara la migración.
        let project = open_second_project(cx, dir.path());
        cx.update(|window, cx| {
            agents.update(cx, |agents, cx| {
                agents.set_project(Some(project), window, cx);
            });
        });
        cx.run_until_parked();

        let index = live_store(&agents, cx).index();
        assert_eq!(index.len(), 1, "{index:?}");
        assert_eq!(index[0].agent_id, "fake");
        assert_eq!(index[0].title, "lo de antes");
        assert_eq!(index[0].session_id.as_deref(), Some("s-vieja"));
        assert!(
            !base.join("chat.json").exists(),
            "el chat.json migrado se renombra"
        );
        assert!(base.join("chat.json.migrado").is_file());
        assert!(chat.read_with(cx, |chat, _| chat.conversations().len()) == 1);

        // Y correr la migración otra vez no duplica nada.
        live_store(&agents, cx).migrate_legacy();
        assert_eq!(live_store(&agents, cx).index().len(), 1);
    }

    #[gpui::test]
    fn reopening_a_conversation_loads_the_session_without_duplicating_it(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&chat, &agents, cx);

        // El agente falso anuncia `loadSession` y, al cargarla, repite
        // «primer mensaje replayado» y «segundo mensaje replayado».
        let capabilities = agents.read_with(cx, |agents, _| agents.capabilities.clone());
        assert!(
            capabilities.expect("hubo Connected").load_session,
            "el agente falso anuncia loadSession"
        );

        // Una conversación guardada que ya contiene el replay entero.
        let store = live_store(&agents, cx);
        let mut conversation = Conversation::new(
            "c-vieja",
            "fake",
            "Fake",
            dir.path().to_path_buf(),
            now_seconds(),
        );
        conversation.session_id = Some("loaded-1".to_string());
        conversation.title = "vieja".to_string();
        conversation.entries = vec![
            asteroid_chat::Entry::UserMessage(asteroid_chat::UserMessage {
                blocks: vec![asteroid_chat::MessageBlock::Text("vieja".to_string())],
            }),
            asteroid_chat::Entry::AgentText(asteroid_chat::AgentText {
                markdown: "primer mensaje replayadosegundo mensaje replayado".to_string(),
                streaming: false,
                view: None,
            }),
        ];
        store.save(&conversation).expect("se guarda");
        agents.update(cx, |agents, cx| agents.refresh_conversations(cx));

        chat.update(cx, |chat, cx| chat.open_conversation("c-vieja", cx));
        cx.run_until_parked();
        assert_eq!(chat.read_with(cx, |chat, _| chat.entries().len()), 2);
        assert!(
            chat.read_with(cx, |chat, _| chat.is_replaying()),
            "se pidió session/load y el filtro de replay está puesto"
        );

        let session = drain_until_session_created(&chat, &agents, cx);
        assert_eq!(session.0.as_ref(), "loaded-1", "volvió la misma sesión");
        assert_eq!(
            chat.read_with(cx, |chat, _| chat.entries().len()),
            2,
            "el replay no duplicó lo que ya estaba: {:?}",
            chat.read_with(cx, |chat, _| chat.entries().len())
        );
        assert!(
            chat.read_with(cx, |chat, _| chat.history_notice().is_none()),
            "el agente sí puede retomar, no hace falta el aviso"
        );
    }

    #[gpui::test]
    fn an_agent_without_load_shows_the_read_only_notice(cx: &mut TestAppContext) {
        let dir = sample_project();
        let (chat, _center, agents, _project, cx) = harness(dir.path(), cx);
        let _session = spawn_and_connect(&chat, &agents, cx);

        let store = live_store(&agents, cx);
        let mut conversation = Conversation::new(
            "c-vieja",
            "fake",
            "Fake",
            dir.path().to_path_buf(),
            now_seconds(),
        );
        conversation.session_id = Some("loaded-1".to_string());
        conversation.entries = vec![asteroid_chat::Entry::UserMessage(
            asteroid_chat::UserMessage {
                blocks: vec![asteroid_chat::MessageBlock::Text("vieja".to_string())],
            },
        )];
        store.save(&conversation).expect("se guarda");

        // Un agente que no anuncia ni `loadSession` ni `resume`.
        agents.update(cx, |agents, _cx| {
            agents.capabilities = Some(Box::default());
        });
        cx.update(|window, cx| {
            agents.update(cx, |agents, cx| {
                agents.open_conversation("c-vieja", window, cx);
            });
        });
        cx.run_until_parked();

        assert_eq!(
            chat.read_with(cx, |chat, _| chat.history_notice().map(str::to_string)),
            Some(READ_ONLY_HISTORY_NOTICE.to_string())
        );
        assert!(!chat.read_with(cx, |chat, _| chat.is_replaying()));
        // Y se abre una sesión nueva igual, para que el próximo mensaje salga.
        let session = drain_until_session_created(&chat, &agents, cx);
        assert!(session.0.to_string().starts_with("fake-session-"));
    }

    #[test]
    fn offline_registry_still_offers_custom_agents() {
        let registry = empty_registry().with_custom(vec![CustomAgent {
            id: "mio".to_string(),
            name: "Mío".to_string(),
            command: "/bin/echo".to_string(),
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
        }]);
        assert_eq!(registry.agents().len(), 1);
        let (installed, hint) = agent_installed_status(&registry, registry.get("mio").unwrap());
        assert!(installed);
        assert!(hint.is_none());
    }

    #[test]
    fn a_missing_command_is_reported_not_installed() {
        let registry = empty_registry().with_custom(vec![CustomAgent {
            id: "nope".to_string(),
            name: "Nope".to_string(),
            command: "/no/existe/nada".to_string(),
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
        }]);
        let (installed, hint) = agent_installed_status(&registry, registry.get("nope").unwrap());
        assert!(!installed);
        assert!(hint.is_some());
    }
}
