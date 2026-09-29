//! The connection modals (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4
//! F2–F5, §6): "Conectar nuevo agente", "Volver a conectar", "Reparar",
//! "Eliminar conexión" and "Renombrar".
//!
//! [`ConnectionsModal`] is one entity with a [`Mode`]; the workspace paints
//! it over everything while it is open and [`crate::agents::Agents`] listens
//! to its [`ModalEvent`]s (a saved connection is activated there, a deleted
//! one loses its conversations there). It talks to
//! [`cincel_connections::Connections`] directly: preparing the private
//! runtime and the adapter (or, for Google Antigravity, downloading its
//! official binary), the login (a hidden pty for Claude and Codex, ACP
//! `authenticate` for Antigravity), `finalize`, `set_identity`, `rename` and
//! `disconnect`.
//!
//! # Threads
//!
//! Preparing (downloads), the login and the disconnect block for seconds or
//! minutes, so with `background` on (the real window) each runs on its own
//! OS thread and reports back over an `async_channel` that a foreground task
//! drains, exactly like [`crate::agents::Agents`] drains the ACP worker. With
//! `background` off (the tests) preparing and disconnecting run inline and
//! the login events are fed by hand through
//! [`ConnectionsModal::handle_login_event`], because GPUI's deterministic test
//! executor does not tolerate a foreign thread waking a foreground task.
//!
//! # Secrets
//!
//! The login URL and the one-time code are shown on screen and never logged;
//! "Ver detalles técnicos" only shows the `LoginEvent::Output` lines, which
//! `cincel-connections` already redacted.

use std::sync::Arc;

use cincel_connections::{
    AdapterProgress, AgentKind, CancelToken, Connection, ConnectionStatus, Connections,
    ConnectionsError, DisconnectReport, Identity, LoginEvent, LoginFailure, LoginSession,
    LogoutStep, PendingConnection, PrepareProgress, RuntimeProgress, suggest_label,
};
use gpui::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, SharedString, Subscription, Task, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, div, px};
use uuid::Uuid;

use crate::theme::ThemeColors;

/// The GPUI key context of the modal: `Enter` and `Esc` belong to it
/// ([`crate::keymap`]).
pub const MODAL_CONTEXT: &str = "ConnectionsModal";

/// Widest the dialog gets (spec 06 §6): it follows the window below this.
const DIALOG_MAX_WIDTH: f32 = 560.;
/// Narrowest the dialog gets, whatever the window.
const DIALOG_MIN_WIDTH: f32 = 420.;
/// Space kept between the dialog and the window edges.
const DIALOG_MARGIN: f32 = 24.;
/// Narrowest an agent card gets before the grid wraps to a new row.
const AGENT_CARD_MIN_WIDTH: f32 = 160.;
/// Height of each button (the save dialog's).
const BUTTON_HEIGHT: f32 = 28.;
/// Lines of "Ver detalles técnicos" kept in memory.
const MAX_OUTPUT_LINES: usize = 400;

/// What the modal tells the rest of the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModalEvent {
    /// "Guardar" of a new connection (F2 step 6): it is in the index now;
    /// activate it (F1).
    Saved {
        /// The new connection.
        id: Uuid,
    },
    /// "Volver a conectar" finished (F4): same profile, same label, fresh
    /// credentials.
    Relogged {
        /// The connection.
        id: Uuid,
    },
    /// "Reparar" finished (F5).
    Repaired {
        /// The connection.
        id: Uuid,
    },
    /// The user confirmed the deletion: stop the connection's process if it
    /// is the active one, then call [`ConnectionsModal::run_delete`] (the
    /// logout must not run next to a live agent on the same profile).
    WillDelete {
        /// The connection.
        id: Uuid,
    },
    /// The deletion ran (F3). The conversations are the workspace's to
    /// delete now.
    Deleted {
        /// The connection.
        id: Uuid,
        /// Its label, for the toast.
        label: String,
        /// Which steps succeeded (`None` when `disconnect` itself failed).
        report: Option<DisconnectReport>,
    },
    /// "Renombrar" saved a new label.
    Renamed {
        /// The connection.
        id: Uuid,
    },
    /// The modal closed.
    Closed,
}

/// Why the connect flow is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// "Conectar nuevo agente" (F2).
    New,
    /// "Volver a conectar" (F4): the login on the existing profile.
    Relogin(Uuid),
    /// "Reparar" (F5): download what is missing, credentials untouched.
    Repair(Uuid),
}

/// The screen of the connect flow (F2's six steps plus the error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// 1. "Elegí un agente".
    Choose,
    /// 2. "Preparando…": private Node and adapter (npm agents), or the
    ///    agent's official binary (Antigravity).
    Preparing,
    /// 3. "Iniciando sesión…".
    StartingLogin,
    /// 4. "Abrí este enlace y aprobá el acceso".
    Link,
    /// 5. "Esperando que apruebes en el navegador…" (after the code went).
    Waiting,
    /// 6. "Listo": identity and, for a new connection, its name.
    Done,
    /// "Reparar" finished.
    Repaired,
    /// Something failed; "Reintentar".
    Failed {
        /// What to tell the user.
        message: String,
    },
}

/// One progress bar of "Preparando…".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bar {
    /// 0–100 when known; `None` shows the indeterminate animation.
    pub percent: Option<f32>,
    /// The line under the bar.
    pub text: String,
    /// Finished.
    pub done: bool,
}

/// State of the connect flow.
#[derive(Debug)]
pub struct ConnectFlow {
    /// Why it runs.
    pub purpose: Purpose,
    /// The screen.
    pub step: Step,
    /// The agent being connected.
    pub kind: Option<AgentKind>,
    /// The existing connection's label ("Volver a conectar", "Reparar").
    pub label: Option<String>,
    /// The half-created profile of a new connection.
    pending: Option<PendingConnection>,
    /// Private Node runtime (npm agents only; shown as done otherwise).
    pub runtime: Bar,
    /// Adapter install, or the binary download.
    pub adapter: Bar,
    /// The binary being downloaded has no published SHA-256: "Google no
    /// publica una suma de verificación para este paquete".
    pub unverified: bool,
    /// The login link (never logged).
    pub url: Option<String>,
    /// The one-time code to type in the browser (Codex device flow).
    pub code: Option<String>,
    /// The program waits for the code the browser gave the user.
    pub needs_code: bool,
    /// Waiting behind another login of the same agent.
    pub queued: bool,
    /// "Ver detalles técnicos": redacted terminal lines.
    pub output: Vec<String>,
    /// Whether "Ver detalles técnicos" is unfolded.
    pub details_open: bool,
    /// What the agent reported at the end.
    pub identity: Option<Identity>,
    /// A short line under the link ("Enlace copiado", no browser…).
    pub notice: Option<String>,
    /// Problem with the pasted code or the name.
    pub field_error: Option<String>,
}

impl ConnectFlow {
    fn new(purpose: Purpose) -> Self {
        Self {
            purpose,
            step: Step::Choose,
            kind: None,
            label: None,
            pending: None,
            runtime: Bar::default(),
            adapter: Bar::default(),
            unverified: false,
            url: None,
            code: None,
            needs_code: false,
            queued: false,
            output: Vec::new(),
            details_open: false,
            identity: None,
            notice: None,
            field_error: None,
        }
    }

    /// Whether a login (or its preparation) is in flight: closing then has to
    /// be confirmed.
    fn in_flight(&self) -> bool {
        matches!(
            self.step,
            Step::Preparing | Step::StartingLogin | Step::Link | Step::Waiting
        )
    }
}

/// The screens of "Eliminar conexión" (F3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteStep {
    /// The list of connections.
    List,
    /// "¿Eliminar «X»? …".
    Confirm {
        /// Connection id.
        id: Uuid,
        /// Its label.
        label: String,
        /// Its agent.
        agent_id: String,
        /// How many conversations go with it.
        conversations: usize,
    },
    /// The deletion is running.
    Running {
        /// Connection id.
        id: Uuid,
        /// Label being deleted.
        label: String,
        /// Its agent (Antigravity's toast adds the Google revocation note).
        agent_id: String,
    },
    /// Something did not go as planned: one line per step.
    Result {
        /// Label deleted.
        label: String,
        /// One line per step.
        lines: Vec<String>,
    },
}

/// What the modal shows.
#[derive(Debug)]
pub enum Mode {
    /// Nothing.
    Closed,
    /// "Conectar nuevo agente", "Volver a conectar" or "Reparar".
    Connect(Box<ConnectFlow>),
    /// "Eliminar conexión".
    Delete(DeleteStep),
    /// "Renombrar".
    Rename {
        /// Connection id.
        id: Uuid,
        /// Problem with the name.
        error: Option<String>,
    },
}

/// A message from the preparation thread.
enum PrepareMessage {
    Progress(PrepareProgress),
    Done(Result<(), PrepareFailure>),
}

/// What `PrepareMessage::Done` carries on failure: the user's own
/// "Cancelar" (`docs/specs/07-etapa5-productividad.md` §10.3) is never shown
/// as "Algo salió mal", so it gets its own variant instead of a message
/// string.
enum PrepareFailure {
    /// `ConnectionsError::Cancelled`: the flow already went back to "Elegí
    /// un agente" (or the modal closed) when "Cancelar" was pressed.
    Cancelled,
    /// Any other error, already formatted by [`prepare_error_message`].
    Message(String),
}

