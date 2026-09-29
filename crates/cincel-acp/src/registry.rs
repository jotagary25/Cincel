//! Agent registry: download, cache and resolve launch commands.
//!
//! The registry lives at
//! `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json` and is
//! cached under `$XDG_CACHE_HOME/cincel/registry.json` with a 24 h TTL.
//!
//! For E0 only the `npx` distribution can be launched; `binary` and `uvx`
//! entries are parsed but [`AgentRegistry::launch_command`] returns
//! [`AcpError::NotSupportedYet`] for them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AcpError, Result};

/// Canonical URL of the official ACP agent registry.
pub const REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// How long a cached registry stays fresh.
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Minimum Node major version required to run `npx` distributions.
pub const MIN_NODE_MAJOR: u32 = 22;

/// Where an [`AgentDescriptor`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrySource {
    /// Freshly downloaded from the CDN.
    Network,
    /// Read from the local cache and still within the TTL.
    Cache,
    /// Read from the local cache after the TTL expired (network unavailable).
    StaleCache,
}

/// One agent as published in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDescriptor {
    /// Stable identifier, e.g. `claude-acp`.
    pub id: String,
    /// Human readable name.
    pub name: String,
    /// Published version of the adapter.
    pub version: String,
    /// Free-form description.
    #[serde(default)]
    pub description: Option<String>,
    /// Source repository.
    #[serde(default)]
    pub repository: Option<String>,
    /// How to obtain and run the agent.
    pub distribution: Distribution,
    /// Icon URL.
    #[serde(default)]
    pub icon: Option<String>,
}

/// Distribution channels for an agent.
///
/// An entry may publish more than one channel at once (`kilo` and `sigit` ship
/// both `binary` and `npx`), so this is a struct of optional channels rather
/// than an enum.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Distribution {
    /// Run through `npx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx: Option<NpxDistribution>,
    /// Run through `uvx` (Python).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uvx: Option<UvxDistribution>,
    /// Prebuilt binaries keyed by `<os>-<arch>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<BTreeMap<String, BinaryTarget>>,
    /// Not part of the wire format: set by [`AgentRegistry::with_custom`] for
    /// agents defined in `settings.json`. Always takes precedence in
    /// [`AgentRegistry::launch_command`].
    #[serde(skip)]
    pub custom: Option<CustomAgent>,
}

impl Distribution {
    /// Short name of the preferred available channel, used in error messages.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        if self.custom.is_some() {
            "custom"
        } else if self.npx.is_some() {
            "npx"
        } else if self.uvx.is_some() {
            "uvx"
        } else if self.binary.is_some() {
            "binary"
        } else {
            "desconocida"
        }
    }

    /// Binary target for the machine we are running on, if any.
    #[must_use]
    pub fn binary_for_this_platform(&self) -> Option<&BinaryTarget> {
        self.binary.as_ref()?.get(&current_platform_key())
    }
}

/// A user-defined agent from `settings.json` (`agents.custom`).
///
/// Cincel does not depend on `cincel-settings` from this crate: the
/// workspace glue in `cincel` maps the settings shape onto this struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomAgent {
    /// Stable identifier. Shadows a registry entry with the same id.
    pub id: String,
    /// Human readable name.
    pub name: String,
    /// Program to execute (absolute path or something resolvable on `PATH`).
    pub command: String,
    /// Arguments for `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// `npx` distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpxDistribution {
    /// Package spec; it usually already carries `@<version>`.
    pub package: String,
    /// Extra arguments appended after the package spec.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables the registry asks for.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// `uvx` distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UvxDistribution {
    /// Package spec.
    pub package: String,
    /// Extra arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// One prebuilt binary target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryTarget {
    /// Archive URL.
    pub archive: String,
    /// Command inside the extracted archive.
    pub cmd: String,
    /// Extra arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables the registry asks for.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// SHA-256 of the archive. Several entries in the live registry omit it,
    /// so the download path (Etapa 2) must treat `None` as "unverifiable".
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Everything needed to spawn an agent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// Program to execute (e.g. `npx`).
    pub program: String,
    /// Arguments for the program.
    pub args: Vec<String>,
    /// Extra environment variables layered on top of the inherited environment.
    pub env: BTreeMap<String, String>,
}

