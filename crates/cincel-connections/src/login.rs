//! Login flow engine (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4 F2
//! steps 3-5), in two modes:
//!
//! * **pty** (Claude, Codex): the provider's own login command runs in a
//!   hidden pseudo-terminal with the profile environment; its output is
//!   scanned for the login link, a one-time code and the "paste the code"
//!   prompt; the user can paste a code back; success is exit code 0 (or a
//!   success file appearing), confirmed by an identity probe.
//! * **ACP authenticate** (Antigravity): the adapter itself is spawned over
//!   ACP with the profile environment, `initialize` (announcing
//!   `elicitation.url`) then `authenticate { methodId }`. The link comes on
//!   the agent's stderr (`Open the following link to authenticate the ACP
//!   server: https://accounts.google.com/...`) or as an `elicitation/create`
//!   in `url` mode; the agent's own local redirect server completes the
//!   login when the user approves in the browser (no code to paste);
//!   success is the `authenticate` response, confirmed by the credentials
//!   file.
//!
//! Logins of the same provider are serialized (Codex's callback listens on a
//! fixed port): a second session for the same agent waits, reporting
//! [`LoginEvent::Queued`], until the first one ends.

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use cincel_acp::acp::schema::v1::{
    AuthMethod, CreateElicitationResponse, ElicitationAcceptAction, ElicitationAction,
    ElicitationMode,
};
use cincel_acp::{AgentCommand, AgentConnection, AgentEvent, LaunchSpec, ProcessEnv};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::adapters::Adapters;
use crate::envutil;
use crate::error::{ConnectionsError, Result};
use crate::identity::{AcpIdentityProbe, CliStatusProbe, IdentityProbe, ProfileProbe};
use crate::paths::remove_dir_within;
use crate::profile::{AgentKind, LoginMethod, Profile};
use crate::runtime::NodePaths;
use crate::scanner::{OutputScanner, find_login_urls, redact_line, strip_ansi};
use crate::store::Identity;

/// Default login timeout (spec 06 §4 F2: 15 minutes).
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How often the session thread checks exit, cancel, timeout and the
/// success file.
const POLL: Duration = Duration::from_millis(50);

/// Events of one [`LoginSession`], in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginEvent {
    /// Another login of the same provider is running; this one waits.
    Queued,
    /// The login command is running.
    Started,
    /// One line of terminal output, ANSI-stripped and redacted (no login
    /// links, codes or tokens): safe for "Ver detalles técnicos" and logs.
    Output(String),
    /// The login link to show and open (raw: never log it).
    UrlDetected(String),
    /// A one-time code the user must type in the browser (raw: never log
    /// it).
    CodeDetected(String),
    /// The program is waiting for the code the browser gave the user: show
    /// "Pegá acá el código que te dio el navegador" and call
    /// [`LoginSession::send_code`].
    NeedsPastedCode,
    /// Logged in. `identity` when the agent (or its CLI) reports one.
    Completed {
        /// Reported identity.
        identity: Option<Identity>,
    },
    /// The login failed; offer "Reintentar".
    Failed {
        /// User-facing message (Spanish).
        message: String,
        /// Machine-readable cause.
        reason: LoginFailure,
    },
    /// [`LoginSession::cancel`] was called; the process group is gone and a
    /// new profile (if any) was removed.
    Cancelled,
}

/// Why a login failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginFailure {
    /// No success within the timeout.
    Timeout,
    /// The command exited with this non-zero code (`None` for a signal).
    Exit(Option<u32>),
    /// The pty or the command could not start.
    Spawn,
    /// The command succeeded but the identity probe says the profile is
    /// not logged in.
    NotLoggedIn,
    /// The agent answered `authenticate` with an error, or does not offer
    /// the login method (ACP mode).
    Rejected,
}

/// The command a [`LoginSession`] runs.
#[derive(Debug, Clone)]
pub struct LoginCommand {
    /// Program (the private `node`, or anything in tests).
    pub program: PathBuf,
    /// Arguments.
    pub args: Vec<String>,
    /// Login environment ([`Profile::login_env`]).
    pub env: ProcessEnv,
    /// Extra variables layered before `env.set` (`BROWSER`, `NO_BROWSER`).
    pub extra_env: BTreeMap<String, String>,
    /// Working directory.
    pub cwd: PathBuf,
}

