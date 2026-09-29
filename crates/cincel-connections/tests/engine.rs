//! Engine tests: private runtime (in-memory downloader, no network),
//! adapter installs with the private npm, binary distributions (a fake
//! Antigravity zip/tar.gz served from memory), connection status, the full
//! login → finalize → launch flow (pty and ACP `authenticate`), identity
//! probes and safe disconnect.
//!
//! The "Node" here is a fake: a shell script shipped inside a tar.xz built
//! by the test, which emulates `npm install` and otherwise runs its script
//! argument with `/bin/sh`. The installed "adapter" is a shell script that
//! execs the fake login program (`--cli ...`) or the fake ACP agent. The
//! fake Antigravity archive holds an `agy_acp_server.par` shell script that
//! execs the fake ACP agent in its Antigravity mode.

#![cfg(unix)]

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use cincel_acp::{AgentCommand, AgentConnection, AgentEvent, AgentRegistry, LaunchSpec};
use cincel_connections::{
    AcpIdentityProbe, AdapterProgress, Adapters, AgentKind, CancelToken, CincelPaths,
    ConnectionStatus, Connections, ConnectionsError, Downloader, IdentityProbe, InstallKind,
    LoginEvent, LogoutStep, NodePaths, NodeVersion, PackageInstaller, PrepareProgress,
    ProbeOutcome, RangeBody, Runtime, RuntimeProgress, disconnect,
};

const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_cincel-connections-fake-agent");
const FAKE_LOGIN: &str = env!("CARGO_BIN_EXE_cincel-connections-fake-login");
const VERSION: &str = "v24.1.0";
const PLATFORM: &str = "linux-x64";
const BASE: &str = "mem://node/";
const CLAUDE_URL: &str =
    "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=s3cr3t";
const TIMEOUT: Duration = Duration::from_secs(30);
const AGY_PLATFORM: &str = "linux-x86_64";
const AGY_VERSION: &str = "1.2.1";
const AGY_ZIP: &str = "mem://agy/agy-acp-server-1.2.1-linux-x86_64.zip";
const AGY_TGZ: &str = "mem://agy/agy-acp-server-1.2.1-linux-x86_64.tar.gz";
const AGY_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth?client_id=fake&redirect_uri=http%3A%2F%2F127.0.0.1%3A40000%2F&state=Ag3nTsT4te";

// ------------------------------------------------------------ fixtures

const FAKE_NODE: &str = r#"#!/bin/sh
# Fake node for cincel-connections tests.
if [ "$2" = "install" ]; then
  shift 2
  prefix=""; cache=""; spec=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --prefix) prefix="$2"; shift 2 ;;
      --cache) cache="$2"; shift 2 ;;
      --*) shift ;;
      *) spec="$1"; shift ;;
    esac
  done
  name="${spec%@*}"
  echo "spec=$spec cache=$cache userconfig=$npm_config_userconfig path=$PATH" >> "$prefix/../npm-install.log"
  dir="$prefix/node_modules/$name"
  mkdir -p "$dir/dist"
  printf '{"name":"%s","bin":{"cmd":"dist/index.js"}}' "$name" > "$dir/package.json"
  cp "$(dirname "$0")/../lib/node_modules/npm/fake-adapter.sh" "$dir/dist/index.js"
  exit 0
fi
exec /bin/sh "$@"
"#;

fn fake_adapter_script(email: &str, marker: &Path, code: &str) -> String {
    format!(
        "#!/bin/sh\nif [ \"$1\" = \"--cli\" ]; then\n  exec {FAKE_LOGIN} --url '{CLAUDE_URL}' --expect {code} --creds-var CLAUDE_CONFIG_DIR\nfi\nFAKE_AUTH_EMAIL={email} FAKE_LOGOUT_MARKER={} exec {FAKE_AGENT} \"$@\"\n",
        marker.display()
    )
}

fn tar_entry(builder: &mut tar::Builder<Vec<u8>>, path: &str, body: &[u8], mode: u32) {
    let mut header = tar::Header::new_gnu();
    header.set_size(body.len() as u64);
    header.set_mode(mode);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    builder
        .append_data(&mut header, path, body)
        .expect("append");
}

/// A Node-like tar.xz: what the runtime keeps plus things it must drop.
fn node_tarball(adapter_script: &str) -> Vec<u8> {
    node_tarball_with(VERSION, FAKE_NODE, adapter_script)
}

/// [`node_tarball`] for another version and fake `node` script.
fn node_tarball_with(version: &str, node_script: &str, adapter_script: &str) -> Vec<u8> {
    let top = format!("node-{version}-{PLATFORM}");
    let mut builder = tar::Builder::new(Vec::new());
    tar_entry(
        &mut builder,
        &format!("{top}/bin/node"),
        node_script.as_bytes(),
        0o755,
    );
    tar_entry(
        &mut builder,
        &format!("{top}/lib/node_modules/npm/bin/npm-cli.js"),
        b"// npm\n",
        0o644,
    );
    tar_entry(
        &mut builder,
        &format!("{top}/lib/node_modules/npm/bin/npx-cli.js"),
        b"// npx\n",
        0o644,
    );
    tar_entry(
        &mut builder,
        &format!("{top}/lib/node_modules/npm/fake-adapter.sh"),
        adapter_script.as_bytes(),
        0o755,
    );
    tar_entry(
        &mut builder,
        &format!("{top}/include/node/node.h"),
        b"/* h */",
        0o644,
    );
    tar_entry(
        &mut builder,
        &format!("{top}/share/doc/node/x.md"),
        b"doc",
        0o644,
    );
    tar_entry(
        &mut builder,
        &format!("{top}/bin/corepack"),
        b"#!/bin/sh\n",
        0o755,
    );
    tar_entry(&mut builder, &format!("{top}/README.md"), b"readme", 0o644);
    tar_entry(&mut builder, &format!("{top}/LICENSE"), b"MIT", 0o644);
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    builder
        .append_link(
            &mut link,
            format!("{top}/bin/npm"),
            "../lib/node_modules/npm/bin/npm-cli.js",
        )
        .expect("symlink");
    let tar_bytes = builder.into_inner().expect("tar");
    let mut xz = Vec::new();
    lzma_rs::xz_compress(&mut std::io::Cursor::new(tar_bytes), &mut xz).expect("xz");
    xz
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// In-memory nodejs.org.
#[derive(Default)]
struct MemDownloader {
    files: HashMap<String, Vec<u8>>,
    /// Number of `open` calls that fail before serving.
    fail_opens: AtomicU32,
    /// Serve only the first half on the first `open` (a dropped download).
    truncate_first: AtomicU32,
    offsets: Mutex<Vec<u64>>,
    gets: Mutex<Vec<String>>,
}

impl MemDownloader {
    fn node(archive: &[u8]) -> Self {
        let name = format!("node-{VERSION}-{PLATFORM}.tar.xz");
        let mut files = HashMap::new();
        files.insert(
            format!("{BASE}index.json"),
            serde_json::to_vec(&serde_json::json!([
                { "version": "v25.0.0", "lts": false, "files": [PLATFORM] },
                { "version": VERSION, "lts": "Krypton", "files": [PLATFORM] },
                { "version": "v22.9.0", "lts": "Jod", "files": [PLATFORM] }
            ]))
            .expect("json"),
        );
        files.insert(
            format!("{BASE}{VERSION}/SHASUMS256.txt"),
            format!(
                "{}  node-{VERSION}-{PLATFORM}.tar.gz\n{}  {name}\n",
                "0".repeat(64),
                sha256_hex(archive)
            )
            .into_bytes(),
        );
        files.insert(format!("{BASE}{VERSION}/{name}"), archive.to_vec());
        Self {
            files,
            ..Self::default()
        }
    }
}

impl Downloader for MemDownloader {
    fn get_bytes(&self, url: &str) -> cincel_connections::Result<Vec<u8>> {
        self.gets.lock().expect("lock").push(url.to_string());
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| ConnectionsError::Network(format!("404 {url}")))
    }

    fn open(&self, url: &str, offset: u64) -> cincel_connections::Result<RangeBody> {
        self.offsets.lock().expect("lock").push(offset);
        if self.fail_opens.load(Ordering::SeqCst) > 0 {
            self.fail_opens.fetch_sub(1, Ordering::SeqCst);
            return Err(ConnectionsError::Network("conexión rechazada".to_string()));
        }
        let body = self
            .files
            .get(url)
            .cloned()
            .ok_or_else(|| ConnectionsError::Network(format!("404 {url}")))?;
        let total = body.len() as u64;
        let mut slice = body[offset as usize..].to_vec();
        if self.truncate_first.load(Ordering::SeqCst) > 0 {
            self.truncate_first.fetch_sub(1, Ordering::SeqCst);
            slice.truncate(slice.len() / 2);
        }
        Ok(RangeBody {
            resumed: offset > 0,
            total: Some(total),
            reader: Box::new(std::io::Cursor::new(slice)),
        })
    }
}

fn registry() -> AgentRegistry {
    AgentRegistry::parse(
        r#"{"version":"1.0.0","agents":[
          {"id":"claude-acp","name":"Claude Agent","version":"0.81.2",
           "distribution":{"npx":{"package":"@agentclientprotocol/claude-agent-acp@0.81.2"}}},
          {"id":"codex-acp","name":"Codex","version":"1.13.1",
           "distribution":{"npx":{"package":"@agentclientprotocol/codex-acp@1.13.1"}}},
          {"id":"gemini","name":"Gemini CLI","version":"0.61.0",
           "distribution":{"npx":{"package":"@google/gemini-cli@0.61.0","args":["--acp"]}}},
          {"id":"antigravity-acp","name":"Google Antigravity","version":"1.2.1",
           "distribution":{"binary":{"linux-x86_64":{
             "archive":"mem://agy/agy-acp-server-1.2.1-linux-x86_64.zip",
             "cmd":"./agy_acp_server.par","args":["--uid="],"env":{},"sha256":null}}}}
        ]}"#,
    )
    .expect("registry")
}

