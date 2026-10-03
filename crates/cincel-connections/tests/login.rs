//! End-to-end tests of the pty login engine against the fake login program
//! in `tests/fake_login/`. No real provider is ever contacted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cincel_acp::{LaunchSpec, ProcessEnv};
use cincel_connections::{
    AcpLogin, AgentKind, Identity, IdentityProbe, LoginCommand, LoginEvent, LoginFailure,
    LoginOptions, LoginSession, PendingCleanup, ProbeOutcome, Profile, ProfileProbe,
    login_event_log_line,
};

const FAKE_LOGIN: &str = env!("CARGO_BIN_EXE_cincel-connections-fake-login");
const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_cincel-connections-fake-agent");
/// The shape of the link `agy_acp_server` 1.2.1 prints to stderr.
const AGY_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&client_id=884354919052-fake.apps.googleusercontent.com&redirect_uri=http%3A%2F%2F127.0.0.1%3A54243%2F&scope=openid&state=Ag3nTsT4te&code_challenge=Ch4llEng3&code_challenge_method=S256&access_type=offline&prompt=consent";
const TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&code_challenge=jogr&code_challenge_method=S256&state=zYkWwG9BW7MqCQM9";
const OPENAI_URL: &str = "https://auth.openai.com/oauth/authorize?response_type=code&client_id=app_EMoam&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback&state=rrwM0ktBX5";
const GOOGLE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=https%3A%2F%2Fcodeassist.google.com%2Fauthcode&access_type=offline&state=g00gle";

fn command(args: &[&str], cwd: &Path) -> LoginCommand {
    LoginCommand {
        program: PathBuf::from(FAKE_LOGIN),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        env: ProcessEnv::new(),
        extra_env: BTreeMap::new(),
        cwd: cwd.to_path_buf(),
    }
}

/// Unique serialization key per test (tests run in parallel).
fn options(key: &str) -> LoginOptions {
    LoginOptions::new(format!("test-{key}"))
}

