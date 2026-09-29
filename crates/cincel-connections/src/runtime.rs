//! Private Node.js runtime (`docs/specs/06-etapa4-conexiones-y-cincel.md`
//! §3): Cincel never uses the system Node. The first time an agent is
//! connected it downloads a Node LTS from nodejs.org into
//! `<data>/runtime/node-<version>/`, verifying the published SHA-256, with
//! progress, resume and retry.
//!
//! Network access goes through the [`Downloader`] trait so tests can serve
//! `index.json`, `SHASUMS256.txt` and the archive from memory.
//!
//! Etapa 5 (`docs/specs/07-etapa5-productividad.md` §10.3, §10.4): every
//! download takes a [`CancelToken`] (checked before each 64 KiB read, between
//! retries and while unpacking; a cancelled run leaves no `.part`, no
//! decompressed tarball and no staging directory), and
//! [`Runtime::update_available`] / [`Runtime::update`] /
//! [`Runtime::prune_old`] follow the settings' version policy
//! ([`NodeVersion`]) without removing the runtime live agents use.

use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::cancel::{CancelReader, CancelToken, CancelWriter, WorkLock, or_cancelled};
use crate::error::{ConnectionsError, Result, io_err};
use crate::paths::{CincelPaths, create_private_dir, write_atomic};

/// Official Node.js distribution root.
pub const NODE_DIST_URL: &str = "https://nodejs.org/dist/";

/// Minimum Node major the adapters need (same bound as `cincel-acp`).
pub const MIN_NODE_MAJOR: u64 = 22;

/// Paths inside one installed runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePaths {
    /// Version, with the leading `v` (`v24.11.1`).
    pub version: String,
    /// `<data>/runtime/node-<version>`.
    pub root: PathBuf,
    /// `<root>/bin`: goes first in the `PATH` of every agent process.
    pub bin_dir: PathBuf,
    /// `<root>/bin/node`.
    pub node: PathBuf,
    /// npm's entry point, `<root>/lib/node_modules/npm/bin/npm-cli.js`
    /// (run it with [`NodePaths::node`]).
    pub npm: PathBuf,
    /// npx's entry point, `<root>/lib/node_modules/npm/bin/npx-cli.js`.
    pub npx: PathBuf,
}

impl NodePaths {
    fn for_root(version: &str, root: PathBuf) -> Self {
        let npm_bin = root
            .join("lib")
            .join("node_modules")
            .join("npm")
            .join("bin");
        Self {
            version: version.to_string(),
            bin_dir: root.join("bin"),
            node: root.join("bin").join("node"),
            npm: npm_bin.join("npm-cli.js"),
            npx: npm_bin.join("npx-cli.js"),
            root,
        }
    }

    /// Whether every file the adapters need is present.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.node.is_file() && self.npm.is_file() && self.npx.is_file()
    }
}

/// Which Node version [`Runtime::ensure`] installs and [`Runtime::update`]
/// moves to (`settings.json` `connections.runtime.node_version`,
/// `docs/specs/07-etapa5-productividad.md` §10.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NodeVersion {
    /// Newest LTS release for this platform with a major of at least
    /// [`MIN_NODE_MAJOR`] (default, `"lts"`).
    #[default]
    Lts,
    /// Newest release of one major line (`"22"`), LTS or not.
    Major(u64),
    /// One exact version (`v24.11.1` or `24.11.1`): pinned, never updated.
    Exact(String),
}

impl NodeVersion {
    /// Parse the settings value: `"lts"`, a major (`"22"`, `"v22"`) or a
    /// full version.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("lts") {
            return Self::Lts;
        }
        let bare = value.strip_prefix('v').unwrap_or(value);
        if !bare.is_empty()
            && bare.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(major) = bare.parse()
        {
            return Self::Major(major);
        }
        Self::Exact(format!("v{bare}"))
    }

    /// Whether an installed `version` (`v24.11.1`) satisfies this request
    /// without downloading anything: any install for [`NodeVersion::Lts`],
    /// the same major for [`NodeVersion::Major`], the same version for
    /// [`NodeVersion::Exact`].
    #[must_use]
    pub fn is_satisfied_by(&self, version: &str) -> bool {
        match self {
            Self::Lts => true,
            Self::Major(major) => version_parts(version).first() == Some(major),
            Self::Exact(exact) => exact == version,
        }
    }
}

