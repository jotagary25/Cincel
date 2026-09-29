//! Adapter installation (`docs/specs/06-etapa4-conexiones-y-cincel.md` §3),
//! at the ACP registry's version, into `<data>/agents/<agent_id>/<version>/`:
//!
//! * `npx` distributions (Claude, Codex): the official npm packages,
//!   installed with the private Node/npm, with npm's cache under
//!   `<cache>/npm` (never the system cache) and no user or global `.npmrc`;
//! * `binary` distributions (Antigravity): the registry's prebuilt archive
//!   for this platform (`.zip` or `.tar.gz`), downloaded with progress and
//!   resume into `<cache>/downloads`, checked against its SHA-256 when the
//!   registry publishes one (Google publishes none: the install is then
//!   marked `verifiable: false` so the UI can say so), after a disk-space
//!   check, and unpacked with its `cmd` made executable.
//!
//! Either way the install lands in a staging directory that is renamed into
//! place, so a half-finished install never looks installed.
//!
//! Etapa 5 (`docs/specs/07-etapa5-productividad.md` §10.3, §10.4): every
//! install takes a [`CancelToken`] (downloads stop within one 64 KiB read,
//! unpacking within one read of one entry, `npm install` has its process
//! group killed; the `.part` and the staging directory are removed), and
//! [`Adapters::update`] installs a new version next to the one in use
//! (D13), which [`Adapters::prune_unused`] removes later.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use cincel_acp::{AgentRegistry, LaunchSpec, ProcessEnv};
use serde::{Deserialize, Serialize};

use crate::cancel::{CancelReader, CancelToken, WorkLock, or_cancelled};
use crate::envutil;
use crate::error::{ConnectionsError, Result, io_err};
use crate::paths::{CincelPaths, create_private_dir, write_atomic};
use crate::profile::{AdapterSource, AgentKind};
use crate::runtime::{
    Downloader, HttpDownloader, NodePaths, download_part, foreign_staging, sha256_file,
};

/// How an installed adapter is run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallKind {
    /// `node <bin> <args>` with the private runtime.
    #[default]
    Npm,
    /// `<install dir>/<bin> <args>`, no Node involved.
    Binary,
}

fn yes() -> bool {
    true
}

/// Marker written inside a finished install: what was installed and how to
/// launch it, so launching needs neither the registry nor the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInstall {
    /// Registry id.
    pub agent_id: String,
    /// npm package or binary archive. Older markers (npm only) omit it.
    #[serde(default)]
    pub distribution: InstallKind,
    /// npm package name, or the archive's file name for binaries.
    pub package: String,
    /// Installed version.
    pub version: String,
    /// The executable, relative to the install dir (the package's script, or
    /// the registry's `cmd` without its leading `./`).
    pub bin: PathBuf,
    /// Extra arguments the registry asks for (Antigravity: `--uid=`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables the registry asks for (binary targets).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Whether the download was checked against a published SHA-256.
    /// `false` for Antigravity: the registry publishes `sha256: null`.
    #[serde(default = "yes")]
    pub verifiable: bool,
}

const MARKER: &str = ".cincel-adapter.json";

/// Free space a binary install of `kind` needs while it runs (archive plus
/// unpacked files). Antigravity 1.2.1: a 333 MB zip that unpacks to 1.05 GB
/// (`agy_acp_server.par` 920 MB + `localharness_external` 133 MB), and the
/// zip is only deleted after unpacking.
#[must_use]
pub fn install_space_needed(kind: AgentKind) -> u64 {
    match kind {
        AgentKind::Antigravity => 1_400_000_000,
        AgentKind::Claude | AgentKind::Codex => 0,
    }
}

/// Download size shown before an install starts ("se descargará (333 MB)"),
/// when it is known up front: Antigravity 1.2.1's zip is 333 MB (the
/// registry does not publish sizes, so this tracks the pinned version).
/// `None` for npm adapters, whose size depends on what npm resolves.
#[must_use]
pub fn download_size_hint(kind: AgentKind) -> Option<u64> {
    match kind {
        AgentKind::Antigravity => Some(333_000_000),
        AgentKind::Claude | AgentKind::Codex => None,
    }
}

/// What an install step reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterProgress {
    /// `npm install` started.
    Installing {
        /// Package spec (`name@version`).
        package: String,
    },
    /// Downloading a binary archive.
    Downloading {
        /// Version being downloaded.
        version: String,
        /// Bytes on disk so far (includes a resumed prefix).
        done: u64,
        /// Total size, when the server sends a length.
        total: Option<u64>,
        /// Whether a SHA-256 will be checked afterwards (`false`: "Google no
        /// publica una suma de verificación para este paquete").
        verifiable: bool,
    },
    /// A download attempt failed and will be retried (resuming).
    Retrying {
        /// 1-based number of the attempt that is about to start.
        attempt: u32,
        /// Why the previous attempt failed.
        reason: String,
    },
    /// Checking the archive's SHA-256.
    Verifying,
    /// Unpacking the archive.
    Extracting,
    /// Finished.
    Done {
        /// Installed version.
        version: String,
    },
}
/// Runs the package manager. The default is [`NpmInstaller`]; tests inject a
/// fake.
pub trait PackageInstaller: Send + Sync {
    /// Install `package_spec` (`name@version`) under `prefix` (so it lands in
    /// `<prefix>/node_modules/<name>`), using `cache` as npm's cache. Must
    /// stop promptly (killing whatever it started) once `cancel` is
    /// cancelled; the caller removes `prefix`.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::InstallFailed`] with the tail of npm's output,
    /// [`ConnectionsError::Cancelled`].
    fn install(
        &self,
        node: &NodePaths,
        package_spec: &str,
        prefix: &Path,
        cache: &Path,
        npmrc: &Path,
        cancel: &CancelToken,
    ) -> Result<()>;
}

/// How often [`NpmInstaller`] checks its cancel token.
const NPM_POLL: Duration = Duration::from_millis(100);

/// `node <npm-cli.js> install --prefix <dir> --cache <cincel cache> ...`
/// with the private runtime first in `PATH`, `npm_config_*` from the
/// user's shell stripped and an empty `userconfig`/`globalconfig`. npm runs
/// as the leader of its own process group; while it runs the token is
/// checked every 100 ms and, once cancelled, the whole group (npm and its
/// lifecycle scripts) is killed and reaped before returning.
#[derive(Debug, Clone, Copy, Default)]
pub struct NpmInstaller;

