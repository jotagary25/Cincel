//! Helpers shared by every GPUI test module of the crate.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use cincel_connections::{CincelPaths, Connections, Identity};
use uuid::Uuid;

/// Redirects the XDG directories into one temporary directory for the whole
/// test binary, so no test touches the real `recents.json`, `layout.json`,
/// conversations or review state.
///
/// `set_var` is process-wide: one guard for every module, or two modules
/// would each point the variables somewhere else while the other one's tests
/// are running (which is how a test used to lose its conversation index).
pub(crate) fn isolate_state() {
    static GUARD: OnceLock<tempfile::TempDir> = OnceLock::new();
    GUARD.get_or_init(|| {
        let dir = tempfile::tempdir().expect("no se pudo crear el directorio temporal");
        // SAFETY: called from the first test to run, before any code in this
        // process has resolved an XDG path; every module goes through here.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", dir.path().join("state"));
            std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
            std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
            std::env::set_var("XDG_CACHE_HOME", dir.path().join("cache"));
        }
        dir
    });
}

/// The login link the fake Claude login prints (with a secret-looking query,
/// like the real one).
pub(crate) const FAKE_LOGIN_URL: &str =
    "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=s3cr3t";
/// The link the fake Antigravity prints to stderr on `authenticate`.
pub(crate) const FAKE_AGY_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth?client_id=fake&redirect_uri=http%3A%2F%2F127.0.0.1%3A40000%2F&state=s3cr3t";
/// The code the fake login expects to be pasted back.
pub(crate) const FAKE_LOGIN_CODE: &str = "CODIGO-1";
/// The email the fake agent reports through `_auth/status_update`.
pub(crate) const FAKE_EMAIL: &str = "ana@example.com";
/// The version the fake runtime pretends to be.
const FAKE_NODE_VERSION: &str = "v24.1.0";
/// The version the fake adapters pretend to be.
const FAKE_ADAPTER_VERSION: &str = "0.0.1";

/// A test helper binary built next to this crate's tests
/// (`cargo build -p cincel-acp --bin cincel-acp-fake-agent` and
/// `cargo build -p cincel-connections --bins`; `cargo test --workspace`
/// builds both before running anything).
pub(crate) fn helper_binary(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("crates/cincel-workspace tiene dos ancestros");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "no se encontró {name} bajo {}; corré `cargo build -p cincel-acp --bin \
         cincel-acp-fake-agent -p cincel-connections --bins` primero",
        target_dir.display()
    );
}

/// A private data directory with a fake Node runtime and fake adapters for
/// the three agents, the same trick `cincel-connections`' engine tests use:
/// the "node" is a shell script that runs its script argument with `/bin/sh`,
/// and each npm "adapter" is a shell script that execs
/// `cincel-connections-fake-login` for the login command and
/// `cincel-acp-fake-agent` (the scripted ACP agent, reporting
/// [`FAKE_EMAIL`]) otherwise. Antigravity is an unpacked "binary"
/// (`agy_acp_server.par`, a shell script) that execs
/// `cincel-connections-fake-agent` in its Antigravity mode: the link on
/// stderr, `authenticate` answered after a short delay, a token file in
/// `GEMINI_HOME`. No network, no real agent, no real account.
pub(crate) struct FakeEnv {
    _dir: tempfile::TempDir,
    pub(crate) paths: CincelPaths,
    pub(crate) connections: Arc<Connections>,
}

impl FakeEnv {
    /// Lays everything out and builds the engine over it.
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = CincelPaths::under(dir.path());
        write_fake_runtime(&paths);
        for agent_id in ["claude-acp", "codex-acp"] {
            write_fake_adapter(&paths, agent_id);
        }
        write_fake_antigravity(&paths);
        let connections = Arc::new(Connections::new(paths.clone()));
        Self {
            _dir: dir,
            paths,
            connections,
        }
    }

    /// Like [`FakeEnv::new`], with `vars` set in the environment of the fake
    /// Claude agent (for instance `FAKE_SHELL_EDITS`, the scripted edits
    /// made straight on disk). Values are single-quoted in the adapter
    /// script, so they may hold anything but a single quote.
    pub(crate) fn with_agent_env(vars: &[(&str, &str)]) -> Self {
        let env = Self::new();
        let exports: String = vars
            .iter()
            .map(|(name, value)| {
                assert!(!value.contains('\''), "sin comillas simples: {value}");
                format!("{name}='{value}' ")
            })
            .collect();
        write_fake_adapter_with(&env.paths, "claude-acp", &exports);
        env
    }

    /// An engine over the same directory but with nothing installed: what a
    /// first run without network looks like.
    pub(crate) fn empty() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = CincelPaths::under(dir.path());
        let connections = Arc::new(Connections::new(paths.clone()));
        Self {
            _dir: dir,
            paths,
            connections,
        }
    }

    /// A saved, logged-in connection (as if F2 had run): a seeded profile
    /// with a credentials file, finalized with `label`.
    pub(crate) fn add_connection(&self, agent_id: &str, label: &str) -> Uuid {
        let store = self.connections.store();
        let pending = store.create_pending(agent_id).expect("perfil pendiente");
        let credentials = pending.profile.credentials_file();
        if let Some(parent) = credentials.parent() {
            std::fs::create_dir_all(parent).expect("carpeta de credenciales");
        }
        std::fs::write(&credentials, "{\"token\":\"falso\"}").expect("credenciales");
        let identity = Identity {
            email: Some(FAKE_EMAIL.to_string()),
            plan: Some("max".to_string()),
            organization: None,
        };
        store
            .finalize(&pending, label, Some(identity))
            .expect("finalize")
            .id
    }

    /// Deletes the credentials of a connection: the "sesión vencida"
    /// simulation of the spec (§10).
    pub(crate) fn expire(&self, id: Uuid) {
        let store = self.connections.store();
        let connection = store.get(id).expect("existe");
        let profile = store.profile(&connection).expect("perfil");
        std::fs::remove_file(profile.credentials_file()).expect("se borran las credenciales");
    }

    /// The profile directory of a connection.
    pub(crate) fn profile_dir(&self, id: Uuid) -> PathBuf {
        self.connections.store().profile_dir(id)
    }
}