/// The ACP login a [`LoginSession`] runs ([`LoginSession::spawn_acp`]).
#[derive(Debug, Clone)]
pub struct AcpLogin {
    /// The adapter in ACP mode ([`Adapters::launch_spec`]).
    pub launch: LaunchSpec,
    /// Login environment ([`Profile::login_env`]).
    pub env: ProcessEnv,
    /// Working directory and sandbox root (the profile directory).
    pub cwd: PathBuf,
    /// The `authenticate` method (`oauth-personal`).
    pub method_id: String,
}

/// What [`plan_login`] decided to run.
#[derive(Debug, Clone)]
pub enum LoginPlan {
    /// The provider's login command in a hidden pty.
    Pty(LoginCommand),
    /// ACP `authenticate` against the adapter.
    Acp(AcpLogin),
}

/// Where the half-created profile lives, so [`LoginSession::cancel`] can
/// remove it (only for new connections, never on "Volver a conectar").
#[derive(Debug, Clone)]
pub struct PendingCleanup {
    /// The connections root (`<data>/connections`), for the path check.
    pub root: PathBuf,
    /// The pending profile directory.
    pub dir: PathBuf,
}

/// Behavior knobs of a [`LoginSession`].
pub struct LoginOptions {
    /// Serialization key: logins with the same key never overlap (the
    /// agent id).
    pub provider_key: String,
    /// Give up after this long.
    pub timeout: Duration,
    /// Pty mode: when this file appears the login counts as done even if
    /// the program keeps running (for interactive logins that never exit by
    /// themselves); the process group is then stopped. ACP mode: the file
    /// must exist once `authenticate` succeeded, or the login fails with
    /// [`LoginFailure::NotLoggedIn`].
    pub success_file: Option<PathBuf>,
    /// Removed on cancel.
    pub cleanup: Option<PendingCleanup>,
    /// Confirms the result after a successful exit.
    pub identity_probe: Option<Box<dyn IdentityProbe>>,
}

impl std::fmt::Debug for LoginOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginOptions")
            .field("provider_key", &self.provider_key)
            .field("timeout", &self.timeout)
            .field("success_file", &self.success_file)
            .field("cleanup", &self.cleanup)
            .finish_non_exhaustive()
    }
}

impl LoginOptions {
    /// Options with the default timeout and nothing else.
    #[must_use]
    pub fn new(provider_key: impl Into<String>) -> Self {
        Self {
            provider_key: provider_key.into(),
            timeout: LOGIN_TIMEOUT,
            success_file: None,
            cleanup: None,
            identity_probe: None,
        }
    }
}

// ------------------------------------------------------ provider locks

static PROVIDER_LOCKS: LazyLock<(Mutex<HashSet<String>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(HashSet::new()), Condvar::new()));

struct ProviderGuard(String);

impl Drop for ProviderGuard {
    fn drop(&mut self) {
        let (lock, condvar) = &*PROVIDER_LOCKS;
        if let Ok(mut held) = lock.lock() {
            held.remove(&self.0);
        }
        condvar.notify_all();
    }
}

/// Take the provider lock, waiting while another login holds it. Returns
/// `None` if `cancelled` becomes true first. `on_wait` runs once if it has
/// to wait.
fn acquire_provider(
    key: &str,
    cancelled: &AtomicBool,
    mut on_wait: impl FnMut(),
) -> Option<ProviderGuard> {
    let (lock, condvar) = &*PROVIDER_LOCKS;
    let mut held = lock.lock().ok()?;
    let mut waited = false;
    while held.contains(key) {
        if !waited {
            waited = true;
            on_wait();
        }
        if cancelled.load(Ordering::SeqCst) {
            return None;
        }
        held = condvar.wait_timeout(held, POLL).ok()?.0;
    }
    if cancelled.load(Ordering::SeqCst) {
        return None;
    }
    held.insert(key.to_string());
    Some(ProviderGuard(key.to_string()))
}

// ------------------------------------------------------------- session

struct Shared {
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    pid: Mutex<Option<u32>>,
    cancelled: AtomicBool,
    /// Codes typed with `send_code`, masked in the echoed output.
    secrets: Mutex<Vec<String>>,
}

