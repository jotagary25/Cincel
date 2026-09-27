//! Binary distribution installer: download, verify and extract prebuilt
//! agent archives into `~/.local/share/cincel/agents/<id>/<version>/`.
//!
//! `uvx` distributions need no installation step (they run straight from the
//! user's `uv` cache), so this module only deals with `binary`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{AcpError, Result};
use crate::registry::{AgentDescriptor, BinaryTarget, LaunchSpec, current_platform_key};

/// What [`crate::registry::AgentRegistry::install`] is about to do, shown to
/// the user before any bytes are downloaded.
#[derive(Debug, Clone)]
pub struct InstallPlan {
    /// Agent being installed.
    pub agent_id: String,
    /// Agent version, used as part of the install path.
    pub version: String,
    /// URL the archive will be downloaded from.
    pub archive_url: String,
    /// Directory the archive will be extracted into.
    pub install_dir: PathBuf,
    /// Expected sha256, when the registry published one.
    pub sha256: Option<String>,
    /// Whether the download can be verified. `false` when the registry entry
    /// omits `sha256`; the UI should warn the user before proceeding.
    pub verifiable: bool,
}

/// Root directory for installed agents: `$XDG_DATA_HOME/cincel/agents/`.
#[must_use]
pub fn agents_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cincel")
        .join("agents")
}

/// Install directory for one `(id, version)` pair.
#[must_use]
pub fn install_dir_for(id: &str, version: &str) -> PathBuf {
    agents_dir().join(id).join(version)
}

/// Strip a leading `./` so the target can be joined onto the install dir.
fn relative_cmd(cmd: &str) -> &str {
    cmd.strip_prefix("./").unwrap_or(cmd)
}

/// Whether `descriptor`'s binary for this platform is already installed.
#[must_use]
pub fn is_installed(descriptor: &AgentDescriptor) -> bool {
    let Some(target) = descriptor.distribution.binary_for_this_platform() else {
        return false;
    };
    install_dir_for(&descriptor.id, &descriptor.version)
        .join(relative_cmd(&target.cmd))
        .is_file()
}

/// Build the plan for installing `descriptor`'s binary on this platform.
///
/// # Errors
///
/// Returns [`AcpError::NoBinaryForPlatform`] when the registry has no entry
/// for `current_platform_key()`.
pub fn plan(descriptor: &AgentDescriptor) -> Result<(InstallPlan, BinaryTarget)> {
    let target = descriptor
        .distribution
        .binary_for_this_platform()
        .cloned()
        .ok_or_else(|| AcpError::NoBinaryForPlatform {
            id: descriptor.id.clone(),
            platform: current_platform_key(),
        })?;
    let install_dir = install_dir_for(&descriptor.id, &descriptor.version);
    let plan = InstallPlan {
        agent_id: descriptor.id.clone(),
        version: descriptor.version.clone(),
        archive_url: target.archive.clone(),
        install_dir,
        sha256: target.sha256.clone(),
        verifiable: target.sha256.is_some(),
    };
    Ok((plan, target))
}

/// Download, verify and extract the archive described by `plan`/`target`,
/// then build the [`LaunchSpec`] for the extracted binary.
///
/// # Errors
///
/// Returns [`AcpError::ChecksumMismatch`] when a published `sha256` does not
/// match the downloaded bytes, and [`AcpError::ExtractFailed`] when the
/// archive format is not `.tar.gz`/`.tgz`/`.zip` or extraction fails.
pub fn download_and_extract(plan: &InstallPlan, target: &BinaryTarget) -> Result<LaunchSpec> {
    let bytes = download(&plan.archive_url)?;

    if let Some(expected) = &plan.sha256 {
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(AcpError::ChecksumMismatch {
                url: plan.archive_url.clone(),
                expected: expected.clone(),
                actual,
            });
        }
    }

    std::fs::create_dir_all(&plan.install_dir).map_err(|source| AcpError::Io {
        path: plan.install_dir.clone(),
        source,
    })?;
    extract(&plan.archive_url, &bytes, &plan.install_dir)?;

    let cmd_path = plan.install_dir.join(relative_cmd(&target.cmd));
    mark_executable(&cmd_path)?;

    Ok(LaunchSpec {
        program: cmd_path.to_string_lossy().into_owned(),
        args: target.args.clone(),
        env: target.env.clone(),
    })
}

fn download(url: &str) -> Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|error| AcpError::RegistryUnavailable(error.to_string()))?;
    response
        .body_mut()
        .read_to_vec()
        .map_err(|error| AcpError::RegistryUnavailable(error.to_string()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn extract(url: &str, bytes: &[u8], dest: &Path) -> Result<()> {
    let lower = url.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        extract_tar_gz(bytes, dest).map_err(|error| AcpError::ExtractFailed {
            url: url.to_string(),
            message: error.to_string(),
        })
    } else if lower.ends_with(".zip") {
        extract_zip(bytes, dest).map_err(|error| AcpError::ExtractFailed {
            url: url.to_string(),
            message: error.to_string(),
        })
    } else {
        Err(AcpError::ExtractFailed {
            url: url.to_string(),
            message: "formato de archivo desconocido (se esperaba .tar.gz o .zip)".to_string(),
        })
    }
}

fn extract_tar_gz(bytes: &[u8], dest: &Path) -> std::io::Result<()> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest)
}

fn extract_zip(bytes: &[u8], dest: &Path) -> std::result::Result<(), String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|error| error.to_string())?;
    archive.extract(dest).map_err(|error| error.to_string())
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|source| AcpError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut perms = metadata.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms).map_err(|source| AcpError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn make_tar_gz(entry_name: &str, content: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, entry_name, content)
            .expect("append");
        let tar_bytes = builder.into_inner().expect("finish tar");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar_bytes).expect("gzip");
        encoder.finish().expect("finish gzip")
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // Known test vector for SHA-256 of the empty string.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(sha256_hex(b"").len(), 64);
    }

    #[test]
    fn extract_tar_gz_writes_files_and_marks_executable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = make_tar_gz("agent-bin", b"#!/bin/sh\necho hi\n");
        extract_tar_gz(&archive, dir.path()).expect("extract");
        let extracted = dir.path().join("agent-bin");
        assert!(extracted.is_file());
        mark_executable(&extracted).expect("chmod");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&extracted)
                .expect("meta")
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "el binario debe quedar ejecutable");
        }
    }

    #[test]
    fn extract_rejects_unknown_format() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = extract("https://example.invalid/agent.rar", b"junk", dir.path())
            .expect_err("formato desconocido");
        assert!(matches!(error, AcpError::ExtractFailed { .. }));
    }
}