/// What `on_prepare_message` does once "Preparando…" is done.
enum PrepareOutcome {
    /// Runtime and adapter ready: start the login.
    Ready,
    /// The user's own "Cancelar" (or an equivalent same-generation
    /// `Cancelled`): back to "Elegí un agente" (or the modal closes),
    /// exactly like the button.
    Cancelled,
    /// Anything else: "Algo salió mal" with the given message.
    Failed(String),
}

/// The connection modals.
pub struct ConnectionsModal {
    connections: Arc<Connections>,
    background: bool,
    focus_handle: FocusHandle,
    mode: Mode,
    /// "¿Cancelar el inicio de sesión?" over the modal (Esc while a login is
    /// in flight, or with an unsaved connection).
    confirm_close: bool,
    /// The running login; dropping it cancels it.
    session: Option<LoginSession>,
    /// The "Preparando…" download/install in flight, if any; cancelling it
    /// really stops the thread (`docs/specs/07-etapa5-productividad.md`
    /// §10.3), unlike the `CancelToken::new()` placeholders of Etapa 4.
    prepare_cancel: Option<CancelToken>,
    /// Foreground tasks draining the current flow's threads.
    tasks: Vec<Task<()>>,
    /// Bumped whenever the flow changes, so a late message from an older
    /// thread is ignored.
    generation: u64,
    code_input: Entity<InputState>,
    label_input: Entity<InputState>,
    rename_input: Entity<InputState>,
    /// The rows of "Eliminar conexión", read when it opened.
    delete_rows: Vec<Connection>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ModalEvent> for ConnectionsModal {}

impl Focusable for ConnectionsModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ConnectionsModal {
    /// A closed modal over `connections`. `background` as in
    /// [`crate::agents::Agents::new`].
    pub fn new(
        connections: Arc<Connections>,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let code_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Pegá acá el código que te dio el navegador")
        });
        let label_input = cx.new(|cx| InputState::new(window, cx).placeholder("Claude · personal"));
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&code_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit_code(window, cx);
                }
            }),
            cx.subscribe_in(&label_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save(window, cx);
                }
            }),
            cx.subscribe_in(&rename_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_rename(window, cx);
                }
            }),
        ];
        Self {
            connections,
            background,
            focus_handle: cx.focus_handle(),
            mode: Mode::Closed,
            confirm_close: false,
            session: None,
            prepare_cancel: None,
            tasks: Vec::new(),
            generation: 0,
            code_input,
            label_input,
            rename_input,
            delete_rows: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Replaces the engine (tests point it at a temp directory).
    pub fn set_connections(&mut self, connections: Arc<Connections>) {
        self.connections = connections;
    }

    // ------------------------------------------------------------ getters

    /// Whether anything is on screen.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !matches!(self.mode, Mode::Closed)
    }

    /// What is on screen.
    #[must_use]
    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// The connect flow, when that is what is on screen.
    #[must_use]
    pub fn connect_flow(&self) -> Option<&ConnectFlow> {
        match &self.mode {
            Mode::Connect(flow) => Some(flow.as_ref()),
            _ => None,
        }
    }

    /// The step of the connect flow.
    #[must_use]
    pub fn step(&self) -> Option<&Step> {
        self.connect_flow().map(|flow| &flow.step)
    }

    /// Whether the "¿Cancelar?" confirmation is up.
    #[must_use]
    pub fn is_confirming_close(&self) -> bool {
        self.confirm_close
    }

    /// The text of the "Nombre de la conexión" field.
    #[must_use]
    pub fn label_text(&self, cx: &App) -> String {
        self.label_input.read(cx).value().to_string()
    }

    /// The running login's event stream (tests feed it back by hand).
    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn login_events(&self) -> Option<async_channel::Receiver<LoginEvent>> {
        self.session
            .as_ref()
            .map(|session| session.events().clone())
    }

    /// The token of "Preparando…"'s in-flight download/install, if any
    /// (`docs/specs/07-etapa5-productividad.md` §10.3): lets a test check
    /// that "Cancelar" sets the *real* token the background thread holds,
    /// not a throwaway one.
    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn prepare_cancel_for_test(&self) -> Option<CancelToken> {
        self.prepare_cancel.clone()
    }

    // ------------------------------------------------------------ opening

    fn reset(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_session();
        self.discard_pending();
        self.tasks.clear();
        self.generation += 1;
        self.confirm_close = false;
        self.mode = mode;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// "Conectar nuevo agente…" (F2 step 1).
    pub fn open_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset(
            Mode::Connect(Box::new(ConnectFlow::new(Purpose::New))),
            window,
            cx,
        );
    }

    /// "Volver a conectar" (F4): F2 from step 3 on the same profile. When
    /// something is missing to run the agent, it is prepared first.
    pub fn open_relogin(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(connection) = self.connections.store().get(id) else {
            crate::toast::warn("Esa conexión ya no existe.", cx);
            return;
        };
        let mut flow = ConnectFlow::new(Purpose::Relogin(id));
        flow.kind = connection.kind().ok();
        flow.label = Some(connection.label.clone());
        let unavailable = matches!(
            self.connections.status(&connection),
            ConnectionStatus::Unavailable { .. }
        );
        self.reset(Mode::Connect(Box::new(flow)), window, cx);
        if unavailable {
            self.start_prepare(window, cx);
        } else {
            self.start_login(window, cx);
        }
    }

    /// "Reparar" (F5): prepare again, credentials untouched.
    pub fn open_repair(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(connection) = self.connections.store().get(id) else {
            crate::toast::warn("Esa conexión ya no existe.", cx);
            return;
        };
        let mut flow = ConnectFlow::new(Purpose::Repair(id));
        flow.kind = connection.kind().ok();
        flow.label = Some(connection.label.clone());
        self.reset(Mode::Connect(Box::new(flow)), window, cx);
        self.start_prepare(window, cx);
    }

    /// "Eliminar conexión…" (F3): the list.
    pub fn open_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_rows = self.connections.store().list().unwrap_or_default();
        self.reset(Mode::Delete(DeleteStep::List), window, cx);
    }

    /// "Renombrar" of the popover's context menu.
    pub fn open_rename(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(connection) = self.connections.store().get(id) else {
            crate::toast::warn("Esa conexión ya no existe.", cx);
            return;
        };
        self.reset(Mode::Rename { id, error: None }, window, cx);
        self.rename_input.update(cx, |input, cx| {
            input.set_value(connection.label.clone(), window, cx);
        });
        let handle = self.rename_input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    // ------------------------------------------------------------ closing

    /// `Esc`: closes when that loses nothing; otherwise asks first.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_close {
            self.confirm_close = false;
            cx.notify();
            return;
        }
        let needs_confirmation = match &self.mode {
            Mode::Connect(flow) => {
                flow.in_flight() || (flow.purpose == Purpose::New && flow.step == Step::Done)
            }
            // A deletion that already started cannot be taken back halfway.
            Mode::Delete(DeleteStep::Running { .. }) => return,
            _ => false,
        };
        if needs_confirmation {
            self.confirm_close = true;
            cx.notify();
        } else {
            self.close(window, cx);
        }
    }

    /// "Seguir acá" of the confirmation.
    pub fn keep_open(&mut self, cx: &mut Context<Self>) {
        self.confirm_close = false;
        cx.notify();
    }

    /// Closes the modal, cancelling a login in flight and removing a profile
    /// that was never saved.
    pub fn close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_open() {
            return;
        }
        self.cancel_session();
        self.discard_pending();
        self.tasks.clear();
        self.generation += 1;
        self.confirm_close = false;
        self.mode = Mode::Closed;
        cx.emit(ModalEvent::Closed);
        cx.notify();
    }

    /// Stops whatever background work the current flow has running: a
    /// login (pty session) and/or the "Preparando…" download/install
    /// thread. Called by every path that resets or closes the modal, so
    /// cancelling "Preparando…" ("Cancelar", `Esc` confirmed, closing the
    /// modal) really stops the download instead of letting it finish
    /// unattended (`docs/specs/07-etapa5-productividad.md` §10.3).
    fn cancel_session(&mut self) {
        if let Some(token) = self.prepare_cancel.take() {
            token.cancel();
        }
        if let Some(session) = self.session.take() {
            session.cancel();
            // Dropping it joins the login thread, which kills the process
            // group and removes the pending profile first.
            drop(session);
        }
    }

    fn discard_pending(&mut self) {
        if let Mode::Connect(flow) = &mut self.mode
            && let Some(pending) = flow.pending.take()
        {
            // Usually gone already (the login thread removes it on cancel).
            let _ = self.connections.store().discard_pending(pending.id);
        }
    }

    // ------------------------------------------------------- connect flow

    fn flow_mut(&mut self) -> Option<&mut ConnectFlow> {
        match &mut self.mode {
            Mode::Connect(flow) => Some(flow.as_mut()),
            _ => None,
        }
    }

    /// Step 1: the user picked an agent.
    pub fn choose(&mut self, kind: AgentKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(flow) = self.flow_mut() else {
            return;
        };
        if flow.step != Step::Choose {
            return;
        }
        flow.kind = Some(kind);
        self.start_prepare(window, cx);
    }

    /// Step 2: private Node and adapter, if they are missing.
    fn start_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(flow) = self.flow_mut() else {
            return;
        };
        let Some(kind) = flow.kind else {
            return;
        };
        flow.step = Step::Preparing;
        flow.unverified = false;
        flow.runtime = if kind.needs_node() {
            Bar {
                percent: None,
                text: "Buscando el entorno de Node…".to_string(),
                done: false,
            }
        } else {
            // Not shown: the agent is a self-contained binary.
            Bar {
                percent: Some(100.),
                text: "No hace falta".to_string(),
                done: true,
            }
        };
        flow.adapter = Bar {
            percent: None,
            text: if kind.needs_node() {
                format!("Adaptador de {}", kind.display_name())
            } else {
                format!("Buscando {}…", kind.full_name())
            },
            done: false,
        };
        self.generation += 1;
        let generation = self.generation;
        // A real token: "Cancelar" (`cancel_session`, called from every
        // reset/close path) sets it, the download/install loops check it
        // and unwind for real (`docs/specs/07-etapa5-productividad.md`
        // §10.3), unlike the `CancelToken::new()` placeholders of Etapa 4.
        let cancel = CancelToken::new();
        self.prepare_cancel = Some(cancel.clone());
        cx.notify();

        if !self.background {
            // Tests: inline and offline (whatever is installed is used).
            let mut steps = Vec::new();
            let result = self
                .connections
                .prepare(None, kind, &mut |step| steps.push(step), &cancel)
                .map(|_| ())
                .map_err(prepare_failure);
            for step in steps {
                self.on_prepare_message(generation, PrepareMessage::Progress(step), window, cx);
            }
            self.on_prepare_message(generation, PrepareMessage::Done(result), window, cx);
            return;
        }

        let (sender, receiver) = async_channel::unbounded::<PrepareMessage>();
        let connections = self.connections.clone();
        let cancel_for_thread = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("cincel-prepare".to_string())
            .spawn(move || {
                // The registry pins the adapter's version; offline, whatever
                // is installed is used.
                let registry = cincel_acp::AgentRegistry::load().ok();
                let result = connections
                    .prepare(
                        registry.as_ref(),
                        kind,
                        &mut |step| {
                            let _ = sender.send_blocking(PrepareMessage::Progress(step));
                        },
                        &cancel_for_thread,
                    )
                    .map(|_| ())
                    .map_err(prepare_failure);
                let _ = sender.send_blocking(PrepareMessage::Done(result));
            });
        if let Err(error) = spawned {
            self.fail(format!("No se pudo preparar el agente: {error}"), cx);
            return;
        }
        self.tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(message) = receiver.recv().await {
                let alive = this
                    .update_in(cx, |modal, window, cx| {
                        modal.on_prepare_message(generation, message, window, cx);
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }

    fn on_prepare_message(
        &mut self,
        generation: u64,
        message: PrepareMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let is_done = matches!(message, PrepareMessage::Done(_));
        let Some(flow) = self.flow_mut() else {
            return;
        };
        let next = match message {
            PrepareMessage::Progress(PrepareProgress::Runtime(step)) => {
                flow.runtime = runtime_bar(&step);
                None
            }
            PrepareMessage::Progress(PrepareProgress::Adapter(step)) => {
                if !flow.runtime.done {
                    flow.runtime = Bar {
                        percent: Some(100.),
                        text: "Entorno de Node listo".to_string(),
                        done: true,
                    };
                }
                if let AdapterProgress::Downloading {
                    verifiable: false, ..
                } = step
                {
                    flow.unverified = true;
                }
                let name = flow.kind.map_or("el agente", AgentKind::full_name);
                flow.adapter = adapter_bar(&step, name);
                None
            }
            PrepareMessage::Done(Ok(())) => {
                if flow.kind.is_none_or(AgentKind::needs_node) {
                    flow.runtime = Bar {
                        percent: Some(100.),
                        text: "Entorno de Node listo".to_string(),
                        done: true,
                    };
                }
                flow.adapter = Bar {
                    percent: Some(100.),
                    text: match flow.kind {
                        Some(kind) if kind.needs_node() => {
                            format!("Adaptador de {} listo", kind.display_name())
                        }
                        Some(kind) => format!("{} listo", kind.full_name()),
                        None => "Adaptador listo".to_string(),
                    },
                    done: true,
                };
                match flow.purpose {
                    Purpose::Repair(_) => {
                        flow.step = Step::Repaired;
                        None
                    }
                    Purpose::New | Purpose::Relogin(_) => Some(PrepareOutcome::Ready),
                }
            }
            // Same-generation `Cancelled`: only the inline test path
            // reaches this (the real thread's late result is discarded by
            // the `generation` guard above, since `cancel_login` bumps it
            // before the thread can unwind). Treated exactly like the
            // button: never "Algo salió mal".
            PrepareMessage::Done(Err(PrepareFailure::Cancelled)) => Some(PrepareOutcome::Cancelled),
            PrepareMessage::Done(Err(PrepareFailure::Message(text))) => {
                Some(PrepareOutcome::Failed(text))
            }
        };
        if is_done {
            self.prepare_cancel = None;
        }
        match next {
            Some(PrepareOutcome::Ready) => self.start_login(window, cx),
            Some(PrepareOutcome::Failed(message)) => self.fail(message, cx),
            Some(PrepareOutcome::Cancelled) => self.cancel_login(window, cx),
            None => cx.notify(),
        }
    }

    /// Step 3: create the pending profile (new connection) and start the
    /// provider's login (a hidden pty, or ACP `authenticate` for
    /// Antigravity).
    fn start_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let connections = self.connections.clone();
        let Some(flow) = self.flow_mut() else {
            return;
        };
        let Some(kind) = flow.kind else {
            return;
        };
        flow.step = Step::StartingLogin;
        flow.url = None;
        flow.code = None;
        flow.needs_code = false;
        flow.queued = false;
        flow.output.clear();
        flow.notice = None;
        flow.field_error = None;
        let purpose = flow.purpose;
        let started = match purpose {
            Purpose::New => match connections.store().create_pending(kind.agent_id()) {
                Ok(pending) => {
                    let session = connections.start_login(&pending);
                    flow.pending = Some(pending);
                    session
                }
                Err(error) => Err(error),
            },
            Purpose::Relogin(id) => connections.start_relogin(id),
            Purpose::Repair(_) => return,
        };
        let session = match started {
            Ok(session) => session,
            Err(error) => {
                self.fail(prepare_error_message(&error), cx);
                return;
            }
        };
        self.generation += 1;
        let generation = self.generation;
        if self.background {
            let events = session.events().clone();
            self.tasks.push(cx.spawn_in(window, async move |this, cx| {
                while let Ok(event) = events.recv().await {
                    let alive = this
                        .update_in(cx, |modal, window, cx| {
                            if modal.generation == generation {
                                modal.handle_login_event(event, window, cx);
                            }
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            }));
        }
        self.session = Some(session);
        cx.notify();
    }

    /// One [`LoginEvent`] of the running login (`docs/specs/modulos/connections.md`,
    /// "Protocolo para la UI").
    pub fn handle_login_event(
        &mut self,
        event: LoginEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(flow) = self.flow_mut() else {
            return;
        };
        let purpose = flow.purpose;
        let kind = flow.kind.unwrap_or(AgentKind::Claude);
        match event {
            LoginEvent::Queued => flow.queued = true,
            LoginEvent::Started => flow.queued = false,
            LoginEvent::Output(line) => {
                // Already redacted by `cincel-connections`: safe to show.
                flow.output.push(line);
                if flow.output.len() > MAX_OUTPUT_LINES {
                    flow.output.remove(0);
                }
            }
            LoginEvent::UrlDetected(url) => {
                flow.url = Some(url);
                if flow.step == Step::StartingLogin {
                    flow.step = Step::Link;
                }
            }
            LoginEvent::CodeDetected(code) => flow.code = Some(code),
            LoginEvent::NeedsPastedCode => {
                flow.needs_code = true;
                if matches!(flow.step, Step::StartingLogin | Step::Waiting) {
                    flow.step = Step::Link;
                }
                let handle = self.code_input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            LoginEvent::Completed { identity } => {
                flow.identity = identity.clone();
                flow.step = Step::Done;
                self.session = None;
                match purpose {
                    Purpose::New => {
                        let existing = self.connections.store().list().unwrap_or_default();
                        let suggestion = suggest_label(kind, &existing);
                        self.label_input.update(cx, |input, cx| {
                            input.set_value(suggestion, window, cx);
                        });
                        let handle = self.label_input.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                    Purpose::Relogin(id) => {
                        if let Some(identity) = identity
                            && let Err(error) =
                                self.connections.store().set_identity(id, Some(identity))
                        {
                            tracing::warn!(%error, "no se pudo guardar la identidad de la conexión");
                        }
                        window.focus(&self.focus_handle, cx);
                    }
                    Purpose::Repair(_) => {}
                }
            }
            LoginEvent::Failed { message, reason } => {
                // The friendly sentence replaces Antigravity's own English
                // wording (§"Avisos del inicio de sesión" (b)); the original
                // stays reachable under "Ver detalles técnicos" instead of
                // being thrown away. `flow`'s last use has to come before
                // `self.session = None` below (both borrow `self`).
                if is_antigravity_link_expired(&message) {
                    flow.output.push(message.clone());
                }
                self.session = None;
                self.fail(failure_message(&reason, &message), cx);
                return;
            }
            LoginEvent::Cancelled => {
                if purpose == Purpose::New {
                    flow.pending = None;
                    flow.step = Step::Choose;
                    self.session = None;
                } else {
                    self.session = None;
                    self.close(window, cx);
                    return;
                }
            }
        }
        cx.notify();
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(flow) = self.flow_mut() {
            flow.step = Step::Failed { message };
        }
        cx.notify();
    }

    /// "Enviar": types the pasted code into the login program.
    pub fn submit_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = self.code_input.read(cx).value().trim().to_string();
        if self.session.is_none() {
            return;
        }
        if code.is_empty() {
            if let Some(flow) = self.flow_mut() {
                flow.field_error = Some("Pegá el código que te mostró el navegador.".to_string());
            }
            cx.notify();
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let sent = session.send_code(&code);
        self.code_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let Some(flow) = self.flow_mut() else {
            return;
        };
        match sent {
            Ok(()) => {
                flow.field_error = None;
                flow.needs_code = false;
                flow.step = Step::Waiting;
            }
            Err(error) => flow.field_error = Some(error.to_string()),
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Fills the code field (tests, which cannot type into it).
    pub fn set_code_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.code_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
    }

    /// Fills the name field (tests).
    pub fn set_label_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.label_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
    }

    /// Fills the rename field (tests).
    pub fn set_rename_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
    }

    /// "Copiar" of the link.
    pub fn copy_url(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.connect_flow().and_then(|flow| flow.url.clone()) else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(url));
        if let Some(flow) = self.flow_mut() {
            flow.notice = Some("Enlace copiado.".to_string());
        }
        cx.notify();
    }

    /// "Abrir en el navegador" (the desktop's handler, the xdg portal on
    /// Linux). Without any browser handler the user is told to copy it.
    pub fn open_url(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.connect_flow().and_then(|flow| flow.url.clone()) else {
            return;
        };
        let notice = if browser_available() {
            cx.open_url(&url);
            "Se abrió el navegador. Si no aparece, copiá el enlace.".to_string()
        } else {
            "No se encontró un navegador en este equipo: copiá el enlace y abrilo a mano."
                .to_string()
        };
        if let Some(flow) = self.flow_mut() {
            flow.notice = Some(notice);
        }
        cx.notify();
    }

    /// "Ver detalles técnicos".
    pub fn toggle_details(&mut self, cx: &mut Context<Self>) {
        if let Some(flow) = self.flow_mut() {
            flow.details_open = !flow.details_open;
        }
        cx.notify();
    }

    /// "Cancelar": kills the login's process group, or stops the
    /// "Preparando…" download/install thread for real
    /// (`docs/specs/07-etapa5-productividad.md` §10.3); a new profile is
    /// removed and the flow goes back to "Elegí un agente".
    pub fn cancel_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_session();
        self.discard_pending();
        self.tasks.clear();
        self.generation += 1;
        self.confirm_close = false;
        let purpose = self.connect_flow().map(|flow| flow.purpose);
        match purpose {
            Some(Purpose::New) => {
                self.mode = Mode::Connect(Box::new(ConnectFlow::new(Purpose::New)));
                window.focus(&self.focus_handle, cx);
                cx.notify();
            }
            _ => self.close(window, cx),
        }
    }

    /// "Reintentar" after an error.
    pub fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(flow) = self.connect_flow() else {
            return;
        };
        if !matches!(flow.step, Step::Failed { .. }) {
            return;
        }
        let purpose = flow.purpose;
        let has_kind = flow.kind.is_some();
        let prepared = flow.runtime.done && flow.adapter.done;
        let never_prepared = flow.runtime.text.is_empty();
        self.cancel_session();
        self.discard_pending();
        self.tasks.clear();
        match purpose {
            Purpose::New if !has_kind => {
                if let Some(flow) = self.flow_mut() {
                    flow.step = Step::Choose;
                }
                cx.notify();
            }
            Purpose::Relogin(_) if prepared || never_prepared => self.start_login(window, cx),
            _ => self.start_prepare(window, cx),
        }
    }

    /// "Guardar" (new connection), "Continuar" (after "Volver a conectar")
    /// or "Cerrar" (after "Reparar").
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = self.label_input.read(cx).value().trim().to_string();
        let Some(flow) = self.connect_flow() else {
            return;
        };
        match (flow.purpose, flow.step.clone()) {
            (Purpose::New, Step::Done) => {
                let Some(pending) = flow.pending.clone() else {
                    return;
                };
                let identity = flow.identity.clone();
                if label.is_empty() {
                    if let Some(flow) = self.flow_mut() {
                        flow.field_error = Some("Poné un nombre para la conexión.".to_string());
                    }
                    cx.notify();
                    return;
                }
                match self
                    .connections
                    .store()
                    .finalize(&pending, &label, identity)
                {
                    Ok(connection) => {
                        // Indexed now: closing must not discard it.
                        if let Some(flow) = self.flow_mut() {
                            flow.pending = None;
                        }
                        self.close(window, cx);
                        cx.emit(ModalEvent::Saved { id: connection.id });
                    }
                    Err(error) => {
                        if let Some(flow) = self.flow_mut() {
                            flow.field_error = Some(error.to_string());
                        }
                        cx.notify();
                    }
                }
            }
            (Purpose::Relogin(id), Step::Done) => {
                self.close(window, cx);
                cx.emit(ModalEvent::Relogged { id });
            }
            (Purpose::Repair(id), Step::Repaired) => {
                self.close(window, cx);
                cx.emit(ModalEvent::Repaired { id });
            }
            _ => {}
        }
    }

    // --------------------------------------------------------------- delete

    /// A row of the delete list: the confirmation, with how many
    /// conversations go with it.
    pub fn pick_for_deletion(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(connection) = self.delete_rows.iter().find(|row| row.id == id).cloned() else {
            return;
        };
        let conversations = crate::conversations::count_for_connection_everywhere(&id.to_string());
        self.mode = Mode::Delete(DeleteStep::Confirm {
            id,
            label: connection.label,
            agent_id: connection.agent_id,
            conversations,
        });
        cx.notify();
    }

    /// "Cancelar" of the confirmation: back to the list.
    pub fn back_to_list(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Delete(DeleteStep::List);
        cx.notify();
    }

    /// "Eliminar": asks the workspace to stop the connection's process
    /// ([`ModalEvent::WillDelete`]), which then calls
    /// [`ConnectionsModal::run_delete`].
    pub fn confirm_delete(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Delete(DeleteStep::Confirm {
            id,
            label,
            agent_id,
            ..
        }) = &self.mode
        else {
            return;
        };
        let (id, label, agent_id) = (*id, label.clone(), agent_id.clone());
        self.mode = Mode::Delete(DeleteStep::Running {
            id,
            label,
            agent_id,
        });
        cx.emit(ModalEvent::WillDelete { id });
        cx.notify();
    }

    /// The deletion itself: logout, stop, remove the profile and the index
    /// entry (`Connections::disconnect`); then the workspace deletes the
    /// conversations ([`ModalEvent::Deleted`]).
    pub fn run_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Delete(DeleteStep::Running {
            id,
            label,
            agent_id,
        }) = &self.mode
        else {
            return;
        };
        let (id, label) = (*id, label.clone());
        let google = revoke_at_google(agent_id);
        self.generation += 1;
        let generation = self.generation;

        if !self.background {
            let result = self
                .connections
                .disconnect(id)
                .map_err(|error| error.to_string());
            self.on_deleted(generation, id, label, google, result, window, cx);
            return;
        }
        let (sender, receiver) = async_channel::bounded::<Result<DisconnectReport, String>>(1);
        let connections = self.connections.clone();
        let spawned = std::thread::Builder::new()
            .name("cincel-disconnect".to_string())
            .spawn(move || {
                let result = connections
                    .disconnect(id)
                    .map_err(|error| error.to_string());
                let _ = sender.send_blocking(result);
            });
        if let Err(error) = spawned {
            self.on_deleted(
                generation,
                id,
                label,
                google,
                Err(error.to_string()),
                window,
                cx,
            );
            return;
        }
        self.tasks.push(cx.spawn_in(window, async move |this, cx| {
            if let Ok(result) = receiver.recv().await {
                let _ = this.update_in(cx, |modal, window, cx| {
                    modal.on_deleted(generation, id, label, google, result, window, cx);
                });
            }
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn on_deleted(
        &mut self,
        generation: u64,
        id: Uuid,
        label: String,
        google: bool,
        result: Result<DisconnectReport, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let (lines, clean, report) = match result {
            Ok(report) => {
                let (lines, clean) = report_lines(&report);
                (lines, clean, Some(report))
            }
            Err(message) => (
                vec![format!("No se pudo eliminar la conexión: {message}")],
                false,
                None,
            ),
        };
        cx.emit(ModalEvent::Deleted {
            id,
            label: label.clone(),
            report: report.clone(),
        });
        if clean {
            let message = if google {
                format!("Se eliminó «{label}». {GOOGLE_REVOKE_NOTE}")
            } else {
                format!("Se eliminó «{label}».")
            };
            crate::toast::info(message, cx);
            self.close(window, cx);
        } else {
            self.mode = Mode::Delete(DeleteStep::Result { label, lines });
            cx.notify();
        }
    }

    // --------------------------------------------------------------- rename

    /// "Guardar" of "Renombrar".
    pub fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Rename { id, .. } = &self.mode else {
            return;
        };
        let id = *id;
        let label = self.rename_input.read(cx).value().trim().to_string();
        match self.connections.store().rename(id, &label) {
            Ok(_) => {
                self.close(window, cx);
                cx.emit(ModalEvent::Renamed { id });
            }
            Err(ConnectionsError::EmptyLabel) => {
                self.mode = Mode::Rename {
                    id,
                    error: Some("El nombre no puede quedar vacío.".to_string()),
                };
                cx.notify();
            }
            Err(error) => {
                self.mode = Mode::Rename {
                    id,
                    error: Some(error.to_string()),
                };
                cx.notify();
            }
        }
    }

    // ----------------------------------------------------------------- keys

    /// `Enter` on the modal (not inside a field, which has its own).
    fn on_confirm(&mut self, _: &ModalConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_close {
            self.close(window, cx);
            return;
        }
        match &self.mode {
            Mode::Connect(flow) => match flow.step {
                Step::Done | Step::Repaired => self.save(window, cx),
                Step::Failed { .. } => self.retry(window, cx),
                Step::Link if flow.needs_code => self.submit_code(window, cx),
                _ => {}
            },
            Mode::Delete(DeleteStep::Confirm { .. }) => self.confirm_delete(window, cx),
            Mode::Delete(DeleteStep::Result { .. }) => self.close(window, cx),
            Mode::Rename { .. } => self.save_rename(window, cx),
            _ => {}
        }
    }

    fn on_cancel(&mut self, _: &ModalCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close(window, cx);
    }
}

/// `Enter` in a connection modal.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = workspace, name = "connections_modal_confirm")]
pub struct ModalConfirm;

/// `Esc` in a connection modal.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = workspace, name = "connections_modal_cancel")]
pub struct ModalCancel;

// ------------------------------------------------------------------ helpers

/// The runtime bar for one [`RuntimeProgress`] step.
fn runtime_bar(step: &RuntimeProgress) -> Bar {
    match step {
        RuntimeProgress::Resolving => Bar {
            percent: None,
            text: "Buscando la versión de Node…".to_string(),
            done: false,
        },
        RuntimeProgress::Downloading {
            version,
            done,
            total,
        } => Bar {
            percent: total
                .filter(|total| *total > 0)
                .map(|total| (*done as f32 / total as f32 * 100.).min(100.)),
            text: match total {
                Some(total) => format!(
                    "Descargando Node {version}: {} de {} MB",
                    done / 1_048_576,
                    total / 1_048_576
                ),
                None => format!("Descargando Node {version}: {} MB", done / 1_048_576),
            },
            done: false,
        },
        RuntimeProgress::Retrying { attempt, reason } => Bar {
            percent: None,
            text: format!("Reintentando la descarga (intento {attempt}): {reason}"),
            done: false,
        },
        RuntimeProgress::Verifying => Bar {
            percent: Some(100.),
            text: "Verificando la descarga…".to_string(),
            done: false,
        },
        RuntimeProgress::Extracting => Bar {
            percent: Some(100.),
            text: "Descomprimiendo…".to_string(),
            done: false,
        },
        RuntimeProgress::Done => Bar {
            percent: Some(100.),
            text: "Entorno de Node listo".to_string(),
            done: true,
        },
    }
}

/// "Para revocar el acceso…" for agents that sign in with a Google account.
const GOOGLE_REVOKE_NOTE: &str = "Para revocar el acceso, hacelo desde tu cuenta de Google.";

/// The note under a binary download without a published SHA-256.
pub const UNVERIFIED_NOTE: &str = "Google no publica una suma de verificación para este paquete: \
     Cincel lo descarga directamente de los servidores de Google, por HTTPS.";

/// Whether deleting a connection of `agent_id` leaves access to revoke from
/// the user's Google account (Antigravity signs in with Google).
fn revoke_at_google(agent_id: &str) -> bool {
    agent_id == AgentKind::Antigravity.agent_id()
}

/// Status line of an agent card: "instalado", or "se descargará" with the
/// download size when it is known before the download starts (binary
/// archives; npm packages only learn theirs while resolving).
fn card_status(installed: bool, kind: AgentKind) -> String {
    if installed {
        return "instalado".to_string();
    }
    match cincel_connections::download_size_hint(kind) {
        Some(bytes) => format!("se descargará ({} MB)", megabytes(bytes)),
        None => "se descargará".to_string(),
    }
}

/// Whole megabytes, decimal (as the download size is published: 333 MB).
fn megabytes(bytes: u64) -> u64 {
    bytes / 1_000_000
}

/// The adapter bar for one [`AdapterProgress`] step of the agent `name`.
fn adapter_bar(step: &AdapterProgress, name: &str) -> Bar {
    match step {
        AdapterProgress::Installing { package } => Bar {
            percent: None,
            text: format!("Instalando {package}…"),
            done: false,
        },
        AdapterProgress::Downloading {
            version,
            done,
            total,
            ..
        } => {
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| (*done as f32 / total as f32 * 100.).min(100.));
            Bar {
                percent,
                text: match (total, percent) {
                    (Some(total), Some(percent)) => format!(
                        "Descargando {name} {version}: {} de {} MB ({:.0} %)",
                        megabytes(*done),
                        megabytes(*total),
                        percent.floor()
                    ),
                    _ => format!("Descargando {name} {version}: {} MB", megabytes(*done)),
                },
                done: false,
            }
        }
        AdapterProgress::Retrying { attempt, reason } => Bar {
            percent: None,
            text: format!("Reintentando la descarga (intento {attempt}): {reason}"),
            done: false,
        },
        AdapterProgress::Verifying => Bar {
            percent: Some(100.),
            text: "Verificando la descarga…".to_string(),
            done: false,
        },
        AdapterProgress::Extracting => Bar {
            percent: Some(100.),
            text: "Descomprimiendo…".to_string(),
            done: false,
        },
        AdapterProgress::Done { version } => Bar {
            percent: Some(100.),
            text: format!("{name} {version} listo"),
            done: true,
        },
    }
}

/// What to say when preparing (or starting the login) failed.
/// Turns a `prepare` error into what `PrepareMessage::Done` carries:
/// `Cancelled` on its own variant (never shown as "Algo salió mal",
/// `docs/specs/07-etapa5-productividad.md` §10.3), anything else formatted
/// by [`prepare_error_message`].
fn prepare_failure(error: ConnectionsError) -> PrepareFailure {
    match error {
        ConnectionsError::Cancelled => PrepareFailure::Cancelled,
        other => PrepareFailure::Message(prepare_error_message(&other)),
    }
}

fn prepare_error_message(error: &ConnectionsError) -> String {
    match error {
        ConnectionsError::Network(detail) => format!(
            "Sin conexión a internet: hace falta conexión para instalar el agente ({detail})."
        ),
        ConnectionsError::RuntimeMissing | ConnectionsError::AdapterMissing(_) => {
            "Hace falta conexión a internet para instalar el agente la primera vez.".to_string()
        }
        ConnectionsError::NotEnoughSpace {
            agent,
            path,
            needed,
            available,
        } => format!(
            "No hay espacio suficiente en disco para instalar {agent}: hacen falta {} libres en \
             {} y hay {}. Liberá espacio y reintentá.",
            cincel_connections::human_size(*needed),
            path.display(),
            cincel_connections::human_size(*available)
        ),
        other => format!("No se pudo preparar el agente: {other}"),
    }
}

/// Whether `message` is Antigravity's own login-server timeout
/// (`agy_acp_server`'s `oauth/credential_manager.py`,
/// `_LOGIN_TIMEOUT_SECONDS = 300`): it gives up on the browser after 5
/// minutes and answers `authenticate` with English text to that effect,
/// well before Cincel's own 15-minute `LOGIN_TIMEOUT`. Recognized by
/// substring, since the exact wording is the provider's, not ours.
fn is_antigravity_link_expired(message: &str) -> bool {
    message.contains("Timed out waiting for the authentication flow")
        || message.contains("Onboarding failed")
}

/// What to say when the login failed.
fn failure_message(reason: &LoginFailure, message: &str) -> String {
    if is_antigravity_link_expired(message) {
        return "El enlace venció sin completarse. Antigravity da 5 minutos para iniciar sesión."
            .to_string();
    }
    match reason {
        LoginFailure::Timeout => {
            "Se agotó el tiempo de espera (15 minutos) sin que aprobaras el acceso.".to_string()
        }
        LoginFailure::Exit(Some(code)) => {
            format!("El inicio de sesión terminó con un error (código {code}). {message}")
        }
        LoginFailure::Exit(None) => format!("El inicio de sesión se interrumpió. {message}"),
        LoginFailure::Spawn => format!("No se pudo iniciar el inicio de sesión: {message}"),
        LoginFailure::NotLoggedIn => {
            "El agente terminó, pero no quedó ninguna sesión iniciada.".to_string()
        }
        LoginFailure::Rejected => format!("El agente rechazó el inicio de sesión: {message}"),
        _ => format!("El inicio de sesión falló: {message}"),
    }
}

/// The muted line next to the link (§"Avisos del inicio de sesión" (a)):
/// Antigravity's own server gives up after 5 minutes
/// (`is_antigravity_link_expired`'s doc); Claude's and Codex's CLIs have no
/// timeout of their own, so the number shown is Cincel's own
/// [`cincel_connections::LOGIN_TIMEOUT`] (15 minutes).
fn link_expiry_notice(kind: Option<AgentKind>) -> &'static str {
    if kind == Some(AgentKind::Antigravity) {
        "Este enlace vence en 5 minutos."
    } else {
        "Abrilo dentro de los próximos 15 minutos."
    }
}