/// A running login. Dropping it cancels the login.
pub struct LoginSession {
    events: async_channel::Receiver<LoginEvent>,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for LoginSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginSession").finish_non_exhaustive()
    }
}

impl LoginSession {
    /// Start the provider's login for `profile` (spec 06 §4 F2 step 3):
    ///
    /// * Claude: `node <claude-agent-acp> --cli auth login --claudeai` with
    ///   `BROWSER` neutralized ([`Profile::login_env`]) so it prints the
    ///   link instead of opening a tab;
    /// * Codex: `node <codex-acp> cli login` (the adapter's bundled codex)
    ///   with `BROWSER` neutralized so it prints the link; completes by its
    ///   local callback;
    /// * Antigravity: ACP `authenticate { methodId: "oauth-personal" }`
    ///   against `agy_acp_server` with `GEMINI_HOME=<profile>` and
    ///   `BROWSER` neutralized; completes by its local redirect server.
    ///
    /// `cleanup` is `Some` for a brand new connection (the profile is
    /// removed on cancel) and `None` for "Volver a conectar". Success is
    /// confirmed by spawning the adapter once and reading
    /// `_auth/status_update` (falling back to the CLI's status command), or
    /// for Antigravity by the token file.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::AdapterMissing`] when the adapter is not
    /// installed, [`ConnectionsError::RuntimeMissing`] for an npm adapter
    /// without `node`.
    pub fn start(
        profile: &Profile,
        node: Option<&NodePaths>,
        adapters: &Adapters,
        cleanup: Option<PendingCleanup>,
    ) -> Result<Self> {
        let (plan, mut options) = plan_login(profile, node, adapters)?;
        options.cleanup = cleanup;
        Ok(match plan {
            LoginPlan::Pty(command) => Self::spawn(command, options),
            LoginPlan::Acp(login) => Self::spawn_acp(login, options),
        })
    }

    /// Run an arbitrary login command with the engine (used by
    /// [`LoginSession::start`] and by the tests' fake login program).
    #[must_use]
    pub fn spawn(command: LoginCommand, options: LoginOptions) -> Self {
        Self::launch(move |shared, tx| run_session(&command, options, shared, tx))
    }

    /// Run an ACP `authenticate` login (used by [`LoginSession::start`] for
    /// Antigravity and by the tests' fake agent). [`LoginSession::send_code`]
    /// is not available in this mode; [`LoginSession::cancel`] shuts the
    /// agent down, which kills its whole process group.
    #[must_use]
    pub fn spawn_acp(login: AcpLogin, options: LoginOptions) -> Self {
        Self::launch(move |shared, tx| run_acp_session(&login, options, shared, tx))
    }

    fn launch(
        body: impl FnOnce(&Arc<Shared>, &async_channel::Sender<LoginEvent>) + Send + 'static,
    ) -> Self {
        let (tx, rx) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            writer: Mutex::new(None),
            pid: Mutex::new(None),
            cancelled: AtomicBool::new(false),
            secrets: Mutex::new(Vec::new()),
        });
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("cincel-login".to_string())
            .spawn(move || body(&thread_shared, &tx));
        let thread = match thread {
            Ok(handle) => Some(handle),
            Err(error) => {
                let (tx, rx2) = async_channel::unbounded();
                let _ = tx.send_blocking(LoginEvent::Failed {
                    message: format!("no se pudo crear el hilo de inicio de sesión: {error}"),
                    reason: LoginFailure::Spawn,
                });
                return Self {
                    events: rx2,
                    shared,
                    thread: None,
                };
            }
        };
        Self {
            events: rx,
            shared,
            thread,
        }
    }

    /// The event stream. Closed after the final event (`Completed`,
    /// `Failed` or `Cancelled`).
    #[must_use]
    pub fn events(&self) -> &async_channel::Receiver<LoginEvent> {
        &self.events
    }

    /// Type `code` into the program followed by Enter. The code is never
    /// logged.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::LoginSpawn`] when the program is not running.
    pub fn send_code(&self, code: &str) -> Result<()> {
        let mut writer =
            self.shared.writer.lock().map_err(|_| {
                ConnectionsError::LoginSpawn("estado interno envenenado".to_string())
            })?;
        let Some(writer) = writer.as_mut() else {
            return Err(ConnectionsError::LoginSpawn(
                "el proceso de inicio de sesión no está corriendo".to_string(),
            ));
        };
        // Registered before writing: the terminal echoes it right away.
        if let Ok(mut secrets) = self.shared.secrets.lock() {
            secrets.push(code.trim().to_string());
        }
        let mut bytes = code.trim().as_bytes().to_vec();
        bytes.push(b'\r');
        writer
            .write_all(&bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| ConnectionsError::LoginSpawn(error.to_string()))
    }

    /// Stop the login: kill the whole process group and remove the pending
    /// profile (when this is a new connection). Ends with
    /// [`LoginEvent::Cancelled`].
    pub fn cancel(&self) {
        self.shared.cancelled.store(true, Ordering::SeqCst);
        if let Ok(pid) = self.shared.pid.lock() {
            kill_group(*pid);
        }
    }

    /// Wait for the session thread to finish (after the final event).
    pub fn join(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LoginSession {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            if !thread.is_finished() {
                self.cancel();
            }
            let _ = thread.join();
        }
    }
}