/// The fake `agy_acp_server.par`: the fake ACP agent in Antigravity mode
/// (no `_auth/status_update`, the link on stderr, a token file on success).
fn fake_agy_script(marker: &Path) -> String {
    fake_agy_script_with_delay(marker, 300)
}

/// Like [`fake_agy_script`], "approving in the browser" after `delay_ms`.
fn fake_agy_script_with_delay(marker: &Path, delay_ms: u64) -> String {
    format!(
        "#!/bin/sh\nFAKE_NO_AUTH_STATUS=1 FAKE_AUTH_URL='{AGY_URL}' FAKE_AUTH_DELAY_MS={delay_ms} \
         FAKE_LOGOUT_MARKER={} FAKE_ENV_MARKER={}.env exec {FAKE_AGENT} \"$@\"\n",
        marker.display(),
        marker.display()
    )
}

/// A zip laid out like Google's: the server (mode 0644, so the installer
/// must make it executable), the harness (0575 like the real one) and an
/// entry that tries to escape the install directory.
fn agy_zip(script: &str) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let entries: [(&str, &[u8], u32); 3] = [
        ("agy_acp_server.par", script.as_bytes(), 0o644),
        ("localharness_external", b"harness", 0o575),
        ("../fuera.txt", b"escape", 0o644),
    ];
    for (name, body, mode) in entries {
        writer
            .start_file(name, SimpleFileOptions::default().unix_permissions(mode))
            .expect("start");
        writer.write_all(body).expect("write");
    }
    writer.finish().expect("zip").into_inner()
}

/// The same layout as a `.tar.gz`.
fn agy_tar_gz(script: &str) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    tar_entry(&mut builder, "agy_acp_server.par", script.as_bytes(), 0o644);
    tar_entry(&mut builder, "localharness_external", b"harness", 0o755);
    let tar_bytes = builder.into_inner().expect("tar");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&tar_bytes).expect("gzip");
    encoder.finish().expect("gzip")
}

fn agy_downloader(archive: &[u8]) -> MemDownloader {
    let mut files = HashMap::new();
    files.insert(AGY_ZIP.to_string(), archive.to_vec());
    MemDownloader {
        files,
        ..MemDownloader::default()
    }
}

fn agy_adapters(env: &Env, downloader: MemDownloader) -> Adapters {
    Adapters::new(env.paths.clone())
        .with_downloader(Box::new(downloader))
        .with_platform(AGY_PLATFORM)
        .with_retry(3, Duration::ZERO)
        .with_free_space(|_| Some(u64::MAX))
}

fn agy_plan(archive: &str, sha256: Option<String>) -> cincel_connections::BinaryPlan {
    cincel_connections::BinaryPlan {
        version: AGY_VERSION.to_string(),
        platform: AGY_PLATFORM.to_string(),
        archive: archive.to_string(),
        cmd: "./agy_acp_server.par".to_string(),
        args: vec!["--uid=".to_string()],
        env: std::collections::BTreeMap::new(),
        sha256,
    }
}

/// Connections whose Antigravity comes from the fake zip (and whose Node
/// runtime, never used by it, would come from the fake tarball).
fn agy_connections(env: &Env) -> Connections {
    agy_connections_with(env, &fake_agy_script(&env.marker))
}

fn agy_connections_with(env: &Env, script: &str) -> Connections {
    let archive = agy_zip(script);
    connections(env, "X").with_adapters(agy_adapters(env, agy_downloader(&archive)))
}

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    paths: CincelPaths,
    marker: PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    Env {
        paths: CincelPaths::under(&root),
        marker: root.join("logout-marker"),
        root,
        _dir: dir,
    }
}

fn runtime(env: &Env, downloader: impl Downloader + 'static) -> Runtime {
    Runtime::new(env.paths.clone())
        .with_downloader(Box::new(downloader))
        .with_base_url(BASE)
        .with_platform(PLATFORM)
        .with_retry(3, Duration::ZERO)
}

fn connections(env: &Env, code: &str) -> Connections {
    let archive = node_tarball(&fake_adapter_script("ana@example.com", &env.marker, code));
    Connections::new(env.paths.clone()).with_runtime(runtime(env, MemDownloader::node(&archive)))
}

fn prepared(env: &Env, kind: AgentKind) -> (Connections, NodePaths) {
    let connections = connections(env, "CODIGO-1");
    let registry = registry();
    let (node, _) = connections
        .prepare(Some(&registry), kind, &mut |_| {}, &CancelToken::new())
        .expect("prepare");
    (connections, node.expect("los adaptadores npm usan Node"))
}

fn next_login(session: &cincel_connections::LoginSession) -> LoginEvent {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(event) = session.events().try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "sin eventos de login");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ------------------------------------------------------------- runtime

#[test]
fn runtime_downloads_verifies_extracts_and_strips() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let runtime = runtime(&env, MemDownloader::node(&archive));
    assert!(runtime.installed().is_none());
    let mut phases = Vec::new();
    let node = runtime
        .ensure(&mut |step| phases.push(step), &CancelToken::new())
        .expect("ensure");
    assert_eq!(node.version, VERSION);
    assert!(node.is_complete());
    assert!(node.root.ends_with(format!("runtime/node-{VERSION}")));
    assert!(
        node.bin_dir.join("npm").exists(),
        "el symlink de npm se conserva"
    );
    for dropped in ["include", "share", "README.md", "bin/corepack"] {
        assert!(
            !node.root.join(dropped).exists(),
            "{dropped} debía descartarse"
        );
    }
    assert!(node.root.join("LICENSE").is_file());
    assert_eq!(phases.first(), Some(&RuntimeProgress::Resolving));
    assert!(
        phases
            .iter()
            .any(|p| matches!(p, RuntimeProgress::Downloading { total: Some(_), .. }))
    );
    assert!(phases.contains(&RuntimeProgress::Verifying));
    assert!(phases.contains(&RuntimeProgress::Extracting));
    assert_eq!(phases.last(), Some(&RuntimeProgress::Done));
    // The archive and the decompressed tar are gone.
    let leftovers: Vec<_> = std::fs::read_dir(env.paths.downloads_dir())
        .expect("downloads")
        .flatten()
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert_eq!(runtime.installed(), Some(node));
}

#[test]
fn installed_runtime_is_reused_without_network() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    runtime(&env, MemDownloader::node(&archive))
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("first install");
    // A downloader with nothing to serve: any request would fail.
    let offline = runtime(&env, MemDownloader::default());
    let node = offline
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("offline reuse");
    assert_eq!(node.version, VERSION);
}

#[test]
fn interrupted_download_is_resumed_with_a_range_request() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let downloader = MemDownloader::node(&archive);
    downloader.truncate_first.store(1, Ordering::SeqCst);
    let runtime = runtime(&env, downloader);
    let mut phases = Vec::new();
    runtime
        .ensure(&mut |step| phases.push(step), &CancelToken::new())
        .expect("ensure");
    assert!(
        phases
            .iter()
            .any(|p| matches!(p, RuntimeProgress::Retrying { attempt: 2, .. })),
        "{phases:?}"
    );
    // The second attempt starts where the first one stopped (its very first
    // progress report already counts the bytes on disk).
    let retry_at = phases
        .iter()
        .position(|p| matches!(p, RuntimeProgress::Retrying { .. }))
        .expect("retry");
    let resumed_from = phases[retry_at..].iter().find_map(|p| match p {
        RuntimeProgress::Downloading { done, .. } => Some(*done),
        _ => None,
    });
    assert!(resumed_from.is_some_and(|done| done > 0), "{phases:?}");
    assert!(runtime.installed().is_some());
}

#[test]
fn network_failures_are_retried() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let downloader = MemDownloader::node(&archive);
    downloader.fail_opens.store(2, Ordering::SeqCst);
    let runtime = runtime(&env, downloader);
    runtime
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("third attempt works");
}

#[test]
fn checksum_mismatch_is_fatal_and_leaves_nothing_installed() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let mut downloader = MemDownloader::node(&archive);
    downloader.files.insert(
        format!("{BASE}{VERSION}/node-{VERSION}-{PLATFORM}.tar.xz"),
        b"archivo corrupto".to_vec(),
    );
    let runtime = runtime(&env, downloader);
    let error = runtime
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect_err("sha mismatch");
    assert!(
        matches!(error, ConnectionsError::ChecksumMismatch { .. }),
        "{error}"
    );
    assert!(runtime.installed().is_none());
    assert!(
        !env.paths
            .downloads_dir()
            .join(format!("node-{VERSION}-{PLATFORM}.tar.xz.part"))
            .exists(),
        "un parcial corrupto no se reanuda"
    );
}

#[test]
fn offline_without_runtime_reports_network_error() {
    let env = env();
    let runtime = runtime(&env, MemDownloader::default());
    assert!(matches!(
        runtime.ensure(&mut |_| {}, &CancelToken::new()),
        Err(ConnectionsError::Network(_))
    ));
}

#[test]
fn exact_version_skips_index_json() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let downloader = MemDownloader::node(&archive);
    let runtime = runtime(&env, downloader).with_version(NodeVersion::parse("24.1.0"));
    let node = runtime
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("ensure");
    assert_eq!(node.version, VERSION);
}

// ------------------------------------------------------------ adapters