/// One line per step of a deletion, and whether all of them went well.
fn report_lines(report: &DisconnectReport) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut clean = true;
    match &report.logout {
        LogoutStep::LoggedOut => lines.push("Sesión cerrada en el agente.".to_string()),
        LogoutStep::NotSupported => lines.push(
            "El agente no permite cerrar la sesión desde Cincel: para revocar el acceso, hacelo \
             desde la cuenta del proveedor."
                .to_string(),
        ),
        LogoutStep::Skipped(why) => {
            clean = false;
            lines.push(format!("No se cerró la sesión en el agente: {why}"));
        }
        LogoutStep::Failed(why) => {
            clean = false;
            lines.push(format!("No se pudo cerrar la sesión en el agente: {why}"));
        }
    }
    if report.profile_removed {
        lines.push("Credenciales borradas de este equipo.".to_string());
    } else {
        clean = false;
        lines.push(format!(
            "No se pudieron borrar las credenciales: {}",
            report
                .profile_error
                .clone()
                .unwrap_or_else(|| "motivo desconocido".to_string())
        ));
    }
    if report.index_removed {
        lines.push("Conexión quitada de la lista.".to_string());
    } else {
        clean = false;
        lines.push("La conexión sigue en la lista para no perder sus credenciales.".to_string());
    }
    (lines, clean)
}