fn emit(tx: &async_channel::Sender<LoginEvent>, event: LoginEvent) {
    let _ = tx.send_blocking(event);
}

fn run_session(
    command: &LoginCommand,
    options: LoginOptions,
    shared: &Arc<Shared>,
    tx: &async_channel::Sender<LoginEvent>,
) {
    let cancelled_early = || {
        if let Some(cleanup) = &options.cleanup {
            let _ = remove_dir_within(&cleanup.root, &cleanup.dir);
        }
        emit(tx, LoginEvent::Cancelled);
    };
    let Some(_guard) = acquire_provider(&options.provider_key, &shared.cancelled, || {
        emit(tx, LoginEvent::Queued);
    }) else {
        cancelled_early();
        return;
    };

    let pty = match native_pty_system().openpty(PtySize {
        rows: 50,
        // Wide enough that no login URL is ever wrapped by a TUI.
        cols: 1000,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pty) => pty,
        Err(error) => {
            emit(
                tx,
                LoginEvent::Failed {
                    message: format!("no se pudo abrir la terminal oculta: {error}"),
                    reason: LoginFailure::Spawn,
                },
            );
            return;
        }
    };
    let mut builder = CommandBuilder::new(&command.program);
    builder.args(&command.args);
    builder.cwd(&command.cwd);
    let mut extra = command.extra_env.clone();
    extra
        .entry("TERM".to_string())
        .or_insert_with(|| "xterm-256color".to_string());
    envutil::apply_pty(&mut builder, &command.env, &extra);

    let mut child = match pty.slave.spawn_command(builder) {
        Ok(child) => child,
        Err(error) => {
            emit(
                tx,
                LoginEvent::Failed {
                    message: format!(
                        "no se pudo ejecutar `{}`: {error}",
                        command.program.display()
                    ),
                    reason: LoginFailure::Spawn,
                },
            );
            return;
        }
    };
    // The child owns the slave now; keeping our copy open would stop the
    // reader from ever seeing EOF.
    drop(pty.slave);
    let pid = child.process_id();
    if let Ok(mut slot) = shared.pid.lock() {
        *slot = pid;
    }
    if let (Ok(writer), Ok(mut slot)) = (pty.master.take_writer(), shared.writer.lock()) {
        *slot = Some(writer);
    }
    // `cancel()` may have run before the pid was known.
    if shared.cancelled.load(Ordering::SeqCst) {
        kill_group(pid);
    }
    emit(tx, LoginEvent::Started);

    let reader = pty.master.try_clone_reader();
    let reader_tx = tx.clone();
    let reader_shared = shared.clone();
    let reader_thread = reader.ok().and_then(|mut reader| {
        std::thread::Builder::new()
            .name("cincel-login-pty".to_string())
            .spawn(move || {
                let mut scanner = OutputScanner::new();
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if let Ok(secrets) = reader_shared.secrets.lock() {
                                for secret in secrets.iter() {
                                    scanner.add_secret(secret);
                                }
                            }
                            for event in scanner.feed(&buffer[..read]) {
                                emit(&reader_tx, event);
                            }
                        }
                    }
                }
                for event in scanner.finish() {
                    emit(&reader_tx, event);
                }
            })
            .ok()
    });

    let deadline = Instant::now() + options.timeout;
    enum End {
        Exited(portable_pty::ExitStatus),
        Cancelled,
        Timeout,
        SuccessFile,
    }
    let end = loop {
        if shared.cancelled.load(Ordering::SeqCst) {
            break End::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) => break End::Exited(status),
            Ok(None) => {}
            Err(_) => {
                break End::Exited(portable_pty::ExitStatus::with_exit_code(1));
            }
        }
        if options
            .success_file
            .as_deref()
            .is_some_and(|file| std::fs::metadata(file).is_ok_and(|meta| meta.len() > 0))
        {
            // Give the program a moment to finish writing its files.
            std::thread::sleep(Duration::from_millis(300));
            break End::SuccessFile;
        }
        if Instant::now() >= deadline {
            break End::Timeout;
        }
        std::thread::sleep(POLL);
    };

    // Whatever happened, nothing of the login may outlive the session.
    kill_group(pid);
    let _ = child.kill();
    let _ = child.wait();
    if let Ok(mut slot) = shared.writer.lock() {
        *slot = None;
    }
    drop(pty.master);
    if let Some(reader_thread) = reader_thread {
        let _ = reader_thread.join();
    }

    let succeeded = match end {
        End::Cancelled => {
            cancelled_early();
            return;
        }
        End::Timeout => {
            emit(tx, timed_out(options.timeout));
            return;
        }
        End::SuccessFile => true,
        End::Exited(status) => {
            if shared.cancelled.load(Ordering::SeqCst) {
                cancelled_early();
                return;
            }
            if !status.success() {
                let code = (status.signal().is_none()).then(|| status.exit_code());
                emit(
                    tx,
                    LoginEvent::Failed {
                        message: match code {
                            Some(code) => {
                                format!("El inicio de sesión terminó con un error (código {code}).")
                            }
                            None => "El inicio de sesión se interrumpió.".to_string(),
                        },
                        reason: LoginFailure::Exit(code),
                    },
                );
                return;
            }
            true
        }
    };
    debug_assert!(succeeded);
    confirm_success(&options, tx);
}

