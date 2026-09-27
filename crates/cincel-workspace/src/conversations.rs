//! Where a project's conversations live
//! (`docs/etapas/etapa-2.md` § correcciones, "Conversaciones").
//!
//! One file per conversation under
//! `~/.local/state/cincel/workspaces/<hash>/conversations/<id>.json`, plus an
//! `index.json` with just enough of each to paint the history popover without
//! reading every transcript. The single `chat.json` of the first version of
//! Etapa 2 is migrated into one conversation the first time a project is
//! opened, and then renamed out of the way.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use cincel_chat::{
    CONVERSATION_VERSION, Conversation, ConversationSummary, LEGACY_CONNECTION_GROUP,
    TranscriptDump, conversation_title,
};
use serde::{Deserialize, Serialize};

/// Directory, under the project's state directory, holding the conversations.
const CONVERSATIONS_DIR: &str = "conversations";
/// The index that lists them.
const INDEX_FILE: &str = "index.json";
/// The single transcript of the previous format.
const LEGACY_FILE: &str = "chat.json";
/// Where the legacy transcript is moved once it has been migrated; it is kept
/// rather than deleted so a bad migration is recoverable by hand.
const LEGACY_BACKUP: &str = "chat.json.migrado";

/// One row of `index.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexEntry {
    /// Conversation id, which is also its file name.
    pub id: String,
    /// Registry id of the agent.
    pub agent_id: String,
    /// Name of that agent, for the popover's group header.
    pub agent_name: String,
    /// First user message, already cut to length.
    pub title: String,
    /// Seconds since the Unix epoch.
    pub updated_at: u64,
    /// ACP session id, when the conversation had one.
    pub session_id: Option<String>,
    /// The connection it belongs to (`None`: saved before connections
    /// existed, read-only).
    #[serde(default)]
    pub connection_id: Option<String>,
}

impl IndexEntry {
    /// The row the chat panel paints, with `when` already formatted. The
    /// group is the stored label; the workspace replaces it with the
    /// connection's current label when it still exists.
    #[must_use]
    pub fn summary(&self, when: String) -> ConversationSummary {
        ConversationSummary {
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            agent_name: self.agent_name.clone(),
            title: self.title.clone(),
            when,
            session_id: self.session_id.clone(),
            connection_id: self.connection_id.clone(),
            group: if self.connection_id.is_some() {
                self.agent_name.clone()
            } else {
                LEGACY_CONNECTION_GROUP.to_string()
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    version: u32,
    conversations: Vec<IndexEntry>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            version: CONVERSATION_VERSION,
            conversations: Vec::new(),
        }
    }
}

/// The conversations of one project.
#[derive(Clone, Debug)]
pub struct ConversationStore {
    dir: PathBuf,
    legacy: PathBuf,
    legacy_backup: PathBuf,
    root: PathBuf,
}

impl ConversationStore {
    /// The store of the project rooted at `root`, or `None` when the platform
    /// has no state directory.
    #[must_use]
    pub fn for_project(root: &Path) -> Option<Self> {
        let base = workspaces_dir()?.join(crate::layout::project_hash(root));
        Some(Self::at(base, root.to_path_buf()))
    }

    /// The store whose project state directory is `base`.
    fn at(base: PathBuf, root: PathBuf) -> Self {
        Self {
            dir: base.join(CONVERSATIONS_DIR),
            legacy: base.join(LEGACY_FILE),
            legacy_backup: base.join(LEGACY_BACKUP),
            root,
        }
    }