/// Whether something can open a URL: `$BROWSER` or `xdg-open` on the `PATH`.
fn browser_available() -> bool {
    if std::env::var_os("BROWSER").is_some_and(|value| !value.is_empty()) {
        return true;
    }
    if cfg!(target_os = "macos") {
        return true;
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("xdg-open").is_file())
    })
}

/// "¿Eliminar «X»? …" with its exact wording (spec 06 §4 F3).
#[must_use]
pub fn delete_question(label: &str, conversations: usize) -> String {
    let tail = if conversations == 1 {
        "su 1 conversación".to_string()
    } else {
        format!("sus {conversations} conversaciones")
    };
    format!(
        "¿Eliminar «{label}»? Se cerrará la sesión, se borrarán sus credenciales de este equipo y {tail}."
    )
}

/// One button of the modals: the save dialog's look (surface, 1 px border,
/// radius 4, 28 px tall). `default` adds the focus ring to the one `Enter`
/// triggers. None is coloured: the words carry the meaning.
fn modal_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    default: bool,
    theme: &ThemeColors,
    scale: f32,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.bg_elevated;
    div()
        .id(id.into())
        .h(px(BUTTON_HEIGHT * scale))
        .px(px(12. * scale))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(4. * scale))
        .bg(theme.bg_surface)
        .border_1()
        .border_color(if default {
            theme.border_focus
        } else {
            theme.border
        })
        .text_color(theme.text)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label.into())
}