/// After the login itself succeeded: run the identity probe and end with
/// `Completed` (or `Failed { NotLoggedIn }` when the probe says the profile
/// is logged out).
fn confirm_success(options: &LoginOptions, tx: &async_channel::Sender<LoginEvent>) {
    let outcome = options
        .identity_probe
        .as_ref()
        .map(|probe| probe.probe())
        .unwrap_or_default();
    if outcome.logged_in == Some(false) {
        emit(tx, not_logged_in());
        return;
    }
    emit(
        tx,
        LoginEvent::Completed {
            identity: outcome.identity,
        },
    );
}

fn not_logged_in() -> LoginEvent {
    LoginEvent::Failed {
        message: "El inicio de sesión terminó pero el agente no encuentra la sesión.".to_string(),
        reason: LoginFailure::NotLoggedIn,
    }
}

fn timed_out(timeout: Duration) -> LoginEvent {
    LoginEvent::Failed {
        message: format!(
            "Se agotó el tiempo de espera ({} min) sin completar el inicio de sesión.",
            timeout.as_secs().div_ceil(60)
        ),
        reason: LoginFailure::Timeout,
    }
}

/// How an ACP login ended, before cleanup.
enum AcpEnd {
    Succeeded,
    Cancelled,
    Failed(LoginEvent),
}

fn auth_method_id(method: &AuthMethod) -> Option<&str> {
    match method {
        AuthMethod::Agent(agent) => Some(agent.id.0.as_ref()),
        AuthMethod::Terminal(terminal) => Some(terminal.id.0.as_ref()),
        _ => None,
    }
}