impl PackageInstaller for NpmInstaller {
    fn install(
        &self,
        node: &NodePaths,
        package_spec: &str,
        prefix: &Path,
        cache: &Path,
        npmrc: &Path,
        cancel: &CancelToken,
    ) -> Result<()> {
        let env = ProcessEnv::new()
            .with_unset("npm_config_*")
            .with_unset("NPM_CONFIG_*")
            .with_unset("NODE_OPTIONS")
            // npm refuses to load the same file as both "user" and "global"
            // config ("double-loading config"), so each level gets its own
            // empty file next to `npmrc`.
            .with_set(
                "npm_config_userconfig",
                npmrc.to_string_lossy().into_owned(),
            )
            .with_set(
                "npm_config_globalconfig",
                npmrc
                    .with_extension("global")
                    .to_string_lossy()
                    .into_owned(),
            )
            .with_set("npm_config_update_notifier", "false")
            .with_set("npm_config_fund", "false")
            .with_set("npm_config_audit", "false")
            .with_path_prepend(&node.bin_dir);
        let mut command = std::process::Command::new(&node.node);
        command
            .arg(&node.npm)
            .arg("install")
            .arg("--prefix")
            .arg(prefix)
            .arg("--cache")
            .arg(cache)
            .arg("--no-save")
            .arg("--loglevel=error")
            .arg(package_spec)
            .current_dir(prefix)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        envutil::apply_std(&mut command, &env, &BTreeMap::new());
        cancel.check()?;
        let failed = |message: String| ConnectionsError::InstallFailed {
            package: package_spec.to_string(),
            message,
        };
        let mut child = command.spawn().map_err(|error| failed(error.to_string()))?;
        // Drain both pipes so npm never blocks on a full one.
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut bytes);
                }
                bytes
            })
        };
        let stdout = drain(
            child
                .stdout
                .take()
                .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        );
        let stderr = drain(
            child
                .stderr
                .take()
                .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        );
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => return Err(failed(error.to_string())),
            }
            if cancel.is_cancelled() {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                // The reader threads end with the pipes; not joined, so a
                // helper that escaped the group cannot stall the cancel.
                return Err(ConnectionsError::Cancelled);
            }
            std::thread::sleep(NPM_POLL);
        };
        if status.success() {
            return Ok(());
        }
        let mut text = String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned();
        text.push_str(&String::from_utf8_lossy(&stdout.join().unwrap_or_default()));
        let tail: String = text
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        Err(ConnectionsError::InstallFailed {
            package: package_spec.to_string(),
            message: tail,
        })
    }
}

/// SIGKILL the process group led by `pid` (npm and everything it spawned).
fn kill_group(pid: u32) {
    #[cfg(unix)]
    if let Some(pid) = rustix::process::Pid::from_raw(pid.cast_signed()) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Free bytes on the filesystem holding `path` (its closest existing
/// ancestor), `None` when unknown.
#[must_use]
pub fn free_space(path: &Path) -> Option<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    #[cfg(unix)]
    {
        let stat = rustix::fs::statvfs(probe).ok()?;
        Some(stat.f_bavail.saturating_mul(stat.f_frsize))
    }
    #[cfg(not(unix))]
    {
        let _ = probe;
        None
    }
}

type FreeSpaceFn = Box<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

/// Installed adapters manager.
pub struct Adapters {
    paths: CincelPaths,
    installer: Box<dyn PackageInstaller>,
    downloader: Box<dyn Downloader>,
    platform: String,
    attempts: u32,
    retry_delay: Duration,
    free_space: FreeSpaceFn,
    space_override: Option<u64>,
}

impl std::fmt::Debug for Adapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adapters")
            .field("paths", &self.paths)
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

/// A `binary` distribution as the registry publishes it for one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPlan {
    /// Registry version.
    pub version: String,
    /// Registry platform key (`linux-x86_64`).
    pub platform: String,
    /// Archive URL (`.zip`, `.tar.gz` or `.tgz`).
    pub archive: String,
    /// Command inside the archive (`./agy_acp_server.par`).
    pub cmd: String,
    /// Arguments (`--uid=`).
    pub args: Vec<String>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
    /// Published SHA-256 of the archive, if any.
    pub sha256: Option<String>,
}

impl BinaryPlan {
    /// Whether the download can be checked (the registry published a
    /// SHA-256). `false` for Antigravity 1.2.1.
    #[must_use]
    pub fn verifiable(&self) -> bool {
        self.sha256.is_some()
    }

    /// The archive's file name (last URL segment, no query).
    #[must_use]
    pub fn archive_name(&self) -> String {
        self.archive
            .split(['?', '#'])
            .next()
            .and_then(|url| url.rsplit('/').next())
            .filter(|name| !name.is_empty())
            .unwrap_or("archivo")
            .to_string()
    }
}

/// The `binary` distribution of `kind` for `platform`.
///
/// # Errors
///
/// [`ConnectionsError::NotInRegistry`] (no entry, or no `binary` channel),
/// [`ConnectionsError::NoBinaryForPlatform`].
pub fn binary_plan(
    registry: &AgentRegistry,
    kind: AgentKind,
    platform: &str,
) -> Result<BinaryPlan> {
    let descriptor = registry
        .get(kind.agent_id())
        .ok_or_else(|| ConnectionsError::NotInRegistry(kind.agent_id().to_string()))?;
    let targets = descriptor
        .distribution
        .binary
        .as_ref()
        .ok_or_else(|| ConnectionsError::NotInRegistry(kind.agent_id().to_string()))?;
    let target = targets
        .get(platform)
        .ok_or_else(|| ConnectionsError::NoBinaryForPlatform {
            agent: kind.full_name().to_string(),
            platform: platform.to_string(),
        })?;
    Ok(BinaryPlan {
        version: descriptor.version.clone(),
        platform: platform.to_string(),
        archive: target.archive.clone(),
        cmd: target.cmd.clone(),
        args: target.args.clone(),
        env: target.env.clone(),
        sha256: target
            .sha256
            .as_ref()
            .map(|digest| digest.trim().to_ascii_lowercase())
            .filter(|digest| !digest.is_empty()),
    })
}