/// Receive the next event, failing the test after `TIMEOUT`.
fn next(session: &LoginSession) -> LoginEvent {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match session.events().try_recv() {
            Ok(event) => return event,
            Err(async_channel::TryRecvError::Closed) => panic!("canal cerrado sin evento final"),
            Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "no llegó ningún evento a tiempo");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// Collect events until one matches `stop`, returning all of them.
fn until(session: &LoginSession, stop: impl Fn(&LoginEvent) -> bool) -> Vec<LoginEvent> {
    let mut seen = Vec::new();
    loop {
        let event = next(session);
        let done = stop(&event);
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn is_final(event: &LoginEvent) -> bool {
    matches!(
        event,
        LoginEvent::Completed { .. } | LoginEvent::Failed { .. } | LoginEvent::Cancelled
    )
}

fn outputs(events: &[LoginEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            LoginEvent::Output(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn assert_no_secret_in_output(events: &[LoginEvent], secrets: &[&str]) {
    for line in outputs(events) {
        for secret in secrets {
            assert!(
                !line.contains(secret),
                "la salida filtró «{secret}»: {line}"
            );
        }
    }
}

#[cfg(unix)]
fn pid_alive(pid: i32) -> bool {
    // Zombies still have a /proc entry; check the state letter.
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat
            .rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z')),
        Err(_) => false,
    }
}

#[test]
fn claude_style_url_then_pasted_code_completes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let session = LoginSession::spawn(
        command(
            &["--url", CLAUDE_URL, "--expect", "PASTED-CODE-123"],
            dir.path(),
        ),
        options("claude-ok"),
    );
    let first = until(&session, |event| {
        matches!(event, LoginEvent::NeedsPastedCode)
    });
    assert!(first.contains(&LoginEvent::Started));
    assert!(first.contains(&LoginEvent::UrlDetected(CLAUDE_URL.to_string())));
    session.send_code("PASTED-CODE-123").expect("send code");
    let rest = until(&session, is_final);
    assert_eq!(rest.last(), Some(&LoginEvent::Completed { identity: None }));
    let all: Vec<LoginEvent> = first.into_iter().chain(rest).collect();
    assert_no_secret_in_output(
        &all,
        &["zYkWwG9BW7MqCQM9", "client_id=9d1c250a", "PASTED-CODE-123"],
    );
    assert!(
        outputs(&all)
            .iter()
            .any(|line| line.contains("https://claude.com/cai/oauth/authorize?[REDACTED]")),
        "la salida conserva el enlace sin su query"
    );
}

#[test]
fn wrong_pasted_code_fails_with_exit_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let session = LoginSession::spawn(
        command(&["--url", CLAUDE_URL, "--expect", "BIEN"], dir.path()),
        options("claude-bad"),
    );
    until(&session, |event| {
        matches!(event, LoginEvent::NeedsPastedCode)
    });
    session.send_code("MAL").expect("send code");
    let events = until(&session, is_final);
    match events.last() {
        Some(LoginEvent::Failed { reason, message }) => {
            assert_eq!(reason, &LoginFailure::Exit(Some(1)));
            assert!(message.contains("código 1"), "{message}");
        }
        other => panic!("se esperaba Failed: {other:?}"),
    }
}

#[test]
fn codex_style_device_code_completes_without_paste() {
    let dir = tempfile::tempdir().expect("tempdir");
    let session = LoginSession::spawn(
        command(
            &["--url", OPENAI_URL, "--code", "WXYZ-12345", "--exit", "0"],
            dir.path(),
        ),
        options("codex-ok"),
    );
    let events = until(&session, is_final);
    assert!(events.contains(&LoginEvent::UrlDetected(OPENAI_URL.to_string())));
    assert!(events.contains(&LoginEvent::CodeDetected("WXYZ-12345".to_string())));
    assert!(!events.contains(&LoginEvent::NeedsPastedCode));
    assert_eq!(
        events.last(),
        Some(&LoginEvent::Completed { identity: None })
    );
    assert_no_secret_in_output(&events, &["WXYZ-12345", "rrwM0ktBX5", "\x1b"]);
}

#[test]
fn google_url_is_detected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let session = LoginSession::spawn(
        command(&["--url", GOOGLE_URL, "--exit", "0"], dir.path()),
        options("google-url"),
    );
    let events = until(&session, is_final);
    assert!(events.contains(&LoginEvent::UrlDetected(GOOGLE_URL.to_string())));
    assert_no_secret_in_output(&events, &["g00gle"]);
}

#[test]
fn secrets_never_reach_output_events() {
    let dir = tempfile::tempdir().expect("tempdir");
    let session = LoginSession::spawn(
        command(
            &["--secret", "sk-ant-oat01-SUPERSECRETO", "--exit", "0"],
            dir.path(),
        ),
        options("redaction"),
    );
    let events = until(&session, is_final);
    let lines = outputs(&events);
    assert!(
        lines.iter().any(|line| line.contains("Bearer [REDACTED]")),
        "{lines:?}"
    );
    assert_no_secret_in_output(&events, &["SUPERSECRETO"]);
}

#[test]
fn nonzero_exit_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let session = LoginSession::spawn(command(&["--exit", "3"], dir.path()), options("exit3"));
    let events = until(&session, is_final);
    assert!(matches!(
        events.last(),
        Some(LoginEvent::Failed {
            reason: LoginFailure::Exit(Some(3)),
            ..
        })
    ));
}

#[test]
fn missing_program_fails_to_spawn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut bad = command(&[], dir.path());
    bad.program = dir.path().join("no-existe");
    let session = LoginSession::spawn(bad, options("spawn"));
    let events = until(&session, is_final);
    assert!(
        matches!(events.last(), Some(LoginEvent::Failed { .. })),
        "{events:?}"
    );
    assert!(session.send_code("x").is_err());
}