fn run_acp_session(
    login: &AcpLogin,
    options: LoginOptions,
    shared: &Arc<Shared>,
    tx: &async_channel::Sender<LoginEvent>,
) {
    let cancelled = || {
        if let Some(cleanup) = &options.cleanup {
            let _ = remove_dir_within(&cleanup.root, &cleanup.dir);
        }
        emit(tx, LoginEvent::Cancelled);
    };
    let Some(_guard) = acquire_provider(&options.provider_key, &shared.cancelled, || {
        emit(tx, LoginEvent::Queued);
    }) else {
        cancelled();
        return;
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            emit(
                tx,
                LoginEvent::Failed {
                    message: format!("no se pudo crear el runtime: {error}"),
                    reason: LoginFailure::Spawn,
                },
            );
            return;
        }
    };
    let mut connection = AgentConnection::start_with_env(login.cwd.clone(), login.env.clone());
    let end = runtime.block_on(drive_acp(&connection, login, &options, shared, tx));
    // Whatever happened, the agent (and its redirect server) goes: shutdown
    // kills the whole process group.
    connection.shutdown();
    match end {
        AcpEnd::Cancelled => cancelled(),
        AcpEnd::Failed(event) => emit(tx, event),
        AcpEnd::Succeeded => {
            if let Some(file) = &options.success_file {
                // The agent writes the token before answering; allow for a
                // slow filesystem anyway.
                let deadline = Instant::now() + Duration::from_secs(3);
                while !std::fs::metadata(file).is_ok_and(|meta| meta.len() > 0) {
                    if Instant::now() >= deadline {
                        emit(tx, not_logged_in());
                        return;
                    }
                    std::thread::sleep(POLL);
                }
            }
            confirm_success(&options, tx);
        }
    }
}

async fn drive_acp(
    connection: &AgentConnection,
    login: &AcpLogin,
    options: &LoginOptions,
    shared: &Arc<Shared>,
    tx: &async_channel::Sender<LoginEvent>,
) -> AcpEnd {
    if connection
        .send(AgentCommand::Spawn {
            launch: login.launch.clone(),
            cwd: login.cwd.clone(),
        })
        .await
        .is_err()
    {
        return AcpEnd::Failed(LoginEvent::Failed {
            message: "la conexión con el agente se cerró".to_string(),
            reason: LoginFailure::Spawn,
        });
    }
    emit(tx, LoginEvent::Started);
    let deadline = Instant::now() + options.timeout;
    let mut urls = HashSet::new();
    let mut connected = false;
    let mut report_url = |url: String| {
        if urls.insert(url.clone()) {
            emit(tx, LoginEvent::UrlDetected(url));
        }
    };
    loop {
        if shared.cancelled.load(Ordering::SeqCst) {
            return AcpEnd::Cancelled;
        }
        if Instant::now() >= deadline {
            return AcpEnd::Failed(timed_out(options.timeout));
        }
        let event = match tokio::time::timeout(POLL, connection.recv()).await {
            Err(_) => continue,
            Ok(Err(_)) => {
                return AcpEnd::Failed(LoginEvent::Failed {
                    message: "la conexión con el agente se cerró".to_string(),
                    reason: LoginFailure::Exit(None),
                });
            }
            Ok(Ok(event)) => event,
        };
        match event {
            AgentEvent::Connected { auth_methods, .. } => {
                connected = true;
                let offered = auth_methods
                    .iter()
                    .any(|method| auth_method_id(method) == Some(login.method_id.as_str()));
                if !offered {
                    return AcpEnd::Failed(LoginEvent::Failed {
                        message: format!(
                            "El agente no ofrece el inicio de sesión «{}».",
                            login.method_id
                        ),
                        reason: LoginFailure::Rejected,
                    });
                }
                if connection
                    .send(AgentCommand::Authenticate {
                        method_id: login.method_id.clone(),
                    })
                    .await
                    .is_err()
                {
                    return AcpEnd::Failed(LoginEvent::Failed {
                        message: "la conexión con el agente se cerró".to_string(),
                        reason: LoginFailure::Exit(None),
                    });
                }
            }
            AgentEvent::Stderr(line) => {
                let clean = strip_ansi(&line);
                for url in find_login_urls(&clean) {
                    report_url(url);
                }
                emit(tx, LoginEvent::Output(redact_line(&clean, &[])));
            }
            AgentEvent::Elicitation { request, reply, .. } => match &request.mode {
                ElicitationMode::Url(url) => {
                    // The modal shows the link with "Abrir en el navegador":
                    // consent is the user opening it.
                    report_url(url.url.clone());
                    reply.respond(CreateElicitationResponse::new(ElicitationAction::Accept(
                        ElicitationAcceptAction::new(),
                    )));
                }
                _ => {
                    // No form can be answered from the login modal.
                    reply.respond(CreateElicitationResponse::new(ElicitationAction::Cancel));
                }
            },
            AgentEvent::AuthSucceeded { .. } => return AcpEnd::Succeeded,
            AgentEvent::AuthFailed { message, .. } => {
                return AcpEnd::Failed(LoginEvent::Failed {
                    message: redact_line(&message, &[]),
                    reason: LoginFailure::Rejected,
                });
            }
            AgentEvent::Exited { code, .. } => {
                return AcpEnd::Failed(LoginEvent::Failed {
                    message: match code {
                        Some(code) => format!("El agente terminó con un error (código {code})."),
                        None => "El agente se interrumpió.".to_string(),
                    },
                    reason: LoginFailure::Exit(code.and_then(|code| u32::try_from(code).ok())),
                });
            }
            AgentEvent::Error { message, .. } => {
                return AcpEnd::Failed(LoginEvent::Failed {
                    message: redact_line(&message, &[]),
                    reason: if connected {
                        LoginFailure::Rejected
                    } else {
                        LoginFailure::Spawn
                    },
                });
            }
            _ => {}
        }
    }
}