#[test]
fn adapters_install_with_the_private_npm_and_cincel_cache() {
    let env = env();
    let (connections, node) = prepared(&env, AgentKind::Codex);
    let adapters = connections.adapters();
    assert_eq!(adapters.installed("codex-acp").as_deref(), Some("1.13.1"));
    let log = std::fs::read_to_string(env.paths.agents_dir().join("codex-acp/npm-install.log"))
        .expect("npm log");
    assert!(
        log.contains("spec=@agentclientprotocol/codex-acp@1.13.1"),
        "{log}"
    );
    assert!(
        log.contains(&format!("cache={}", env.paths.npm_cache_dir().display())),
        "{log}"
    );
    assert!(
        log.contains(&format!(
            "userconfig={}",
            env.paths.data_dir.join("npmrc").display()
        )),
        "{log}"
    );
    assert!(
        log.contains(&format!("path={}", node.bin_dir.display())),
        "el Node privado va primero en PATH: {log}"
    );
    let launch = adapters
        .launch_spec("codex-acp", Some(&node))
        .expect("launch");
    assert_eq!(launch.program, node.node.display().to_string());
    assert!(launch.args[0].ends_with("node_modules/@agentclientprotocol/codex-acp/dist/index.js"));
    assert_eq!(launch.args.len(), 1);
    assert!(matches!(
        adapters.launch_spec("codex-acp", None),
        Err(ConnectionsError::RuntimeMissing)
    ));
    // No staging leftovers.
    let names: Vec<String> = std::fs::read_dir(env.paths.agents_dir().join("codex-acp"))
        .expect("ls")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|name| !name.starts_with('.')), "{names:?}");
}

// ------------------------------------------------- binary distributions

fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).expect("meta").permissions().mode() & 0o777
}

#[test]
fn binary_install_downloads_unpacks_and_marks_it_unverifiable_without_sha256() {
    let env = env();
    let archive = agy_zip(&fake_agy_script(&env.marker));
    let adapters = agy_adapters(&env, agy_downloader(&archive));
    let plan = adapters
        .binary_plan(&registry(), AgentKind::Antigravity)
        .expect("plan");
    assert!(!plan.verifiable());
    let mut steps = Vec::new();
    let install = adapters
        .install_binary(
            AgentKind::Antigravity,
            &plan,
            &mut |step| steps.push(step),
            &CancelToken::new(),
        )
        .expect("install");
    assert_eq!(install.distribution, InstallKind::Binary);
    assert_eq!(install.version, AGY_VERSION);
    assert!(!install.verifiable, "Google no publica sha256");
    assert_eq!(install.args, vec!["--uid="]);
    assert!(steps.iter().any(|step| matches!(
        step,
        AdapterProgress::Downloading {
            total: Some(_),
            verifiable: false,
            ..
        }
    )));
    assert!(
        !steps.contains(&AdapterProgress::Verifying),
        "sin suma no hay verificación"
    );
    assert!(steps.contains(&AdapterProgress::Extracting));
    assert_eq!(
        steps.last(),
        Some(&AdapterProgress::Done {
            version: AGY_VERSION.to_string()
        })
    );
    let dir = adapters.install_dir("antigravity-acp", AGY_VERSION);
    let server = dir.join("agy_acp_server.par");
    assert_ne!(mode(&server) & 0o111, 0, "chmod +x del cmd");
    assert!(dir.join("localharness_external").is_file());
    assert_eq!(
        mode(&dir.join("localharness_external")) & 0o500,
        0o500,
        "los bits del zip se conservan"
    );
    assert!(
        !env.paths
            .agents_dir()
            .join("antigravity-acp/fuera.txt")
            .exists()
            && !env.paths.agents_dir().join("fuera.txt").exists(),
        "nada se escribe fuera de la carpeta de instalación"
    );
    // The archive is gone once unpacked; nothing half-done is left.
    let downloads: Vec<_> = std::fs::read_dir(env.paths.downloads_dir())
        .expect("downloads")
        .flatten()
        .collect();
    assert!(downloads.is_empty(), "{downloads:?}");
    let names: Vec<String> = std::fs::read_dir(env.paths.agents_dir().join("antigravity-acp"))
        .expect("ls")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|name| !name.starts_with('.')), "{names:?}");
    // Launch: the unpacked binary itself with the registry's args, no Node.
    let launch = adapters
        .launch_spec("antigravity-acp", None)
        .expect("launch");
    assert_eq!(launch.program, server.display().to_string());
    assert_eq!(launch.args, vec!["--uid="]);
    assert_eq!(adapters.installed_adapter("antigravity-acp"), Some(install));
    assert_eq!(
        adapters.update_available(&registry(), AgentKind::Antigravity),
        None
    );
}

#[test]
fn binary_install_checks_a_published_sha256() {
    let env = env();
    let archive = agy_zip(&fake_agy_script(&env.marker));
    let adapters = agy_adapters(&env, agy_downloader(&archive));
    let mut steps = Vec::new();
    let install = adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_ZIP, Some(sha256_hex(&archive))),
            &mut |step| steps.push(step),
            &CancelToken::new(),
        )
        .expect("install");
    assert!(install.verifiable);
    assert!(steps.contains(&AdapterProgress::Verifying));

    let other = self::env();
    let adapters = agy_adapters(&other, agy_downloader(&archive));
    let error = adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_ZIP, Some("0".repeat(64))),
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect_err("suma distinta");
    assert!(
        matches!(error, ConnectionsError::ChecksumMismatch { .. }),
        "{error}"
    );
    assert!(adapters.installed("antigravity-acp").is_none());
}

#[test]
fn interrupted_binary_download_is_resumed_with_a_range_request() {
    let env = env();
    let archive = agy_zip(&fake_agy_script(&env.marker));
    let downloader = agy_downloader(&archive);
    downloader.truncate_first.store(1, Ordering::SeqCst);
    let adapters = agy_adapters(&env, downloader);
    let mut steps = Vec::new();
    adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_ZIP, None),
            &mut |step| steps.push(step),
            &CancelToken::new(),
        )
        .expect("install");
    let retry_at = steps
        .iter()
        .position(|step| matches!(step, AdapterProgress::Retrying { attempt: 2, .. }))
        .expect("reintento");
    let resumed_from = steps[retry_at..].iter().find_map(|step| match step {
        AdapterProgress::Downloading { done, .. } => Some(*done),
        _ => None,
    });
    assert!(resumed_from.is_some_and(|done| done > 0), "{steps:?}");
    assert!(adapters.installed("antigravity-acp").is_some());
}

#[test]
fn binary_install_refuses_to_start_without_disk_space() {
    let env = env();
    let archive = agy_zip(&fake_agy_script(&env.marker));
    let adapters = Adapters::new(env.paths.clone())
        .with_downloader(Box::new(agy_downloader(&archive)))
        .with_platform(AGY_PLATFORM)
        .with_free_space(|_| Some(800_000_000));
    let error = adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_ZIP, None),
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect_err("sin espacio");
    match &error {
        ConnectionsError::NotEnoughSpace {
            needed, available, ..
        } => {
            assert_eq!(*needed, 1_400_000_000);
            assert_eq!(*available, 800_000_000);
        }
        other => panic!("se esperaba NotEnoughSpace: {other}"),
    }
    assert!(error.to_string().contains("1,4 GB"), "{error}");
    assert!(
        std::fs::read_dir(env.paths.downloads_dir())
            .map(|entries| entries.count())
            .unwrap_or(0)
            == 0,
        "no se descargó nada"
    );
}

#[test]
fn binary_install_also_checks_the_unpacked_size_of_the_zip() {
    let env = env();
    let archive = agy_zip(&fake_agy_script(&env.marker));
    // Enough for the (overridden) estimate, not for what the zip unpacks to.
    let adapters = agy_adapters(&env, agy_downloader(&archive))
        .with_space_needed(1)
        .with_free_space(|_| Some(10));
    let error = adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_ZIP, None),
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect_err("sin espacio para descomprimir");
    assert!(
        matches!(error, ConnectionsError::NotEnoughSpace { .. }),
        "{error}"
    );
    assert!(adapters.installed("antigravity-acp").is_none());
}

#[test]
fn binary_install_unpacks_tar_gz_archives_too() {
    let env = env();
    let archive = agy_tar_gz(&fake_agy_script(&env.marker));
    let mut files = HashMap::new();
    files.insert(AGY_TGZ.to_string(), archive);
    let adapters = agy_adapters(
        &env,
        MemDownloader {
            files,
            ..MemDownloader::default()
        },
    );
    let install = adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_TGZ, None),
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("install");
    let server = adapters
        .install_dir("antigravity-acp", AGY_VERSION)
        .join(&install.bin);
    assert_ne!(mode(&server) & 0o111, 0);
}

#[test]
fn a_corrupt_archive_fails_and_leaves_nothing_installed() {
    let env = env();
    let adapters = agy_adapters(&env, agy_downloader(b"esto no es un zip"));
    let error = adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_ZIP, None),
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect_err("zip roto");
    assert!(matches!(error, ConnectionsError::Extract { .. }), "{error}");
    assert!(adapters.installed("antigravity-acp").is_none());
    let downloads = std::fs::read_dir(env.paths.downloads_dir())
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(downloads, 0, "un archivo roto no se reanuda");
}

#[test]
fn update_available_compares_with_the_registry() {
    let env = env();
    let (connections, _) = prepared(&env, AgentKind::Claude);
    assert_eq!(
        connections
            .adapters()
            .update_available(&registry(), AgentKind::Claude),
        None
    );
    let newer = AgentRegistry::parse(
        r#"{"version":"1.0.0","agents":[{"id":"claude-acp","name":"Claude","version":"0.82.0",
           "distribution":{"npx":{"package":"@agentclientprotocol/claude-agent-acp@0.82.0"}}}]}"#,
    )
    .expect("parse");
    assert_eq!(
        connections
            .adapters()
            .update_available(&newer, AgentKind::Claude)
            .as_deref(),
        Some("0.82.0")
    );
}

struct FailingInstaller;

impl PackageInstaller for FailingInstaller {
    fn install(
        &self,
        _node: &NodePaths,
        package_spec: &str,
        _prefix: &Path,
        _cache: &Path,
        _npmrc: &Path,
        _cancel: &CancelToken,
    ) -> cincel_connections::Result<()> {
        Err(ConnectionsError::InstallFailed {
            package: package_spec.to_string(),
            message: "npm ERR! network".to_string(),
        })
    }
}