#[cfg(unix)]
#[test]
fn cancel_kills_the_process_group_and_removes_the_pending_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("connections");
    let pending = root.join("pendiente");
    std::fs::create_dir_all(&pending).expect("mkdir");
    let pid_file = dir.path().join("child.pid");
    let mut opts = options("cancel");
    opts.cleanup = Some(PendingCleanup {
        root: root.clone(),
        dir: pending.clone(),
    });
    let session = LoginSession::spawn(
        command(
            &[
                "--url",
                CLAUDE_URL,
                "--spawn-child",
                pid_file.to_str().expect("utf8"),
                "--hang",
            ],
            &pending,
        ),
        opts,
    );
    until(&session, |event| {
        matches!(event, LoginEvent::UrlDetected(_))
    });
    // Wait for the grandchild's pid.
    let deadline = Instant::now() + TIMEOUT;
    let child_pid: i32 = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = text.trim().parse()
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "el hijo nunca escribió su pid");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(pid_alive(child_pid));
    session.cancel();
    let events = until(&session, is_final);
    assert_eq!(events.last(), Some(&LoginEvent::Cancelled));
    assert!(!pending.exists(), "el perfil a medio crear se borró");
    assert!(root.exists(), "la raíz de conexiones queda intacta");
    let deadline = Instant::now() + Duration::from_secs(5);
    while pid_alive(child_pid) {
        assert!(
            Instant::now() < deadline,
            "el nieto {child_pid} sigue vivo tras cancelar"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn cancel_without_cleanup_keeps_the_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let session = LoginSession::spawn(command(&["--hang"], dir.path()), options("relogin"));
    until(&session, |event| matches!(event, LoginEvent::Started));
    session.cancel();
    let events = until(&session, is_final);
    assert_eq!(events.last(), Some(&LoginEvent::Cancelled));
    assert!(dir.path().exists());
}

#[test]
fn timeout_is_reported_and_the_process_stopped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut opts = options("timeout");
    opts.timeout = Duration::from_millis(400);
    let started = Instant::now();
    let session = LoginSession::spawn(command(&["--url", CLAUDE_URL, "--hang"], dir.path()), opts);
    let events = until(&session, is_final);
    assert!(matches!(
        events.last(),
        Some(LoginEvent::Failed {
            reason: LoginFailure::Timeout,
            ..
        })
    ));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn success_file_ends_an_interactive_login() {
    let dir = tempfile::tempdir().expect("tempdir");
    let creds = dir.path().join("creds").join("token.json");
    std::fs::create_dir_all(creds.parent().expect("parent")).expect("mkdir");
    let mut opts = options("success-file");
    opts.success_file = Some(creds.clone());
    let session = LoginSession::spawn(
        command(
            &[
                "--url",
                GOOGLE_URL,
                "--write-after-ms",
                "300",
                creds.to_str().expect("utf8"),
                "--hang",
            ],
            dir.path(),
        ),
        opts,
    );
    let events = until(&session, is_final);
    assert_eq!(
        events.last(),
        Some(&LoginEvent::Completed { identity: None })
    );
}

struct FixedProbe(ProbeOutcome);

impl IdentityProbe for FixedProbe {
    fn probe(&self) -> ProbeOutcome {
        self.0.clone()
    }
}

#[test]
fn identity_probe_result_is_attached_to_completed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = Identity {
        email: Some("ana@example.com".to_string()),
        plan: Some("Claude Max".to_string()),
        organization: None,
    };
    let mut opts = options("probe-ok");
    opts.identity_probe = Some(Box::new(FixedProbe(ProbeOutcome {
        logged_in: Some(true),
        identity: Some(identity.clone()),
    })));
    let session = LoginSession::spawn(command(&["--exit", "0"], dir.path()), opts);
    let events = until(&session, is_final);
    assert_eq!(
        events.last(),
        Some(&LoginEvent::Completed {
            identity: Some(identity)
        })
    );
}