/// SIGKILL the whole process group led by `pid` (the pty child is a
/// session leader, so its group holds every helper it spawned).
fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid
        && let Some(pid) = rustix::process::Pid::from_raw(pid.cast_signed())
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Build the per-provider login and options (without cleanup).
///
/// # Errors
///
/// [`ConnectionsError::AdapterMissing`], [`ConnectionsError::RuntimeMissing`]
/// (npm adapter without `node`).
pub fn plan_login(
    profile: &Profile,
    node: Option<&NodePaths>,
    adapters: &Adapters,
) -> Result<(LoginPlan, LoginOptions)> {
    let kind = profile.kind();
    let agent_id = kind.agent_id();
    let mut options = LoginOptions::new(agent_id);
    options.identity_probe = Some(default_probe(profile, node, adapters)?);
    if let LoginMethod::AcpAuthenticate(method_id) = kind.login_method() {
        options.success_file = Some(profile.credentials_file());
        let login = AcpLogin {
            launch: adapters.launch_spec(agent_id, node)?,
            env: profile.login_env(node.map(|node| node.bin_dir.as_path())),
            cwd: profile.dir().to_path_buf(),
            method_id: method_id.to_string(),
        };
        return Ok((LoginPlan::Acp(login), options));
    }
    let node = node.ok_or(ConnectionsError::RuntimeMissing)?;
    // `BROWSER` neutralized for both CLIs: they print the link and Cincel
    // shows it; neither opens a browser tab by itself.
    let env = profile.login_env(Some(&node.bin_dir));
    let spec = if kind == AgentKind::Claude {
        adapters.command_spec(agent_id, node, &["--cli", "auth", "login", "--claudeai"])?
    } else {
        adapters.command_spec(agent_id, node, &["cli", "login"])?
    };
    let command = LoginCommand {
        program: PathBuf::from(&spec.program),
        args: spec.args,
        env,
        extra_env: BTreeMap::new(),
        cwd: profile.dir().to_path_buf(),
    };
    Ok((LoginPlan::Pty(command), options))
}