#[test]
fn failed_adapter_install_leaves_nothing_behind() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let node = runtime(&env, MemDownloader::node(&archive))
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("runtime");
    let adapters = Adapters::new(env.paths.clone()).with_installer(Box::new(FailingInstaller));
    let mut steps = Vec::new();
    let error = adapters
        .install(
            AgentKind::Codex,
            "1.13.1",
            &[],
            &node,
            &mut |step| steps.push(step),
            &CancelToken::new(),
        )
        .expect_err("falla");
    assert!(matches!(
        adapters.install(
            AgentKind::Antigravity,
            "1.2.1",
            &[],
            &node,
            &mut |_| {},
            &CancelToken::new()
        ),
        Err(ConnectionsError::InstallFailed { .. })
    ));
    assert!(matches!(error, ConnectionsError::InstallFailed { .. }));
    assert!(matches!(steps[0], AdapterProgress::Installing { .. }));
    assert!(adapters.installed("codex-acp").is_none());
    let leftovers = std::fs::read_dir(env.paths.agents_dir().join("codex-acp"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
    assert!(matches!(
        adapters.launch_spec("codex-acp", Some(&node)),
        Err(ConnectionsError::AdapterMissing(_))
    ));
}

#[test]
fn offline_prepare_uses_what_is_installed_or_explains_what_is_missing() {
    let env = env();
    let fresh = connections(&env, "X");
    assert!(matches!(
        fresh.prepare(None, AgentKind::Claude, &mut |_| {}, &CancelToken::new()),
        Err(ConnectionsError::RuntimeMissing)
    ));
    let (connections, _) = prepared(&env, AgentKind::Claude);
    let mut progress = Vec::new();
    connections
        .prepare(
            None,
            AgentKind::Claude,
            &mut |step| progress.push(step),
            &CancelToken::new(),
        )
        .expect("offline con todo instalado");
    assert!(progress.iter().all(|step| !matches!(
        step,
        PrepareProgress::Runtime(RuntimeProgress::Downloading { .. })
    )));
    assert!(matches!(
        connections.prepare(None, AgentKind::Codex, &mut |_| {}, &CancelToken::new()),
        Err(ConnectionsError::AdapterMissing(_))
    ));
}

// -------------------------------------------------------------- status

#[test]
fn status_is_computed_from_profile_runtime_and_adapter() {
    let env = env();
    let connections = connections(&env, "X");
    let pending = connections
        .store()
        .create_pending("claude-acp")
        .expect("pending");
    let saved = connections
        .store()
        .finalize(&pending, "Claude · personal", None)
        .expect("finalize");
    assert!(matches!(
        connections.status(&saved),
        ConnectionStatus::Unavailable { .. }
    ));
    connections
        .prepare(
            Some(&registry()),
            AgentKind::Claude,
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("prepare");
    assert_eq!(connections.status(&saved), ConnectionStatus::SessionExpired);
    std::fs::write(pending.profile.credentials_file(), "{\"t\":1}").expect("creds");
    assert_eq!(connections.status(&saved), ConnectionStatus::Connected);
    // Simulated expiry: the credentials file disappears.
    pending.profile.clear_credentials().expect("clear");
    assert_eq!(connections.status(&saved), ConnectionStatus::SessionExpired);
    // Adapter removed by hand: "No disponible" / "Reparar".
    std::fs::remove_file(env.paths.agents_dir().join("claude-acp/current")).expect("rm");
    match connections.status(&saved) {
        ConnectionStatus::Unavailable { reason } => assert!(reason.contains("Claude"), "{reason}"),
        other => panic!("se esperaba Unavailable: {other:?}"),
    }
    let listed = connections.list_with_status().expect("list");
    assert_eq!(listed.len(), 1);
}

// -------------------------------------------------- full flow (F2, F1)

#[test]
fn new_connection_login_finalize_and_launch_end_to_end() {
    let env = env();
    let (connections, _) = prepared(&env, AgentKind::Claude);
    let pending = connections
        .store()
        .create_pending("claude-acp")
        .expect("pending");
    let session = connections.start_login(&pending).expect("login");
    let mut url = None;
    loop {
        match next_login(&session) {
            LoginEvent::UrlDetected(found) => url = Some(found),
            LoginEvent::NeedsPastedCode => break,
            LoginEvent::Output(line) => assert!(!line.contains("s3cr3t"), "{line}"),
            LoginEvent::Failed { message, .. } => panic!("login falló: {message}"),
            _ => {}
        }
    }
    assert_eq!(url.as_deref(), Some(CLAUDE_URL));
    session.send_code("CODIGO-1").expect("code");
    let identity = loop {
        match next_login(&session) {
            LoginEvent::Completed { identity } => break identity,
            LoginEvent::Failed { message, .. } => panic!("login falló: {message}"),
            _ => {}
        }
    };
    // Identity confirmed by spawning the adapter and reading
    // `_auth/status_update`.
    let identity = identity.expect("identidad informada por el agente");
    assert_eq!(identity.email.as_deref(), Some("ana@example.com"));
    assert_eq!(identity.plan.as_deref(), Some("max"));
    assert!(
        pending.profile.has_credentials(),
        "credenciales dentro del perfil"
    );

    let saved = connections
        .store()
        .finalize(&pending, "Claude · personal", Some(identity))
        .expect("finalize");
    assert_eq!(connections.status(&saved), ConnectionStatus::Connected);

    // F1: launch the agent with the connection's profile.
    let launch = connections.agent_launch(&saved).expect("launch");
    assert!(launch.env.set.contains(&(
        "CLAUDE_CONFIG_DIR".to_string(),
        pending.profile.dir().display().to_string()
    )));
    let mut agent = AgentConnection::start_with_env(pending.profile.dir(), launch.env);
    let email = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt")
        .block_on(async {
            agent
                .send(AgentCommand::Spawn {
                    launch: launch.launch,
                    cwd: pending.profile.dir().to_path_buf(),
                })
                .await
                .expect("spawn");
            tokio::time::timeout(TIMEOUT, async {
                loop {
                    if let AgentEvent::AuthStatus { account, .. } = agent.recv().await.expect("ev")
                    {
                        return account.and_then(|account| account.email);
                    }
                }
            })
            .await
            .expect("timeout")
        });
    agent.shutdown();
    assert_eq!(email.as_deref(), Some("ana@example.com"));
}

#[test]
fn agent_runs_with_only_the_private_runtime_in_path() {
    let env = env();
    let (connections, node) = prepared(&env, AgentKind::Claude);
    let pending = connections
        .store()
        .create_pending("claude-acp")
        .expect("pending");
    let saved = connections
        .store()
        .finalize(&pending, "Claude", None)
        .expect("finalize");
    let mut launch = connections.agent_launch(&saved).expect("launch");
    // No system PATH at all besides a directory that does not exist.
    launch
        .env
        .set
        .push(("PATH".to_string(), "/no/existe".to_string()));
    let mut agent = AgentConnection::start_with_env(pending.profile.dir(), launch.env.clone());
    let connected = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt")
        .block_on(async {
            agent
                .send(AgentCommand::Spawn {
                    launch: launch.launch.clone(),
                    cwd: pending.profile.dir().to_path_buf(),
                })
                .await
                .expect("spawn");
            tokio::time::timeout(TIMEOUT, async {
                loop {
                    match agent.recv().await.expect("ev") {
                        AgentEvent::Connected { .. } => return true,
                        AgentEvent::Exited { .. } => return false,
                        _ => {}
                    }
                }
            })
            .await
            .expect("timeout")
        });
    agent.shutdown();
    assert!(connected, "el agente arranca sin Node del sistema");
    assert!(
        launch
            .launch
            .program
            .starts_with(&node.root.display().to_string())
    );
}

#[test]
fn relogin_cancel_keeps_the_existing_profile() {
    let env = env();
    let (connections, _) = prepared(&env, AgentKind::Claude);
    let pending = connections
        .store()
        .create_pending("claude-acp")
        .expect("pending");
    let saved = connections
        .store()
        .finalize(&pending, "Claude", None)
        .expect("finalize");
    let session = connections.start_relogin(saved.id).expect("relogin");
    loop {
        if matches!(next_login(&session), LoginEvent::NeedsPastedCode) {
            break;
        }
    }
    session.cancel();
    loop {
        if matches!(next_login(&session), LoginEvent::Cancelled) {
            break;
        }
    }
    assert!(
        pending.profile.dir().is_dir(),
        "Volver a conectar no borra el perfil"
    );
    assert!(connections.store().get(saved.id).is_ok());
}

#[test]
fn new_login_cancel_removes_the_pending_profile() {
    let env = env();
    let (connections, _) = prepared(&env, AgentKind::Claude);
    let pending = connections
        .store()
        .create_pending("claude-acp")
        .expect("pending");
    let session = connections.start_login(&pending).expect("login");
    loop {
        if matches!(next_login(&session), LoginEvent::UrlDetected(_)) {
            break;
        }
    }
    session.cancel();
    loop {
        if matches!(next_login(&session), LoginEvent::Cancelled) {
            break;
        }
    }
    assert!(!pending.profile.dir().exists());
    assert!(connections.store().list().expect("list").is_empty());
}

// ------------------------------------------------------------ identity

#[test]
fn acp_probe_reads_status_push_or_falls_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let probe = AcpIdentityProbe {
        launch: LaunchSpec::new(FAKE_AGENT, Vec::new())
            .with_env("FAKE_AUTH_EMAIL", "a@example.com"),
        env: cincel_acp::ProcessEnv::new(),
        cwd: dir.path().to_path_buf(),
        wait: TIMEOUT,
        fallback: None,
    };
    let outcome = probe.probe();
    assert_eq!(outcome.logged_in, Some(true));
    assert_eq!(
        outcome
            .identity
            .and_then(|identity| identity.email)
            .as_deref(),
        Some("a@example.com")
    );

    let logged_out = AcpIdentityProbe {
        launch: LaunchSpec::new(FAKE_AGENT, Vec::new()),
        env: cincel_acp::ProcessEnv::new(),
        cwd: dir.path().to_path_buf(),
        wait: TIMEOUT,
        fallback: None,
    };
    assert_eq!(logged_out.probe().logged_in, Some(false));

    struct Fallback;
    impl IdentityProbe for Fallback {
        fn probe(&self) -> ProbeOutcome {
            ProbeOutcome {
                logged_in: Some(true),
                identity: None,
            }
        }
    }
    let silent = AcpIdentityProbe {
        launch: LaunchSpec::new(FAKE_AGENT, Vec::new()).with_env("FAKE_NO_AUTH_STATUS", "1"),
        env: cincel_acp::ProcessEnv::new(),
        cwd: dir.path().to_path_buf(),
        wait: TIMEOUT,
        fallback: Some(Box::new(Fallback)),
    };
    assert_eq!(silent.probe().logged_in, Some(true));
}

// ---------------------------------------------------------- disconnect

#[test]
fn disconnect_logs_out_with_the_profile_variable_then_forgets() {
    let env = env();
    let (connections, _) = prepared(&env, AgentKind::Claude);
    let keep = connections
        .store()
        .create_pending("claude-acp")
        .expect("keep");
    let keep = connections
        .store()
        .finalize(&keep, "Claude · trabajo", None)
        .expect("keep");
    let gone = connections
        .store()
        .create_pending("claude-acp")
        .expect("gone");
    let gone_dir = gone.profile.dir().to_path_buf();
    let gone = connections
        .store()
        .finalize(&gone, "Claude · personal", None)
        .expect("gone");

    let report = connections.disconnect(gone.id).expect("disconnect");
    assert_eq!(report.logout, LogoutStep::LoggedOut);
    assert!(report.process_stopped);
    assert!(report.forgotten(), "{report:?}");
    // The logout ran with CLAUDE_CONFIG_DIR pointing at *that* profile.
    let marker = std::fs::read_to_string(&env.marker).expect("marker");
    assert_eq!(marker, gone_dir.display().to_string());
    assert!(!gone_dir.exists());
    let left = connections.store().list().expect("list");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, keep.id);
    assert!(connections.store().profile_dir(keep.id).is_dir());
    assert!(env.root.join("data/connections").is_dir());
}