/// `v24.11.1` -> `[24, 11, 1]` (non-numeric parts dropped).
pub(crate) fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Whether "Actualizar a…" should offer `latest` given the installed
/// version and the settings' [`NodeVersion`] policy. Pulled out of
/// [`Runtime::update_available`] so the invariant is unit-testable without a
/// network: under the default [`NodeVersion::Lts`] (`policy.is_satisfied_by`
/// always `true`), this reduces to `latest` being numerically newer than
/// `installed` — a lower LTS release is never offered, whichever source
/// (the bundled index or a fresh nodejs.org check) `latest` came from.
/// [`NodeVersion::Major`] additionally offers switching to a different major
/// line the policy now asks for, even when that line's newest release is
/// numerically lower (a deliberate move, not an "update").
fn should_offer_update(policy: &NodeVersion, installed: &str, latest: &str) -> bool {
    let newer = version_parts(latest) > version_parts(installed);
    latest != installed && (newer || !policy.is_satisfied_by(installed))
}

/// Phase reported by [`Runtime::ensure`]'s progress callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeProgress {
    /// Reading `index.json` to pick the version.
    Resolving,
    /// Downloading the archive (`total` when the server sends a length).
    Downloading {
        /// Version being downloaded.
        version: String,
        /// Bytes on disk so far (includes a resumed prefix).
        done: u64,
        /// Total size, when known.
        total: Option<u64>,
    },
    /// A download attempt failed and will be retried.
    Retrying {
        /// 1-based number of the attempt that is about to start.
        attempt: u32,
        /// Why the previous attempt failed.
        reason: String,
    },
    /// Checking the SHA-256.
    Verifying,
    /// Unpacking.
    Extracting,
    /// Ready.
    Done,
}

/// A readable HTTP body, positioned at the requested offset.
pub struct RangeBody {
    /// `true` when the server honored the offset (HTTP 206); `false` means
    /// the body starts at byte 0 and the partial file must be discarded.
    pub resumed: bool,
    /// Full size of the resource, when known.
    pub total: Option<u64>,
    /// The bytes.
    pub reader: Box<dyn Read + Send>,
}

/// Network access used by [`Runtime`].
pub trait Downloader: Send + Sync {
    /// Fetch a small resource whole (`index.json`, `SHASUMS256.txt`).
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::Network`] on any failure.
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>>;

    /// Open `url` starting at byte `offset` (a `Range` request when
    /// `offset > 0`).
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::Network`] on any failure.
    fn open(&self, url: &str, offset: u64) -> Result<RangeBody>;
}

/// [`Downloader`] backed by `ureq` (rustls).
#[derive(Debug, Clone)]
pub struct HttpDownloader {
    agent: ureq::Agent,
}

impl Default for HttpDownloader {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Downloader for HttpDownloader {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|error| ConnectionsError::Network(error.to_string()))?;
        if !response.status().is_success() {
            return Err(ConnectionsError::Network(format!(
                "{url}: HTTP {}",
                response.status()
            )));
        }
        response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|error| ConnectionsError::Network(error.to_string()))
    }

    fn open(&self, url: &str, offset: u64) -> Result<RangeBody> {
        let mut request = self.agent.get(url);
        if offset > 0 {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let response = request
            .call()
            .map_err(|error| ConnectionsError::Network(error.to_string()))?;
        let status = response.status().as_u16();
        let length = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        match status {
            206 => Ok(RangeBody {
                resumed: true,
                total: length.map(|length| length + offset),
                reader: Box::new(response.into_body().into_reader()),
            }),
            200 => Ok(RangeBody {
                resumed: false,
                total: length,
                reader: Box::new(response.into_body().into_reader()),
            }),
            // The partial file already holds everything.
            416 if offset > 0 => Ok(RangeBody {
                resumed: true,
                total: Some(offset),
                reader: Box::new(std::io::empty()),
            }),
            other => Err(ConnectionsError::Network(format!("{url}: HTTP {other}"))),
        }
    }
}

