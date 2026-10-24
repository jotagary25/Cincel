//! End-to-end GPUI tests of updating the private Node runtime and an npm
//! adapter from the settings tab, and of cancelling either update in
//! progress (`docs/specs/08-etapa6-cierre-1-0.md` §5.6.1).
//!
//! `settings_view.rs`'s own `set_operation_for_test` doc comment explains
//! why a real cancellation test needs more than the plain `FakeEnv`: with no
//! controllable registry or downloader, an update resolves almost instantly
//! and races the test's single-threaded executor. This file extends that
//! fixture instead of relying on the synthetic row: [`GatedNodeDownloader`]
//! and [`GateInstaller`] each block their one network/install call on an
//! `mpsc` channel the test releases explicitly, only after
//! `cx.run_until_parked()` has already shown "Actualizando…" — the same "a
//! real background thread races the test's single-threaded executor"
//! pattern `connection_cancel_tests.rs` documents for the connect flow's own
//! "Preparando…". A bounded `recv_timeout` (never a real sleep loop) turns a
//! wrong assumption about that plumbing into a clear failure instead of a
//! hung test.
//!
//! The adapter *update* (no cancel) is already covered end to end by
//! `settings_view_tests.rs`'s `a_newer_adapter_in_the_registry_is_offered_and_installed`;
//! this file adds the two cases that were still missing: Node's own update
//! completing for real, and cancelling either kind mid-flight.
//!
//! The update thread never wakes a GPUI task: it fills a channel that
//! `Agents` drains on an executor timer (`UPDATE_POLL_INTERVAL`), so a test
//! waiting for an update's result advances the executor clock
//! ([`wait_for_info`]) instead of only running until parked. The two
//! regression tests at the end hold back the update check and an update
//! until the GPUI side is already waiting, the exact interleaving that made
//! the test scheduler report "Your test is not deterministic" in the CI.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use cincel_acp::AgentRegistry;
use cincel_connections::{
    Adapters, CancelToken, Connections, Downloader, NodePaths, NodeVersion, PackageInstaller,
    RangeBody, Runtime,
};
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::UPDATE_POLL_INTERVAL;
use crate::project::ProjectOptions;
use crate::settings_view::{SettingsView, SettingsViewEvent, UpdateTarget};
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

/// How long a gate's `recv_timeout` waits before giving up: generous for a
/// slow CI runner, but short enough that a wrong assumption fails fast
/// instead of burning the whole test's time budget.
const GATE_TIMEOUT: Duration = Duration::from_secs(5);

const BASE_URL: &str = "mem://node/";
const NODE_VERSION: &str = "v24.9.0";
/// Matches `FakeEnv`'s own fake runtime (`test_support.rs`'s
/// `FAKE_NODE_VERSION`), so `Runtime::installed()` sees it as already there.
const INSTALLED_NODE_VERSION: &str = "v24.1.0";

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// `watch_files: true` is what makes `Workspace::new` build `Agents` with
/// `background: true` (`agents_background = options.project_options.
/// watch_files`, `workspace.rs`): without it every update would run
/// synchronously on the calling thread (fine for the already-covered "happy
/// path" tests, but it would make a gate's `recv` block the only thread the
/// test has, since nothing would ever call the matching `send`). Everything
/// else about the project stays inert; there is no project directory at all
/// in these tests.
fn workspace_window(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let options = WorkspaceOptions {
        project_options: ProjectOptions {
            watch_files: true,
            ..ProjectOptions::inert()
        },
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

/// A registry with nothing newer than what `FakeEnv` already installed, so
/// only the Node runtime (never the adapter) offers an update, unless a test
/// asks for the opposite with [`registry_with_claude`].
fn stable_registry() -> AgentRegistry {
    AgentRegistry::parse(
        r#"{"version":"1.0.0","agents":[{"id":"claude-acp","name":"Claude","version":"0.0.1",
           "distribution":{"npx":{"package":"@agentclientprotocol/claude-agent-acp@0.0.1"}}}]}"#,
    )
    .expect("registry")
}

/// A registry publishing Claude's adapter at `version` (for the adapter
/// cancel test).
fn registry_with_claude(version: &str) -> AgentRegistry {
    AgentRegistry::parse(&format!(
        r#"{{"version":"1.0.0","agents":[{{"id":"claude-acp","name":"Claude","version":"{version}",
           "distribution":{{"npx":{{"package":"@agentclientprotocol/claude-agent-acp@{version}"}}}}}}]}}"#
    ))
    .expect("registry")
}

