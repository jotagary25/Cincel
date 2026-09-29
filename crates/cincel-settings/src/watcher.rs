//! Hot reload: a watcher over the three configuration locations.
//!
//! `docs/specs/modulos/workspace.md` asks for settings, keymap and theme to
//! reload without restarting. The watcher never reloads anything itself: it
//! says *what* changed and the caller decides when to re-read it, which keeps
//! this crate free of threads that touch the UI.
//!
//! Directories are watched, not files, because editors save configuration the
//! atomic way (write `settings.json.tmp`, rename over the original): a watch
//! on the file itself would follow the deleted inode and go silent after the
//! first save.
//!
//! Only events that mean *the content changed* are reported. `notify` asks the
//! kernel for `IN_OPEN` as well, so merely reading a watched file produces an
//! event; reporting those would make a host that re-reads its configuration on
//! every event reload forever (`docs/etapas/etapa-1.md`, hallazgo 4).

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_channel::{Receiver, Sender};
use cincel_watch::Debouncer;
use notify::{EventKind, RecursiveMode};

use crate::paths::Paths;

/// Default debounce, matching the project watcher.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(100);

/// What changed on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsEvent {
    /// `settings.json` changed.
    SettingsChanged,
    /// `keymap.json` changed.
    KeymapChanged,
    /// Some file under `themes/` changed.
    ThemeChanged,
}

/// Something went wrong starting the watcher.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// There is no home directory, so there is nothing to watch.
    #[error("no hay un directorio de configuración XDG")]
    NoConfigDir,
    /// The configuration directory could not be created.
    #[error("no se pudo crear «{path}»: {source}")]
    CreateDir {
        /// Directory that could not be created.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// `notify` refused to watch.
    #[error("no se pudo observar la configuración: {0}")]
    Notify(#[from] notify::Error),
}

/// Watches the configuration directory and reports which file changed.
///
/// The watcher lives as long as this value: drop it to stop watching.
pub struct SettingsWatcher {
    paths: Paths,
    _debouncer: Debouncer,
}

impl SettingsWatcher {
    /// Watches the real XDG configuration directory.
    pub fn new() -> Result<(SettingsWatcher, Receiver<SettingsEvent>), WatchError> {
        let paths = Paths::resolve().ok_or(WatchError::NoConfigDir)?;
        SettingsWatcher::watch(paths, DEFAULT_DEBOUNCE)
    }

    /// Watches `paths`, debouncing bursts for `debounce`.
    ///
    /// The configuration and themes directories are created when missing:
    /// `notify` cannot watch a path that does not exist, and Cincel writes
    /// there anyway.
    pub fn watch(
        paths: Paths,
        debounce: Duration,
    ) -> Result<(SettingsWatcher, Receiver<SettingsEvent>), WatchError> {
        for dir in [&paths.config_dir, &paths.themes] {
            std::fs::create_dir_all(dir).map_err(|source| WatchError::CreateDir {
                path: dir.clone(),
                source,
            })?;
        }

        let (sender, receiver) = async_channel::unbounded();
        let watched = paths.clone();
        // `cincel-watch`: no wakeups while nothing changes (M8).
        let mut debouncer = Debouncer::new(
            "cincel-watch-settings",
            debounce,
            move |batch: cincel_watch::Batch| {
                for error in batch.errors {
                    tracing::warn!(%error, "error observando la configuración");
                }
                let paths = batch
                    .events
                    .iter()
                    .filter(|event| changes_content(&event.kind))
                    .flat_map(|event| event.paths.iter());
                emit(&watched, paths, &sender);
            },
        )?;

        // The themes directory is inside the config directory, so one
        // recursive watch would cover both; watching it explicitly keeps the
        // watch alive if the user replaces the directory with a symlink.
        debouncer.watch(&paths.config_dir, RecursiveMode::NonRecursive)?;
        debouncer.watch(&paths.themes, RecursiveMode::Recursive)?;

        Ok((
            SettingsWatcher {
                paths,
                _debouncer: debouncer,
            },
            receiver,
        ))
    }

    /// The locations being watched.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }
}

/// Whether a `notify` event kind means the file may hold something new.
///
/// Opening or reading a file (`EventKind::Access`) does not, and neither does
/// whatever a backend cannot classify (`EventKind::Other`). Everything else —
/// creations, modifications, removals, and the catch-all `Any` — does.
fn changes_content(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_) | EventKind::Other)
}

/// Turns a batch of changed paths into at most one event per kind.
fn emit<'a>(
    paths: &Paths,
    changed: impl Iterator<Item = &'a PathBuf>,
    sender: &Sender<SettingsEvent>,
) {
    let mut pending: Vec<SettingsEvent> = Vec::new();
    for path in changed {
        let Some(event) = classify(paths, path) else {
            continue;
        };
        if !pending.contains(&event) {
            pending.push(event);
        }
    }
    for event in pending {
        // The channel is unbounded, so this only fails once every receiver is
        // gone, which is a normal shutdown.
        if sender.try_send(event).is_err() {
            tracing::debug!("nadie escucha los cambios de configuración");
            return;
        }
    }
}