/// The private Node runtime manager.
pub struct Runtime {
    paths: CincelPaths,
    downloader: Box<dyn Downloader>,
    base_url: String,
    version: NodeVersion,
    platform: Option<String>,
    attempts: u32,
    retry_delay: Duration,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("paths", &self.paths)
            .field("base_url", &self.base_url)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl Runtime {
    /// Runtime under `paths`, downloading from nodejs.org over HTTPS.
    #[must_use]
    pub fn new(paths: CincelPaths) -> Self {
        Self {
            paths,
            downloader: Box::new(HttpDownloader::default()),
            base_url: NODE_DIST_URL.to_string(),
            version: NodeVersion::Lts,
            platform: node_platform(),
            attempts: 3,
            retry_delay: Duration::from_secs(2),
        }
    }

    /// Replace the network layer (tests).
    #[must_use]
    pub fn with_downloader(mut self, downloader: Box<dyn Downloader>) -> Self {
        self.downloader = downloader;
        self
    }

    /// Replace the distribution root (tests, mirrors). Must end with `/`.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Which version to install.
    #[must_use]
    pub fn with_version(mut self, version: NodeVersion) -> Self {
        self.version = version;
        self
    }

    /// Download attempts and pause between them.
    #[must_use]
    pub fn with_retry(mut self, attempts: u32, delay: Duration) -> Self {
        self.attempts = attempts.max(1);
        self.retry_delay = delay;
        self
    }

    /// Force a platform key (`linux-x64`...), tests only.
    #[must_use]
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    fn current_file(&self) -> PathBuf {
        self.paths.runtime_dir().join("current")
    }

    /// In-process lock key of the runtime directory ([`WorkLock`]).
    fn lock_key(&self) -> String {
        format!("runtime:{}", self.paths.runtime_dir().display())
    }

    /// The installed runtime, without touching the network. `None` when
    /// nothing (complete) is installed.
    #[must_use]
    pub fn installed(&self) -> Option<NodePaths> {
        let version = std::fs::read_to_string(self.current_file()).ok()?;
        let version = version.trim();
        if version.is_empty() || !version.starts_with('v') || version.contains('/') {
            return None;
        }
        let paths = NodePaths::for_root(
            version,
            self.paths.runtime_dir().join(format!("node-{version}")),
        );
        paths.is_complete().then_some(paths)
    }

    /// Make sure a runtime is installed and return its paths. Uses the
    /// installed one when it satisfies the requested version (offline-safe,
    /// [`NodeVersion::is_satisfied_by`]); otherwise resolves, downloads
    /// (resuming a partial file, retrying), verifies and extracts.
    /// Cancelling `cancel` stops it within one 64 KiB read (or one archive
    /// entry) and removes the partial download and the staging directory.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::Network`], [`ConnectionsError::ChecksumMismatch`],
    /// [`ConnectionsError::NoNodeRelease`], [`ConnectionsError::Extract`],
    /// [`ConnectionsError::Cancelled`].
    pub fn ensure(
        &self,
        progress: &mut dyn FnMut(RuntimeProgress),
        cancel: &CancelToken,
    ) -> Result<NodePaths> {
        let _lock = WorkLock::acquire(&self.lock_key(), cancel)?;
        if let Some(installed) = self.installed()
            && self.version.is_satisfied_by(&installed.version)
        {
            progress(RuntimeProgress::Done);
            return Ok(installed);
        }
        progress(RuntimeProgress::Resolving);
        let platform = self.platform()?;
        let version = self.resolve_version(&platform, cancel)?;
        self.fetch_and_install(&platform, &version, progress, cancel)
    }

    /// The version [`Runtime::update`] would install, when it differs from
    /// the installed one (the "Actualizar a vX.Y.Z" button). Resolves
    /// against nodejs.org's `index.json` (network) following the settings
    /// policy: [`NodeVersion::Lts`], the newest LTS with a major of at least
    /// [`MIN_NODE_MAJOR`]; [`NodeVersion::Major`], the newest release of that
    /// line; [`NodeVersion::Exact`], never (pinned, no network). `None` too
    /// when nothing is installed (preparing a connection installs it) or the
    /// installed version is already the newest.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::Network`] (offline), [`ConnectionsError::Index`],
    /// [`ConnectionsError::NoNodeRelease`].
    pub fn update_available(&self) -> Result<Option<String>> {
        if matches!(self.version, NodeVersion::Exact(_)) {
            return Ok(None);
        }
        let Some(installed) = self.installed() else {
            return Ok(None);
        };
        let platform = self.platform()?;
        let latest = self.resolve_version(&platform, &CancelToken::new())?;
        Ok(should_offer_update(&self.version, &installed.version, &latest).then_some(latest))
    }

    /// Install the version [`Runtime::update_available`] offers and move
    /// `current` to it. The previous runtime stays on disk (agents started
    /// with it keep running; `docs/specs/07-etapa5-productividad.md` D13)
    /// until [`Runtime::prune_old`] runs on the next start. New npm
    /// connections use the new runtime; a live one keeps its own until it
    /// reconnects. Returns the installed runtime unchanged when it is
    /// already the resolved version. Cancellable like [`Runtime::ensure`].
    ///
    /// # Errors
    ///
    /// Same as [`Runtime::ensure`].
    pub fn update(
        &self,
        progress: &mut dyn FnMut(RuntimeProgress),
        cancel: &CancelToken,
    ) -> Result<NodePaths> {
        let _lock = WorkLock::acquire(&self.lock_key(), cancel)?;
        progress(RuntimeProgress::Resolving);
        let platform = self.platform()?;
        let version = self.resolve_version(&platform, cancel)?;
        if let Some(installed) = self.installed()
            && installed.version == version
        {
            progress(RuntimeProgress::Done);
            return Ok(installed);
        }
        self.fetch_and_install(&platform, &version, progress, cancel)
    }

    /// Remove every runtime that `current` does not point at, plus staging
    /// directories left by a crashed install of another process. Call it at
    /// start-up, before any agent runs (nothing is in use then). Does
    /// nothing when no valid runtime is installed. Returns the removed
    /// directory names.
    ///
    /// # Errors
    ///
    /// I/O errors removing a directory.
    pub fn prune_old(&self) -> Result<Vec<String>> {
        let Some(installed) = self.installed() else {
            return Ok(Vec::new());
        };
        let keep = format!("node-{}", installed.version);
        let runtime_dir = self.paths.runtime_dir();
        let mut removed = Vec::new();
        let Ok(entries) = std::fs::read_dir(&runtime_dir) else {
            return Ok(removed);
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stale = if name.starts_with("node-") {
                name != keep
            } else {
                name.starts_with(".node-") && foreign_staging(&name)
            };
            if stale && entry.path().is_dir() {
                std::fs::remove_dir_all(entry.path()).map_err(io_err(entry.path()))?;
                removed.push(name);
            }
        }
        Ok(removed)
    }

    fn platform(&self) -> Result<String> {
        self.platform.clone().ok_or_else(|| {
            ConnectionsError::NoNodeRelease(format!(
                "{}-{} no tiene binarios de Node en tar.xz",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        })
    }

    fn staging_dir(&self, version: &str) -> PathBuf {
        self.paths
            .runtime_dir()
            .join(format!(".node-{version}.tmp-{}", std::process::id()))
    }

    /// Download, verify and install `version`, removing the partial file,
    /// the decompressed tarball and the staging directory when cancelled.
    fn fetch_and_install(
        &self,
        platform: &str,
        version: &str,
        progress: &mut dyn FnMut(RuntimeProgress),
        cancel: &CancelToken,
    ) -> Result<NodePaths> {
        let archive = format!("node-{version}-{platform}.tar.xz");
        let part = self.paths.downloads_dir().join(format!("{archive}.part"));
        let result =
            self.fetch_and_install_inner(platform, version, &archive, &part, progress, cancel);
        if matches!(result, Err(ConnectionsError::Cancelled)) {
            let _ = std::fs::remove_file(&part);
            let _ = std::fs::remove_file(decompressed_path(&part));
            let _ = std::fs::remove_dir_all(self.staging_dir(version));
        }
        result
    }

    fn fetch_and_install_inner(
        &self,
        platform: &str,
        version: &str,
        archive: &str,
        part: &Path,
        progress: &mut dyn FnMut(RuntimeProgress),
        cancel: &CancelToken,
    ) -> Result<NodePaths> {
        let expected = self.expected_sha256(version, archive, cancel)?;
        let url = format!("{}{version}/{archive}", self.base_url);

        create_private_dir(&self.paths.cache_dir)?;
        let downloads = self.paths.downloads_dir();
        std::fs::create_dir_all(&downloads).map_err(io_err(&downloads))?;

        let mut last_error = None;
        for attempt in 1..=self.attempts {
            cancel.check()?;
            if attempt > 1 {
                progress(RuntimeProgress::Retrying {
                    attempt,
                    reason: last_error
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                });
                cancel.sleep(self.retry_delay)?;
            }
            let downloaded = download_part(
                self.downloader.as_ref(),
                &url,
                part,
                cancel,
                RUNTIME_REPORT_EVERY,
                &mut |done, total| {
                    progress(RuntimeProgress::Downloading {
                        version: version.to_string(),
                        done,
                        total,
                    });
                },
            );
            match downloaded {
                Ok(()) => {}
                Err(ConnectionsError::Cancelled) => return Err(ConnectionsError::Cancelled),
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            }
            cancel.check()?;
            progress(RuntimeProgress::Verifying);
            let actual = sha256_file(part)?;
            if !actual.eq_ignore_ascii_case(&expected) {
                // A corrupt partial file must not be resumed again.
                let _ = std::fs::remove_file(part);
                last_error = Some(ConnectionsError::ChecksumMismatch {
                    file: archive.to_string(),
                    expected: expected.clone(),
                    actual,
                });
                continue;
            }
            cancel.check()?;
            progress(RuntimeProgress::Extracting);
            let installed = self.install_archive(part, version, platform, cancel)?;
            let _ = std::fs::remove_file(part);
            progress(RuntimeProgress::Done);
            return Ok(installed);
        }
        Err(last_error.unwrap_or_else(|| ConnectionsError::Network("sin intentos".to_string())))
    }

    fn resolve_version(&self, platform: &str, cancel: &CancelToken) -> Result<String> {
        cancel.check()?;
        let pick: fn(&[u8], &str, &NodeVersion) -> Result<String> =
            |body, platform, wanted| match wanted {
                NodeVersion::Major(major) => pick_major(body, platform, *major),
                _ => pick_lts(body, platform),
            };
        match &self.version {
            NodeVersion::Exact(version) => Ok(version.clone()),
            wanted => {
                let body = self
                    .downloader
                    .get_bytes(&format!("{}index.json", self.base_url))
                    .map_err(|error| or_cancelled(cancel, error))?;
                cancel.check()?;
                pick(&body, platform, wanted)
            }
        }
    }

    fn expected_sha256(
        &self,
        version: &str,
        archive: &str,
        cancel: &CancelToken,
    ) -> Result<String> {
        cancel.check()?;
        let body = self
            .downloader
            .get_bytes(&format!("{}{version}/SHASUMS256.txt", self.base_url))
            .map_err(|error| or_cancelled(cancel, error))?;
        cancel.check()?;
        let text = String::from_utf8_lossy(&body);
        text.lines()
            .find_map(|line| {
                let mut parts = line.split_whitespace();
                let digest = parts.next()?;
                let name = parts.next()?;
                (name == archive).then(|| digest.to_ascii_lowercase())
            })
            .ok_or_else(|| {
                ConnectionsError::NoNodeRelease(format!("{archive} no figura en SHASUMS256.txt"))
            })
    }

    /// Unpack into staging, rename into place and point `current` at it.
    /// Older runtimes are left for [`Runtime::prune_old`].
    fn install_archive(
        &self,
        archive: &Path,
        version: &str,
        platform: &str,
        cancel: &CancelToken,
    ) -> Result<NodePaths> {
        let runtime_dir = self.paths.runtime_dir();
        create_private_dir(&runtime_dir)?;
        let staging = self.staging_dir(version);
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
        }
        std::fs::create_dir_all(&staging).map_err(io_err(&staging))?;
        let file_name = archive
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let top = format!("node-{version}-{platform}");
        if let Err(error) = extract_node_tar_xz(archive, &top, &staging, cancel) {
            let _ = std::fs::remove_dir_all(&staging);
            if cancel.is_cancelled() {
                return Err(ConnectionsError::Cancelled);
            }
            return Err(ConnectionsError::Extract {
                file: file_name,
                message: error.to_string(),
            });
        }
        let installed = NodePaths::for_root(version, staging.clone());
        if !installed.is_complete() {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(ConnectionsError::Extract {
                file: file_name,
                message: "faltan node, npm-cli.js o npx-cli.js en el archivo".to_string(),
            });
        }
        if cancel.is_cancelled() {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(ConnectionsError::Cancelled);
        }
        let final_root = runtime_dir.join(format!("node-{version}"));
        if final_root.exists() {
            std::fs::remove_dir_all(&final_root).map_err(io_err(&final_root))?;
        }
        std::fs::rename(&staging, &final_root).map_err(io_err(&final_root))?;
        write_atomic(&self.current_file(), version.as_bytes())?;
        Ok(NodePaths::for_root(version, final_root))
    }
}

/// Progress granularity of the Node download (bytes between reports).
const RUNTIME_REPORT_EVERY: u64 = 256 * 1024;

/// Size of every read of a download: the cancel token is checked before
/// each one (D14).
pub(crate) const DOWNLOAD_CHUNK: usize = 64 * 1024;

/// Whether a staging directory name (`.<x>.tmp-<pid>`) belongs to another
/// process than this one (a crashed install: safe to remove at start-up).
pub(crate) fn foreign_staging(name: &str) -> bool {
    name.rsplit_once(".tmp-")
        .and_then(|(_, pid)| pid.parse::<u32>().ok())
        .is_some_and(|pid| pid != std::process::id())
}

/// Download `url` into `part`, resuming from its current size (`Range`),
/// reading 64 KiB at a time and checking `cancel` before every read.
/// `on_progress(done, total)` runs at the start, every `report_every` bytes
/// and at the end. When cancelled the response is dropped (closing the
/// connection), `part` is removed and [`ConnectionsError::Cancelled`] is
/// returned; any other failure keeps `part` so the next attempt resumes.
pub(crate) fn download_part(
    downloader: &dyn Downloader,
    url: &str,
    part: &Path,
    cancel: &CancelToken,
    report_every: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<()> {
    let result = download_part_inner(downloader, url, part, cancel, report_every, on_progress);
    if matches!(result, Err(ConnectionsError::Cancelled)) {
        let _ = std::fs::remove_file(part);
    }
    result
}

fn download_part_inner(
    downloader: &dyn Downloader,
    url: &str,
    part: &Path,
    cancel: &CancelToken,
    report_every: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<()> {
    cancel.check()?;
    let offset = std::fs::metadata(part).map_or(0, |meta| meta.len());
    let body = downloader
        .open(url, offset)
        .map_err(|error| or_cancelled(cancel, error))?;
    let mut file = if body.resumed && offset > 0 {
        std::fs::OpenOptions::new()
            .append(true)
            .open(part)
            .map_err(io_err(part))?
    } else {
        std::fs::File::create(part).map_err(io_err(part))?
    };
    let mut done = if body.resumed { offset } else { 0 };
    let total = body.total;
    on_progress(done, total);
    let mut reader = body.reader;
    let mut buffer = vec![0u8; DOWNLOAD_CHUNK];
    let mut reported = done;
    loop {
        cancel.check()?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| or_cancelled(cancel, ConnectionsError::Network(error.to_string())))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(io_err(part))?;
        done += read as u64;
        if done - reported >= report_every {
            reported = done;
            on_progress(done, total);
        }
    }
    file.flush().map_err(io_err(part))?;
    if reported != done {
        on_progress(done, total);
    }
    if let Some(total) = total
        && done < total
    {
        return Err(ConnectionsError::Network(format!(
            "descarga incompleta ({done} de {total} bytes)"
        )));
    }
    Ok(())
}
/// nodejs.org platform key for this machine (`linux-x64`, `darwin-arm64`...),
/// `None` where no `.tar.xz` build exists (Windows).
#[must_use]
pub fn node_platform() -> Option<String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

/// Pick the newest LTS release of `index.json` that ships `platform` and is
/// at least [`MIN_NODE_MAJOR`].
///
/// # Errors
///
/// [`ConnectionsError::Index`] for malformed JSON,
/// [`ConnectionsError::NoNodeRelease`] when nothing matches.
pub fn pick_lts(index_json: &[u8], platform: &str) -> Result<String> {
    let releases: Vec<serde_json::Value> = serde_json::from_slice(index_json)?;
    // nodejs.org lists `files` as `linux-x64` (tar.gz and tar.xz both).
    let wanted = platform.to_string();
    let mut best: Option<(Vec<u64>, String)> = None;
    for release in releases {
        let Some(version) = release.get("version").and_then(|value| value.as_str()) else {
            continue;
        };
        let is_lts = release
            .get("lts")
            .is_some_and(|lts| lts.as_str().is_some() || lts.as_bool() == Some(true));
        let has_platform = release
            .get("files")
            .and_then(|files| files.as_array())
            .is_some_and(|files| files.iter().any(|file| file.as_str() == Some(&wanted)));
        let parts: Vec<u64> = version
            .trim_start_matches('v')
            .split('.')
            .filter_map(|part| part.parse().ok())
            .collect();
        if !is_lts || !has_platform || parts.first().copied().unwrap_or(0) < MIN_NODE_MAJOR {
            continue;
        }
        if best.as_ref().is_none_or(|(current, _)| parts > *current) {
            best = Some((parts, version.to_string()));
        }
    }
    best.map(|(_, version)| version)
        .ok_or_else(|| ConnectionsError::NoNodeRelease(format!("ninguna LTS para {platform}")))
}

/// Pick the newest release of `index.json` of the `major` line (LTS or not)
/// that ships `platform`.
///
/// # Errors
///
/// [`ConnectionsError::Index`] for malformed JSON,
/// [`ConnectionsError::NoNodeRelease`] when nothing matches.
pub fn pick_major(index_json: &[u8], platform: &str, major: u64) -> Result<String> {
    let releases: Vec<serde_json::Value> = serde_json::from_slice(index_json)?;
    let mut best: Option<(Vec<u64>, String)> = None;
    for release in releases {
        let Some(version) = release.get("version").and_then(|value| value.as_str()) else {
            continue;
        };
        let has_platform = release
            .get("files")
            .and_then(|files| files.as_array())
            .is_some_and(|files| files.iter().any(|file| file.as_str() == Some(platform)));
        let parts = version_parts(version);
        if !has_platform || parts.first() != Some(&major) {
            continue;
        }
        if best.as_ref().is_none_or(|(current, _)| parts > *current) {
            best = Some((parts, version.to_string()));
        }
    }
    best.map(|(_, version)| version).ok_or_else(|| {
        ConnectionsError::NoNodeRelease(format!("ninguna versión {major}.x para {platform}"))
    })
}
/// SHA-256 of a file, lowercase hex, streamed.
///
/// # Errors
///
/// I/O errors.
pub fn sha256_file(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path).map_err(io_err(path))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(io_err(path))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether a path inside the Node archive (relative to its top directory)
/// is needed to run `node`, `npm` and `npx`. Headers, docs, man pages and
/// corepack are dropped; the license is kept.
#[must_use]
pub fn keep_runtime_entry(relative: &Path) -> bool {
    let text = relative.to_string_lossy();
    matches!(
        text.as_ref(),
        "bin/node" | "bin/npm" | "bin/npx" | "LICENSE"
    ) || relative.starts_with("lib/node_modules/npm")
}

/// Decompress `archive` (`.tar.xz`), strip the `top/` directory and unpack
/// only the entries [`keep_runtime_entry`] wants into `dest`. The tarball is
/// decompressed to a temporary file next to `archive` first (streamed; the
/// xz decoder does not need the whole archive in memory). Every read and
/// write checks `cancel`, and so does every tar entry; the temporary tarball
/// is removed whatever happens.
fn extract_node_tar_xz(
    archive: &Path,
    top: &str,
    dest: &Path,
    cancel: &CancelToken,
) -> std::io::Result<()> {
    let tar_path = decompressed_path(archive);
    let decompressed = (|| {
        let mut input = BufReader::new(CancelReader {
            inner: std::fs::File::open(archive)?,
            token: cancel,
        });
        let mut output = std::io::BufWriter::new(CancelWriter {
            inner: std::fs::File::create(&tar_path)?,
            token: cancel,
        });
        lzma_rs::xz_decompress(&mut input, &mut output)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        output.flush()
    })();
    let result = decompressed.and_then(|()| unpack_filtered(&tar_path, top, dest, cancel));
    let _ = std::fs::remove_file(&tar_path);
    result
}

/// Where [`extract_node_tar_xz`] decompresses `archive` to.
fn decompressed_path(archive: &Path) -> PathBuf {
    archive.with_extension("tar-decompressed")
}

fn unpack_filtered(
    tar_path: &Path,
    top: &str,
    dest: &Path,
    cancel: &CancelToken,
) -> std::io::Result<()> {
    let mut tar = tar::Archive::new(BufReader::new(CancelReader {
        inner: std::fs::File::open(tar_path)?,
        token: cancel,
    }));
    tar.set_preserve_permissions(true);
    for entry in tar.entries()? {
        if cancel.is_cancelled() {
            return Err(CancelToken::io_error());
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Ok(relative) = path.strip_prefix(top) else {
            continue;
        };
        if relative.as_os_str().is_empty() || !keep_runtime_entry(relative) {
            continue;
        }
        // Never write outside `dest`.
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            continue;
        }
        let target = dest.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_version_parses_settings_values() {
        assert_eq!(NodeVersion::parse("lts"), NodeVersion::Lts);
        assert_eq!(NodeVersion::parse(" LTS "), NodeVersion::Lts);
        assert_eq!(NodeVersion::parse(""), NodeVersion::Lts);
        assert_eq!(
            NodeVersion::parse("24.1.0"),
            NodeVersion::Exact("v24.1.0".to_string())
        );
        assert_eq!(
            NodeVersion::parse("v22.3.0"),
            NodeVersion::Exact("v22.3.0".to_string())
        );
    }

    #[test]
    fn node_version_parses_major_lines() {
        assert_eq!(NodeVersion::parse("22"), NodeVersion::Major(22));
        assert_eq!(NodeVersion::parse(" v24 "), NodeVersion::Major(24));
        assert!(NodeVersion::Major(22).is_satisfied_by("v22.3.0"));
        assert!(!NodeVersion::Major(22).is_satisfied_by("v24.1.0"));
        assert!(NodeVersion::Lts.is_satisfied_by("v24.1.0"));
        assert!(NodeVersion::parse("22.11.1").is_satisfied_by("v22.11.1"));
        assert!(!NodeVersion::parse("22.11.1").is_satisfied_by("v22.12.0"));
    }

    #[test]
    fn pick_major_takes_newest_of_the_line() {
        let index = serde_json::json!([
            { "version": "v25.2.0", "lts": false, "files": ["linux-x64"] },
            { "version": "v24.11.1", "lts": "Krypton", "files": ["linux-x64"] },
            { "version": "v22.21.0", "lts": "Jod", "files": ["darwin-arm64"] },
            { "version": "v22.20.0", "lts": "Jod", "files": ["linux-x64"] },
            { "version": "v22.9.0", "lts": "Jod", "files": ["linux-x64"] }
        ]);
        let body = serde_json::to_vec(&index).expect("json");
        assert_eq!(pick_major(&body, "linux-x64", 22).expect("22"), "v22.20.0");
        assert_eq!(pick_major(&body, "linux-x64", 25).expect("25"), "v25.2.0");
        assert!(matches!(
            pick_major(&body, "linux-x64", 23),
            Err(ConnectionsError::NoNodeRelease(_))
        ));
    }

    #[test]
    fn staging_of_other_processes_is_foreign() {
        assert!(foreign_staging(".node-v24.1.0.tmp-1"));
        assert!(!foreign_staging(&format!(
            ".node-v24.1.0.tmp-{}",
            std::process::id()
        )));
        assert!(!foreign_staging(".node-v24.1.0"));
    }

    #[test]
    fn pick_lts_takes_newest_lts_with_platform() {
        let index = serde_json::json!([
            { "version": "v25.2.0", "lts": false, "files": ["linux-x64"] },
            { "version": "v24.11.1", "lts": "Krypton", "files": ["linux-x64", "darwin-arm64"] },
            { "version": "v24.9.0", "lts": "Krypton", "files": ["linux-x64"] },
            { "version": "v22.20.0", "lts": "Jod", "files": ["linux-x64"] },
            { "version": "v24.12.0", "lts": "Krypton", "files": ["darwin-arm64"] },
            { "version": "v20.19.0", "lts": "Iron", "files": ["linux-x64"] }
        ]);
        let body = serde_json::to_vec(&index).expect("json");
        assert_eq!(pick_lts(&body, "linux-x64").expect("lts"), "v24.11.1");
        assert_eq!(pick_lts(&body, "darwin-arm64").expect("lts"), "v24.12.0");
        assert!(matches!(
            pick_lts(&body, "linux-riscv64"),
            Err(ConnectionsError::NoNodeRelease(_))
        ));
    }

    #[test]
    fn only_runtime_entries_are_kept() {
        for keep in [
            "bin/node",
            "bin/npm",
            "bin/npx",
            "LICENSE",
            "lib/node_modules/npm/bin/npm-cli.js",
            "lib/node_modules/npm/package.json",
        ] {
            assert!(keep_runtime_entry(Path::new(keep)), "{keep}");
        }
        for drop in [
            "include/node/node.h",
            "share/man/man1/node.1",
            "bin/corepack",
            "lib/node_modules/corepack/package.json",
            "README.md",
            "CHANGELOG.md",
        ] {
            assert!(!keep_runtime_entry(Path::new(drop)), "{drop}");
        }
    }

    #[test]
    fn should_offer_update_never_offers_a_lower_lts() {
        // The bundled index (or a stale fetch) publishing an older release
        // than what is already installed must never be offered.
        assert!(!should_offer_update(
            &NodeVersion::Lts,
            "v24.11.1",
            "v22.20.0"
        ));
        assert!(should_offer_update(
            &NodeVersion::Lts,
            "v22.20.0",
            "v24.11.1"
        ));
        assert!(!should_offer_update(
            &NodeVersion::Lts,
            "v24.11.1",
            "v24.11.1"
        ));
        // Major(22) with v24 installed offers switching to the line the
        // policy now asks for, even though it is numerically lower: a
        // deliberate move, not a regression of the default LTS policy.
        assert!(should_offer_update(
            &NodeVersion::Major(22),
            "v24.11.1",
            "v22.20.0"
        ));
        assert!(!should_offer_update(
            &NodeVersion::Major(22),
            "v22.20.0",
            "v22.9.0"
        ));
    }

    #[test]
    fn platform_key_matches_nodejs_naming() {
        if let Some(platform) = node_platform() {
            assert!(
                ["linux-x64", "linux-arm64", "darwin-x64", "darwin-arm64"]
                    .contains(&platform.as_str())
            );
        }
    }
}