/// The version the registry publishes for `kind` and its extra arguments,
/// checking that it comes through the expected channel (the expected `npx`
/// package, or a `binary` archive for this machine).
///
/// # Errors
///
/// [`ConnectionsError::NotInRegistry`], [`ConnectionsError::NoBinaryForPlatform`].
pub fn registry_version(
    registry: &AgentRegistry,
    kind: AgentKind,
) -> Result<(String, Vec<String>)> {
    let AdapterSource::Npm { package, .. } = kind.source() else {
        let plan = binary_plan(
            registry,
            kind,
            &cincel_acp::registry::current_platform_key(),
        )?;
        return Ok((plan.version, plan.args));
    };
    let descriptor = registry
        .get(kind.agent_id())
        .ok_or_else(|| ConnectionsError::NotInRegistry(kind.agent_id().to_string()))?;
    let npx = descriptor
        .distribution
        .npx
        .as_ref()
        .ok_or_else(|| ConnectionsError::NotInRegistry(kind.agent_id().to_string()))?;
    let name = package_name(&npx.package);
    if name != package {
        return Err(ConnectionsError::NotInRegistry(format!(
            "{} (publica {name})",
            kind.agent_id()
        )));
    }
    let version = match npx.package.rsplit_once('@') {
        Some((head, version)) if !head.is_empty() => version.to_string(),
        _ => descriptor.version.clone(),
    };
    Ok((version, npx.args.clone()))
}

/// `@scope/name@1.2.3` -> `@scope/name`.
fn package_name(spec: &str) -> &str {
    let (scope, rest) = match spec.strip_prefix('@') {
        Some(rest) => ("@", rest),
        None => ("", spec),
    };
    match rest.find('@') {
        Some(at) => &spec[..scope.len() + at],
        None => spec,
    }
}

/// `./agy_acp_server.par` -> `agy_acp_server.par`, refusing anything that
/// would leave the install directory.
fn relative_cmd(cmd: &str) -> Option<PathBuf> {
    let path = PathBuf::from(cmd.trim_start_matches("./"));
    let normal = !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    normal.then_some(path)
}

fn valid_version(version: &str) -> bool {
    !version.is_empty() && !version.contains('/') && !version.contains("..")
}

/// `x.y.z[-prerelease][+build]` split into its numeric core (missing
/// components read as `0`) and its prerelease identifiers, if any. Build
/// metadata (`+...`) never affects comparisons and is dropped.
fn split_version(version: &str) -> (Vec<u64>, Option<String>) {
    let version = version.trim().trim_start_matches('v');
    let version = version.split('+').next().unwrap_or(version);
    let (core, prerelease) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre.to_string())),
        None => (version, None),
    };
    let parts = core
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect();
    (parts, prerelease)
}

/// Compares two prerelease suffixes identifier by identifier: a numeric
/// identifier compares numerically, anything else lexically, and a longer
/// list outranks a prefix of itself (semver's own precedence rule).
fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        return match (left.next(), right.next()) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(x), Some(y)) => match (x.parse::<u64>(), y.parse::<u64>()) {
                (Ok(x), Ok(y)) => match x.cmp(&y) {
                    Ordering::Equal => continue,
                    order => order,
                },
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => match x.cmp(y) {
                    Ordering::Equal => continue,
                    order => order,
                },
            },
        };
    }
}

/// Numeric semver comparison of two versions (`x.y.z`, with a `-prerelease`
/// suffix always sorting below its plain release: `1.0.0-beta` < `1.0.0`).
/// Used by [`Adapters::update_available`] so "Actualizar a…" never offers a
/// version that is not actually newer than what is installed, whichever of
/// the two version sources (the catalog Cincel ships, or what "Buscar
/// actualizaciones" fetched) it comes from.
fn compare_versions(a: &str, b: &str) -> Ordering {
    let (core_a, pre_a) = split_version(a);
    let (core_b, pre_b) = split_version(b);
    let len = core_a.len().max(core_b.len());
    for index in 0..len {
        let x = core_a.get(index).copied().unwrap_or(0);
        let y = core_b.get(index).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => {}
            order => return order,
        }
    }
    match (pre_a, pre_b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => compare_prerelease(&a, &b),
    }
}

/// Whether `candidate` is strictly newer than `installed`, per
/// [`compare_versions`].
fn is_newer_version(candidate: &str, installed: &str) -> bool {
    compare_versions(candidate, installed) == Ordering::Greater
}

impl Adapters {
    /// Manager under `paths`, installing with npm and downloading over HTTPS.
    #[must_use]
    pub fn new(paths: CincelPaths) -> Self {
        Self {
            paths,
            installer: Box::new(NpmInstaller),
            downloader: Box::new(HttpDownloader::default()),
            platform: cincel_acp::registry::current_platform_key(),
            attempts: 3,
            retry_delay: Duration::from_secs(2),
            free_space: Box::new(free_space),
            space_override: None,
        }
    }

    /// Replace the package installer (tests).
    #[must_use]
    pub fn with_installer(mut self, installer: Box<dyn PackageInstaller>) -> Self {
        self.installer = installer;
        self
    }

    /// Replace the network layer of binary downloads (tests).
    #[must_use]
    pub fn with_downloader(mut self, downloader: Box<dyn Downloader>) -> Self {
        self.downloader = downloader;
        self
    }