impl LaunchSpec {
    /// Build a launch spec from an arbitrary command (user-defined agents).
    #[must_use]
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            env: BTreeMap::new(),
        }
    }

    /// Add (or replace) one environment variable.
    #[must_use]
    pub fn with_env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }

    /// Opt in to `NO_BROWSER=1` for this launch. It is no longer injected by
    /// default: it hides the browser-based login methods of Claude, Codex
    /// and Gemini (`docs/research/06-acp-estado-y-cuentas.md` §0.1).
    #[must_use]
    pub fn with_no_browser(self) -> Self {
        self.with_env("NO_BROWSER", "1")
    }

    /// Render the command the way a user would type it in a shell.
    #[must_use]
    pub fn to_shell_string(&self) -> String {
        let mut out = self.program.clone();
        for arg in &self.args {
            out.push(' ');
            out.push_str(arg);
        }
        out
    }
}

/// Wire shape of `registry.json`.
#[derive(Debug, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    version: String,
    // Parsed one by one so a single malformed entry does not sink the whole
    // registry: the CDN adds fields and agents faster than this crate follows.
    agents: Vec<serde_json::Value>,
}

/// The parsed registry.
#[derive(Debug, Clone)]
pub struct AgentRegistry {
    version: String,
    agents: Vec<AgentDescriptor>,
    source: RegistrySource,
}

