//! Agent registry: download, cache and resolve launch commands.
//!
//! The registry lives at
//! `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json` and is
//! cached under `$XDG_CACHE_HOME/asteroid/registry.json` with a 24 h TTL.
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
    /// Run through `npx`. The only channel E0 can launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx: Option<NpxDistribution>,
    /// Run through `uvx` (Python). Parsed but not launchable in E0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uvx: Option<UvxDistribution>,
    /// Prebuilt binaries keyed by `<os>-<arch>`. Parsed but not launchable in E0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<BTreeMap<String, BinaryTarget>>,
}

impl Distribution {
    /// Short name of the preferred available channel, used in error messages.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        if self.npx.is_some() {
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

    /// Default cache location: `$XDG_CACHE_HOME/asteroid/registry.json`.
    #[must_use]
    pub fn cache_path() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("asteroid")
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

    /// Build the command needed to run `descriptor`.
    ///
    /// Only `npx` distributions are supported in E0.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::NotSupportedYet`] for `binary` and `uvx`.
    pub fn launch_command(&self, descriptor: &AgentDescriptor) -> Result<LaunchSpec> {
        match &descriptor.distribution.npx {
            Some(npx) => {
                let package = pin_version(&npx.package, &descriptor.version);
                let mut args = vec!["-y".to_string(), package];
                args.extend(npx.args.iter().cloned());
                Ok(LaunchSpec {
                    program: "npx".to_string(),
                    args,
                    env: npx.env.clone(),
                })
            }
            None => Err(AcpError::NotSupportedYet {
                id: descriptor.id.clone(),
                kind: descriptor.distribution.kind().to_string(),
            }),
        }
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

/// Append `@version` to a package spec unless it already pins one.
fn pin_version(package: &str, version: &str) -> String {
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
        let spec = registry.launch_command(descriptor).expect("spec");
        assert_eq!(spec.program, "npx");
        assert_eq!(
            spec.args,
            vec!["-y", "@agentclientprotocol/claude-agent-acp@0.79.0"]
        );
    }

    #[test]
    fn npx_launch_command_pins_missing_version_and_keeps_args() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("sin-version").expect("sin-version");
        let spec = registry.launch_command(descriptor).expect("spec");
        assert_eq!(spec.args, vec!["-y", "@scope/pkg@1.2.3", "--acp"]);
        assert_eq!(spec.env.get("A").map(String::as_str), Some("1"));
    }

    #[test]
    fn binary_distribution_is_not_supported_yet() {
        let registry = AgentRegistry::parse(SAMPLE).expect("parse");
        let descriptor = registry.get("amp-acp").expect("amp");
        let error = registry
            .launch_command(descriptor)
            .expect_err("no soportado");
        assert!(matches!(error, AcpError::NotSupportedYet { ref kind, .. } if kind == "binary"));
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