#[test]
fn a_retired_gemini_connection_is_unavailable_and_deletes_directly() {
    let env = env();
    let connections = connections(&env, "X");
    let pending = connections
        .store()
        .create_pending("claude-acp")
        .expect("pending");
    let saved = connections
        .store()
        .finalize(&pending, "Gemini · personal", None)
        .expect("finalize");
    // A connection saved before Gemini was retired.
    let index = std::fs::read_to_string(env.paths.index_file()).expect("index");
    std::fs::write(
        env.paths.index_file(),
        index.replace("\"claude-acp\"", "\"gemini\""),
    )
    .expect("index");
    let saved = connections
        .store()
        .get(saved.id)
        .expect("sigue en el índice");
    assert_eq!(saved.agent_id, "gemini");
    match connections.status(&saved) {
        ConnectionStatus::Unavailable { reason } => {
            assert!(reason.contains("Antigravity"), "{reason}");
        }
        other => panic!("se esperaba Unavailable: {other:?}"),
    }
    let report = connections.disconnect(saved.id).expect("disconnect");
    assert_eq!(report.logout, LogoutStep::NotSupported);
    assert!(report.forgotten());
    assert!(!pending.profile.dir().exists());
    assert!(!env.marker.exists(), "no se lanzó ningún agente");
}

#[test]
fn disconnect_without_adapter_skips_logout_but_deletes() {
    let env = env();
    let connections = connections(&env, "X");
    let pending = connections
        .store()
        .create_pending("codex-acp")
        .expect("pending");
    let saved = connections
        .store()
        .finalize(&pending, "Codex", None)
        .expect("finalize");
    let report = connections.disconnect(saved.id).expect("disconnect");
    assert!(matches!(report.logout, LogoutStep::Skipped(_)));
    assert!(report.forgotten());
}

#[test]
fn disconnect_agent_without_logout_capability() {
    let env = env();
    let connections = connections(&env, "X");
    let pending = connections
        .store()
        .create_pending("codex-acp")
        .expect("pending");
    let saved = connections
        .store()
        .finalize(&pending, "Codex", None)
        .expect("finalize");
    let launch = LaunchSpec::new(FAKE_AGENT, Vec::new())
        .with_env("FAKE_NO_LOGOUT", "1")
        .with_env("FAKE_LOGOUT_MARKER", env.marker.display().to_string());
    let report = disconnect(
        connections.store(),
        saved.id,
        Some((launch, Some(Path::new("/no/existe/bin")))),
        TIMEOUT,
    )
    .expect("disconnect");
    assert_eq!(report.logout, LogoutStep::NotSupported);
    assert!(report.forgotten());
    assert!(!env.marker.exists());
}

#[test]
fn disconnect_unknown_connection_is_an_error() {
    let env = env();
    let connections = connections(&env, "X");
    assert!(matches!(
        connections.disconnect(uuid::Uuid::new_v4()),
        Err(ConnectionsError::UnknownConnection(_))
    ));
}

#[test]
fn no_log_line_of_a_login_contains_link_or_code() {
    // Spec 06 §10: "Ningún log contiene enlaces de login, códigos ni
    // tokens". Every `Output` is what may be logged; check a whole flow.
    let env = env();
    let (connections, _) = prepared(&env, AgentKind::Claude);
    let pending = connections
        .store()
        .create_pending("claude-acp")
        .expect("pending");
    let session = connections.start_login(&pending).expect("login");
    let mut lines = Vec::new();
    loop {
        match next_login(&session) {
            LoginEvent::Output(line) => lines.push(line),
            LoginEvent::NeedsPastedCode => session.send_code("CODIGO-1").expect("code"),
            LoginEvent::Completed { .. } => break,
            LoginEvent::Failed { message, .. } => panic!("{message}"),
            _ => {}
        }
    }
    let mut log = std::io::Cursor::new(Vec::new());
    for line in &lines {
        writeln!(log, "{line}").expect("write");
    }
    let log = String::from_utf8(log.into_inner()).expect("utf8");
    assert!(!log.contains("s3cr3t"), "{log}");
    assert!(!log.contains("client_id=abc"), "{log}");
    assert!(!log.contains("CODIGO-1"), "{log}");
}

// --------------------------------------------------- Antigravity (F1-F4)