/// Opens the settings tab on Connections, with `engine` behind `Agents` and
/// `registry` as the (already resolved, no network) agent catalog.
fn connections_window(
    engine: std::sync::Arc<Connections>,
    registry: AgentRegistry,
    cx: &mut TestAppContext,
) -> (
    Entity<Workspace>,
    Entity<SettingsView>,
    &mut VisualTestContext,
) {
    let (workspace, cx) = workspace_window(cx);
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| {
        agents.set_registry_loader(std::sync::Arc::new(move |_| Ok(registry.clone())));
        agents.set_connections(engine, cx);
    });
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::actions::OpenConnections), cx));
    cx.run_until_parked();
    let view = workspace
        .read_with(cx, |workspace, cx| {
            workspace.center().read(cx).settings_view().cloned()
        })
        .expect("la pestaña de configuración está abierta");
    (workspace, view, cx)
}

// --------------------------------------------------------------- Node

/// A minimal, valid Node release: a `tar.xz` with only `bin/node`,
/// `lib/node_modules/npm/bin/{npm-cli.js,npx-cli.js}` under
/// `node-{version}-{platform}/` (`keep_runtime_entry`, `cincel-connections`'s
/// `runtime.rs`), the same shape `cincel-connections/tests/engine.rs` builds
/// for its own `MemDownloader`.
fn node_archive(version: &str, platform: &str) -> Vec<u8> {
    let top = format!("node-{version}-{platform}");
    let mut builder = tar::Builder::new(Vec::new());
    let entry = |builder: &mut tar::Builder<Vec<u8>>, path: String, body: &[u8], mode: u32| {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(mode);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder
            .append_data(&mut header, path, body)
            .expect("tar entry");
    };
    entry(
        &mut builder,
        format!("{top}/bin/node"),
        b"#!/bin/sh\nexec /bin/sh \"$@\"\n",
        0o755,
    );
    entry(
        &mut builder,
        format!("{top}/lib/node_modules/npm/bin/npm-cli.js"),
        b"// npm\n",
        0o644,
    );
    entry(
        &mut builder,
        format!("{top}/lib/node_modules/npm/bin/npx-cli.js"),
        b"// npx\n",
        0o644,
    );
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

/// In-memory nodejs.org, always serving the same version and archive.
struct NodeIndex {
    archive_name: String,
    archive: Vec<u8>,
    version: String,
}

impl NodeIndex {
    fn new(version: &str, platform: &str) -> Self {
        Self {
            archive_name: format!("node-{version}-{platform}.tar.xz"),
            archive: node_archive(version, platform),
            version: version.to_string(),
        }
    }

    fn index_json(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            { "version": self.version, "lts": "Test", "files": [platform_key()] },
            { "version": INSTALLED_NODE_VERSION, "lts": "Test", "files": [platform_key()] },
        ]))
        .expect("json")
    }

    fn shasums(&self) -> Vec<u8> {
        format!("{}  {}\n", sha256_hex(&self.archive), self.archive_name).into_bytes()
    }
}

fn platform_key() -> String {
    cincel_connections::node_platform().expect("plataforma con binarios de Node")
}

/// [`Downloader`] that always serves the same in-memory release, with no
/// gate: for the test that lets the update actually finish.
struct NodeDownloader(NodeIndex);

impl Downloader for NodeDownloader {
    fn get_bytes(&self, url: &str) -> cincel_connections::Result<Vec<u8>> {
        if url == format!("{BASE_URL}index.json") {
            Ok(self.0.index_json())
        } else if url == format!("{BASE_URL}{}/SHASUMS256.txt", self.0.version) {
            Ok(self.0.shasums())
        } else {
            Err(cincel_connections::ConnectionsError::Network(format!(
                "404 {url}"
            )))
        }
    }

    fn open(&self, url: &str, offset: u64) -> cincel_connections::Result<RangeBody> {
        let expected = format!("{BASE_URL}{}/{}", self.0.version, self.0.archive_name);
        if url != expected {
            return Err(cincel_connections::ConnectionsError::Network(format!(
                "404 {url}"
            )));
        }
        let total = self.0.archive.len() as u64;
        let slice = self.0.archive[offset as usize..].to_vec();
        Ok(RangeBody {
            resumed: offset > 0,
            total: Some(total),
            reader: Box::new(std::io::Cursor::new(slice)),
        })
    }
}