impl AgentRegistry {
    /// Load the registry, preferring a fresh cache, then the network, then a
    /// stale cache.
    ///
    /// # Errors
    ///
    /// Fails only when there is neither a usable cache nor network access.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::cache_path())
    }

    /// Same as [`AgentRegistry::load`] with an explicit cache file (used by tests).
    ///
    /// # Errors
    ///
    /// Fails only when there is neither a usable cache nor network access.
    pub fn load_from(cache: &Path) -> Result<Self> {
        if let Some(registry) = Self::read_cache(cache, CACHE_TTL) {
            return Ok(registry);
        }

        match Self::fetch() {
            Ok(registry) => {
                if let Err(error) = registry.write_cache(cache) {
                    tracing::warn!(%error, "no se pudo escribir la caché del registro");
                }
                Ok(registry)
            }
            Err(error) => {
                // No network: fall back to whatever is cached, however old.
                if let Some(mut stale) = Self::read_cache(cache, Duration::MAX) {
                    stale.source = RegistrySource::StaleCache;
                    tracing::warn!(%error, "usando caché vencida del registro");
                    return Ok(stale);
                }
                Err(error)
            }
        }
    }

    /// Download the registry, bypassing the cache.
    ///
    /// # Errors
    ///
    /// Fails on network or parse errors.
    pub fn fetch() -> Result<Self> {
        let body = ureq::get(REGISTRY_URL)
            .call()
            .map_err(|error| AcpError::RegistryUnavailable(error.to_string()))?
            .body_mut()
            .read_to_string()
            .map_err(|error| AcpError::RegistryUnavailable(error.to_string()))?;
        let mut registry = Self::parse(&body)?;
        registry.source = RegistrySource::Network;
        Ok(registry)
    }

    /// Parse a registry payload.
    ///
    /// # Errors
    ///
    /// Fails when the JSON does not match the registry schema.
    pub fn parse(body: &str) -> Result<Self> {
        let file: RegistryFile = serde_json::from_str(body)?;
        let agents = file
            .agents
            .into_iter()
            .filter_map(|value| {
                let id = value
                    .get("id")
                    .and_then(|id| id.as_str())
                    .unwrap_or("?")
                    .to_string();
                match serde_json::from_value::<AgentDescriptor>(value) {
                    Ok(descriptor) => Some(descriptor),
                    Err(error) => {
                        tracing::warn!(%id, %error, "entrada del registro ignorada");
                        None
                    }
                }
            })
            .collect();
        Ok(Self {
            version: file.version,
            agents,
            source: RegistrySource::Network,
        })
    }

    /// Default cache location: `$XDG_CACHE_HOME/cincel/registry.json`.
    #[must_use]
    pub fn cache_path() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cincel")
            .join("registry.json")
    }

    /// Registry schema version as published by the CDN.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Where this instance came from.
    #[must_use]
    pub fn source(&self) -> RegistrySource {
        self.source
    }

    /// All known agents.
    #[must_use]
    pub fn agents(&self) -> &[AgentDescriptor] {
        &self.agents
    }

    /// Look up one agent by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&AgentDescriptor> {
        self.agents.iter().find(|agent| agent.id == id)
    }

    /// Merge user-defined agents on top of the loaded registry.
    ///
    /// A custom agent whose `id` clashes with a remote entry replaces it
    /// entirely (`docs/specs/modulos/acp.md`); it is otherwise appended.
    #[must_use]
    pub fn with_custom(mut self, custom: Vec<CustomAgent>) -> Self {
        for agent in custom {
            self.agents.retain(|existing| existing.id != agent.id);
            self.agents.push(AgentDescriptor {
                id: agent.id.clone(),
                name: agent.name.clone(),
                version: "custom".to_string(),
                description: None,
                repository: None,
                icon: None,
                distribution: Distribution {
                    custom: Some(agent),
                    ..Distribution::default()
                },
            });
        }
        self
    }

    /// Build the command needed to run `descriptor`.
    ///
    /// Channel precedence: `custom` (settings) > `npx` > `uvx` > `binary`.
    /// `npx` requires a working `node`; `uvx` requires `uv`; `binary` must
    /// already be installed via [`AgentRegistry::install`].
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::NodeMissing`], [`AcpError::UvMissing`],
    /// [`AcpError::NotInstalled`] or [`AcpError::NotSupportedYet`] (no
    /// distribution channel at all) depending on what is missing.
    pub fn launch_command(&self, descriptor: &AgentDescriptor) -> Result<LaunchSpec> {
        if let Some(custom) = &descriptor.distribution.custom {
            return Ok(LaunchSpec {
                program: custom.command.clone(),
                args: custom.args.clone(),
                env: custom.env.clone(),
            });
        }
        if let Some(npx) = &descriptor.distribution.npx {
            ensure_node_available().map_err(|_| AcpError::NodeMissing {
                found: node_major_version()
                    .map(|major| format!("node {major} (se necesita >= {MIN_NODE_MAJOR})"))
                    .unwrap_or_else(|| "no se encontró `node` en el PATH".to_string()),
            })?;
            let package = pin_version(&npx.package, &descriptor.version);
            let mut args = vec!["-y".to_string(), package];
            args.extend(npx.args.iter().cloned());
            return Ok(LaunchSpec {
                program: "npx".to_string(),
                args,
                env: npx.env.clone(),
            });
        }
        if let Some(uvx) = &descriptor.distribution.uvx {
            ensure_uv_available()?;
            let package = pin_version(&uvx.package, &descriptor.version);
            let mut args = vec![package];
            args.extend(uvx.args.iter().cloned());
            return Ok(LaunchSpec {
                program: "uvx".to_string(),
                args,
                env: uvx.env.clone(),
            });
        }
        if descriptor.distribution.binary.is_some() {
            if crate::install::is_installed(descriptor) {
                let (_, target) = crate::install::plan(descriptor)?;
                let cmd_path = crate::install::install_dir_for(&descriptor.id, &descriptor.version)
                    .join(target.cmd.strip_prefix("./").unwrap_or(&target.cmd));
                return Ok(LaunchSpec {
                    program: cmd_path.to_string_lossy().into_owned(),
                    args: target.args.clone(),
                    env: target.env.clone(),
                });
            }
            return Err(AcpError::NotInstalled {
                id: descriptor.id.clone(),
                kind: "binary".to_string(),
            });
        }
        Err(AcpError::NotSupportedYet {
            id: descriptor.id.clone(),
            kind: descriptor.distribution.kind().to_string(),
        })
    }

    /// Like [`AgentRegistry::launch_command`], but `npx` distributions run
    /// through the given `node` binary (Cincel's private runtime) instead of
    /// the global `npx`: the program is `node` itself and the first argument
    /// is npm's `npx-cli.js` from the same Node installation
    /// (`<prefix>/lib/node_modules/npm/bin/npx-cli.js` for
    /// `node = <prefix>/bin/node`). The system `node` is never probed. The
    /// caller still has to put `node`'s directory first in the child's
    /// `PATH` ([`crate::ProcessEnv::path_prepend`]) so the package's own
    /// `#!/usr/bin/env node` shebang resolves to the same runtime.
    ///
    /// Other channels behave exactly as in [`AgentRegistry::launch_command`].
    ///
    /// # Errors
    ///
    /// Same as [`AgentRegistry::launch_command`], minus `NodeMissing`.
    pub fn launch_command_with_node(
        &self,
        descriptor: &AgentDescriptor,
        node: &Path,
    ) -> Result<LaunchSpec> {
        if descriptor.distribution.custom.is_none()
            && let Some(npx) = &descriptor.distribution.npx
        {
            let package = pin_version(&npx.package, &descriptor.version);
            let mut args = vec![
                npx_cli_for_node(node).to_string_lossy().into_owned(),
                "-y".to_string(),
                package,
            ];
            args.extend(npx.args.iter().cloned());
            return Ok(LaunchSpec {
                program: node.to_string_lossy().into_owned(),
                args,
                env: npx.env.clone(),
            });
        }
        self.launch_command(descriptor)
    }

    /// Whether `descriptor`'s `binary` distribution is already installed for
    /// this platform. Always `false` for `custom`, `npx` and `uvx` channels
    /// (they need no install step).
    #[must_use]
    pub fn is_installed(&self, descriptor: &AgentDescriptor) -> bool {
        crate::install::is_installed(descriptor)
    }

    /// Download, verify and extract a `binary` distribution, then return the
    /// [`LaunchSpec`] for the extracted command.
    ///
    /// `confirm` is called with the [`crate::install::InstallPlan`] before any
    /// network access; when it returns `false` nothing is downloaded and
    /// [`AcpError::InstallDeclined`] is returned. When the registry omitted a
    /// `sha256`, `plan.verifiable` is `false` so the caller can warn the user
    /// before confirming.
    ///
    /// # Errors
    ///
    /// See [`AcpError::NoBinaryForPlatform`], [`AcpError::ChecksumMismatch`],
    /// [`AcpError::ExtractFailed`] and [`AcpError::InstallDeclined`].
    pub fn install(
        &self,
        descriptor: &AgentDescriptor,
        confirm: impl FnOnce(&crate::install::InstallPlan) -> bool,
    ) -> Result<LaunchSpec> {
        let (plan, target) = crate::install::plan(descriptor)?;
        if !confirm(&plan) {
            return Err(AcpError::InstallDeclined);
        }
        crate::install::download_and_extract(&plan, &target)
    }

    fn read_cache(path: &Path, ttl: Duration) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let age = metadata.modified().ok()?.elapsed().unwrap_or_default();
        if age > ttl {
            return None;
        }
        let body = std::fs::read_to_string(path).ok()?;
        let mut registry = Self::parse(&body).ok()?;
        registry.source = RegistrySource::Cache;
        Some(registry)
    }

    fn write_cache(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AcpError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let body = serde_json::json!({
            "version": self.version,
            "agents": self.agents,
        });
        std::fs::write(path, serde_json::to_vec(&body)?).map_err(|source| AcpError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// npm's `npx-cli.js` inside the Node installation that owns `node`
/// (`<prefix>/bin/node` -> `<prefix>/lib/node_modules/npm/bin/npx-cli.js`).
#[must_use]
pub fn npx_cli_for_node(node: &Path) -> PathBuf {
    let prefix = node
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    prefix
        .join("lib")
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npx-cli.js")
}

/// Append `@version` to a package spec unless it already pins one.
#[must_use]
pub fn pin_version(package: &str, version: &str) -> String {
    // `@scope/name` has a leading `@` that is not a version separator.
    let tail = package.strip_prefix('@').unwrap_or(package);
    if tail.contains('@') || version.is_empty() {
        package.to_string()
    } else {
        format!("{package}@{version}")
    }
}

/// Current target triple as the registry spells it, e.g. `linux-x86_64`.
#[must_use]
pub fn current_platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = std::env::consts::ARCH;
    format!("{os}-{arch}")
}

/// Major version of the `node` binary on `PATH`, if any.
#[must_use]
pub fn node_major_version() -> Option<u32> {
    let output = std::process::Command::new("node")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim().trim_start_matches('v');
    text.split('.').next()?.parse().ok()
}

/// Check that `node` is new enough for `npx` distributions.
///
/// # Errors
///
/// Returns a descriptive error when node is missing or too old.
pub fn ensure_node_available() -> Result<u32> {
    match node_major_version() {
        Some(major) if major >= MIN_NODE_MAJOR => Ok(major),
        Some(major) => Err(AcpError::RegistryUnavailable(format!(
            "node {major} es demasiado viejo, se necesita >= {MIN_NODE_MAJOR}"
        ))),
        None => Err(AcpError::RegistryUnavailable(
            "no se encontró `node` en el PATH".to_string(),
        )),
    }
}

/// Whether `uv`/`uvx` is on `PATH`, needed for `uvx` distributions.
#[must_use]
pub fn uv_available() -> bool {
    std::process::Command::new("uvx")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Check that `uvx` is available.
///
/// # Errors
///
/// Returns [`AcpError::UvMissing`] when `uvx` is not on `PATH`.
pub fn ensure_uv_available() -> Result<()> {
    if uv_available() {
        Ok(())
    } else {
        Err(AcpError::UvMissing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version": "1.0.0",
      "agents": [
        {
          "id": "claude-acp",
          "name": "Claude Agent",
          "version": "0.79.0",
          "distribution": { "npx": { "package": "@agentclientprotocol/claude-agent-acp@0.79.0" } },
          "icon": "https://example.invalid/claude.svg"
        },
        {
          "id": "sin-version",
          "name": "Sin version",
          "version": "1.2.3",
          "distribution": { "npx": { "package": "@scope/pkg", "args": ["--acp"], "env": {"A": "1"} } }
        },
        {
          "id": "amp-acp",
          "name": "Amp",
          "version": "0.9.0",
          "distribution": { "binary": { "linux-x86_64": {
            "archive": "https://example.invalid/amp.tar.gz",
            "cmd": "./amp-acp",
            "sha256": "00"
          } } }
        },
        {
          "id": "sin-sha",
          "name": "Sin sha",
          "version": "1.1.1",
          "distribution": { "binary": { "linux-x86_64": {
            "archive": "https://example.invalid/x.zip",
            "cmd": "./x",
            "args": ["--uid="]
          } } }
        },
        {
          "id": "basura",
          "name": "Sin distribution"
        }
      ]
    }"#;

    #[test]
    fn parses_all_distribution_kinds() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        // `basura` has no `distribution` and is skipped instead of failing.
        assert_eq!(registry.agents().len(), 4);
        assert!(registry.get("basura").is_none());
        assert_eq!(registry.version(), "1.0.0");
        assert!(
            registry
                .get("amp-acp")
                .expect("amp")
                .distribution
                .binary
                .is_some()
        );
    }

    #[test]
    fn npx_launch_command_keeps_pinned_version() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("claude-acp").expect("claude");
        // The system-Node path needs `node` on `PATH`; a bare CI runner may
        // not have it, and then `NodeMissing` is the only right answer.
        match registry.launch_command(descriptor) {
            Ok(spec) => {
                assert!(ensure_node_available().is_ok());
                assert_eq!(spec.program, "npx");
                assert_eq!(
                    spec.args,
                    vec!["-y", "@agentclientprotocol/claude-agent-acp@0.79.0"]
                );
            }
            Err(AcpError::NodeMissing { .. }) => {
                assert!(ensure_node_available().is_err());
                eprintln!("sin node en PATH: se comprueba solo el error");
            }
            Err(other) => panic!("error inesperado: {other:?}"),
        }
    }

    #[test]
    fn npx_launch_with_private_node_uses_node_and_npx_cli() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("sin-version").expect("sin-version");
        let node = Path::new("/data/cincel/runtime/node-v24.1.0/bin/node");
        let spec = registry
            .launch_command_with_node(descriptor, node)
            .expect("spec");
        assert_eq!(spec.program, "/data/cincel/runtime/node-v24.1.0/bin/node");
        assert_eq!(
            spec.args,
            vec![
                "/data/cincel/runtime/node-v24.1.0/lib/node_modules/npm/bin/npx-cli.js",
                "-y",
                "@scope/pkg@1.2.3",
                "--acp"
            ]
        );
        assert_eq!(spec.env.get("A").map(String::as_str), Some("1"));
    }

    #[test]
    fn private_node_launch_keeps_custom_agents_as_is() {
        let registry = AgentRegistry::parse(SAMPLE)
            .expect("parse")
            .with_custom(vec![CustomAgent {
                id: "claude-acp".to_string(),
                name: "Claude local".to_string(),
                command: "/usr/local/bin/claude-acp".to_string(),
                args: vec![],
                env: BTreeMap::new(),
            }]);
        let descriptor = registry.get("claude-acp").expect("claude");
        let spec = registry
            .launch_command_with_node(descriptor, Path::new("/x/bin/node"))
            .expect("spec");
        assert_eq!(spec.program, "/usr/local/bin/claude-acp");
    }

    #[test]
    fn no_browser_is_opt_in() {
        let spec = LaunchSpec::new("agent", Vec::new());
        assert!(!spec.env.contains_key("NO_BROWSER"));
        let spec = spec.with_no_browser();
        assert_eq!(spec.env.get("NO_BROWSER").map(String::as_str), Some("1"));
    }

    #[test]
    fn npx_launch_command_pins_missing_version_and_keeps_args() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("sin-version").expect("sin-version");
        match registry.launch_command(descriptor) {
            Ok(spec) => {
                assert!(ensure_node_available().is_ok());
                assert_eq!(spec.args, vec!["-y", "@scope/pkg@1.2.3", "--acp"]);
                assert_eq!(spec.env.get("A").map(String::as_str), Some("1"));
            }
            Err(AcpError::NodeMissing { .. }) => {
                assert!(ensure_node_available().is_err());
                eprintln!("sin node en PATH: se comprueba solo el error");
            }
            Err(other) => panic!("error inesperado: {other:?}"),
        }
    }

    #[test]
    fn binary_distribution_needs_install_first() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("amp-acp").expect("amp");
        assert!(!registry.is_installed(descriptor));
        let error = registry
            .launch_command(descriptor)
            .expect_err("no instalado");
        // Whether the registry entry matches this platform decides which of
        // the two "not ready yet" errors comes back.
        if descriptor.distribution.binary_for_this_platform().is_some() {
            assert!(matches!(error, AcpError::NotInstalled { ref kind, .. } if kind == "binary"));
        } else {
            assert!(matches!(error, AcpError::NoBinaryForPlatform { .. }));
        }
    }

    #[test]
    fn uvx_launch_command_pins_version_without_dash_y() {
        let registry = AgentRegistry::parse(
            r#"{"version":"1.0.0","agents":[{
                "id": "uvx-agent",
                "name": "Uvx",
                "version": "2.0.0",
                "distribution": { "uvx": { "package": "some-acp-agent" } }
            }]}"#,
        )
        .expect("parse");
        let descriptor = registry.get("uvx-agent").expect("uvx-agent");
        // `uvx` is a dev prerequisite here but not on a bare CI runner: the
        // launch command needs it on `PATH`, so without it the only right
        // answer is `UvMissing`, and that is what gets checked instead.
        match registry.launch_command(descriptor) {
            Ok(spec) => {
                assert!(uv_available(), "sin uvx no debería haber comando");
                assert_eq!(spec.program, "uvx");
                assert_eq!(spec.args, vec!["some-acp-agent@2.0.0"]);
            }
            Err(AcpError::UvMissing) => {
                assert!(!uv_available(), "con uvx debería haber comando");
                eprintln!("sin uvx en PATH: se comprueba solo el error");
            }
            Err(other) => panic!("error inesperado: {other:?}"),
        }
    }

    #[test]
    fn custom_agent_shadows_registry_entry_with_same_id() {
        let registry = AgentRegistry::parse(SAMPLE)
            .expect("parse")
            .with_custom(vec![CustomAgent {
                id: "claude-acp".to_string(),
                name: "Claude local".to_string(),
                command: "/usr/local/bin/claude-acp".to_string(),
                args: vec!["--stdio".to_string()],
                env: BTreeMap::new(),
            }]);
        // Still four agents: the custom entry replaced, not appended.
        assert_eq!(registry.agents().len(), 4);
        let descriptor = registry.get("claude-acp").expect("claude-acp");
        assert_eq!(descriptor.name, "Claude local");
        let spec = registry.launch_command(descriptor).expect("spec");
        assert_eq!(spec.program, "/usr/local/bin/claude-acp");
        assert_eq!(spec.args, vec!["--stdio"]);
    }

    #[test]
    fn custom_agent_without_id_clash_is_appended() {
        let registry = AgentRegistry::parse(SAMPLE)
            .expect("parse")
            .with_custom(vec![CustomAgent {
                id: "mi-agente".to_string(),
                name: "Mi agente".to_string(),
                command: "mi-agente".to_string(),
                args: vec![],
                env: BTreeMap::new(),
            }]);
        assert_eq!(registry.agents().len(), 5);
        assert!(registry.get("mi-agente").is_some());
    }

    #[test]
    fn install_plan_is_not_verifiable_without_sha256() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("sin-sha").expect("sin-sha");
        if descriptor.distribution.binary_for_this_platform().is_none() {
            return; // Not this platform's entry; nothing to assert.
        }
        let mut seen_plan = None;
        let error = registry
            .install(descriptor, |plan| {
                seen_plan = Some(plan.clone());
                false
            })
            .expect_err("declined");
        assert!(matches!(error, AcpError::InstallDeclined));
        let plan = seen_plan.expect("confirm fue llamado");
        assert!(!plan.verifiable, "sin sha256 el plan no es verificable");
        assert_eq!(plan.sha256, None);
    }

    #[test]
    fn install_plan_is_verifiable_with_sha256() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("amp-acp").expect("amp-acp");
        if descriptor.distribution.binary_for_this_platform().is_none() {
            return;
        }
        let error = registry
            .install(descriptor, |plan| {
                assert!(plan.verifiable);
                assert_eq!(plan.sha256.as_deref(), Some("00"));
                false
            })
            .expect_err("declined");
        assert!(matches!(error, AcpError::InstallDeclined));
    }

    #[test]
    fn binary_targets_may_omit_sha256() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let targets = registry
            .get("sin-sha")
            .expect("sin-sha")
            .distribution
            .binary
            .as_ref()
            .expect("binary");
        let target = targets.get("linux-x86_64").expect("target");
        assert!(target.sha256.is_none());
        assert_eq!(target.args, vec!["--uid="]);
    }

    #[test]
    fn cache_roundtrip_respects_ttl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("registry.json");
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        registry.write_cache(&path).expect("write");

        let fresh = AgentRegistry::read_cache(&path, CACHE_TTL).expect("cache");
        assert_eq!(fresh.source(), RegistrySource::Cache);
        assert_eq!(fresh.agents().len(), 4);

        assert!(AgentRegistry::read_cache(&path, Duration::ZERO).is_none());
    }
}