/// The identity probe for `profile`: ACP status push with a CLI fallback
/// (Claude, Codex), or the profile files (Antigravity: the token file,
/// which has no email, and no `_auth/status_update` from the agent).
///
/// # Errors
///
/// [`ConnectionsError::AdapterMissing`], [`ConnectionsError::RuntimeMissing`].
pub fn default_probe(
    profile: &Profile,
    node: Option<&NodePaths>,
    adapters: &Adapters,
) -> Result<Box<dyn IdentityProbe>> {
    let kind = profile.kind();
    let agent_id = kind.agent_id();
    if kind == AgentKind::Antigravity {
        return Ok(Box::new(ProfileProbe {
            profile: profile.clone(),
        }));
    }
    let node = node.ok_or(ConnectionsError::RuntimeMissing)?;
    let env = profile.process_env(Some(&node.bin_dir));
    let cwd = profile.dir().to_path_buf();
    let status_args: &[&str] = match kind {
        AgentKind::Claude => &["--cli", "auth", "status", "--json"],
        _ => &["cli", "login", "status"],
    };
    let fallback: Box<dyn IdentityProbe> = Box::new(CliStatusProbe {
        kind,
        command: adapters.command_spec(agent_id, node, status_args)?,
        env: env.clone(),
        cwd: cwd.clone(),
    });
    Ok(Box::new(AcpIdentityProbe {
        launch: adapters.launch_spec(agent_id, Some(node))?,
        env,
        cwd,
        wait: Duration::from_secs(20),
        fallback: Some(fallback),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An installed npm adapter marker plus a private runtime under `root`.
    fn fake_install(root: &std::path::Path, agent_id: &str) -> (Adapters, NodePaths) {
        let paths = crate::paths::CincelPaths::under(root);
        let dir = paths.agents_dir().join(agent_id).join("1.0.0");
        std::fs::create_dir_all(dir.join("dist")).expect("dir");
        std::fs::write(dir.join("dist/index.js"), "// adapter\n").expect("bin");
        let marker = serde_json::json!({
            "agent_id": agent_id,
            "package": "fake",
            "version": "1.0.0",
            "bin": "dist/index.js",
            "args": [],
        });
        std::fs::write(
            dir.join(".cincel-adapter.json"),
            serde_json::to_vec(&marker).expect("json"),
        )
        .expect("marker");
        std::fs::write(paths.agents_dir().join(agent_id).join("current"), "1.0.0")
            .expect("current");
        let node_root = root.join("runtime/node-v24.0.0");
        let node = NodePaths {
            version: "v24.0.0".to_string(),
            bin_dir: node_root.join("bin"),
            node: node_root.join("bin/node"),
            npm: node_root.join("npm-cli.js"),
            npx: node_root.join("npx-cli.js"),
            root: node_root,
        };
        (Adapters::new(paths), node)
    }

    #[test]
    fn claude_and_codex_logins_run_with_a_neutral_browser() {
        let neutral = crate::profile::neutral_browser();
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let root = tempfile::tempdir().expect("tempdir");
            let (adapters, node) = fake_install(root.path(), kind.agent_id());
            let profile = Profile::new(kind, root.path().join("connections/x"));
            let (plan, _) = plan_login(&profile, Some(&node), &adapters).expect("plan");
            let LoginPlan::Pty(command) = plan else {
                panic!("{kind:?}: login por pty");
            };
            let browser = command
                .env
                .set
                .iter()
                .find(|(name, _)| name == "BROWSER")
                .map(|(_, value)| value.clone());
            assert_eq!(browser, Some(neutral.clone()), "{kind:?}");
            // Nothing in `extra_env` overrides it.
            assert!(!command.extra_env.contains_key("BROWSER"), "{kind:?}");
            let resolved = envutil::resolve(&command.env, &command.extra_env);
            let last = resolved
                .iter()
                .rev()
                .find(|(name, _)| name == "BROWSER")
                .and_then(|(_, value)| value.clone());
            assert_eq!(last, Some(neutral.clone().into()), "{kind:?}");
        }
    }

    #[test]
    fn provider_lock_serializes_and_releases() {
        let cancelled = AtomicBool::new(false);
        let first = acquire_provider("test-lock-a", &cancelled, || {}).expect("first");
        let waited = Arc::new(AtomicBool::new(false));
        let waited_thread = waited.clone();
        let handle = std::thread::spawn(move || {
            let cancelled = AtomicBool::new(false);
            let _second = acquire_provider("test-lock-a", &cancelled, || {
                waited_thread.store(true, Ordering::SeqCst);
            })
            .expect("second");
        });
        std::thread::sleep(Duration::from_millis(150));
        assert!(waited.load(Ordering::SeqCst), "el segundo debe esperar");
        drop(first);
        handle.join().expect("join");
        // Different keys never block each other.
        let _a = acquire_provider("test-lock-b", &cancelled, || panic!("no debe esperar"));
        let _b = acquire_provider("test-lock-c", &cancelled, || panic!("no debe esperar"));
    }

    #[test]
    fn provider_lock_wait_is_cancellable() {
        let cancelled = AtomicBool::new(false);
        let _held = acquire_provider("test-lock-d", &cancelled, || {}).expect("held");
        let flag = Arc::new(AtomicBool::new(false));
        let flag_thread = flag.clone();
        let handle = std::thread::spawn(move || {
            acquire_provider("test-lock-d", &flag_thread, || {}).is_none()
        });
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::SeqCst);
        assert!(handle.join().expect("join"), "cancelado mientras esperaba");
    }
}