    /// The store of every project that ever saved a conversation, for what
    /// spans projects: deleting a connection forgets its conversations
    /// everywhere (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4 F3).
    #[must_use]
    pub fn all() -> Vec<Self> {
        let Some(workspaces) = workspaces_dir() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&workspaces) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|base| base.join(CONVERSATIONS_DIR).is_dir())
            .map(|base| Self::at(base.clone(), base))
            .collect()
    }

    /// How many conversations of this project belong to `connection_id`.
    #[must_use]
    pub fn count_for_connection(&self, connection_id: &str) -> usize {
        self.read_index()
            .conversations
            .iter()
            .filter(|entry| entry.connection_id.as_deref() == Some(connection_id))
            .count()
    }

    /// Deletes every conversation of this project that belongs to
    /// `connection_id` (files and index rows). Returns how many went.
    pub fn delete_for_connection(&self, connection_id: &str) -> usize {
        let mut index = self.read_index();
        let (gone, kept): (Vec<IndexEntry>, Vec<IndexEntry>) = index
            .conversations
            .into_iter()
            .partition(|entry| entry.connection_id.as_deref() == Some(connection_id));
        for entry in &gone {
            let _ = std::fs::remove_file(self.path_of(&entry.id));
        }
        index.conversations = kept;
        if !gone.is_empty()
            && let Err(error) = self.write_index(&index)
        {
            tracing::warn!(%error, "no se pudo reescribir index.json tras borrar conversaciones");
        }
        gone.len()
    }

    /// The directory the conversations are written to.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The conversations of this project, newest first.
    #[must_use]
    pub fn index(&self) -> Vec<IndexEntry> {
        let mut entries = self.read_index().conversations;
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
        entries
    }

    /// Reads one conversation, or `None` when it is gone or unreadable.
    #[must_use]
    pub fn load(&self, id: &str) -> Option<Conversation> {
        let raw = std::fs::read_to_string(self.path_of(id)).ok()?;
        match serde_json::from_str::<Conversation>(&raw) {
            Ok(conversation) => Some(conversation),
            Err(error) => {
                tracing::warn!(%error, id, "no se pudo leer la conversación guardada");
                None
            }
        }
    }

    /// Writes `conversation` and refreshes its row in the index.
    pub fn save(&self, conversation: &Conversation) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let serialized = serde_json::to_string_pretty(conversation)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        std::fs::write(self.path_of(&conversation.id), serialized)?;
        let mut index = self.read_index();
        let row = IndexEntry {
            id: conversation.id.clone(),
            agent_id: conversation.agent_id.clone(),
            agent_name: conversation.agent_name.clone(),
            title: conversation.title.clone(),
            updated_at: conversation.updated_at,
            session_id: conversation.session_id.clone(),
            connection_id: conversation.connection_id.clone(),
        };
        match index
            .conversations
            .iter_mut()
            .find(|entry| entry.id == conversation.id)
        {
            Some(existing) => *existing = row,
            None => index.conversations.push(row),
        }
        self.write_index(&index)
    }

    /// Removes a conversation, returning what was removed so the undo toast
    /// can put it back.
    pub fn delete(&self, id: &str) -> Option<Conversation> {
        let conversation = self.load(id);
        let _ = std::fs::remove_file(self.path_of(id));
        let mut index = self.read_index();
        let before = index.conversations.len();
        index.conversations.retain(|entry| entry.id != id);
        if index.conversations.len() != before {
            let _ = self.write_index(&index);
        }
        conversation
    }

    /// Turns a leftover `chat.json` into one conversation and renames it.
    ///
    /// Returns the id of the migrated conversation, or `None` when there was
    /// nothing to migrate. Runs once per project: the legacy file is renamed
    /// to `chat.json.migrado` right after, so a second call does nothing.
    pub fn migrate_legacy(&self) -> Option<String> {
        if !self.legacy.is_file() {
            return None;
        }
        let raw = std::fs::read_to_string(&self.legacy).ok()?;
        let dump: TranscriptDump = match serde_json::from_str(&raw) {
            Ok(dump) => dump,
            Err(error) => {
                tracing::warn!(%error, "el chat.json anterior no se pudo leer; se deja como está");
                return None;
            }
        };
        let when = std::fs::metadata(&self.legacy)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .map(seconds_since_epoch)
            .unwrap_or_else(now_seconds);
        let agent_id = dump.agent_id.clone().unwrap_or_default();
        let conversation = Conversation {
            version: CONVERSATION_VERSION,
            id: new_conversation_id(),
            agent_name: agent_id.clone(),
            agent_id,
            connection_id: None,
            session_id: dump.session_id.clone(),
            cwd: self.root.clone(),
            created_at: when,
            updated_at: when,
            title: conversation_title(&dump.entries),
            entries: dump.entries,
        };
        if let Err(error) = self.save(&conversation) {
            tracing::warn!(%error, "no se pudo migrar el chat.json anterior");
            return None;
        }
        if let Err(error) = std::fs::rename(&self.legacy, &self.legacy_backup) {
            tracing::warn!(%error, "no se pudo renombrar el chat.json migrado");
        }
        Some(conversation.id)
    }

    fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn read_index(&self) -> Index {
        let path = self.dir.join(INDEX_FILE);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Index::default();
        };
        serde_json::from_str(&raw).unwrap_or_else(|error| {
            tracing::warn!(%error, path = %path.display(), "index.json ilegible; se reconstruye");
            Index::default()
        })
    }

    fn write_index(&self, index: &Index) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let serialized = serde_json::to_string_pretty(index)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        std::fs::write(self.dir.join(INDEX_FILE), serialized)
    }
}