#[test]
fn probe_saying_logged_out_turns_success_into_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut opts = options("probe-no");
    opts.identity_probe = Some(Box::new(FixedProbe(ProbeOutcome {
        logged_in: Some(false),
        identity: None,
    })));
    let session = LoginSession::spawn(command(&["--exit", "0"], dir.path()), opts);
    let events = until(&session, is_final);
    assert!(matches!(
        events.last(),
        Some(LoginEvent::Failed {
            reason: LoginFailure::NotLoggedIn,
            ..
        })
    ));
}

#[test]
fn logins_of_the_same_provider_are_serialized() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = LoginSession::spawn(
        command(&["--url", OPENAI_URL, "--hang"], dir.path()),
        options("serial"),
    );
    until(&first, |event| matches!(event, LoginEvent::Started));
    let second = LoginSession::spawn(
        command(&["--url", OPENAI_URL, "--exit", "0"], dir.path()),
        options("serial"),
    );
    assert_eq!(next(&second), LoginEvent::Queued);
    // Still queued while the first one runs.
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        second.events().is_empty(),
        "el segundo no debe arrancar todavía"
    );
    first.cancel();
    until(&first, is_final);
    let events = until(&second, is_final);
    assert!(events.contains(&LoginEvent::Started));
    assert_eq!(
        events.last(),
        Some(&LoginEvent::Completed { identity: None })
    );
}

#[test]
fn different_providers_do_not_wait_for_each_other() {
    let dir = tempfile::tempdir().expect("tempdir");
    let codex = LoginSession::spawn(command(&["--hang"], dir.path()), options("par-codex"));
    until(&codex, |event| matches!(event, LoginEvent::Started));
    let claude = LoginSession::spawn(command(&["--exit", "0"], dir.path()), options("par-claude"));
    let events = until(&claude, is_final);
    assert!(!events.contains(&LoginEvent::Queued));
    codex.cancel();
    until(&codex, is_final);
}

#[test]
fn profile_env_reaches_the_login_process() {
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = Profile::new(AgentKind::Claude, dir.path().join("perfil"));
    std::fs::create_dir_all(profile.dir()).expect("mkdir");
    let mut login = command(
        &[
            "--print-env",
            "CLAUDE_CONFIG_DIR",
            "--print-env",
            "NO_BROWSER",
            "--print-env",
            "BROWSER",
            "--creds-var",
            "CLAUDE_CONFIG_DIR",
            "--exit",
            "0",
        ],
        profile.dir(),
    );
    login.env = profile.process_env(None);
    login
        .extra_env
        .insert("BROWSER".to_string(), "/bin/true".to_string());
    let session = LoginSession::spawn(login, options("env"));
    let events = until(&session, is_final);
    let lines = outputs(&events);
    assert!(
        lines.contains(&format!(
            "env CLAUDE_CONFIG_DIR={}",
            profile.dir().display()
        )),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"env NO_BROWSER=<unset>".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"env BROWSER=/bin/true".to_string()),
        "{lines:?}"
    );
    assert!(
        profile.has_credentials(),
        "el login escribe en el perfil aislado"
    );
}

#[test]
fn dropping_a_session_cancels_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let session = LoginSession::spawn(
        command(
            &["--spawn-child", pid_file.to_str().expect("utf8"), "--hang"],
            dir.path(),
        ),
        options("drop"),
    );
    until(&session, |event| matches!(event, LoginEvent::Started));
    let started = Instant::now();
    drop(session);
    assert!(started.elapsed() < Duration::from_secs(10));
}

// ------------------------------------------------------ ACP authenticate

struct AcpFixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    profile: Profile,
    marker: PathBuf,
}

fn acp_fixture() -> AcpFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("connections");
    let profile = Profile::new(AgentKind::Antigravity, root.join("perfil"));
    std::fs::create_dir_all(profile.dir()).expect("perfil");
    AcpFixture {
        marker: dir.path().join("env-marker"),
        root,
        profile,
        _dir: dir,
    }
}