    /// Force the registry platform key (`linux-x86_64`...), tests only.
    #[must_use]
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = platform.into();
        self
    }

    /// Download attempts and pause between them.
    #[must_use]
    pub fn with_retry(mut self, attempts: u32, delay: Duration) -> Self {
        self.attempts = attempts.max(1);
        self.retry_delay = delay;
        self
    }

    /// Replace the free-space probe (tests).
    #[must_use]
    pub fn with_free_space(
        mut self,
        probe: impl Fn(&Path) -> Option<u64> + Send + Sync + 'static,
    ) -> Self {
        self.free_space = Box::new(probe);
        self
    }

    /// Override [`install_space_needed`] (tests).
    #[must_use]
    pub fn with_space_needed(mut self, bytes: u64) -> Self {
        self.space_override = Some(bytes);
        self
    }

    /// `<data>/agents/<agent_id>/<version>`.
    #[must_use]
    pub fn install_dir(&self, agent_id: &str, version: &str) -> PathBuf {
        self.paths.agents_dir().join(agent_id).join(version)
    }

    /// In-process lock key of one agent's install directory ([`WorkLock`]).
    fn agent_lock(&self, agent_id: &str) -> String {
        format!("agent:{}", self.paths.agents_dir().join(agent_id).display())
    }

    fn current_file(&self, agent_id: &str) -> PathBuf {
        self.paths.agents_dir().join(agent_id).join("current")
    }

    /// The installed adapter for `agent_id`, if its install finished.
    #[must_use]
    pub fn installed_adapter(&self, agent_id: &str) -> Option<AdapterInstall> {
        let version = std::fs::read_to_string(self.current_file(agent_id)).ok()?;
        let version = version.trim();
        if !valid_version(version) {
            return None;
        }
        let dir = self.install_dir(agent_id, version);
        let marker: AdapterInstall =
            serde_json::from_slice(&std::fs::read(dir.join(MARKER)).ok()?).ok()?;
        dir.join(&marker.bin).is_file().then_some(marker)
    }

    /// Installed version for `agent_id`, if any.
    #[must_use]
    pub fn installed(&self, agent_id: &str) -> Option<String> {
        self.installed_adapter(agent_id)
            .map(|install| install.version)
    }

    /// The registry's version when it is numerically newer than the
    /// installed one (the "Actualizar" button, [`compare_versions`]). `None`
    /// when up to date, older (the catalog Cincel ships can lag behind an
    /// adapter installed through a fresher "Buscar actualizaciones" check),
    /// not installed, or the registry does not publish the agent.
    #[must_use]
    pub fn update_available(&self, registry: &AgentRegistry, kind: AgentKind) -> Option<String> {
        let installed = self.installed(kind.agent_id())?;
        let latest = match kind.source() {
            AdapterSource::Npm { .. } => registry_version(registry, kind).ok()?.0,
            AdapterSource::Binary => binary_plan(registry, kind, &self.platform).ok()?.version,
        };
        is_newer_version(&latest, &installed).then_some(latest)
    }

    /// The registry's binary distribution of `kind` for this machine (what
    /// "Preparando…" is about to download, before it starts).
    ///
    /// # Errors
    ///
    /// See [`binary_plan`].
    pub fn binary_plan(&self, registry: &AgentRegistry, kind: AgentKind) -> Result<BinaryPlan> {
        binary_plan(registry, kind, &self.platform)
    }

    /// Put `staging` in place as `<agent_id>/<version>`, point `current` at
    /// it and drop older versions and leftovers, except `keep` (the version
    /// a live connection may still be running, D13). Installs of one agent
    /// never overlap inside the process ([`WorkLock`]).
    fn commit(
        &self,
        agent_id: &str,
        version: &str,
        staging: &Path,
        keep: Option<&str>,
    ) -> Result<()> {
        let agent_root = self.paths.agents_dir().join(agent_id);
        let final_dir = self.install_dir(agent_id, version);
        if final_dir.exists() {
            std::fs::remove_dir_all(&final_dir).map_err(io_err(&final_dir))?;
        }
        std::fs::rename(staging, &final_dir).map_err(io_err(&final_dir))?;
        write_atomic(&self.current_file(agent_id), version.as_bytes())?;
        if let Ok(entries) = std::fs::read_dir(&agent_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() && name != version && Some(name.as_str()) != keep {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        Ok(())
    }

    fn staging_dir(&self, agent_id: &str, version: &str) -> Result<PathBuf> {
        let agent_root = self.paths.agents_dir().join(agent_id);
        std::fs::create_dir_all(&agent_root).map_err(io_err(&agent_root))?;
        let staging = agent_root.join(format!(".{version}.tmp-{}", std::process::id()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
        }
        std::fs::create_dir_all(&staging).map_err(io_err(&staging))?;
        Ok(staging)
    }

    /// Install the npm adapter of `kind` at `version` (with the registry's
    /// extra `args`) using the private runtime. Older versions are removed
    /// afterwards. Cancelling kills `npm install` and removes the staging
    /// directory.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::InstallFailed`] (also for a binary agent),
    /// [`ConnectionsError::Cancelled`] or I/O errors.
    pub fn install(
        &self,
        kind: AgentKind,
        version: &str,
        args: &[String],
        node: &NodePaths,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<AdapterInstall> {
        let _lock = WorkLock::acquire(&self.agent_lock(kind.agent_id()), cancel)?;
        self.install_npm(kind, version, args, node, None, progress, cancel)
    }

    #[allow(clippy::too_many_arguments)]
    fn install_npm(
        &self,
        kind: AgentKind,
        version: &str,
        args: &[String],
        node: &NodePaths,
        keep: Option<&str>,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<AdapterInstall> {
        let agent_id = kind.agent_id();
        let Some(package) = kind.npm_package() else {
            return Err(ConnectionsError::InstallFailed {
                package: agent_id.to_string(),
                message: "no se instala con npm".to_string(),
            });
        };
        if !valid_version(version) {
            return Err(ConnectionsError::InstallFailed {
                package: package.to_string(),
                message: format!("versión inválida `{version}`"),
            });
        }
        cancel.check()?;
        let cache = self.paths.npm_cache_dir();
        std::fs::create_dir_all(&cache).map_err(io_err(&cache))?;
        let npmrc = self.paths.data_dir.join("npmrc");
        for file in [npmrc.clone(), npmrc.with_extension("global")] {
            if !file.exists() {
                write_atomic(&file, b"")?;
            }
        }
        let staging = self.staging_dir(agent_id, version)?;

        let spec = format!("{package}@{version}");
        progress(AdapterProgress::Installing {
            package: spec.clone(),
        });
        let installed = self
            .installer
            .install(node, &spec, &staging, &cache, &npmrc, cancel)
            .map_err(|error| or_cancelled(cancel, error))
            .and_then(|()| cancel.check())
            .and_then(|()| package_bin(&staging, kind));
        let bin = match installed {
            Ok(bin) => bin,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        let install = AdapterInstall {
            agent_id: agent_id.to_string(),
            distribution: InstallKind::Npm,
            package: package.to_string(),
            version: version.to_string(),
            bin,
            args: args.to_vec(),
            env: BTreeMap::new(),
            verifiable: true,
        };
        if let Err(error) =
            write_atomic(&staging.join(MARKER), &serde_json::to_vec_pretty(&install)?)
                .and_then(|()| cancel.check())
                .and_then(|()| self.commit(agent_id, version, &staging, keep))
        {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        progress(AdapterProgress::Done {
            version: version.to_string(),
        });
        Ok(install)
    }

    /// Download, check and unpack the binary distribution `plan` of `kind`:
    /// disk-space check, resumable download (`Range`) with retries into
    /// `<cache>/downloads`, SHA-256 when published, unpack into a staging
    /// directory, `chmod +x` of the `cmd`, then rename into place. The
    /// archive is deleted once unpacked. Cancelling (during the download,
    /// a retry pause, the check or the unpacking) removes the `.part` and
    /// the staging directory.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::NotEnoughSpace`], [`ConnectionsError::Network`],
    /// [`ConnectionsError::ChecksumMismatch`], [`ConnectionsError::Extract`],
    /// [`ConnectionsError::InstallFailed`], [`ConnectionsError::Cancelled`],
    /// I/O errors.
    pub fn install_binary(
        &self,
        kind: AgentKind,
        plan: &BinaryPlan,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<AdapterInstall> {
        let _lock = WorkLock::acquire(&self.agent_lock(kind.agent_id()), cancel)?;
        self.install_binary_locked(kind, plan, None, progress, cancel)
    }

    fn install_binary_locked(
        &self,
        kind: AgentKind,
        plan: &BinaryPlan,
        keep: Option<&str>,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<AdapterInstall> {
        let agent_id = kind.agent_id();
        let archive_name = plan.archive_name();
        if !valid_version(&plan.version) {
            return Err(ConnectionsError::InstallFailed {
                package: archive_name,
                message: format!("versión inválida `{}`", plan.version),
            });
        }
        let Some(cmd) = relative_cmd(&plan.cmd) else {
            return Err(ConnectionsError::InstallFailed {
                package: archive_name,
                message: format!("comando inválido `{}`", plan.cmd),
            });
        };
        let format =
            ArchiveFormat::from_url(&plan.archive).ok_or_else(|| ConnectionsError::Extract {
                file: archive_name.clone(),
                message: "formato desconocido (se esperaba .zip o .tar.gz)".to_string(),
            })?;
        cancel.check()?;

        create_private_dir(&self.paths.data_dir)?;
        create_private_dir(&self.paths.cache_dir)?;
        let downloads = self.paths.downloads_dir();
        std::fs::create_dir_all(&downloads).map_err(io_err(&downloads))?;
        let agents = self.paths.agents_dir();
        std::fs::create_dir_all(&agents).map_err(io_err(&agents))?;
        let part = downloads.join(format!(
            "{agent_id}-{}-{}.{}.part",
            plan.version,
            plan.platform,
            format.extension()
        ));

        // Room for the archive (minus what a resumed download already has)
        // and its unpacked files.
        let needed = self
            .space_override
            .unwrap_or_else(|| install_space_needed(kind))
            .saturating_sub(std::fs::metadata(&part).map_or(0, |meta| meta.len()));
        self.check_space(kind, &[&downloads, &agents], needed)?;

        let verifiable = plan.verifiable();
        if let Err(error) = self.download_verified(plan, &part, verifiable, progress, cancel) {
            if matches!(error, ConnectionsError::Cancelled) {
                let _ = std::fs::remove_file(&part);
            }
            return Err(error);
        }

        progress(AdapterProgress::Extracting);
        if let ArchiveFormat::Zip = format
            && let Ok(unpacked) = zip_unpacked_size(&part)
        {
            // The exact figure, now that the archive is here.
            self.check_space(kind, &[&agents], unpacked)?;
        }
        let staging = self.staging_dir(agent_id, &plan.version)?;
        let result = cancel
            .check()
            .and_then(|()| {
                unpack(format, &part, &staging, cancel).map_err(|message| {
                    or_cancelled(
                        cancel,
                        ConnectionsError::Extract {
                            file: archive_name.clone(),
                            message,
                        },
                    )
                })
            })
            .and_then(|()| {
                let program = staging.join(&cmd);
                if !program.is_file() {
                    return Err(ConnectionsError::InstallFailed {
                        package: archive_name.clone(),
                        message: format!("el archivo no trae `{}`", plan.cmd),
                    });
                }
                mark_executable(&program)
            })
            .map(|()| AdapterInstall {
                agent_id: agent_id.to_string(),
                distribution: InstallKind::Binary,
                package: archive_name.clone(),
                version: plan.version.clone(),
                bin: cmd.clone(),
                args: plan.args.clone(),
                env: plan.env.clone(),
                verifiable,
            })
            .and_then(|install| {
                write_atomic(&staging.join(MARKER), &serde_json::to_vec_pretty(&install)?)?;
                cancel.check()?;
                self.commit(agent_id, &plan.version, &staging, keep)?;
                Ok(install)
            });
        match result {
            Ok(install) => {
                let _ = std::fs::remove_file(&part);
                progress(AdapterProgress::Done {
                    version: plan.version.clone(),
                });
                Ok(install)
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging);
                // An archive that does not unpack is useless to resume, and a
                // cancelled install leaves nothing behind.
                if matches!(
                    error,
                    ConnectionsError::Extract { .. } | ConnectionsError::Cancelled
                ) {
                    let _ = std::fs::remove_file(&part);
                }
                Err(error)
            }
        }
    }

    /// The download attempts of [`Adapters::install_binary`]: resumable
    /// download plus the SHA-256 check, retried.
    fn download_verified(
        &self,
        plan: &BinaryPlan,
        part: &Path,
        verifiable: bool,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<()> {
        let mut last_error = None;
        for attempt in 1..=self.attempts {
            cancel.check()?;
            if attempt > 1 {
                progress(AdapterProgress::Retrying {
                    attempt,
                    reason: last_error
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                });
                cancel.sleep(self.retry_delay)?;
            }
            let report = |done: u64, total: Option<u64>| AdapterProgress::Downloading {
                version: plan.version.clone(),
                done,
                total,
                verifiable,
            };
            // One report per MB is plenty for a 333 MB bar.
            match download_part(
                self.downloader.as_ref(),
                &plan.archive,
                part,
                cancel,
                1_000_000,
                &mut |done, total| progress(report(done, total)),
            ) {
                Ok(()) => {}
                Err(ConnectionsError::Cancelled) => return Err(ConnectionsError::Cancelled),
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            }
            if let Some(expected) = &plan.sha256 {
                cancel.check()?;
                progress(AdapterProgress::Verifying);
                let actual = sha256_file(part)?;
                if !actual.eq_ignore_ascii_case(expected) {
                    // A corrupt partial file must not be resumed again.
                    let _ = std::fs::remove_file(part);
                    last_error = Some(ConnectionsError::ChecksumMismatch {
                        file: plan.archive_name(),
                        expected: expected.clone(),
                        actual,
                    });
                    continue;
                }
            }
            return Ok(());
        }
        Err(last_error.unwrap_or_else(|| ConnectionsError::Network("sin intentos".to_string())))
    }

    fn check_space(&self, kind: AgentKind, dirs: &[&Path], needed: u64) -> Result<()> {
        if needed == 0 {
            return Ok(());
        }
        for dir in dirs {
            if let Some(available) = (self.free_space)(dir)
                && available < needed
            {
                return Err(ConnectionsError::NotEnoughSpace {
                    agent: kind.full_name().to_string(),
                    path: dir.to_path_buf(),
                    needed,
                    available,
                });
            }
        }
        Ok(())
    }

    /// Install the registry's version of `kind` unless one is already
    /// installed. Offline (`registry` is `None`), whatever is installed is
    /// used. `node` is only needed by npm adapters. A second call for the
    /// same agent waits (cancellably) for a running one to finish, then
    /// sees what it installed.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::AdapterMissing`] offline with nothing installed,
    /// [`ConnectionsError::RuntimeMissing`] for an npm adapter without
    /// `node`, [`ConnectionsError::NotInRegistry`],
    /// [`ConnectionsError::Cancelled`], install errors.
    pub fn ensure(
        &self,
        registry: Option<&AgentRegistry>,
        kind: AgentKind,
        node: Option<&NodePaths>,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<AdapterInstall> {
        let _lock = WorkLock::acquire(&self.agent_lock(kind.agent_id()), cancel)?;
        let installed = self.installed_adapter(kind.agent_id());
        let Some(registry) = registry else {
            return installed
                .ok_or_else(|| ConnectionsError::AdapterMissing(kind.agent_id().to_string()));
        };
        if let Some(installed) = installed {
            // Keep working with what is installed; updating is explicit
            // ("Actualizar", see `update_available`).
            return Ok(installed);
        }
        self.install_registry_version(registry, kind, node, None, progress, cancel)
    }

    /// "Actualizar a X.Y.Z": install the registry's version of `kind` (npm
    /// or binary) in staging next to the installed one and move `current`
    /// to it. The previously installed version is **kept** on disk, since a
    /// live connection may still be running it (D13); [`Adapters::prune_unused`]
    /// removes it once nothing uses it (the workspace calls it at start-up
    /// and when a connection stops). Other stale versions are removed. When
    /// the registry's version is already installed, returns it unchanged.
    /// Cancellable like [`Adapters::install`] / [`Adapters::install_binary`];
    /// a cancelled update leaves the installed version untouched.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::RuntimeMissing`] for an npm adapter without
    /// `node`, [`ConnectionsError::NotInRegistry`],
    /// [`ConnectionsError::Cancelled`], install errors.
    pub fn update(
        &self,
        registry: &AgentRegistry,
        kind: AgentKind,
        node: Option<&NodePaths>,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<AdapterInstall> {
        let _lock = WorkLock::acquire(&self.agent_lock(kind.agent_id()), cancel)?;
        let installed = self.installed_adapter(kind.agent_id());
        let latest = match kind.source() {
            AdapterSource::Npm { .. } => registry_version(registry, kind)?.0,
            AdapterSource::Binary => self.binary_plan(registry, kind)?.version,
        };
        if let Some(installed) = installed.as_ref()
            && installed.version == latest
        {
            progress(AdapterProgress::Done {
                version: latest.clone(),
            });
            return Ok(installed.clone());
        }
        let keep = installed.as_ref().map(|install| install.version.as_str());
        self.install_registry_version(registry, kind, node, keep, progress, cancel)
    }

    /// Remove every installed version that is neither `current` nor listed
    /// in `in_use` (`(agent_id, version)` pairs of live connections), plus
    /// staging directories left by crashed installs of other processes.
    /// Returns the removed `(agent_id, version)` pairs. Call it at start-up
    /// (before anything runs, with an empty `in_use`) and when a connection
    /// stops. Agents with an install in progress in this process are
    /// skipped.
    ///
    /// # Errors
    ///
    /// I/O errors removing a directory.
    pub fn prune_unused(&self, in_use: &[(&str, &str)]) -> Result<Vec<(String, String)>> {
        let mut removed = Vec::new();
        let Ok(agents) = std::fs::read_dir(self.paths.agents_dir()) else {
            return Ok(removed);
        };
        for agent in agents.flatten() {
            let agent_id = agent.file_name().to_string_lossy().into_owned();
            if !agent.path().is_dir() || agent_id.starts_with('.') {
                continue;
            }
            // Only when no install of this agent is running here.
            let Ok(_lock) = WorkLock::try_acquire(&self.agent_lock(&agent_id)) else {
                continue;
            };
            let current = self.installed(&agent_id);
            let Ok(versions) = std::fs::read_dir(agent.path()) else {
                continue;
            };
            for version in versions.flatten() {
                let name = version.file_name().to_string_lossy().into_owned();
                if !version.path().is_dir() {
                    continue;
                }
                let stale = if name.starts_with('.') {
                    foreign_staging(&name)
                } else {
                    current.as_deref() != Some(name.as_str())
                        && !in_use
                            .iter()
                            .any(|(id, used)| *id == agent_id && *used == name)
                };
                // Without a valid `current`, only leftovers go.
                if stale && (current.is_some() || name.starts_with('.')) {
                    std::fs::remove_dir_all(version.path()).map_err(io_err(version.path()))?;
                    removed.push((agent_id.clone(), name));
                }
            }
        }
        Ok(removed)
    }

    /// Install the registry's version of `kind` (lock held by the caller).
    fn install_registry_version(
        &self,
        registry: &AgentRegistry,
        kind: AgentKind,
        node: Option<&NodePaths>,
        keep: Option<&str>,
        progress: &mut dyn FnMut(AdapterProgress),
        cancel: &CancelToken,
    ) -> Result<AdapterInstall> {
        match kind.source() {
            AdapterSource::Npm { .. } => {
                let node = node.ok_or(ConnectionsError::RuntimeMissing)?;
                let (version, args) = registry_version(registry, kind)?;
                self.install_npm(kind, &version, &args, node, keep, progress, cancel)
            }
            AdapterSource::Binary => {
                let plan = self.binary_plan(registry, kind)?;
                self.install_binary_locked(kind, &plan, keep, progress, cancel)
            }
        }
    }

    /// Absolute path of the adapter's executable (script or binary).
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::AdapterMissing`].
    pub fn bin_path(&self, agent_id: &str) -> Result<PathBuf> {
        let install = self
            .installed_adapter(agent_id)
            .ok_or_else(|| ConnectionsError::AdapterMissing(agent_id.to_string()))?;
        Ok(self
            .install_dir(agent_id, &install.version)
            .join(install.bin))
    }

    /// How to run the installed adapter in ACP mode: the private `node` with
    /// the package's script (npm), or the unpacked `cmd` itself (binary,
    /// `node` ignored), plus the registry's extra arguments. The binary is
    /// run by absolute path: Antigravity finds `localharness_external` next
    /// to its own executable (`main.py` looks in `dirname(argv[0])`), so the
    /// process keeps the caller's working directory (the project). The
    /// profile environment is not part of the [`LaunchSpec`]: pass
    /// [`crate::Profile::process_env`] to
    /// [`cincel_acp::AgentConnection::start_with_env`].
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::AdapterMissing`], [`ConnectionsError::RuntimeMissing`]
    /// (npm adapter without `node`).
    pub fn launch_spec(&self, agent_id: &str, node: Option<&NodePaths>) -> Result<LaunchSpec> {
        let install = self
            .installed_adapter(agent_id)
            .ok_or_else(|| ConnectionsError::AdapterMissing(agent_id.to_string()))?;
        let program = self
            .install_dir(agent_id, &install.version)
            .join(&install.bin);
        match install.distribution {
            InstallKind::Npm => {
                let node = node.ok_or(ConnectionsError::RuntimeMissing)?;
                let mut args = vec![program.to_string_lossy().into_owned()];
                args.extend(install.args.iter().cloned());
                Ok(LaunchSpec::new(
                    node.node.to_string_lossy().into_owned(),
                    args,
                ))
            }
            InstallKind::Binary => {
                let mut launch =
                    LaunchSpec::new(program.to_string_lossy().into_owned(), install.args.clone());
                for (name, value) in install.env {
                    launch = launch.with_env(name, value);
                }
                Ok(launch)
            }
        }
    }

    /// Like [`Adapters::launch_spec`] for an npm adapter with extra arguments
    /// appended instead of the registry's ACP ones (login and status
    /// commands: `--cli auth login --claudeai`, `cli login`...).
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::AdapterMissing`].
    pub fn command_spec(
        &self,
        agent_id: &str,
        node: &NodePaths,
        extra: &[&str],
    ) -> Result<LaunchSpec> {
        let script = self.bin_path(agent_id)?;
        let mut args = vec![script.to_string_lossy().into_owned()];
        args.extend(extra.iter().map(|arg| (*arg).to_string()));
        Ok(LaunchSpec::new(
            node.node.to_string_lossy().into_owned(),
            args,
        ))
    }
}

/// Resolve the executable script of the installed package from its
/// `package.json` `bin` field (string, or map keyed by command name).
fn package_bin(prefix: &Path, kind: AgentKind) -> Result<PathBuf> {
    let AdapterSource::Npm { package, command } = kind.source() else {
        return Err(ConnectionsError::InstallFailed {
            package: kind.agent_id().to_string(),
            message: "no es un paquete npm".to_string(),
        });
    };
    let package_dir = prefix.join("node_modules").join(package);
    let manifest_path = package_dir.join("package.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).map_err(io_err(&manifest_path))?)?;
    let relative = match manifest.get("bin") {
        Some(serde_json::Value::String(path)) => Some(path.clone()),
        Some(serde_json::Value::Object(map)) => map
            .get(command)
            .or_else(|| map.values().next())
            .and_then(|value| value.as_str())
            .map(str::to_string),
        _ => None,
    }
    .ok_or_else(|| ConnectionsError::InstallFailed {
        package: package.to_string(),
        message: "package.json no declara `bin`".to_string(),
    })?;
    let relative = PathBuf::from(relative.trim_start_matches("./"));
    let bin = Path::new("node_modules").join(package).join(relative);
    if !prefix.join(&bin).is_file() {
        return Err(ConnectionsError::InstallFailed {
            package: package.to_string(),
            message: format!("no existe {}", bin.display()),
        });
    }
    Ok(bin)
}

/// Archive formats of `binary` distributions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    fn from_url(url: &str) -> Option<Self> {
        let path = url
            .split(['?', '#'])
            .next()
            .unwrap_or(url)
            .to_ascii_lowercase();
        if path.ends_with(".zip") {
            Some(Self::Zip)
        } else if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
            Some(Self::TarGz)
        } else {
            None
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
        }
    }
}

/// Sum of the uncompressed sizes in a zip's central directory.
fn zip_unpacked_size(archive: &Path) -> std::result::Result<u64, String> {
    let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(|error| error.to_string())?;
    let mut total = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|error| error.to_string())?;
        total = total.saturating_add(entry.size());
    }
    Ok(total)
}

/// Unpack `archive` into `dest`, streaming from disk (a 1 GB archive never
/// sits in memory) and never writing outside `dest`. `cancel` is checked
/// before every entry and every read, so even one huge entry stops promptly.
fn unpack(
    format: ArchiveFormat,
    archive: &Path,
    dest: &Path,
    cancel: &CancelToken,
) -> std::result::Result<(), String> {
    match format {
        ArchiveFormat::Zip => unpack_zip(archive, dest, cancel),
        ArchiveFormat::TarGz => unpack_tar_gz(archive, dest, cancel),
    }
}

fn unpack_zip(
    archive: &Path,
    dest: &Path,
    cancel: &CancelToken,
) -> std::result::Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(|error| error.to_string())?;
    for index in 0..zip.len() {
        if cancel.is_cancelled() {
            return Err(CancelToken::io_error().to_string());
        }
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = dest.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut out = std::fs::File::create(&target).map_err(|error| error.to_string())?;
        let mut reader = CancelReader {
            inner: &mut entry,
            token: cancel,
        };
        std::io::copy(&mut reader, &mut out).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            // Keep the archive's bits, but the owner must be able to replace
            // the file on the next update.
            let mode = (mode & 0o777) | 0o600;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn unpack_tar_gz(
    archive: &Path,
    dest: &Path,
    cancel: &CancelToken,
) -> std::result::Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(CancelReader {
        inner: file,
        token: cancel,
    })));
    tar.set_preserve_permissions(true);
    for entry in tar.entries().map_err(|error| error.to_string())? {
        if cancel.is_cancelled() {
            return Err(CancelToken::io_error().to_string());
        }
        let mut entry = entry.map_err(|error| error.to_string())?;
        // `unpack_in` refuses paths that would land outside `dest`.
        entry.unpack_in(dest).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(io_err(path))?
        .permissions()
        .mode();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o111))
        .map_err(io_err(path))
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = r#"{"version":"1.0.0","agents":[
      {"id":"antigravity-acp","name":"Google Antigravity","version":"1.2.1",
       "distribution":{"binary":{
         "linux-x86_64":{"archive":"https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-1.2.1-linux-x86_64.zip",
                         "cmd":"./agy_acp_server.par","args":["--uid="],"env":{},"sha256":null},
         "darwin-aarch64":{"archive":"https://example.invalid/agy.tar.gz","cmd":"./agy_acp_server.par",
                           "sha256":"ABCDEF"}}}},
      {"id":"codex-acp","name":"Codex","version":"1.13.1",
       "distribution":{"npx":{"package":"@otro/paquete@1.0.0"}}},
      {"id":"claude-acp","name":"Claude","version":"0.81.2",
       "distribution":{"npx":{"package":"@agentclientprotocol/claude-agent-acp@0.81.2","args":["--x"]}}}
    ]}"#;

    #[test]
    fn package_name_strips_version_keeping_scope() {
        assert_eq!(
            package_name("@agentclientprotocol/claude-agent-acp@0.81.2"),
            "@agentclientprotocol/claude-agent-acp"
        );
        assert_eq!(package_name("@scope/sin-version"), "@scope/sin-version");
        assert_eq!(package_name("left-pad@1.0.0"), "left-pad");
    }

    #[test]
    fn registry_version_reads_pinned_spec_and_args() {
        let registry = AgentRegistry::parse(REGISTRY).expect("parse");
        let (version, args) = registry_version(&registry, AgentKind::Claude).expect("claude");
        assert_eq!(version, "0.81.2");
        assert_eq!(args, vec!["--x"]);
        assert!(matches!(
            registry_version(&registry, AgentKind::Codex),
            Err(ConnectionsError::NotInRegistry(_))
        ));
    }

    #[test]
    fn binary_plan_reads_the_platform_target() {
        let registry = AgentRegistry::parse(REGISTRY).expect("parse");
        let plan = binary_plan(&registry, AgentKind::Antigravity, "linux-x86_64").expect("plan");
        assert_eq!(plan.version, "1.2.1");
        assert_eq!(plan.cmd, "./agy_acp_server.par");
        assert_eq!(plan.args, vec!["--uid="]);
        assert_eq!(plan.sha256, None);
        assert!(!plan.verifiable(), "Google no publica sha256");
        assert_eq!(plan.archive_name(), "agy-acp-server-1.2.1-linux-x86_64.zip");
        let mac = binary_plan(&registry, AgentKind::Antigravity, "darwin-aarch64").expect("mac");
        assert_eq!(mac.sha256.as_deref(), Some("abcdef"));
        assert!(mac.verifiable());
        assert!(mac.args.is_empty());
        match binary_plan(&registry, AgentKind::Antigravity, "linux-riscv64") {
            Err(ConnectionsError::NoBinaryForPlatform { platform, .. }) => {
                assert_eq!(platform, "linux-riscv64");
            }
            other => panic!("se esperaba NoBinaryForPlatform: {other:?}"),
        }
        assert!(matches!(
            binary_plan(&registry, AgentKind::Codex, "linux-x86_64"),
            Err(ConnectionsError::NotInRegistry(_))
        ));
    }

    #[test]
    fn commands_and_formats_are_validated() {
        assert_eq!(
            relative_cmd("./agy_acp_server.par"),
            Some(PathBuf::from("agy_acp_server.par"))
        );
        assert_eq!(relative_cmd("bin/agent"), Some(PathBuf::from("bin/agent")));
        assert_eq!(relative_cmd("../fuera"), None);
        assert_eq!(relative_cmd("/abs/agent"), None);
        assert_eq!(relative_cmd("./"), None);
        assert_eq!(
            ArchiveFormat::from_url("https://x/a.ZIP?sig=1"),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://x/a.tar.gz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://x/a.tgz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(ArchiveFormat::from_url("https://x/a.rar"), None);
    }

    #[test]
    fn old_markers_without_distribution_read_as_npm() {
        let marker: AdapterInstall = serde_json::from_str(
            r#"{"agent_id":"claude-acp","package":"p","version":"1","bin":"x.js","args":[]}"#,
        )
        .expect("marker");
        assert_eq!(marker.distribution, InstallKind::Npm);
        assert!(marker.verifiable);
    }

    #[test]
    fn free_space_walks_up_to_an_existing_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("no/existe/todavia");
        if cfg!(unix) {
            assert!(free_space(&missing).is_some_and(|bytes| bytes > 0));
        }
    }

    #[test]
    fn version_comparison_is_numeric_and_treats_prerelease_as_lower() {
        assert!(is_newer_version("0.84.0", "0.81.2"));
        assert!(!is_newer_version("0.81.2", "0.84.0"));
        assert!(!is_newer_version("0.81.2", "0.81.2"));
        // A double-digit component must not be compared lexically.
        assert!(is_newer_version("1.10.0", "1.9.0"));
        // A prerelease always sorts below its own release.
        assert!(!is_newer_version("1.0.0-beta.1", "1.0.0"));
        assert!(is_newer_version("1.0.0", "1.0.0-beta.1"));
        // Two prereleases compare identifier by identifier, numerically.
        assert!(is_newer_version("1.0.0-beta.10", "1.0.0-beta.9"));
        assert!(!is_newer_version("1.0.0-beta.9", "1.0.0-beta.10"));
    }

    /// Writes a finished install directly (no installer, no network) so
    /// [`Adapters::installed`] and [`Adapters::update_available`] see it.
    fn fake_install(paths: &CincelPaths, agent_id: &str, version: &str) {
        let dir = paths.agents_dir().join(agent_id).join(version);
        std::fs::create_dir_all(&dir).expect("install dir");
        std::fs::write(dir.join("bin.js"), b"").expect("bin");
        let install = AdapterInstall {
            agent_id: agent_id.to_string(),
            distribution: InstallKind::Npm,
            package: "paquete".to_string(),
            version: version.to_string(),
            bin: PathBuf::from("bin.js"),
            args: Vec::new(),
            env: BTreeMap::new(),
            verifiable: true,
        };
        std::fs::write(
            dir.join(MARKER),
            serde_json::to_vec(&install).expect("marker"),
        )
        .expect("write marker");
        std::fs::write(
            paths.agents_dir().join(agent_id).join("current"),
            version.as_bytes(),
        )
        .expect("write current");
    }

    #[test]
    fn update_available_never_offers_a_lower_catalog_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = CincelPaths::under(dir.path());
        fake_install(&paths, "claude-acp", "0.84.0");
        let adapters = Adapters::new(paths);
        // REGISTRY publishes claude-acp 0.81.2, older than what is installed.
        let registry = AgentRegistry::parse(REGISTRY).expect("parse");
        assert_eq!(
            adapters.update_available(&registry, AgentKind::Claude),
            None,
            "0.81.2 no es más nuevo que 0.84.0: no se ofrece"
        );
    }

    #[test]
    fn update_available_offers_a_higher_catalog_version() {
        const NEWER_REGISTRY: &str = r#"{"version":"1.0.0","agents":[
          {"id":"claude-acp","name":"Claude","version":"0.84.0",
           "distribution":{"npx":{"package":"@agentclientprotocol/claude-agent-acp@0.84.0"}}}
        ]}"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = CincelPaths::under(dir.path());
        fake_install(&paths, "claude-acp", "0.81.2");
        let adapters = Adapters::new(paths);
        let registry = AgentRegistry::parse(NEWER_REGISTRY).expect("parse");
        assert_eq!(
            adapters.update_available(&registry, AgentKind::Claude),
            Some("0.84.0".to_string())
        );
    }
}