/// `~/.local/state/cincel/workspaces`, where every project keeps its state.
fn workspaces_dir() -> Option<PathBuf> {
    Some(dirs::state_dir()?.join("cincel").join("workspaces"))
}

/// How many conversations of `connection_id` exist in any project (the "sus
/// N conversaciones" of the delete confirmation).
#[must_use]
pub fn count_for_connection_everywhere(connection_id: &str) -> usize {
    ConversationStore::all()
        .iter()
        .map(|store| store.count_for_connection(connection_id))
        .sum()
}

/// Deletes the conversations of `connection_id` in every project. Returns how
/// many went.
pub fn delete_for_connection_everywhere(connection_id: &str) -> usize {
    ConversationStore::all()
        .iter()
        .map(|store| store.delete_for_connection(connection_id))
        .sum()
}

/// Seconds since the Unix epoch, right now.
#[must_use]
pub fn now_seconds() -> u64 {
    seconds_since_epoch(SystemTime::now())
}

fn seconds_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// A fresh conversation id, shaped like a UUID so it reads as one in a
/// directory listing. The clock gives it its order and a counter keeps two
/// conversations created in the same nanosecond apart.
#[must_use]
pub fn new_conversation_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default() as u64;
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let hex = format!("{nanos:016x}{counter:016x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// A rough Spanish "hace…" for a stored conversation; the chat crate needs no
/// clock of its own (`chat.md`), so this crate supplies one.
#[must_use]
pub fn format_when(updated_at: u64) -> String {
    let now = now_seconds();
    let elapsed = now.saturating_sub(updated_at);
    match elapsed {
        0..=59 => "hace un momento".to_string(),
        60..=3_599 => format!("hace {} min", elapsed / 60),
        3_600..=86_399 => format!("hace {} h", elapsed / 3_600),
        86_400..=172_799 => "ayer".to_string(),
        _ => format!("hace {} d", elapsed / 86_400),
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_uuid_shaped() {
        let first = new_conversation_id();
        let second = new_conversation_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 36);
        assert_eq!(first.matches('-').count(), 4);
    }

    #[test]
    fn relative_times_read_like_spanish() {
        let now = now_seconds();
        assert_eq!(format_when(now), "hace un momento");
        assert_eq!(format_when(now - 300), "hace 5 min");
        assert_eq!(format_when(now - 7_200), "hace 2 h");
        assert_eq!(format_when(now - 90_000), "ayer");
        assert_eq!(format_when(now - 3 * 86_400), "hace 3 d");
    }
}