// ---------------------------------------------------------------- render

impl Render for ConnectionsModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_open() {
            return div().into_any_element();
        }
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let body = match &self.mode {
            Mode::Closed => div().into_any_element(),
            Mode::Connect(flow) => self.render_connect(flow, &theme, scale, cx),
            Mode::Delete(step) => self.render_delete(step, &theme, scale, cx),
            Mode::Rename { error, .. } => self.render_rename(error.as_deref(), &theme, scale, cx),
        };
        let confirm = self
            .confirm_close
            .then(|| self.render_confirm_close(&theme, scale, cx));

        div()
            .id("connections-modal")
            .key_context(MODAL_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_cancel))
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            // Scrim: what is behind stays visible but out of reach.
            .bg(theme.bg_app.alpha(0.4))
            .p(px(DIALOG_MARGIN * scale))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                v_flex()
                    .id("connections-dialog")
                    .debug_selector(|| "connections-dialog".to_string())
                    .relative()
                    // Follows the window between the two bounds (spec 06 §6).
                    .w_full()
                    .max_w(px(DIALOG_MAX_WIDTH * scale))
                    .min_w(px(DIALOG_MIN_WIDTH * scale))
                    .max_h(px(620. * scale))
                    .overflow_x_hidden()
                    .overflow_y_scroll()
                    .p_4()
                    .gap_3()
                    .rounded(px(6. * scale))
                    .bg(theme.bg_elevated)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .text_sm()
                    .text_color(theme.text)
                    .child(body)
                    .children(confirm),
            )
            .into_any_element()
    }
}