#[test]
fn antigravity_prepares_without_node_and_reports_the_download() {
    let env = env();
    let connections = agy_connections(&env);
    assert!(!connections.is_ready(AgentKind::Antigravity));
    let mut steps = Vec::new();
    let (node, install) = connections
        .prepare(
            Some(&registry()),
            AgentKind::Antigravity,
            &mut |step| steps.push(step),
            &CancelToken::new(),
        )
        .expect("prepare");
    assert!(node.is_none(), "Antigravity no necesita Node");
    assert!(
        connections.runtime().installed().is_none(),
        "ni lo descarga"
    );
    assert!(
        steps
            .iter()
            .all(|step| matches!(step, PrepareProgress::Adapter(_))),
        "{steps:?}"
    );
    assert!(steps.iter().any(|step| matches!(
        step,
        PrepareProgress::Adapter(AdapterProgress::Downloading {
            verifiable: false,
            ..
        })
    )));
    assert!(!install.verifiable);
    assert!(connections.is_ready(AgentKind::Antigravity));
    // Offline afterwards: what is installed is enough.
    connections
        .prepare(
            None,
            AgentKind::Antigravity,
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("offline");
}

#[test]
fn antigravity_status_needs_no_node() {
    let env = env();
    let connections = agy_connections(&env);
    let pending = connections
        .store()
        .create_pending("antigravity-acp")
        .expect("pending");
    let saved = connections
        .store()
        .finalize(&pending, "Antigravity · personal", None)
        .expect("finalize");
    match connections.status(&saved) {
        ConnectionStatus::Unavailable { reason } => {
            assert!(reason.contains("Google Antigravity"), "{reason}");
        }
        other => panic!("se esperaba Unavailable: {other:?}"),
    }
    connections
        .prepare(
            Some(&registry()),
            AgentKind::Antigravity,
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("prepare");
    assert_eq!(connections.status(&saved), ConnectionStatus::SessionExpired);
    let token = pending.profile.credentials_file();
    std::fs::create_dir_all(token.parent().expect("padre")).expect("mkdir");
    std::fs::write(&token, "{\"refresh_token\":\"x\"}").expect("token");
    assert_eq!(connections.status(&saved), ConnectionStatus::Connected);
}

#[test]
fn antigravity_login_finalize_launch_relogin_and_disconnect_end_to_end() {
    let env = env();
    let connections = agy_connections(&env);
    connections
        .prepare(
            Some(&registry()),
            AgentKind::Antigravity,
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("prepare");
    let pending = connections
        .store()
        .create_pending("antigravity-acp")
        .expect("pending");
    let session = connections.start_login(&pending).expect("login");
    let mut url = None;
    let identity = loop {
        match next_login(&session) {
            LoginEvent::UrlDetected(found) => url = Some(found),
            LoginEvent::NeedsPastedCode => panic!("Antigravity no pide código"),
            LoginEvent::Output(line) => assert!(!line.contains("Ag3nTsT4te"), "{line}"),
            LoginEvent::Completed { identity } => break identity,
            LoginEvent::Failed { message, .. } => panic!("login falló: {message}"),
            _ => {}
        }
    };
    assert_eq!(url.as_deref(), Some(AGY_URL));
    assert_eq!(identity, None, "el token de Antigravity no trae email");
    assert!(pending.profile.has_credentials());
    assert!(
        pending
            .profile
            .credentials_file()
            .ends_with("antigravity-acp/acp_token.json")
    );
    let seen_env = std::fs::read_to_string(format!("{}.env", env.marker.display())).expect("env");
    assert!(
        seen_env.contains(&format!("GEMINI_HOME={}", pending.profile.dir().display())),
        "{seen_env}"
    );
    assert!(seen_env.contains("BROWSER=/bin/true") || seen_env.contains("BROWSER=true"));

    let saved = connections
        .store()
        .finalize(&pending, "Antigravity · personal", identity)
        .expect("finalize");
    assert_eq!(connections.status(&saved), ConnectionStatus::Connected);

    // F1: the unpacked binary with the profile environment, no Node.
    let launch = connections.agent_launch(&saved).expect("launch");
    assert!(launch.launch.program.ends_with("agy_acp_server.par"));
    assert_eq!(launch.launch.args, vec!["--uid="]);
    assert!(launch.env.path_prepend.is_empty());
    assert!(launch.env.set.contains(&(
        "GEMINI_HOME".to_string(),
        pending.profile.dir().display().to_string()
    )));
    assert!(launch.env.set.iter().any(|(name, _)| name == "BROWSER"));
    let mut agent = AgentConnection::start_with_env(pending.profile.dir(), launch.env);
    let connected = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt")
        .block_on(async {
            agent
                .send(AgentCommand::Spawn {
                    launch: launch.launch,
                    cwd: pending.profile.dir().to_path_buf(),
                })
                .await
                .expect("spawn");
            tokio::time::timeout(TIMEOUT, async {
                loop {
                    match agent.recv().await.expect("ev") {
                        AgentEvent::Connected { capabilities, .. } => {
                            return cincel_acp::agent_supports_logout(&capabilities);
                        }
                        AgentEvent::Exited { .. } => return false,
                        _ => {}
                    }
                }
            })
            .await
            .expect("timeout")
        });
    agent.shutdown();
    assert!(connected, "arranca y anuncia logout");

    // F4: "Volver a conectar" is the same ACP login on the same profile.
    pending.profile.clear_credentials().expect("vence");
    assert_eq!(connections.status(&saved), ConnectionStatus::SessionExpired);
    let session = connections.start_relogin(saved.id).expect("relogin");
    loop {
        match next_login(&session) {
            LoginEvent::Completed { .. } => break,
            LoginEvent::Failed { message, .. } => panic!("relogin falló: {message}"),
            _ => {}
        }
    }
    assert_eq!(connections.status(&saved), ConnectionStatus::Connected);

    // F3: ACP logout with GEMINI_HOME pointing at this profile, then delete.
    let report = connections.disconnect(saved.id).expect("disconnect");
    assert_eq!(report.logout, LogoutStep::LoggedOut);
    assert!(report.forgotten(), "{report:?}");
    let marker = std::fs::read_to_string(&env.marker).expect("marker");
    assert_eq!(marker, pending.profile.dir().display().to_string());
    assert!(!pending.profile.dir().exists());
}

#[test]
fn antigravity_login_cancel_removes_the_pending_profile() {
    let env = env();
    let connections = agy_connections_with(&env, &fake_agy_script_with_delay(&env.marker, 60_000));
    connections
        .prepare(
            Some(&registry()),
            AgentKind::Antigravity,
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("prepare");
    let pending = connections
        .store()
        .create_pending("antigravity-acp")
        .expect("pending");
    let session = connections.start_login(&pending).expect("login");
    loop {
        if matches!(next_login(&session), LoginEvent::UrlDetected(_)) {
            break;
        }
    }
    session.cancel();
    loop {
        match next_login(&session) {
            LoginEvent::Cancelled => break,
            LoginEvent::Completed { .. } | LoginEvent::Failed { .. } => {
                panic!("se esperaba Cancelled")
            }
            _ => {}
        }
    }
    assert!(!pending.profile.dir().exists(), "el login borró el perfil");
    assert!(connections.store().list().expect("list").is_empty());
}

/// Serves one local file as if it were the archive URL (with `Range`).
struct FileDownloader(PathBuf);

impl Downloader for FileDownloader {
    fn get_bytes(&self, url: &str) -> cincel_connections::Result<Vec<u8>> {
        Err(ConnectionsError::Network(format!(
            "sin red en esta prueba: {url}"
        )))
    }

    fn open(&self, _url: &str, offset: u64) -> cincel_connections::Result<RangeBody> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let mut file = std::fs::File::open(&self.0)
            .map_err(|error| ConnectionsError::Network(error.to_string()))?;
        let total = file
            .metadata()
            .map_err(|error| ConnectionsError::Network(error.to_string()))?
            .len();
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| ConnectionsError::Network(error.to_string()))?;
        Ok(RangeBody {
            resumed: offset > 0,
            total: Some(total),
            reader: Box::new(file.take(total - offset)),
        })
    }
}

/// Installs Google's real archive (never run by default: 333 MB that
/// unpacks to 1 GB). `CINCEL_REAL_AGY_ZIP=<agy-acp-server-*.zip>` installs
/// it into a temp dir with the real registry entry shape and checks that the
/// unpacked server answers `initialize` as `antigravity-acp`, run from
/// another working directory with an isolated `GEMINI_HOME`.
#[test]
#[ignore = "necesita el zip real de Antigravity (CINCEL_REAL_AGY_ZIP)"]
fn real_antigravity_archive_installs_and_initializes() {
    let Some(zip) = std::env::var_os("CINCEL_REAL_AGY_ZIP") else {
        return;
    };
    let env = env();
    let adapters = Adapters::new(env.paths.clone())
        .with_downloader(Box::new(FileDownloader(PathBuf::from(zip))))
        .with_platform(AGY_PLATFORM);
    let mut last_percent = None;
    let install = adapters
        .install_binary(
            AgentKind::Antigravity,
            &agy_plan(
                "https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-1.2.1-linux-x86_64.zip",
                None,
            ),
            &mut |step| {
                if let AdapterProgress::Downloading {
                    done,
                    total: Some(total),
                    ..
                } = step
                {
                    last_percent = Some(done * 100 / total);
                }
            },
            &CancelToken::new(),
        )
        .expect("install");
    assert_eq!(last_percent, Some(100));
    assert!(!install.verifiable);
    let profile = cincel_connections::Profile::new(AgentKind::Antigravity, env.root.join("perfil"));
    std::fs::create_dir_all(profile.dir()).expect("perfil");
    let launch = adapters
        .launch_spec("antigravity-acp", None)
        .expect("launch");
    let mut agent = AgentConnection::start_with_env(&env.root, profile.process_env(None));
    let title = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt")
        .block_on(async {
            agent
                .send(AgentCommand::Spawn {
                    launch,
                    cwd: env.root.clone(),
                })
                .await
                .expect("spawn");
            tokio::time::timeout(Duration::from_secs(120), async {
                loop {
                    match agent.recv().await.expect("ev") {
                        AgentEvent::Connected {
                            agent_info,
                            capabilities,
                            ..
                        } => {
                            assert!(cincel_acp::agent_supports_logout(&capabilities));
                            assert!(!cincel_acp::agent_supports_auth_status(&capabilities));
                            return agent_info.map(|info| info.name);
                        }
                        AgentEvent::Exited { stderr_tail, .. } => panic!("salió: {stderr_tail}"),
                        _ => {}
                    }
                }
            })
            .await
            .expect("timeout")
        });
    agent.shutdown();
    assert_eq!(title.as_deref(), Some("antigravity-acp"));
    assert!(!profile.has_credentials());
}

// ------------------------------------------------ cancel (spec 07 §10.3)

/// A download that trickles zeros: `per_mb` per MB, read by read (so the
/// 64 KiB reads of the engine see a steady stream).
struct SlowDownloader {
    files: HashMap<String, Vec<u8>>,
    size: u64,
    per_mb: Duration,
}

struct SlowReader {
    left: u64,
    per_mb: Duration,
}

impl std::io::Read for SlowReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = (buf.len() as u64).min(self.left);
        if count == 0 {
            return Ok(0);
        }
        std::thread::sleep(self.per_mb.mul_f64(count as f64 / 1_000_000.0));
        buf[..count as usize].fill(0);
        self.left -= count;
        Ok(count as usize)
    }
}

impl Downloader for SlowDownloader {
    fn get_bytes(&self, url: &str) -> cincel_connections::Result<Vec<u8>> {
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| ConnectionsError::Network(format!("404 {url}")))
    }

    fn open(&self, _url: &str, offset: u64) -> cincel_connections::Result<RangeBody> {
        Ok(RangeBody {
            resumed: offset > 0,
            total: Some(self.size),
            reader: Box::new(SlowReader {
                left: self.size - offset,
                per_mb: self.per_mb,
            }),
        })
    }
}

/// 1 MB every 100 ms, 50 MB in total: never finishes within a test.
fn slow(files: HashMap<String, Vec<u8>>) -> SlowDownloader {
    SlowDownloader {
        files,
        size: 50_000_000,
        per_mb: Duration::from_millis(100),
    }
}