/// Same `index.json`/`SHASUMS256.txt` as [`NodeDownloader`], but `open` (the
/// archive body) blocks on a rendezvous channel until the test releases it —
/// deterministically, with no sleep — so the update is provably still
/// running when "Cancelar" is pressed.
struct GatedNodeDownloader {
    index: NodeIndex,
    /// `Mutex` only to make the type `Sync` (`Downloader` requires it); a
    /// single `open` call ever happens at a time in these tests.
    gate: std::sync::Mutex<mpsc::Receiver<()>>,
}

impl Downloader for GatedNodeDownloader {
    fn get_bytes(&self, url: &str) -> cincel_connections::Result<Vec<u8>> {
        if url == format!("{BASE_URL}index.json") {
            Ok(self.index.index_json())
        } else if url == format!("{BASE_URL}{}/SHASUMS256.txt", self.index.version) {
            Ok(self.index.shasums())
        } else {
            Err(cincel_connections::ConnectionsError::Network(format!(
                "404 {url}"
            )))
        }
    }

    fn open(&self, _url: &str, _offset: u64) -> cincel_connections::Result<RangeBody> {
        self.gate
            .lock()
            .expect("gate lock")
            .recv_timeout(GATE_TIMEOUT)
            .expect("la prueba tenía que liberar la descarga antes del tiempo límite");
        // Released after "Cancelar" already fired: report it, exactly as a
        // real interrupted read would (`cincel-connections`'s own
        // `CancelReader`).
        Err(cincel_connections::ConnectionsError::Cancelled)
    }
}

fn node_engine(env: &FakeEnv, downloader: Box<dyn Downloader>) -> std::sync::Arc<Connections> {
    let runtime = Runtime::new(env.paths.clone())
        .with_downloader(downloader)
        .with_base_url(BASE_URL)
        .with_version(NodeVersion::Lts)
        .with_retry(1, Duration::ZERO);
    std::sync::Arc::new(Connections::new(env.paths.clone()).with_runtime(runtime))
}

/// §5.6.1: "Actualizar a vX" on the private Node completes for real —
/// downloaded, verified, extracted, `current` moved — through the exact
/// settings-tab event the button emits.
#[gpui::test]
fn updating_node_from_the_settings_tab_completes(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    let downloader = NodeDownloader(NodeIndex::new(NODE_VERSION, &platform_key()));
    let engine = node_engine(&env, Box::new(downloader));
    let (_workspace, view, cx) = connections_window(engine.clone(), stable_registry(), cx);

    // The update check runs on the background executor, deterministic here;
    // `wait_for_info` is the same helper the install below needs.
    let info = wait_for_info(&view, cx, |info| info.node.update.is_some());
    assert_eq!(info.node.installed.as_deref(), Some(INSTALLED_NODE_VERSION));
    assert_eq!(
        info.node.update.as_deref(),
        Some(NODE_VERSION),
        "«Actualizar a {NODE_VERSION}» tiene que ofrecerse"
    );

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::UpdateNode, cx)
    });
    cx.run_until_parked();

    // The install runs on a real thread too: wait for the view to report it
    // done before looking at the runtime, or a slow machine fails here.
    let info = wait_for_info(&view, cx, |info| {
        info.operation.is_none() && info.node.installed.as_deref() == Some(NODE_VERSION)
    });
    assert_eq!(
        engine
            .runtime()
            .installed()
            .map(|installed| installed.version),
        Some(NODE_VERSION.to_string()),
        "el runtime instalado tiene que apuntar a la versión nueva"
    );
    assert_eq!(info.node.installed.as_deref(), Some(NODE_VERSION));
    assert_eq!(info.node.update, None, "ya no queda nada para ofrecer");
    assert!(info.operation.is_none());
}

