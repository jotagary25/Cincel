//! Isolated, comparable app profiles (D5, §3.5): a fresh folder per run, no
//! extensions, no language servers, no telemetry or updates, no cursor blink.
//! `HOME` and the XDG folders point inside the profile too, so nothing of the
//! real user's configuration is read or written.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// The apps the bench knows how to isolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Cincel,
    Zed,
    Antigravity,
    VsCode,
}

impl AppKind {
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "cincel" => AppKind::Cincel,
            "zed" => AppKind::Zed,
            "antigravity" => AppKind::Antigravity,
            "vscode" | "code" => AppKind::VsCode,
            other => bail!("app desconocida: {other} (cincel, zed, antigravity o vscode)"),
        })
    }

    /// The Wayland `app_id` of its window, when known in advance.
    pub fn default_app_id(self) -> Option<&'static str> {
        match self {
            AppKind::Cincel => Some("dev.cincel.Cincel"),
            AppKind::Zed => Some("dev.zed.Zed"),
            // Electron derives it from the executable/desktop name; the first
            // new window is taken and its app_id recorded.
            AppKind::Antigravity | AppKind::VsCode => None,
        }
    }
}

/// Environment and extra arguments that isolate one run of an app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    pub env: Vec<(String, String)>,
    /// Arguments placed right after the executable, before the user's.
    pub args: Vec<String>,
}

pub const CINCEL_SETTINGS: &str = "{\n  \"editor\": {\n    \"cursor_blink\": false\n  }\n}\n";

pub const ZED_SETTINGS: &str = r#"{
  "enable_language_server": false,
  "auto_update": false,
  "telemetry": { "diagnostics": false, "metrics": false },
  "cursor_blink": false,
  "disable_ai": true,
  "features": { "edit_prediction_provider": "none" },
  "show_edit_predictions": false,
  "session": { "trust_all_worktrees": true }
}
"#;

/// Application-scope storage key that marks Antigravity IDE's onboarding
/// as done.
pub const ANTIGRAVITY_ONBOARDING_KEY: &str = "antigravityOnboarding";

pub const CODE_SETTINGS: &str = r#"{
  "editor.cursorBlinking": "solid",
  "telemetry.telemetryLevel": "off",
  "update.mode": "none",
  "workbench.startupEditor": "none",
  "workbench.secondarySideBar.defaultVisibility": "hidden"
}
"#;

fn write(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creando {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("escribiendo {}", path.display()))
}

/// Creates a fresh profile for `kind` in `dir` (which must not exist or be
/// empty: a profile is never reused) and returns how to launch with it.
pub fn prepare(kind: AppKind, dir: &Path) -> Result<Profile> {
    if dir.exists() && fs::read_dir(dir)?.next().is_some() {
        bail!("la carpeta de perfil {} ya tiene contenido", dir.display());
    }
    let path = |relative: &str| dir.join(relative).to_string_lossy().into_owned();
    for sub in ["home", "config", "data", "state", "cache"] {
        fs::create_dir_all(dir.join(sub))?;
    }
    let mut profile = Profile {
        env: vec![
            ("HOME".into(), path("home")),
            ("XDG_CONFIG_HOME".into(), path("config")),
            ("XDG_DATA_HOME".into(), path("data")),
            ("XDG_STATE_HOME".into(), path("state")),
            ("XDG_CACHE_HOME".into(), path("cache")),
        ],
        args: Vec::new(),
    };
    match kind {
        AppKind::Cincel => {
            write(&dir.join("config/cincel/settings.json"), CINCEL_SETTINGS)?;
            profile
                .env
                .push(("CINCEL_CONFIG_DIR".into(), path("config/cincel")));
        }
        AppKind::Zed => {
            // `trust_all_worktrees` skips the "Restricted Mode" dialog, the
            // Zed counterpart of `--disable-workspace-trust` (the corpus is
            // synthetic and language servers are off anyway).
            // `--user-data-dir` keeps config under `<dir>/config`; the XDG
            // copy covers versions that read `$XDG_CONFIG_HOME/zed`.
            write(&dir.join("zed-data/config/settings.json"), ZED_SETTINGS)?;
            write(&dir.join("config/zed/settings.json"), ZED_SETTINGS)?;
            profile.args = vec!["--user-data-dir".into(), path("zed-data")];
        }
        AppKind::Antigravity | AppKind::VsCode => {
            write(&dir.join("user-data/User/settings.json"), CODE_SETTINGS)?;
            if kind == AppKind::Antigravity {
                // Without this key a fresh profile opens a sign-in onboarding
                // instead of the editor (see `vscdb`).
                let db = crate::vscdb::item_table(&[(ANTIGRAVITY_ONBOARDING_KEY, "true")])?;
                let path = dir.join("user-data/User/globalStorage/state.vscdb");
                fs::create_dir_all(path.parent().unwrap_or(dir))?;
                fs::write(&path, db).with_context(|| format!("escribiendo {}", path.display()))?;
            }
            fs::create_dir_all(dir.join("ext"))?;
            profile.args = vec![
                "--user-data-dir".into(),
                path("user-data"),
                "--extensions-dir".into(),
                path("ext"),
                "--disable-extensions".into(),
                "--disable-workspace-trust".into(),
                "--skip-welcome".into(),
                "--ozone-platform=wayland".into(),
            ];
        }
    }
    Ok(profile)
}
