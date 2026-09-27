//! [`ConnectionStore`]: `connections.json` plus one private profile directory
//! per connection (`docs/specs/06-etapa4-conexiones-y-cincel.md` §2).
//!
//! The index never holds tokens or credentials: those are whatever the agent
//! itself writes inside its profile directory.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ConnectionsError, Result, io_err};
use crate::paths::{CincelPaths, create_private_dir, remove_dir_within, write_atomic};
use crate::profile::{AgentKind, Profile};

/// Identity the agent reported for a connection (only when it does).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Account email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Plan string as the agent reports it ("Claude Max", "plus"...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Organization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
}

impl Identity {
    /// Whether no field is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.email.is_none() && self.plan.is_none() && self.organization.is_none()
    }

    /// Gray line for the popover: `"gary@… · Max"`-style, `None` when empty.
    #[must_use]
    pub fn summary(&self) -> Option<String> {
        let parts: Vec<&str> = [self.email.as_deref(), self.plan.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if parts.is_empty() {
            self.organization.clone()
        } else {
            Some(parts.join(" · "))
        }
    }
}

/// One saved connection. Status is not stored: see
/// [`crate::Connections::status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// Also the name of the profile directory.
    pub id: Uuid,
    /// Registry id: `claude-acp`, `codex-acp` or `antigravity-acp`.
    pub agent_id: String,
    /// User-chosen label, never empty.
    pub label: String,
    /// Identity, when the agent reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    /// Creation time, seconds since the Unix epoch.
    pub created_at: u64,
    /// Last activation time, seconds since the Unix epoch.
    pub last_used_at: u64,
}