/// Polls the connections info until `done` holds, or for ten seconds: the
/// installs run on real threads here, and the CI's shared runners can take a
/// while to schedule them. Each round advances the executor clock by
/// `UPDATE_POLL_INTERVAL`, which is what lets `Agents` drain the update
/// thread's channel (and runs everything else until parked).
fn wait_for_info(
    view: &Entity<SettingsView>,
    cx: &mut VisualTestContext,
    done: impl Fn(&crate::settings_view::ConnectionsInfo) -> bool,
) -> crate::settings_view::ConnectionsInfo {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.executor().advance_clock(UPDATE_POLL_INTERVAL);
        let info = view.read_with(cx, |view, _| view.connections_info().clone());
        if done(&info) || std::time::Instant::now() >= deadline {
            return info;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// §5.6.1: cancelling a Node update mid-download returns to "Actualizar a
/// vX" at once, and never leaves a `.part` file or a staging directory.
#[gpui::test]
fn cancelling_a_node_update_leaves_no_leftovers(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    let (tx, rx) = mpsc::channel();
    let downloader = GatedNodeDownloader {
        index: NodeIndex::new(NODE_VERSION, &platform_key()),
        gate: std::sync::Mutex::new(rx),
    };
    let engine = node_engine(&env, Box::new(downloader));
    let (_workspace, view, cx) = connections_window(engine.clone(), stable_registry(), cx);

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::UpdateNode, cx)
    });
    cx.run_until_parked();

    let info = view.read_with(cx, |view, _| view.connections_info().clone());
    assert_eq!(
        info.operation.as_ref().map(|operation| operation.target),
        Some(UpdateTarget::Node),
        "«Actualizando…» tiene que estar en curso mientras la descarga está frenada"
    );

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::CancelUpdate, cx)
    });
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |view, _| view.connections_info().operation.is_none()),
        "«Cancelar» vuelve a «Actualizar a vX» sin esperar al hilo"
    );

    // Let the gated thread notice the cancellation and unwind; its late
    // result must never resurrect as an error.
    tx.send(()).ok();
    cx.run_until_parked();

    assert_eq!(
        engine
            .runtime()
            .installed()
            .map(|installed| installed.version),
        Some(INSTALLED_NODE_VERSION.to_string()),
        "la versión instalada no cambió"
    );
    assert!(
        std::fs::read_dir(env.paths.downloads_dir())
            .into_iter()
            .flatten()
            .flatten()
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".part")),
        "sin .part"
    );
    let staging: Vec<_> = std::fs::read_dir(env.paths.runtime_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(staging.is_empty(), "sin carpeta de staging: {staging:?}");
}

// ------------------------------------------------------------ adapter

/// [`PackageInstaller`] whose `install` blocks on a rendezvous channel until
/// the test releases it, then reports it as cancelled — the adapter
/// equivalent of [`GatedNodeDownloader`].
struct GateInstaller {
    /// `Mutex` only to make the type `Sync` (`PackageInstaller` requires
    /// it); a single `install` call ever happens at a time in these tests.
    gate: std::sync::Mutex<mpsc::Receiver<()>>,
}

impl PackageInstaller for GateInstaller {
    fn install(
        &self,
        _node: &NodePaths,
        _package_spec: &str,
        _prefix: &Path,
        _cache: &Path,
        _npmrc: &Path,
        _cancel: &CancelToken,
    ) -> cincel_connections::Result<()> {
        self.gate
            .lock()
            .expect("gate lock")
            .recv_timeout(GATE_TIMEOUT)
            .expect("la prueba tenía que liberar la instalación antes del tiempo límite");
        Err(cincel_connections::ConnectionsError::Cancelled)
    }
}

/// §5.6.1 ("Lo mismo para el adaptador"): cancelling an adapter update
/// mid-install returns to "Actualizar a vX" at once and installs nothing.
#[gpui::test]
fn cancelling_an_adapter_update_leaves_the_old_version_installed(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    env.add_connection("claude-acp", "Claude · personal");
    let (tx, rx) = mpsc::channel();
    let adapters = Adapters::new(env.paths.clone()).with_installer(Box::new(GateInstaller {
        gate: std::sync::Mutex::new(rx),
    }));
    let engine = std::sync::Arc::new(
        Connections::new(env.paths.clone())
            .with_runtime(
                Runtime::new(env.paths.clone()).with_version(NodeVersion::parse("24.1.0")),
            )
            .with_adapters(adapters),
    );
    let (_workspace, view, cx) =
        connections_window(engine.clone(), registry_with_claude("0.0.2"), cx);

    view.update(cx, |view, cx| {
        view.request(
            SettingsViewEvent::UpdateAdapter(cincel_connections::AgentKind::Claude),
            cx,
        )
    });
    cx.run_until_parked();

    let info = view.read_with(cx, |view, _| view.connections_info().clone());
    assert_eq!(
        info.operation.as_ref().map(|operation| operation.target),
        Some(UpdateTarget::Adapter(cincel_connections::AgentKind::Claude)),
        "«Actualizando…» tiene que estar en curso mientras la instalación está frenada"
    );

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::CancelUpdate, cx)
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.connections_info().operation.is_none()));

    tx.send(()).ok();
    cx.run_until_parked();

    assert_eq!(
        engine.adapters().installed("claude-acp").as_deref(),
        Some("0.0.1"),
        "la versión instalada no cambió"
    );
}