/// Names in `dir` (empty when it does not exist).
fn names_in(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// No `.part`, decompressed tarball or staging directory anywhere.
fn assert_no_leftovers(env: &Env) {
    let downloads = names_in(&env.paths.downloads_dir());
    assert!(downloads.is_empty(), "descargas a medias: {downloads:?}");
    let runtime = names_in(&env.paths.runtime_dir());
    assert!(
        runtime.iter().all(|name| !name.starts_with('.')),
        "staging del runtime: {runtime:?}"
    );
    for agent in names_in(&env.paths.agents_dir()) {
        let inside = names_in(&env.paths.agents_dir().join(&agent));
        assert!(
            inside.iter().all(|name| !name.starts_with('.')),
            "staging de {agent}: {inside:?}"
        );
    }
}

/// Run `work` on a thread, cancel `token` after `after`, and return the
/// result plus how long the thread took to finish once cancelled.
fn cancel_after<T: Send>(
    token: &CancelToken,
    after: Duration,
    work: impl FnOnce() -> T + Send,
) -> (T, Duration) {
    std::thread::scope(|scope| {
        let handle = scope.spawn(work);
        std::thread::sleep(after);
        let cancelled_at = Instant::now();
        token.cancel();
        let result = handle.join().expect("join");
        (result, cancelled_at.elapsed())
    })
}

#[test]
fn cancelling_a_node_download_stops_within_500_ms_and_leaves_nothing() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let runtime = runtime(&env, slow(MemDownloader::node(&archive).files));
    let token = CancelToken::new();
    let downloaded = Mutex::new(0u64);
    let (result, took) = cancel_after(&token, Duration::from_millis(300), || {
        runtime.ensure(
            &mut |step| {
                if let RuntimeProgress::Downloading { done, .. } = step {
                    *downloaded.lock().expect("lock") = done;
                }
            },
            &token,
        )
    });
    assert!(
        matches!(result, Err(ConnectionsError::Cancelled)),
        "{result:?}"
    );
    assert!(took < Duration::from_millis(500), "tardó {took:?}");
    assert!(*downloaded.lock().expect("lock") > 0, "la descarga empezó");
    assert!(runtime.installed().is_none());
    assert_no_leftovers(&env);
}

#[test]
fn cancelling_a_binary_download_stops_within_500_ms_and_leaves_nothing() {
    let env = env();
    let mut files = HashMap::new();
    files.insert(AGY_ZIP.to_string(), Vec::new());
    let adapters = Adapters::new(env.paths.clone())
        .with_downloader(Box::new(slow(files)))
        .with_platform(AGY_PLATFORM)
        .with_free_space(|_| Some(u64::MAX));
    let token = CancelToken::new();
    let (result, took) = cancel_after(&token, Duration::from_millis(300), || {
        adapters.install_binary(
            AgentKind::Antigravity,
            &agy_plan(AGY_ZIP, None),
            &mut |_| {},
            &token,
        )
    });
    assert!(
        matches!(result, Err(ConnectionsError::Cancelled)),
        "{result:?}"
    );
    assert!(took < Duration::from_millis(500), "tardó {took:?}");
    assert!(adapters.installed("antigravity-acp").is_none());
    assert_no_leftovers(&env);
}

/// A fake node whose `npm install` records its pid and the pid of a child
/// `sleep`, then waits for it (as long as nobody kills the group).
fn sleeping_npm(root: &Path) -> String {
    format!(
        "#!/bin/sh\nif [ \"$2\" = \"install\" ]; then\n  echo $$ > '{root}/npm.pid'\n  sleep 30 &\n  echo $! > '{root}/sleep.pid'\n  wait\n  exit 0\nfi\nexec /bin/sh \"$@\"\n",
        root = root.display()
    )
}

/// Whether `pid` is a live (non-zombie) process.
fn alive(pid: i32) -> bool {
    if Path::new("/proc/self").exists() {
        return std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            !stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
        });
    }
    rustix::process::Pid::from_raw(pid)
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

fn read_pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[test]
fn cancelling_npm_install_kills_its_process_group() {
    let env = env();
    let archive = node_tarball_with(VERSION, &sleeping_npm(&env.root), "#!/bin/sh\n");
    let node = runtime(&env, MemDownloader::node(&archive))
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("runtime");
    let adapters = Adapters::new(env.paths.clone());
    let token = CancelToken::new();
    let (npm_pid, sleep_pid) = (env.root.join("npm.pid"), env.root.join("sleep.pid"));
    let (result, took, pids) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            adapters.install(AgentKind::Codex, "1.13.1", &[], &node, &mut |_| {}, &token)
        });
        let deadline = Instant::now() + TIMEOUT;
        let pids = loop {
            if let (Some(npm), Some(sleep)) = (read_pid(&npm_pid), read_pid(&sleep_pid)) {
                break (npm, sleep);
            }
            assert!(Instant::now() < deadline, "npm falso no arrancó");
            std::thread::sleep(Duration::from_millis(20));
        };
        let cancelled_at = Instant::now();
        token.cancel();
        let result = handle.join().expect("join");
        (result, cancelled_at.elapsed(), pids)
    });
    assert!(
        matches!(result, Err(ConnectionsError::Cancelled)),
        "{result:?}"
    );
    assert!(took < Duration::from_millis(500), "tardó {took:?}");
    let (npm, sleep) = pids;
    assert!(!alive(npm), "el proceso npm sigue vivo");
    // The orphaned `sleep` is reaped by init: give it a moment.
    let deadline = Instant::now() + Duration::from_secs(3);
    while alive(sleep) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(sleep), "el grupo de procesos de npm sigue vivo");
    assert!(adapters.installed("codex-acp").is_none());
    assert_no_leftovers(&env);
}

#[test]
fn cancelling_while_unpacking_node_cleans_up() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let runtime = runtime(&env, MemDownloader::node(&archive));
    let token = CancelToken::new();
    let result = runtime.ensure(
        &mut |step| {
            if step == RuntimeProgress::Extracting {
                token.cancel();
            }
        },
        &token,
    );
    assert!(
        matches!(result, Err(ConnectionsError::Cancelled)),
        "{result:?}"
    );
    assert!(runtime.installed().is_none());
    assert!(!env.paths.runtime_dir().join("current").exists());
    assert_no_leftovers(&env);
}

#[test]
fn cancelling_while_unpacking_a_binary_cleans_up() {
    let env = env();
    let archive = agy_zip(&fake_agy_script(&env.marker));
    let adapters = agy_adapters(&env, agy_downloader(&archive));
    let token = CancelToken::new();
    let result = adapters.install_binary(
        AgentKind::Antigravity,
        &agy_plan(AGY_ZIP, None),
        &mut |step| {
            if step == AdapterProgress::Extracting {
                token.cancel();
            }
        },
        &token,
    );
    assert!(
        matches!(result, Err(ConnectionsError::Cancelled)),
        "{result:?}"
    );
    assert!(adapters.installed("antigravity-acp").is_none());
    assert!(
        !adapters
            .install_dir("antigravity-acp", AGY_VERSION)
            .exists()
    );
    assert_no_leftovers(&env);
}

#[test]
fn cancelling_a_retry_pause_returns_promptly() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let downloader = MemDownloader::node(&archive);
    downloader.fail_opens.store(10, Ordering::SeqCst);
    let runtime = runtime(&env, downloader).with_retry(3, Duration::from_secs(20));
    let token = CancelToken::new();
    let (result, took) = cancel_after(&token, Duration::from_millis(200), || {
        runtime.ensure(&mut |_| {}, &token)
    });
    assert!(
        matches!(result, Err(ConnectionsError::Cancelled)),
        "{result:?}"
    );
    assert!(took < Duration::from_millis(500), "tardó {took:?}");
    assert_no_leftovers(&env);
}

#[test]
fn prepare_with_a_cancelled_token_does_nothing() {
    let env = env();
    let connections = connections(&env, "X");
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(
        connections.prepare(Some(&registry()), AgentKind::Claude, &mut |_| {}, &token),
        Err(ConnectionsError::Cancelled)
    ));
    assert!(connections.runtime().installed().is_none());
    assert!(names_in(&env.paths.downloads_dir()).is_empty());
}

#[test]
fn a_second_download_waits_for_the_cancelled_one_to_unwind() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    let runtime = runtime(&env, slow(MemDownloader::node(&archive).files));
    let (first, second) = (CancelToken::new(), CancelToken::new());
    let second_started = Mutex::new(None);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| runtime.ensure(&mut |_| {}, &first));
        std::thread::sleep(Duration::from_millis(150));
        let b = scope.spawn(|| {
            runtime.ensure(
                &mut |_| {
                    second_started
                        .lock()
                        .expect("lock")
                        .get_or_insert_with(Instant::now);
                },
                &second,
            )
        });
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            second_started.lock().expect("lock").is_none(),
            "la segunda descarga no espera a la primera"
        );
        first.cancel();
        let a = a.join().expect("join a");
        let first_done = Instant::now();
        assert!(matches!(a, Err(ConnectionsError::Cancelled)), "{a:?}");
        std::thread::sleep(Duration::from_millis(150));
        second.cancel();
        let b = b.join().expect("join b");
        assert!(matches!(b, Err(ConnectionsError::Cancelled)), "{b:?}");
        let started = second_started.lock().expect("lock").expect("empezó");
        assert!(
            started + Duration::from_millis(100) >= first_done,
            "la segunda empezó antes de que terminara la primera"
        );
    });
    assert_no_leftovers(&env);
}

// ------------------------------------------------ updates (spec 07 §10.4)

fn claude_registry(version: &str) -> AgentRegistry {
    AgentRegistry::parse(&format!(
        r#"{{"version":"1.0.0","agents":[{{"id":"claude-acp","name":"Claude","version":"{version}",
           "distribution":{{"npx":{{"package":"@agentclientprotocol/claude-agent-acp@{version}"}}}}}}]}}"#
    ))
    .expect("registry")
}