/// Which of the three documents `path` belongs to.
fn classify(paths: &Paths, path: &Path) -> Option<SettingsEvent> {
    if path == paths.settings {
        return Some(SettingsEvent::SettingsChanged);
    }
    if path == paths.keymap {
        return Some(SettingsEvent::KeymapChanged);
    }
    if path.starts_with(&paths.themes) {
        // `themes/` itself changes when a theme is added or removed.
        return Some(SettingsEvent::ThemeChanged);
    }
    // An atomic save shows up as `settings.json.tmp` + a rename; the rename
    // lands on the real name, so temporary files are simply ignored.
    None
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Drains the channel until `wanted` shows up or `timeout` elapses.
    fn wait_for(
        receiver: &Receiver<SettingsEvent>,
        wanted: SettingsEvent,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match receiver.try_recv() {
                Ok(event) if event == wanted => return true,
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        false
    }

    #[test]
    fn reading_a_watched_file_is_not_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.themes).unwrap();
        std::fs::write(&paths.settings, "{ \"ui_font_size\": 15 }").unwrap();

        let (_watcher, events) =
            SettingsWatcher::watch(paths.clone(), Duration::from_millis(50)).unwrap();
        // Let the events of creating the files above go by.
        std::thread::sleep(Duration::from_millis(250));
        while events.try_recv().is_ok() {}

        // Exactly what a reload does: read the three documents.
        let _ = std::fs::read_to_string(&paths.settings).unwrap();
        let _ = std::fs::read_to_string(&paths.keymap);
        let _ = std::fs::read_dir(&paths.themes).map(|dir| dir.count());
        std::thread::sleep(Duration::from_millis(300));

        let seen: Vec<SettingsEvent> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        assert!(
            seen.is_empty(),
            "leer no debería avisar de cambios: {seen:?}"
        );
    }

    #[test]
    fn writing_a_watched_file_is_still_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.themes).unwrap();

        let (_watcher, events) =
            SettingsWatcher::watch(paths.clone(), Duration::from_millis(50)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        while events.try_recv().is_ok() {}

        std::fs::write(&paths.settings, "{ \"ui_font_size\": 16 }").unwrap();
        assert!(wait_for(
            &events,
            SettingsEvent::SettingsChanged,
            Duration::from_secs(5)
        ));
    }

    #[test]
    fn classifies_the_three_documents() {
        let paths = Paths::under("/cfg");
        assert_eq!(
            classify(&paths, Path::new("/cfg/settings.json")),
            Some(SettingsEvent::SettingsChanged)
        );
        assert_eq!(
            classify(&paths, Path::new("/cfg/keymap.json")),
            Some(SettingsEvent::KeymapChanged)
        );
        assert_eq!(
            classify(&paths, Path::new("/cfg/themes/mio.json")),
            Some(SettingsEvent::ThemeChanged)
        );
        assert_eq!(classify(&paths, Path::new("/cfg/otro.json")), None);
    }

    #[test]
    fn reports_every_document() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let (_watcher, receiver) =
            SettingsWatcher::watch(paths.clone(), Duration::from_millis(50)).unwrap();

        std::fs::write(&paths.settings, "{}").unwrap();
        assert!(
            wait_for(
                &receiver,
                SettingsEvent::SettingsChanged,
                Duration::from_secs(5)
            ),
            "no llegó SettingsChanged"
        );

        std::fs::write(&paths.keymap, "[]").unwrap();
        assert!(
            wait_for(
                &receiver,
                SettingsEvent::KeymapChanged,
                Duration::from_secs(5)
            ),
            "no llegó KeymapChanged"
        );

        std::fs::write(paths.themes.join("mio.json"), "{}").unwrap();
        assert!(
            wait_for(
                &receiver,
                SettingsEvent::ThemeChanged,
                Duration::from_secs(5)
            ),
            "no llegó ThemeChanged"
        );
    }

    #[test]
    fn an_atomic_save_still_reports_the_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let (_watcher, receiver) =
            SettingsWatcher::watch(paths.clone(), Duration::from_millis(50)).unwrap();

        let temp = paths.config_dir.join("settings.json.tmp");
        std::fs::write(&temp, "{ \"ui_font_size\": 20 }").unwrap();
        std::fs::rename(&temp, &paths.settings).unwrap();
        assert!(wait_for(
            &receiver,
            SettingsEvent::SettingsChanged,
            Duration::from_secs(5)
        ));
    }
}