// ------------------------------------------------- determinism (CI 0.2.3)

/// How long [`SlowRegistryLoader`] holds the update check back: far longer
/// than the GPUI side needs to start waiting for it.
const SLOW_CHECK: Duration = Duration::from_millis(200);

/// Regression (CI of the 0.2.3 release): the registry answers only after the
/// GPUI side is already waiting for the check. When the check ran on its own
/// thread, its answer woke the waiting GPUI task from that thread and the
/// test scheduler failed the test as non-deterministic; on the background
/// executor the check runs on the scheduler's own thread.
#[gpui::test]
fn a_slow_update_check_reaches_the_tab_without_a_foreign_wake(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    let downloader = NodeDownloader(NodeIndex::new(NODE_VERSION, &platform_key()));
    let engine = node_engine(&env, Box::new(downloader));
    let (workspace, cx) = workspace_window(cx);
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    let registry = stable_registry();
    agents.update(cx, |agents, cx| {
        agents.set_registry_loader(std::sync::Arc::new(move |_| {
            std::thread::sleep(SLOW_CHECK);
            Ok(registry.clone())
        }));
        agents.set_connections(engine, cx);
    });
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::actions::OpenConnections), cx));
    cx.run_until_parked();
    let view = workspace
        .read_with(cx, |workspace, cx| {
            workspace.center().read(cx).settings_view().cloned()
        })
        .expect("la pestaña de configuración está abierta");

    let info = wait_for_info(&view, cx, |info| {
        info.catalog == crate::settings_view::CatalogState::Checked
    });
    assert_eq!(info.catalog, crate::settings_view::CatalogState::Checked);
    assert_eq!(info.node.update.as_deref(), Some(NODE_VERSION));
}

/// [`NodeDownloader`] whose archive body waits for the test to release it,
/// then serves the release normally: the update finishes for real, but only
/// once the GPUI side is provably waiting for it.
struct HeldNodeDownloader {
    inner: NodeDownloader,
    /// `Mutex` only to make the type `Sync` (`Downloader` requires it).
    gate: std::sync::Mutex<mpsc::Receiver<()>>,
}

impl Downloader for HeldNodeDownloader {
    fn get_bytes(&self, url: &str) -> cincel_connections::Result<Vec<u8>> {
        self.inner.get_bytes(url)
    }

    fn open(&self, url: &str, offset: u64) -> cincel_connections::Result<RangeBody> {
        self.gate
            .lock()
            .expect("gate lock")
            .recv_timeout(GATE_TIMEOUT)
            .expect("la prueba tenía que liberar la descarga antes del tiempo límite");
        self.inner.open(url, offset)
    }
}

/// Regression (CI of the 0.2.3 release): the update thread reports its
/// progress and its end only after the GPUI side is waiting for them. When
/// the GPUI task awaited the channel, every message woke it from the update
/// thread and the test scheduler failed the test as non-deterministic; the
/// task now drains the channel on an executor timer.
#[gpui::test]
fn a_held_node_update_finishes_without_a_foreign_wake(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    let (tx, rx) = mpsc::channel();
    let downloader = HeldNodeDownloader {
        inner: NodeDownloader(NodeIndex::new(NODE_VERSION, &platform_key())),
        gate: std::sync::Mutex::new(rx),
    };
    let engine = node_engine(&env, Box::new(downloader));
    let (_workspace, view, cx) = connections_window(engine.clone(), stable_registry(), cx);
    let info = wait_for_info(&view, cx, |info| info.node.update.is_some());
    assert_eq!(info.node.update.as_deref(), Some(NODE_VERSION));

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::UpdateNode, cx)
    });
    // The GPUI side is now waiting; only then does the download go on.
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |view, _| view.connections_info().operation.is_some()),
        "«Actualizando…» tiene que estar en curso mientras la descarga está frenada"
    );
    tx.send(())
        .expect("el hilo de la actualización espera la descarga");

    let info = wait_for_info(&view, cx, |info| {
        info.operation.is_none() && info.node.installed.as_deref() == Some(NODE_VERSION)
    });
    assert!(info.operation.is_none(), "la actualización terminó");
    assert_eq!(info.node.installed.as_deref(), Some(NODE_VERSION));
    assert_eq!(
        engine
            .runtime()
            .installed()
            .map(|installed| installed.version),
        Some(NODE_VERSION.to_string())
    );
}