impl Connection {
    /// The agent kind (the index only ever holds supported ids).
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnsupportedAgent`] if the index was edited by hand.
    pub fn kind(&self) -> Result<AgentKind> {
        AgentKind::from_agent_id(&self.agent_id)
    }
}

/// A profile directory created for a login in progress, not yet in the
/// index (spec 06 §4 F2: "Recién ahí se persiste la conexión").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingConnection {
    /// Future connection id.
    pub id: Uuid,
    /// Agent being connected.
    pub kind: AgentKind,
    /// The seeded, empty profile.
    pub profile: Profile,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexFile {
    #[serde(default = "index_version")]
    version: u32,
    #[serde(default)]
    connections: Vec<Connection>,
}

fn index_version() -> u32 {
    1
}

/// Current time in seconds since the Unix epoch.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// The connection index and profile directories.
#[derive(Debug, Clone)]
pub struct ConnectionStore {
    paths: CincelPaths,
}

impl ConnectionStore {
    /// Store rooted at `paths` (nothing is touched on disk until needed).
    #[must_use]
    pub fn new(paths: CincelPaths) -> Self {
        Self { paths }
    }

    /// Roots in use.
    #[must_use]
    pub fn paths(&self) -> &CincelPaths {
        &self.paths
    }

    /// Profile directory for a connection id.
    #[must_use]
    pub fn profile_dir(&self, id: Uuid) -> PathBuf {
        self.paths.connections_dir().join(id.to_string())
    }

    /// Profile of a saved connection.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnsupportedAgent`] for a hand-edited index.
    pub fn profile(&self, connection: &Connection) -> Result<Profile> {
        Ok(Profile::new(
            connection.kind()?,
            self.profile_dir(connection.id),
        ))
    }

    fn read_index(&self) -> Result<IndexFile> {
        let path = self.paths.index_file();
        match std::fs::read(&path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(IndexFile {
                version: index_version(),
                connections: Vec::new(),
            }),
            Err(error) => Err(io_err(path)(error)),
        }
    }

    fn write_index(&self, index: &IndexFile) -> Result<()> {
        create_private_dir(&self.paths.data_dir)?;
        write_atomic(&self.paths.index_file(), &serde_json::to_vec_pretty(index)?)
    }

    fn update<T>(&self, change: impl FnOnce(&mut Vec<Connection>) -> Result<T>) -> Result<T> {
        let mut index = self.read_index()?;
        let out = change(&mut index.connections)?;
        self.write_index(&index)?;
        Ok(out)
    }

    /// Every saved connection, most recently used first.
    ///
    /// # Errors
    ///
    /// I/O or parse errors reading the index.
    pub fn list(&self) -> Result<Vec<Connection>> {
        let mut connections = self.read_index()?.connections;
        connections.sort_by_key(|connection| std::cmp::Reverse(connection.last_used_at));
        Ok(connections)
    }

    /// One connection by id.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnknownConnection`] when absent.
    pub fn get(&self, id: Uuid) -> Result<Connection> {
        self.read_index()?
            .connections
            .into_iter()
            .find(|connection| connection.id == id)
            .ok_or_else(|| ConnectionsError::UnknownConnection(id.to_string()))
    }

    /// Create and seed an empty private profile for a new login. Nothing is
    /// added to the index until [`ConnectionStore::finalize`].
    ///
    /// # Errors
    ///
    /// Unsupported agent id or I/O errors.
    pub fn create_pending(&self, agent_id: &str) -> Result<PendingConnection> {
        let kind = AgentKind::from_agent_id(agent_id)?;
        create_private_dir(&self.paths.data_dir)?;
        create_private_dir(&self.paths.connections_dir())?;
        let id = Uuid::new_v4();
        let dir = self.profile_dir(id);
        create_private_dir(&dir)?;
        let profile = Profile::new(kind, dir);
        profile.seed()?;
        Ok(PendingConnection { id, kind, profile })
    }

    /// Persist a pending connection once the login succeeded and the user
    /// named it.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::EmptyLabel`], [`ConnectionsError::UnknownPending`]
    /// if the profile directory is gone, I/O errors.
    pub fn finalize(
        &self,
        pending: &PendingConnection,
        label: &str,
        identity: Option<Identity>,
    ) -> Result<Connection> {
        let label = normalize_label(label)?;
        if !pending.profile.dir().is_dir() {
            return Err(ConnectionsError::UnknownPending(pending.id.to_string()));
        }
        let now = now_secs();
        let connection = Connection {
            id: pending.id,
            agent_id: pending.kind.agent_id().to_string(),
            label,
            identity: identity.filter(|identity| !identity.is_empty()),
            created_at: now,
            last_used_at: now,
        };
        let stored = connection.clone();
        self.update(move |connections| {
            connections.retain(|existing| existing.id != stored.id);
            connections.push(stored);
            Ok(())
        })?;
        Ok(connection)
    }

    /// Drop a pending profile (login cancelled or failed). Refuses ids that
    /// are already saved connections.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnsafePath`] or I/O errors.
    pub fn discard_pending(&self, id: Uuid) -> Result<()> {
        if self.get(id).is_ok() {
            return Ok(());
        }
        remove_dir_within(&self.paths.connections_dir(), &self.profile_dir(id)).map(|_| ())
    }

    /// Change a connection's label.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::EmptyLabel`], [`ConnectionsError::UnknownConnection`].
    pub fn rename(&self, id: Uuid, label: &str) -> Result<Connection> {
        let label = normalize_label(label)?;
        self.update(|connections| {
            let connection = find_mut(connections, id)?;
            connection.label = label;
            Ok(connection.clone())
        })
    }

    /// Mark a connection as just used.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnknownConnection`].
    pub fn touch(&self, id: Uuid) -> Result<Connection> {
        self.update(|connections| {
            let connection = find_mut(connections, id)?;
            connection.last_used_at = now_secs().max(connection.last_used_at);
            Ok(connection.clone())
        })
    }

    /// Replace the stored identity (after "Volver a conectar" or a status
    /// push).
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnknownConnection`].
    pub fn set_identity(&self, id: Uuid, identity: Option<Identity>) -> Result<Connection> {
        self.update(|connections| {
            let connection = find_mut(connections, id)?;
            connection.identity = identity.filter(|identity| !identity.is_empty());
            Ok(connection.clone())
        })
    }

    /// Delete a connection: its profile directory (validated to be inside
    /// the connections root) and its index entry. Returns whether the
    /// directory existed. Logging out of the agent first is
    /// [`crate::disconnect`]'s job.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnknownConnection`], [`ConnectionsError::UnsafePath`].
    pub fn delete(&self, id: Uuid) -> Result<bool> {
        self.get(id)?;
        let removed = self.remove_profile_dir(id)?;
        self.remove_from_index(id)?;
        Ok(removed)
    }

    /// Remove only the profile directory of `id` (validated).
    pub(crate) fn remove_profile_dir(&self, id: Uuid) -> Result<bool> {
        remove_dir_within(&self.paths.connections_dir(), &self.profile_dir(id))
    }

    /// Remove only the index entry of `id`.
    pub(crate) fn remove_from_index(&self, id: Uuid) -> Result<()> {
        self.update(|connections| {
            connections.retain(|connection| connection.id != id);
            Ok(())
        })
    }
}