#[test]
fn adapter_update_keeps_the_version_in_use_until_pruned() {
    let env = env();
    let (connections, node) = prepared(&env, AgentKind::Claude);
    let adapters = connections.adapters();
    let newer = claude_registry("0.82.0");
    assert_eq!(
        adapters
            .update_available(&newer, AgentKind::Claude)
            .as_deref(),
        Some("0.82.0")
    );
    let mut steps = Vec::new();
    let install = adapters
        .update(
            &newer,
            AgentKind::Claude,
            Some(&node),
            &mut |step| steps.push(step),
            &CancelToken::new(),
        )
        .expect("update");
    assert_eq!(install.version, "0.82.0");
    assert_eq!(
        steps.last(),
        Some(&AdapterProgress::Done {
            version: "0.82.0".to_string()
        })
    );
    assert_eq!(adapters.installed("claude-acp").as_deref(), Some("0.82.0"));
    let old = adapters.install_dir("claude-acp", "0.81.2");
    assert!(old.is_dir(), "la versión en uso sigue en disco");
    let launch = adapters
        .launch_spec("claude-acp", Some(&node))
        .expect("launch");
    assert!(launch.args[0].contains("/0.82.0/"), "{launch:?}");
    assert_eq!(adapters.update_available(&newer, AgentKind::Claude), None);

    // A live connection still runs 0.81.2: nothing goes.
    assert!(
        adapters
            .prune_unused(&[("claude-acp", "0.81.2")])
            .expect("prune")
            .is_empty()
    );
    assert!(old.is_dir());
    // Next start (nothing running): the old version goes.
    assert_eq!(
        adapters.prune_unused(&[]).expect("prune"),
        vec![("claude-acp".to_string(), "0.81.2".to_string())]
    );
    assert!(!old.exists());
    assert!(adapters.install_dir("claude-acp", "0.82.0").is_dir());

    // Updating to what is installed is a no-op.
    let again = adapters
        .update(
            &newer,
            AgentKind::Claude,
            Some(&node),
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("no-op");
    assert_eq!(again, install);
}

#[test]
fn a_cancelled_update_leaves_the_installed_version_alone() {
    let env = env();
    let (connections, node) = prepared(&env, AgentKind::Claude);
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(
        connections.adapters().update(
            &claude_registry("0.82.0"),
            AgentKind::Claude,
            Some(&node),
            &mut |_| {},
            &token,
        ),
        Err(ConnectionsError::Cancelled)
    ));
    assert_eq!(
        connections.adapters().installed("claude-acp").as_deref(),
        Some("0.81.2")
    );
}

#[test]
fn binary_update_keeps_the_old_version_until_pruned() {
    let env = env();
    let script = fake_agy_script(&env.marker);
    let newer_zip = "mem://agy/agy-acp-server-1.3.0-linux-x86_64.zip";
    let mut files = HashMap::new();
    files.insert(AGY_ZIP.to_string(), agy_zip(&script));
    files.insert(newer_zip.to_string(), agy_zip(&script));
    let adapters = agy_adapters(
        &env,
        MemDownloader {
            files,
            ..MemDownloader::default()
        },
    );
    adapters
        .ensure(
            Some(&registry()),
            AgentKind::Antigravity,
            None,
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("install 1.2.1");
    let newer = AgentRegistry::parse(&format!(
        r#"{{"version":"1.0.0","agents":[{{"id":"antigravity-acp","name":"Google Antigravity","version":"1.3.0",
           "distribution":{{"binary":{{"linux-x86_64":{{"archive":"{newer_zip}",
             "cmd":"./agy_acp_server.par","args":["--uid="],"sha256":null}}}}}}}}]}}"#
    ))
    .expect("registry");
    assert_eq!(
        adapters
            .update_available(&newer, AgentKind::Antigravity)
            .as_deref(),
        Some("1.3.0")
    );
    let install = adapters
        .update(
            &newer,
            AgentKind::Antigravity,
            None,
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("update");
    assert_eq!(install.version, "1.3.0");
    assert_eq!(
        adapters.installed("antigravity-acp").as_deref(),
        Some("1.3.0")
    );
    assert!(adapters.install_dir("antigravity-acp", "1.2.1").is_dir());
    assert_eq!(
        adapters.prune_unused(&[]).expect("prune"),
        vec![("antigravity-acp".to_string(), "1.2.1".to_string())]
    );
    assert!(!adapters.install_dir("antigravity-acp", "1.2.1").exists());
    assert!(names_in(&env.paths.downloads_dir()).is_empty());
}

/// nodejs.org with the given `(version, lts)` releases in `index.json`, and
/// SHASUMS + archive for the ones in `archives`.
fn node_dist(
    releases: &[(&str, serde_json::Value)],
    archives: &[(&str, Vec<u8>)],
) -> MemDownloader {
    let mut files = HashMap::new();
    let index: Vec<_> = releases
        .iter()
        .map(|(version, lts)| serde_json::json!({ "version": version, "lts": lts, "files": [PLATFORM] }))
        .collect();
    files.insert(
        format!("{BASE}index.json"),
        serde_json::to_vec(&index).expect("json"),
    );
    for (version, archive) in archives {
        let name = format!("node-{version}-{PLATFORM}.tar.xz");
        files.insert(
            format!("{BASE}{version}/SHASUMS256.txt"),
            format!("{}  {name}\n", sha256_hex(archive)).into_bytes(),
        );
        files.insert(format!("{BASE}{version}/{name}"), archive.clone());
    }
    MemDownloader {
        files,
        ..MemDownloader::default()
    }
}

const NEWER_NODE: &str = "v24.2.0";

#[test]
fn node_update_offers_the_new_lts_and_keeps_the_old_runtime_until_pruned() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    runtime(&env, MemDownloader::node(&archive))
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("v24.1.0");
    let newer = node_tarball_with(NEWER_NODE, FAKE_NODE, "#!/bin/sh\n");
    let dist = node_dist(
        &[
            ("v25.0.0", serde_json::json!(false)),
            (NEWER_NODE, serde_json::json!("Krypton")),
            (VERSION, serde_json::json!("Krypton")),
        ],
        &[(NEWER_NODE, newer)],
    );
    let runtime = runtime(&env, dist);
    assert_eq!(
        runtime.update_available().expect("check").as_deref(),
        Some(NEWER_NODE),
        "la LTS más nueva, no la 25 (no LTS)"
    );
    let mut steps = Vec::new();
    let node = runtime
        .update(&mut |step| steps.push(step), &CancelToken::new())
        .expect("update");
    assert_eq!(node.version, NEWER_NODE);
    assert_eq!(steps.last(), Some(&RuntimeProgress::Done));
    assert_eq!(runtime.installed(), Some(node));
    let old = env.paths.runtime_dir().join(format!("node-{VERSION}"));
    assert!(old.is_dir(), "el Node en uso sigue en disco");
    assert_eq!(runtime.update_available().expect("check"), None);
    assert_eq!(
        runtime.prune_old().expect("prune"),
        vec![format!("node-{VERSION}")]
    );
    assert!(!old.exists());
    assert!(runtime.installed().is_some());
    assert_no_leftovers(&env);
}

#[test]
fn pinned_node_version_never_offers_an_update() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    runtime(&env, MemDownloader::node(&archive))
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("v24.1.0");
    // Pinned: no network at all (this downloader would fail every request).
    let pinned = runtime(&env, MemDownloader::default()).with_version(NodeVersion::parse("24.1.0"));
    assert_eq!(pinned.update_available().expect("check"), None);
    // Same setting with a newer release published: still nothing.
    let dist = node_dist(
        &[
            (NEWER_NODE, serde_json::json!("Krypton")),
            (VERSION, serde_json::json!("Krypton")),
        ],
        &[],
    );
    let pinned = runtime(&env, dist).with_version(NodeVersion::parse("v24.1.0"));
    assert_eq!(pinned.update_available().expect("check"), None);
}

#[test]
fn node_major_policy_follows_its_line() {
    let env = env();
    let archive = node_tarball("#!/bin/sh\n");
    runtime(&env, MemDownloader::node(&archive))
        .ensure(&mut |_| {}, &CancelToken::new())
        .expect("v24.1.0");
    let releases = [
        ("v25.1.0", serde_json::json!(false)),
        (NEWER_NODE, serde_json::json!(false)),
        (VERSION, serde_json::json!("Krypton")),
        ("v22.9.0", serde_json::json!("Jod")),
    ];
    let line_24 = runtime(&env, node_dist(&releases, &[])).with_version(NodeVersion::parse("24"));
    assert_eq!(
        line_24.update_available().expect("check").as_deref(),
        Some(NEWER_NODE)
    );
    let line_25 = runtime(&env, node_dist(&releases, &[])).with_version(NodeVersion::parse("25"));
    assert_eq!(
        line_25.update_available().expect("check").as_deref(),
        Some("v25.1.0")
    );
    // LTS policy: v24.1.0 is already the newest LTS >= 22 here.
    let lts = runtime(&env, node_dist(&releases, &[]));
    assert_eq!(lts.update_available().expect("check"), None);
    // Offline: the check fails instead of pretending there is nothing new.
    assert!(matches!(
        runtime(&env, MemDownloader::default()).update_available(),
        Err(ConnectionsError::Network(_))
    ));
}

#[test]
fn startup_prune_removes_old_runtimes_and_adapters() {
    let env = env();
    let (connections, node) = prepared(&env, AgentKind::Claude);
    connections
        .adapters()
        .update(
            &claude_registry("0.82.0"),
            AgentKind::Claude,
            Some(&node),
            &mut |_| {},
            &CancelToken::new(),
        )
        .expect("update");
    // A stale runtime and a crashed staging directory of another process.
    let stale_runtime = env.paths.runtime_dir().join("node-v22.1.0");
    std::fs::create_dir_all(stale_runtime.join("bin")).expect("mkdir");
    let crashed = env.paths.agents_dir().join("claude-acp/.0.9.0.tmp-1");
    std::fs::create_dir_all(&crashed).expect("mkdir");
    // With a live connection, only adapters not in use go (runtimes stay).
    let report = connections
        .prune_unused(&[("claude-acp", "0.81.2")])
        .expect("prune");
    assert!(report.runtimes.is_empty(), "{report:?}");
    assert_eq!(
        report.adapters,
        vec![("claude-acp".to_string(), ".0.9.0.tmp-1".to_string())]
    );
    assert!(stale_runtime.is_dir());
    let report = connections.prune_unused(&[]).expect("prune");
    assert_eq!(report.runtimes, vec!["node-v22.1.0".to_string()]);
    assert_eq!(
        report.adapters,
        vec![("claude-acp".to_string(), "0.81.2".to_string())]
    );
    assert!(connections.is_ready(AgentKind::Claude));
    assert_eq!(
        connections.prune_unused(&[]).expect("again"),
        Default::default()
    );
}