impl AcpFixture {
    fn login(&self, vars: &[(&str, &str)]) -> AcpLogin {
        let mut launch = LaunchSpec::new(FAKE_AGENT, Vec::new())
            .with_env("FAKE_NO_AUTH_STATUS", "1")
            .with_env("FAKE_ENV_MARKER", self.marker.display().to_string());
        for (name, value) in vars {
            launch = launch.with_env(*name, *value);
        }
        AcpLogin {
            launch,
            env: self.profile.process_env(None),
            cwd: self.profile.dir().to_path_buf(),
            method_id: "oauth-personal".to_string(),
        }
    }

    fn options(&self, key: &str) -> LoginOptions {
        let mut options = options(key);
        options.success_file = Some(self.profile.credentials_file());
        options.identity_probe = Some(Box::new(ProfileProbe {
            profile: self.profile.clone(),
        }));
        options
    }

    fn marker_value(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(&self.marker)
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")).map(str::to_string))
    }

    fn agent_pid(&self) -> i32 {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(pid) = self.marker_value("PID").and_then(|pid| pid.parse().ok()) {
                return pid;
            }
            assert!(Instant::now() < deadline, "el agente no escribió su pid");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn acp_login_shows_the_stderr_link_and_completes_when_the_agent_answers() {
    let fx = acp_fixture();
    let session = LoginSession::spawn_acp(
        fx.login(&[("FAKE_AUTH_URL", AGY_URL), ("FAKE_AUTH_DELAY_MS", "400")]),
        fx.options("acp-ok"),
    );
    let events = until(&session, is_final);
    assert!(events.contains(&LoginEvent::Started));
    assert!(
        events.contains(&LoginEvent::UrlDetected(AGY_URL.to_string())),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, LoginEvent::NeedsPastedCode)),
        "sin código que pegar"
    );
    assert_eq!(
        events.last(),
        Some(&LoginEvent::Completed { identity: None })
    );
    // The stderr line reaches "Ver detalles técnicos" without its query.
    let lines = outputs(&events);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Open the following link") && line.contains("[REDACTED]")),
        "{lines:?}"
    );
    assert_no_secret_in_output(&events, &["Ag3nTsT4te", "Ch4llEng3", "127.0.0.1"]);
    assert!(fx.profile.has_credentials(), "el token quedó en el perfil");
    // The agent ran with the profile as GEMINI_HOME and a muted browser.
    assert_eq!(
        fx.marker_value("GEMINI_HOME").as_deref(),
        Some(fx.profile.dir().to_str().expect("utf8"))
    );
    assert!(
        fx.marker_value("BROWSER")
            .is_some_and(|value| value.ends_with("true"))
    );
    #[cfg(unix)]
    {
        let pid = fx.agent_pid();
        let deadline = Instant::now() + TIMEOUT;
        while pid_alive(pid) {
            assert!(Instant::now() < deadline, "el agente siguió vivo");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn acp_login_reports_an_email_when_the_token_has_one() {
    let fx = acp_fixture();
    let session = LoginSession::spawn_acp(
        fx.login(&[
            ("FAKE_AUTH_URL", AGY_URL),
            ("FAKE_TOKEN_EMAIL", "ana@example.com"),
        ]),
        fx.options("acp-email"),
    );
    let events = until(&session, is_final);
    assert_eq!(
        events.last(),
        Some(&LoginEvent::Completed {
            identity: Some(Identity {
                email: Some("ana@example.com".to_string()),
                plan: None,
                organization: None,
            })
        })
    );
}

#[test]
fn acp_login_takes_the_link_from_a_url_elicitation() {
    let fx = acp_fixture();
    let session = LoginSession::spawn_acp(
        fx.login(&[("FAKE_AUTH_ELICIT_URL", AGY_URL)]),
        fx.options("acp-elicit"),
    );
    let events = until(&session, is_final);
    assert!(
        events.contains(&LoginEvent::UrlDetected(AGY_URL.to_string())),
        "{events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&LoginEvent::Completed { identity: None })
    );
}

#[test]
fn acp_login_error_response_fails_as_rejected() {
    let fx = acp_fixture();
    let session = LoginSession::spawn_acp(
        fx.login(&[("FAKE_AUTH_URL", AGY_URL), ("FAKE_AUTH_FAIL", "1")]),
        fx.options("acp-fail"),
    );
    let events = until(&session, is_final);
    assert!(
        matches!(
            events.last(),
            Some(LoginEvent::Failed {
                reason: LoginFailure::Rejected,
                ..
            })
        ),
        "{events:?}"
    );
    assert!(!fx.profile.has_credentials());
}

#[test]
fn acp_login_fails_when_the_agent_exits() {
    let fx = acp_fixture();
    let session = LoginSession::spawn_acp(
        fx.login(&[("FAKE_AUTH_URL", AGY_URL), ("FAKE_AUTH_EXIT", "1")]),
        fx.options("acp-exit"),
    );
    let events = until(&session, is_final);
    assert!(
        matches!(
            events.last(),
            Some(LoginEvent::Failed {
                reason: LoginFailure::Exit(_),
                ..
            })
        ),
        "{events:?}"
    );
}

#[test]
fn acp_login_without_a_token_file_is_not_logged_in() {
    let fx = acp_fixture();
    let session = LoginSession::spawn_acp(
        fx.login(&[("FAKE_AUTH_NO_TOKEN", "1")]),
        fx.options("acp-no-token"),
    );
    let events = until(&session, is_final);
    assert!(
        matches!(
            events.last(),
            Some(LoginEvent::Failed {
                reason: LoginFailure::NotLoggedIn,
                ..
            })
        ),
        "{events:?}"
    );
}

#[test]
fn acp_login_with_a_method_the_agent_does_not_offer_fails() {
    let fx = acp_fixture();
    let mut login = fx.login(&[]);
    login.method_id = "gemini-api-key".to_string();
    let session = LoginSession::spawn_acp(login, fx.options("acp-method"));
    let events = until(&session, is_final);
    match events.last() {
        Some(LoginEvent::Failed {
            reason: LoginFailure::Rejected,
            message,
        }) => assert!(message.contains("gemini-api-key"), "{message}"),
        other => panic!("se esperaba Rejected: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn acp_login_cancel_kills_the_agent_and_removes_the_pending_profile() {
    let fx = acp_fixture();
    let mut options = fx.options("acp-cancel");
    options.cleanup = Some(PendingCleanup {
        root: fx.root.clone(),
        dir: fx.profile.dir().to_path_buf(),
    });
    let session = LoginSession::spawn_acp(
        fx.login(&[("FAKE_AUTH_URL", AGY_URL), ("FAKE_AUTH_DELAY_MS", "60000")]),
        options,
    );
    until(&session, |event| {
        matches!(event, LoginEvent::UrlDetected(_))
    });
    let pid = fx.agent_pid();
    assert!(pid_alive(pid));
    assert!(session.send_code("X").is_err(), "no hay código que pegar");
    session.cancel();
    let rest = until(&session, is_final);
    assert_eq!(rest.last(), Some(&LoginEvent::Cancelled));
    let deadline = Instant::now() + TIMEOUT;
    while pid_alive(pid) {
        assert!(Instant::now() < deadline, "el agente siguió vivo");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!fx.profile.dir().exists(), "el perfil pendiente se borró");
}

#[cfg(unix)]
#[test]
fn acp_login_times_out_and_stops_the_agent() {
    let fx = acp_fixture();
    let mut options = fx.options("acp-timeout");
    options.timeout = Duration::from_millis(1500);
    let session = LoginSession::spawn_acp(
        fx.login(&[("FAKE_AUTH_URL", AGY_URL), ("FAKE_AUTH_DELAY_MS", "60000")]),
        options,
    );
    let events = until(&session, is_final);
    assert!(
        matches!(
            events.last(),
            Some(LoginEvent::Failed {
                reason: LoginFailure::Timeout,
                ..
            })
        ),
        "{events:?}"
    );
    let pid = fx.agent_pid();
    let deadline = Instant::now() + TIMEOUT;
    while pid_alive(pid) {
        assert!(Instant::now() < deadline, "el agente siguió vivo");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Probe against the real `agy_acp_server` (never run by default: it needs
/// the 920 MB binary). `CINCEL_REAL_AGY=<dir with agy_acp_server.par>` runs
/// `authenticate` in a throwaway `GEMINI_HOME`, checks that the Google link
/// arrives, then cancels: no login is ever completed.
#[cfg(unix)]
#[test]
#[ignore = "necesita el binario real de Antigravity (CINCEL_REAL_AGY)"]
fn real_antigravity_prints_its_login_link() {
    let Some(dir) = std::env::var_os("CINCEL_REAL_AGY") else {
        return;
    };
    let fx = acp_fixture();
    let program = PathBuf::from(dir).join("agy_acp_server.par");
    let login = AcpLogin {
        launch: LaunchSpec::new(program.display().to_string(), vec!["--uid=".to_string()]),
        env: fx.profile.process_env(None),
        cwd: fx.profile.dir().to_path_buf(),
        method_id: "oauth-personal".to_string(),
    };
    let session = LoginSession::spawn_acp(login, fx.options("acp-real"));
    let events = until(&session, |event| {
        matches!(event, LoginEvent::UrlDetected(_)) || is_final(event)
    });
    match events.last() {
        Some(LoginEvent::UrlDetected(url)) => {
            assert!(url.starts_with("https://accounts.google.com/"));
            assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
        }
        other => panic!("se esperaba el enlace: {other:?}"),
    }
    session.cancel();
    assert_eq!(
        until(&session, is_final).last(),
        Some(&LoginEvent::Cancelled)
    );
    assert!(!fx.profile.has_credentials());
}

/// §"Registro" (d): `login.rs`'s `emit` logs one line per step through
/// [`login_event_log_line`] — this checks that line, the exact string a
/// `tracing` subscriber would see, never carries the raw link, the one-time
/// code, a `Failed` message, an `Output` line or an [`Identity`]'s fields,
/// for every variant that can hold one.
#[test]
fn login_log_line_never_carries_the_link_the_code_or_the_identity() {
    let secret_url = "https://example.test/oauth?code=SUPER-SECRET-TOKEN";
    let secret_code = "PASTE-ME-1234";
    let secret_email = "persona@example.com";

    let events = [
        LoginEvent::UrlDetected(secret_url.to_string()),
        LoginEvent::CodeDetected(secret_code.to_string()),
        LoginEvent::Output(format!("abrí {secret_url} y pegá {secret_code}")),
        LoginEvent::Failed {
            message: format!("no se pudo completar {secret_url}"),
            reason: LoginFailure::Timeout,
        },
        LoginEvent::Completed {
            identity: Some(Identity {
                email: Some(secret_email.to_string()),
                plan: Some("Claude Max".to_string()),
                organization: Some("Acme".to_string()),
            }),
        },
    ];

    for event in &events {
        let line = login_event_log_line(event);
        assert!(!line.contains(secret_url), "filtró el enlace: {line}");
        assert!(!line.contains(secret_code), "filtró el código: {line}");
        assert!(!line.contains(secret_email), "filtró el email: {line}");
        assert!(!line.contains("Acme"), "filtró la organización: {line}");
    }

    // The line still says which step it was and, for `Failed`, why —
    // otherwise the log would be useless.
    assert_eq!(
        login_event_log_line(&LoginEvent::Started),
        "evento=iniciado"
    );
    assert_eq!(
        login_event_log_line(&LoginEvent::NeedsPastedCode),
        "evento=esperando_codigo_pegado"
    );
    assert_eq!(
        login_event_log_line(&LoginEvent::Cancelled),
        "evento=cancelado"
    );
    assert_eq!(
        login_event_log_line(&LoginEvent::Failed {
            message: "no importa".to_string(),
            reason: LoginFailure::Timeout,
        }),
        "evento=fallido motivo=Timeout"
    );
    assert_eq!(
        login_event_log_line(&LoginEvent::Completed { identity: None }),
        "evento=completado identidad_recibida=false"
    );
}