fn write_executable(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("carpeta");
    }
    std::fs::write(path, body).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

fn write_fake_runtime(paths: &CincelPaths) {
    let root = paths
        .runtime_dir()
        .join(format!("node-{FAKE_NODE_VERSION}"));
    write_executable(&root.join("bin/node"), "#!/bin/sh\nexec /bin/sh \"$@\"\n");
    let npm = root.join("lib/node_modules/npm/bin");
    std::fs::create_dir_all(&npm).expect("npm");
    std::fs::write(npm.join("npm-cli.js"), "// npm\n").expect("npm-cli");
    std::fs::write(npm.join("npx-cli.js"), "// npx\n").expect("npx-cli");
    std::fs::write(paths.runtime_dir().join("current"), FAKE_NODE_VERSION).expect("current");
}

fn write_fake_adapter(paths: &CincelPaths, agent_id: &str) {
    write_fake_adapter_with(paths, agent_id, "");
}

/// [`write_fake_adapter`] with `exports` (`NAME='value' …`) in front of the
/// agent's `exec`.
fn write_fake_adapter_with(paths: &CincelPaths, agent_id: &str, exports: &str) {
    let login = helper_binary("cincel-connections-fake-login");
    let agent = helper_binary("cincel-acp-fake-agent");
    let login = login.display();
    let agent = agent.display();
    // The login command of each provider (`cincel_connections::plan_login`):
    // Claude `--cli auth login --claudeai` (link + pasted code), Codex
    // `cli login` (link, finishes by itself).
    let script = if agent_id == "claude-acp" {
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--cli\" ]; then\n  exec {login} --url '{FAKE_LOGIN_URL}' \
             --expect {FAKE_LOGIN_CODE} --creds-var CLAUDE_CONFIG_DIR\nfi\n\
             {exports}FAKE_AUTH_EMAIL={FAKE_EMAIL} exec {agent} \"$@\"\n"
        )
    } else {
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"cli\" ]; then\n  exec {login} --url \
             'https://auth.openai.com/oauth/authorize?state=s3cr3t' --creds-var CODEX_HOME \
             --creds-file auth.json\nfi\n{exports}FAKE_AUTH_EMAIL={FAKE_EMAIL} exec {agent} \"$@\"\n"
        )
    };
    let dir = paths.agents_dir().join(agent_id).join(FAKE_ADAPTER_VERSION);
    write_executable(&dir.join("dist/index.js"), &script);
    let package = if agent_id == "claude-acp" {
        "@agentclientprotocol/claude-agent-acp"
    } else {
        "@agentclientprotocol/codex-acp"
    };
    let marker = serde_json::json!({
        "agent_id": agent_id,
        "package": package,
        "version": FAKE_ADAPTER_VERSION,
        "bin": "dist/index.js",
        "args": [],
    });
    std::fs::write(
        dir.join(".cincel-adapter.json"),
        serde_json::to_vec_pretty(&marker).expect("json"),
    )
    .expect("marcador");
    std::fs::write(
        paths.agents_dir().join(agent_id).join("current"),
        FAKE_ADAPTER_VERSION,
    )
    .expect("current");
}

/// Antigravity as `Adapters::install_binary` leaves it: the unpacked
/// `agy_acp_server.par` plus a marker saying it came from an archive without
/// a published SHA-256.
fn write_fake_antigravity(paths: &CincelPaths) {
    let agent = helper_binary("cincel-connections-fake-agent");
    let agent = agent.display();
    let dir = paths
        .agents_dir()
        .join("antigravity-acp")
        .join(FAKE_ADAPTER_VERSION);
    write_executable(
        &dir.join("agy_acp_server.par"),
        &format!(
            "#!/bin/sh\nFAKE_NO_AUTH_STATUS=1 FAKE_AUTH_URL='{FAKE_AGY_URL}' FAKE_AUTH_DELAY_MS=100 \
             exec {agent} \"$@\"\n"
        ),
    );
    let marker = serde_json::json!({
        "agent_id": "antigravity-acp",
        "distribution": "binary",
        "package": "agy-acp-server-0.0.1-linux-x86_64.zip",
        "version": FAKE_ADAPTER_VERSION,
        "bin": "agy_acp_server.par",
        "args": ["--uid="],
        "verifiable": false,
    });
    std::fs::write(
        dir.join(".cincel-adapter.json"),
        serde_json::to_vec_pretty(&marker).expect("json"),
    )
    .expect("marcador");
    std::fs::write(
        paths.agents_dir().join("antigravity-acp").join("current"),
        FAKE_ADAPTER_VERSION,
    )
    .expect("current");
}