impl ConnectionsModal {
    fn title(text: impl Into<SharedString>, theme: &ThemeColors, scale: f32) -> gpui::Div {
        div()
            .text_size(px(14. * scale))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text)
            .child(text.into())
    }

    fn muted(text: impl Into<SharedString>, theme: &ThemeColors) -> gpui::Div {
        div()
            .whitespace_normal()
            .text_color(theme.text_muted)
            .child(text.into())
    }

    fn render_connect(
        &self,
        flow: &ConnectFlow,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let name = flow
            .kind
            .map(AgentKind::display_name)
            .unwrap_or("el agente");
        let mut column = v_flex().w_full().min_w_0().gap_3();
        match &flow.step {
            Step::Choose => {
                column = column
                    .child(Self::title("Elegí un agente", theme, scale))
                    .child(Self::muted(
                        "Iniciás sesión con tu suscripción; Cincel guarda las credenciales en \
                         un perfil propio, aparte de las de tu terminal.",
                        theme,
                    ))
                    .child(self.render_agent_grid(theme, scale, cx))
                    .child(h_flex().justify_end().child(
                        modal_button("connect-close", "Cancelar", false, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                        ),
                    ));
            }
            Step::Preparing => {
                let needs_node = flow.kind.is_none_or(AgentKind::needs_node);
                let full_name = flow.kind.map_or("El agente", AgentKind::full_name);
                column = column
                    .child(Self::title("Preparando…", theme, scale))
                    .child(Self::muted(
                        if needs_node {
                            format!(
                                "{name} no viene dentro de Cincel: es el programa oficial de su fabricante. \
                                 Cincel lo descarga una sola vez (y el entorno que necesita para correr) \
                                 en su propia carpeta, sin instalar nada en tu sistema. \
                                 Solo volverá a descargarse si aceptás una actualización."
                            )
                        } else {
                            format!(
                                "{full_name} no viene dentro de Cincel: es el agente oficial de Google. \
                                 Cincel lo descarga una sola vez (unos 333 MB, que ocupan 1 GB al \
                                 descomprimirse) en su propia carpeta, sin instalar nada en tu sistema. \
                                 Solo volverá a descargarse si aceptás una actualización."
                            )
                        },
                        theme,
                    ))
                    .when(needs_node, |this| {
                        this.child(progress_row(
                            "prepare-runtime",
                            "Entorno de Node",
                            &flow.runtime,
                            theme,
                        ))
                    })
                    .child(progress_row(
                        "prepare-adapter",
                        if needs_node {
                            "Adaptador"
                        } else {
                            "Agente oficial"
                        },
                        &flow.adapter,
                        theme,
                    ))
                    .when(flow.unverified, |this| {
                        this.child(
                            div()
                                .id("prepare-unverified")
                                .whitespace_normal()
                                .text_xs()
                                .text_color(theme.status_warning)
                                .child(UNVERIFIED_NOTE),
                        )
                    })
                    .child(h_flex().justify_end().child(
                        modal_button("prepare-cancel", "Cancelar", false, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_login(window, cx)
                            }),
                        ),
                    ));
            }
            Step::StartingLogin => {
                column = column
                    .child(Self::title("Iniciando sesión…", theme, scale))
                    .child(h_flex().gap_2().items_center().child(Spinner::new()).child(
                        Self::muted(
                            if flow.queued {
                                format!("Esperando que termine otro inicio de sesión de {name}…")
                            } else {
                                format!("Abriendo el inicio de sesión de {name}…")
                            },
                            theme,
                        ),
                    ))
                    .child(self.render_details(flow, theme, scale, cx))
                    .child(h_flex().justify_end().child(
                        modal_button("login-cancel", "Cancelar", false, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_login(window, cx)
                            }),
                        ),
                    ));
            }
            Step::Link | Step::Waiting => {
                let waiting = flow.step == Step::Waiting;
                column = column
                    .child(Self::title(
                        if waiting {
                            "Esperando que apruebes en el navegador…"
                        } else {
                            "Abrí este enlace y aprobá el acceso"
                        },
                        theme,
                        scale,
                    ))
                    .children(
                        flow.url
                            .clone()
                            .map(|url| self.render_link(&url, theme, scale, cx)),
                    )
                    .when(flow.url.is_some(), |this| {
                        this.child(
                            Self::muted(link_expiry_notice(flow.kind), theme)
                                .id("login-link-expiry")
                                .debug_selector(|| "login-link-expiry".to_string()),
                        )
                    })
                    .children(flow.notice.clone().map(|notice| Self::muted(notice, theme)))
                    .when(
                        flow.kind == Some(AgentKind::Antigravity) && flow.url.is_some(),
                        |this| {
                            this.child(Self::muted(
                                "Cuando apruebes el acceso, Google vuelve a Cincel solo: no hay \
                                 código que pegar.",
                                theme,
                            ))
                        },
                    )
                    .children(flow.code.clone().map(|code| {
                        v_flex()
                            .gap_1()
                            .child(Self::muted("Escribí este código en el navegador:", theme))
                            .child(
                                div()
                                    .id("login-device-code")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4. * scale))
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(theme.border)
                                    .font_family(mono_family(cx))
                                    .text_size(px(16. * scale))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(code)),
                            )
                    }))
                    .when(flow.needs_code, |this| {
                        this.child(
                            v_flex()
                                .gap_1()
                                .child(div().child("Pegá acá el código que te dio el navegador"))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(div().flex_1().child(Input::new(&self.code_input)))
                                        .child(
                                            modal_button(
                                                "login-send-code",
                                                "Enviar",
                                                true,
                                                theme,
                                                scale,
                                            )
                                            .on_click(
                                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                                    this.submit_code(window, cx)
                                                }),
                                            ),
                                        ),
                                ),
                        )
                    })
                    .children(flow.field_error.clone().map(|error| {
                        div()
                            .text_color(theme.status_error)
                            .child(SharedString::from(error))
                    }))
                    .child(h_flex().gap_2().items_center().child(Spinner::new()).child(
                        Self::muted("Esperando que apruebes en el navegador…", theme),
                    ))
                    .child(self.render_details(flow, theme, scale, cx))
                    .child(h_flex().justify_end().child(
                        modal_button("login-cancel", "Cancelar", false, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_login(window, cx)
                            }),
                        ),
                    ));
            }
            Step::Done => {
                let who = flow.identity.as_ref().and_then(identity_sentence);
                let headline = match &who {
                    Some(who) => format!("Listo: conectado como {who}"),
                    None => "Conectado".to_string(),
                };
                column = column.child(Self::title("Listo", theme, scale)).child(
                    div()
                        .text_color(theme.status_ok)
                        .child(SharedString::from(headline)),
                );
                match flow.purpose {
                    Purpose::New => {
                        column = column
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().child("Nombre de la conexión"))
                                    .child(Input::new(&self.label_input)),
                            )
                            .children(flow.field_error.clone().map(|error| {
                                div()
                                    .text_color(theme.status_error)
                                    .child(SharedString::from(error))
                            }))
                            .child(
                                h_flex().justify_end().gap_2().child(
                                    modal_button("connect-save", "Guardar", true, theme, scale)
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.save(window, cx)
                                            },
                                        )),
                                ),
                            );
                    }
                    _ => {
                        let label = flow.label.clone().unwrap_or_default();
                        column = column
                            .child(Self::muted(
                                format!("«{label}» conserva su nombre y sus conversaciones."),
                                theme,
                            ))
                            .child(
                                h_flex().justify_end().child(
                                    modal_button("relogin-done", "Continuar", true, theme, scale)
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.save(window, cx)
                                            },
                                        )),
                                ),
                            );
                    }
                }
            }
            Step::Repaired => {
                let label = flow.label.clone().unwrap_or_default();
                column = column
                    .child(Self::title("Listo", theme, scale))
                    .child(div().text_color(theme.status_ok).child(SharedString::from(format!(
                        "«{label}» ya tiene todo lo que necesita. Las credenciales no se tocaron."
                    ))))
                    .child(
                        h_flex().justify_end().child(
                            modal_button("repair-done", "Cerrar", true, theme, scale).on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| this.save(window, cx)),
                            ),
                        ),
                    );
            }
            Step::Failed { message } => {
                column = column
                    .child(Self::title("No se pudo conectar", theme, scale))
                    // §"Avisos del inicio de sesión" (c): a link already
                    // detected before the error stays on screen, above it —
                    // "Abrir en el navegador" and "Copiar" keep working even
                    // after the login itself failed.
                    .children(
                        flow.url
                            .clone()
                            .map(|url| self.render_link(&url, theme, scale, cx)),
                    )
                    .child(
                        div()
                            .id("connect-error")
                            .debug_selector(|| "connect-error".to_string())
                            .whitespace_normal()
                            .text_color(theme.status_error)
                            .child(SharedString::from(message.clone())),
                    )
                    .child(self.render_details(flow, theme, scale, cx))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                modal_button("connect-fail-close", "Cerrar", false, theme, scale)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.close(window, cx)
                                    })),
                            )
                            .child(
                                modal_button("connect-retry", "Reintentar", true, theme, scale)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.retry(window, cx)
                                    })),
                            ),
                    );
            }
        }
        column.into_any_element()
    }

    /// The agent cards of step 1 (Claude, Codex, Google Antigravity, and
    /// whatever joins them): a wrapping grid of cards at least
    /// [`AGENT_CARD_MIN_WIDTH`] wide that grow to share each row, so any
    /// number of agents lays out in rows inside the dialog.
    fn render_agent_grid(
        &self,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let hover = theme.bg_surface;
        let focus = theme.border_focus;
        h_flex()
            .id("agent-grid")
            .debug_selector(|| "agent-grid".to_string())
            .w_full()
            .min_w_0()
            .flex_wrap()
            // Cards of one row share its height.
            .items_stretch()
            .gap_2()
            .children(AgentKind::ALL.iter().enumerate().map(|(index, kind)| {
                let kind = *kind;
                let status = card_status(self.connections.is_ready(kind), kind);
                v_flex()
                    .id(("agent-card", index))
                    .debug_selector(move || format!("agent-card-{index}"))
                    .flex_grow_1()
                    .flex_shrink(1.)
                    .flex_basis(px(AGENT_CARD_MIN_WIDTH * scale))
                    .min_w(px(AGENT_CARD_MIN_WIDTH * scale))
                    .overflow_hidden()
                    .p_3()
                    .gap_1()
                    .items_start()
                    .rounded(px(4. * scale))
                    .bg(theme.bg_app)
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover).border_color(focus))
                    .child(cincel_chat::provider_icon(
                        kind.agent_id(),
                        px(22. * scale),
                        theme.text,
                        theme.bg_surface,
                        theme.border,
                    ))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_size(px(14. * scale))
                            .font_weight(FontWeight::MEDIUM)
                            .child(kind.full_name()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_size(px(12. * scale))
                            .text_color(theme.text_muted)
                            .child(kind.description()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_size(px(12. * scale))
                            .text_color(theme.text_muted)
                            .child(status),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.choose(kind, window, cx);
                    }))
            }))
            .into_any_element()
    }

    /// The login link in monospace with "Copiar" and "Abrir en el navegador".
    fn render_link(
        &self,
        url: &str,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .id("login-url")
                    .debug_selector(|| "login-url".to_string())
                    .p_2()
                    .rounded(px(4. * scale))
                    .bg(theme.bg_surface)
                    .border_1()
                    .border_color(theme.border)
                    .font_family(mono_family(cx))
                    .text_xs()
                    .whitespace_normal()
                    .child(SharedString::from(url.to_string())),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        modal_button("login-copy", "Copiar", false, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, _window, cx| this.copy_url(cx)),
                        ),
                    )
                    .child(
                        modal_button("login-open", "Abrir en el navegador", true, theme, scale)
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _window, cx| this.open_url(cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// "Ver detalles técnicos": the redacted terminal output.
    fn render_details(
        &self,
        flow: &ConnectFlow,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = flow.details_open;
        let accent = theme.text_accent;
        v_flex()
            .gap_1()
            .child(
                div()
                    .id("login-details-toggle")
                    .cursor_pointer()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .hover(move |style| style.text_color(accent))
                    .child(if open {
                        "▾ Ver detalles técnicos"
                    } else {
                        "▸ Ver detalles técnicos"
                    })
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _window, cx| this.toggle_details(cx)),
                    ),
            )
            .when(open, |this| {
                let text = if flow.output.is_empty() {
                    "(sin salida todavía)".to_string()
                } else {
                    flow.output.join("\n")
                };
                this.child(
                    div()
                        .id("login-details")
                        .max_h(px(160. * scale))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded(px(4. * scale))
                        .bg(theme.bg_editor)
                        .border_1()
                        .border_color(theme.border)
                        .font_family(mono_family(cx))
                        .text_xs()
                        .text_color(theme.text_muted)
                        .whitespace_normal()
                        .child(SharedString::from(text)),
                )
            })
            .into_any_element()
    }

    fn render_delete(
        &self,
        step: &DeleteStep,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut column = v_flex().gap_3();
        match step {
            DeleteStep::List => {
                let selection = theme.selection;
                column = column
                    .child(Self::title("Eliminar conexión", theme, scale))
                    .child(Self::muted("Elegí la conexión que querés olvidar.", theme))
                    .when(self.delete_rows.is_empty(), |this| {
                        this.child(Self::muted("No hay conexiones guardadas.", theme))
                    })
                    .child(
                        v_flex()
                            .gap_0p5()
                            .children(self.delete_rows.iter().enumerate().map(|(index, row)| {
                                let id = row.id;
                                h_flex()
                                    .id(("delete-row", index))
                                    .gap_2()
                                    .px_2()
                                    .py_1()
                                    .items_center()
                                    .rounded(px(4. * scale))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(selection))
                                    .child(cincel_chat::provider_icon(
                                        &row.agent_id,
                                        px(18. * scale),
                                        theme.text,
                                        theme.bg_surface,
                                        theme.border,
                                    ))
                                    .child(
                                        div().flex_1().child(SharedString::from(row.label.clone())),
                                    )
                                    .children(
                                        row.identity
                                            .as_ref()
                                            .and_then(|identity| identity.summary())
                                            .map(|summary| {
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.text_muted)
                                                    .child(SharedString::from(summary))
                                            }),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _window, cx| {
                                            this.pick_for_deletion(id, cx);
                                        },
                                    ))
                            })),
                    )
                    .child(h_flex().justify_end().child(
                        modal_button("delete-close", "Cancelar", false, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                        ),
                    ));
            }
            DeleteStep::Confirm {
                label,
                agent_id,
                conversations,
                ..
            } => {
                let google = revoke_at_google(agent_id);
                column = column
                    .child(Self::title("Eliminar conexión", theme, scale))
                    .child(
                        div()
                            .id("delete-question")
                            .whitespace_normal()
                            .child(SharedString::from(delete_question(label, *conversations))),
                    )
                    .when(google, |this| {
                        this.child(Self::muted(GOOGLE_REVOKE_NOTE, theme))
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                modal_button("delete-cancel", "Cancelar", false, theme, scale)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.back_to_list(cx)
                                    })),
                            )
                            .child(
                                modal_button("delete-confirm", "Eliminar", true, theme, scale)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.confirm_delete(window, cx)
                                    })),
                            ),
                    );
            }
            DeleteStep::Running { label, .. } => {
                column = column
                    .child(Self::title("Eliminar conexión", theme, scale))
                    .child(h_flex().gap_2().items_center().child(Spinner::new()).child(
                        Self::muted(
                            format!("Cerrando la sesión de «{label}» y borrando sus datos…"),
                            theme,
                        ),
                    ));
            }
            DeleteStep::Result { label, lines } => {
                column = column
                    .child(Self::title(format!("Eliminar «{label}»"), theme, scale))
                    .child(v_flex().gap_1().children(lines.iter().map(|line| {
                        div()
                            .whitespace_normal()
                            .child(SharedString::from(format!("• {line}")))
                    })))
                    .child(h_flex().justify_end().child(
                        modal_button("delete-result-close", "Cerrar", true, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                        ),
                    ));
            }
        }
        column.into_any_element()
    }

    fn render_rename(
        &self,
        error: Option<&str>,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        v_flex()
            .gap_3()
            .child(Self::title("Renombrar conexión", theme, scale))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().child("Nombre de la conexión"))
                    .child(Input::new(&self.rename_input)),
            )
            .children(error.map(|error| {
                div()
                    .text_color(theme.status_error)
                    .child(SharedString::from(error.to_string()))
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        modal_button("rename-cancel", "Cancelar", false, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                        ),
                    )
                    .child(
                        modal_button("rename-save", "Guardar", true, theme, scale).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.save_rename(window, cx)
                            }),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_confirm_close(
        &self,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let unsaved = matches!(
            &self.mode,
            Mode::Connect(flow) if flow.step == Step::Done && flow.purpose == Purpose::New
        );
        let question = if unsaved {
            "La conexión todavía no se guardó. Si cerrás ahora, se descarta."
        } else {
            "Hay un inicio de sesión en curso. Si cerrás ahora, se cancela."
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6. * scale))
            .bg(theme.bg_elevated.alpha(0.92))
            .child(
                v_flex()
                    .id("connections-confirm-close")
                    .w_full()
                    .max_w(px(380. * scale))
                    .mx_4()
                    .p_4()
                    .gap_3()
                    .rounded(px(6. * scale))
                    .bg(theme.bg_elevated)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .child(div().whitespace_normal().child(question))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                modal_button("confirm-keep", "Seguir acá", false, theme, scale)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.keep_open(cx)
                                    })),
                            )
                            .child(
                                modal_button("confirm-close", "Cerrar igual", true, theme, scale)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.close(window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// A labelled progress bar of "Preparando…".
fn progress_row(
    id: &'static str,
    title: &'static str,
    bar: &Bar,
    theme: &ThemeColors,
) -> gpui::AnyElement {
    v_flex()
        .gap_1()
        .child(
            h_flex().justify_between().child(div().child(title)).child(
                div()
                    .text_xs()
                    .text_color(if bar.done {
                        theme.status_ok
                    } else {
                        theme.text_muted
                    })
                    .child(if bar.done { "listo" } else { "" }),
            ),
        )
        .child(
            Progress::new(id)
                .loading(bar.percent.is_none() && !bar.done)
                .value(bar.percent.unwrap_or(if bar.done { 100. } else { 0. })),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.text_muted)
                .child(SharedString::from(bar.text.clone())),
        )
        .into_any_element()
}

/// "ana@example.com (Claude Max)" for the "Listo" line.
fn identity_sentence(identity: &Identity) -> Option<String> {
    match (&identity.email, &identity.plan) {
        (Some(email), Some(plan)) => Some(format!("{email} ({plan})")),
        (Some(email), None) => Some(email.clone()),
        (None, Some(plan)) => Some(plan.clone()),
        (None, None) => identity.organization.clone(),
    }
}

/// The interface's monospace family (the link, the code, the output).
fn mono_family(cx: &App) -> SharedString {
    gpui_kit::base::Theme::global(cx)
        .tokens
        .typography
        .mono
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_cards_say_installed_or_the_download_size() {
        assert_eq!(card_status(true, AgentKind::Antigravity), "instalado");
        assert_eq!(
            card_status(false, AgentKind::Antigravity),
            "se descargará (333 MB)"
        );
        assert_eq!(card_status(false, AgentKind::Claude), "se descargará");
        assert_eq!(card_status(true, AgentKind::Codex), "instalado");
    }

    #[test]
    fn the_delete_question_uses_the_exact_wording() {
        assert_eq!(
            delete_question("Codex · trabajo", 3),
            "¿Eliminar «Codex · trabajo»? Se cerrará la sesión, se borrarán sus credenciales de \
             este equipo y sus 3 conversaciones."
        );
        assert!(delete_question("X", 1).ends_with("y su 1 conversación."));
        assert!(delete_question("X", 0).ends_with("y sus 0 conversaciones."));
    }

    #[test]
    fn every_login_failure_has_a_clear_message() {
        assert!(failure_message(&LoginFailure::Timeout, "").contains("tiempo de espera"));
        assert!(failure_message(&LoginFailure::Exit(Some(1)), "boom").contains("código 1"));
        assert!(failure_message(&LoginFailure::Spawn, "pty").contains("pty"));
        assert!(failure_message(&LoginFailure::NotLoggedIn, "").contains("ninguna sesión"));
        assert!(failure_message(&LoginFailure::Rejected, "denegado").contains("denegado"));
        let space = prepare_error_message(&ConnectionsError::NotEnoughSpace {
            agent: "Google Antigravity".into(),
            path: "/datos".into(),
            needed: 1_400_000_000,
            available: 300_000_000,
        });
        assert!(
            space
                .starts_with("No hay espacio suficiente en disco para instalar Google Antigravity"),
            "{space}"
        );
        assert!(
            space.contains("1,4 GB") && space.contains("300 MB"),
            "{space}"
        );
        assert!(
            prepare_error_message(&ConnectionsError::Network("dns".into()))
                .contains("Sin conexión a internet")
        );
        assert!(prepare_error_message(&ConnectionsError::RuntimeMissing).contains("internet"));
    }

    /// §"Avisos del inicio de sesión" (a) y (b).
    #[test]
    fn antigravity_link_expiry_gets_a_plain_language_message() {
        assert!(is_antigravity_link_expired(
            "Onboarding failed: Timed out waiting for the authentication flow to complete"
        ));
        assert!(is_antigravity_link_expired("Onboarding failed: otra cosa"));
        assert!(!is_antigravity_link_expired("denegado"));

        let message = failure_message(
            &LoginFailure::Rejected,
            "Onboarding failed: Timed out waiting for the authentication flow to complete",
        );
        assert_eq!(
            message,
            "El enlace venció sin completarse. Antigravity da 5 minutos para iniciar sesión."
        );
        // A `Rejected` failure that is *not* the link expiring keeps the raw
        // wording, same as before.
        assert!(failure_message(&LoginFailure::Rejected, "denegado").contains("denegado"));

        assert_eq!(
            link_expiry_notice(Some(AgentKind::Antigravity)),
            "Este enlace vence en 5 minutos."
        );
        assert_eq!(
            link_expiry_notice(Some(AgentKind::Claude)),
            "Abrilo dentro de los próximos 15 minutos."
        );
        assert_eq!(
            link_expiry_notice(Some(AgentKind::Codex)),
            "Abrilo dentro de los próximos 15 minutos."
        );
    }

    #[test]
    fn runtime_progress_fills_the_bar() {
        let bar = runtime_bar(&RuntimeProgress::Downloading {
            version: "v24.1.0".into(),
            done: 25,
            total: Some(100),
        });
        assert_eq!(bar.percent, Some(25.));
        assert!(!bar.done);
        assert!(runtime_bar(&RuntimeProgress::Resolving).percent.is_none());
        assert!(runtime_bar(&RuntimeProgress::Done).done);
        assert!(
            adapter_bar(
                &AdapterProgress::Done {
                    version: "1.0".into()
                },
                "Codex"
            )
            .done
        );
        // The 333 MB download of Antigravity, in decimal MB with a percentage.
        let bar = adapter_bar(
            &AdapterProgress::Downloading {
                version: "1.2.1".into(),
                done: 120_000_000,
                total: Some(333_590_110),
                verifiable: false,
            },
            "Google Antigravity",
        );
        assert_eq!(
            bar.text,
            "Descargando Google Antigravity 1.2.1: 120 de 333 MB (35 %)"
        );
        assert!(
            bar.percent
                .is_some_and(|percent| (35.0..36.0).contains(&percent))
        );
        assert!(
            adapter_bar(&AdapterProgress::Extracting, "Google Antigravity")
                .text
                .contains("Descomprimiendo")
        );
    }

    #[test]
    fn a_partial_deletion_is_reported_step_by_step() {
        let report = DisconnectReport {
            logout: LogoutStep::Failed("tiempo agotado".into()),
            process_stopped: true,
            profile_removed: true,
            index_removed: true,
            profile_error: None,
        };
        let (lines, clean) = report_lines(&report);
        assert!(!clean);
        assert!(lines[0].contains("tiempo agotado"), "{lines:?}");
        let no_logout = DisconnectReport {
            logout: LogoutStep::NotSupported,
            ..report
        };
        let (lines, clean) = report_lines(&no_logout);
        assert!(clean);
        assert!(lines[0].contains("cuenta del proveedor"), "{lines:?}");
        assert!(revoke_at_google("antigravity-acp"));
        assert!(!revoke_at_google("claude-acp"));
    }

    #[test]
    fn the_done_line_names_the_account() {
        let identity = Identity {
            email: Some("ana@example.com".into()),
            plan: Some("Claude Max".into()),
            organization: None,
        };
        assert_eq!(
            identity_sentence(&identity).as_deref(),
            Some("ana@example.com (Claude Max)")
        );
        assert_eq!(identity_sentence(&Identity::default()), None);
    }
}