fn find_mut(connections: &mut [Connection], id: Uuid) -> Result<&mut Connection> {
    connections
        .iter_mut()
        .find(|connection| connection.id == id)
        .ok_or_else(|| ConnectionsError::UnknownConnection(id.to_string()))
}

fn normalize_label(label: &str) -> Result<String> {
    let label = label.trim();
    if label.is_empty() {
        Err(ConnectionsError::EmptyLabel)
    } else {
        Ok(label.to_string())
    }
}

/// Suggested label for a new connection: `"Claude · personal"`, or
/// `"Claude · personal 2"` when taken.
#[must_use]
pub fn suggest_label(kind: AgentKind, existing: &[Connection]) -> String {
    let base = format!("{} · personal", kind.display_name());
    if !existing.iter().any(|connection| connection.label == base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| {
            !existing
                .iter()
                .any(|connection| &connection.label == candidate)
        })
        .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ConnectionStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ConnectionStore::new(CincelPaths::under(dir.path()));
        (dir, store)
    }

    #[test]
    fn empty_store_lists_nothing_and_touches_no_disk() {
        let (dir, store) = store();
        assert!(store.list().expect("list").is_empty());
        assert!(!dir.path().join("data").exists());
    }

    #[test]
    fn pending_profile_is_private_and_not_indexed() {
        let (_dir, store) = store();
        let pending = store.create_pending("codex-acp").expect("pending");
        assert!(pending.profile.dir().is_dir());
        assert!(pending.profile.dir().join("config.toml").is_file());
        assert!(store.list().expect("list").is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(pending.profile.dir())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        store.discard_pending(pending.id).expect("discard");
        assert!(!pending.profile.dir().exists());
    }

    #[test]
    fn finalize_rename_touch_and_delete() {
        let (_dir, store) = store();
        let pending = store.create_pending("claude-acp").expect("pending");
        assert!(matches!(
            store.finalize(&pending, "   ", None),
            Err(ConnectionsError::EmptyLabel)
        ));
        let identity = Identity {
            email: Some("gary@example.com".to_string()),
            plan: Some("Claude Max".to_string()),
            organization: None,
        };
        let connection = store
            .finalize(&pending, " Claude · personal ", Some(identity.clone()))
            .expect("finalize");
        assert_eq!(connection.label, "Claude · personal");
        assert_eq!(connection.identity, Some(identity));
        assert_eq!(store.list().expect("list").len(), 1);

        let renamed = store.rename(connection.id, "Trabajo").expect("rename");
        assert_eq!(renamed.label, "Trabajo");
        assert_eq!(store.get(connection.id).expect("get").label, "Trabajo");
        let touched = store.touch(connection.id).expect("touch");
        assert!(touched.last_used_at >= connection.last_used_at);

        let profile_dir = store.profile_dir(connection.id);
        assert!(store.delete(connection.id).expect("delete"));
        assert!(!profile_dir.exists());
        assert!(store.list().expect("list").is_empty());
        assert!(matches!(
            store.get(connection.id),
            Err(ConnectionsError::UnknownConnection(_))
        ));
    }

    #[test]
    fn index_never_contains_credentials() {
        let (_dir, store) = store();
        let pending = store.create_pending("claude-acp").expect("pending");
        std::fs::write(
            pending.profile.credentials_file(),
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat-SECRETO"}}"#,
        )
        .expect("write creds");
        store.finalize(&pending, "Claude", None).expect("finalize");
        let index = std::fs::read_to_string(store.paths().index_file()).expect("index");
        assert!(!index.contains("SECRETO"));
        assert!(!index.contains("accessToken"));
        let value: serde_json::Value = serde_json::from_str(&index).expect("json");
        assert_eq!(value["version"], 1);
        assert_eq!(value["connections"][0]["agent_id"], "claude-acp");
    }

    #[test]
    fn delete_keeps_other_connections() {
        let (_dir, store) = store();
        let a = store.create_pending("codex-acp").expect("a");
        let b = store.create_pending("codex-acp").expect("b");
        let a = store.finalize(&a, "Codex · personal", None).expect("a");
        let b = store.finalize(&b, "Codex · trabajo", None).expect("b");
        store.delete(a.id).expect("delete");
        let left = store.list().expect("list");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, b.id);
        assert!(store.profile_dir(b.id).is_dir());
    }

    #[test]
    fn discard_pending_never_deletes_a_saved_connection() {
        let (_dir, store) = store();
        let pending = store.create_pending("antigravity-acp").expect("pending");
        let saved = store
            .finalize(&pending, "Antigravity", None)
            .expect("finalize");
        store.discard_pending(saved.id).expect("no-op");
        assert!(store.profile_dir(saved.id).is_dir());
    }

    #[test]
    fn identity_updates_and_empty_identity_is_dropped() {
        let (_dir, store) = store();
        let pending = store.create_pending("codex-acp").expect("pending");
        let saved = store
            .finalize(&pending, "Codex", Some(Identity::default()))
            .expect("finalize");
        assert_eq!(saved.identity, None);
        let identity = Identity {
            email: Some("x@example.com".to_string()),
            ..Identity::default()
        };
        let updated = store
            .set_identity(saved.id, Some(identity.clone()))
            .expect("set");
        assert_eq!(updated.identity, Some(identity));
    }

    #[test]
    fn list_is_most_recent_first_and_labels_are_suggested() {
        let (_dir, store) = store();
        let first = store.create_pending("claude-acp").expect("a");
        let first = store
            .finalize(&first, "Claude · personal", None)
            .expect("a");
        let second = store.create_pending("claude-acp").expect("b");
        let second = store.finalize(&second, "Otra", None).expect("b");
        // Force an older timestamp on the second one through the index.
        store
            .update(|connections| {
                if let Some(entry) = connections.iter_mut().find(|c| c.id == second.id) {
                    entry.last_used_at = 1;
                }
                Ok(())
            })
            .expect("update");
        let list = store.list().expect("list");
        assert_eq!(list[0].id, first.id);
        assert_eq!(
            suggest_label(AgentKind::Claude, &list),
            "Claude · personal 2"
        );
        assert_eq!(suggest_label(AgentKind::Codex, &list), "Codex · personal");
        assert_eq!(
            suggest_label(AgentKind::Antigravity, &list),
            "Antigravity · personal"
        );
        let summary = Identity {
            email: Some("gary@example.com".to_string()),
            plan: Some("Max".to_string()),
            organization: None,
        }
        .summary();
        assert_eq!(summary.as_deref(), Some("gary@example.com · Max"));
    }
}
