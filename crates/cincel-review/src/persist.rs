//! Persistence per `03-arquitectura.md §6`.
//!
//! ```text
//! <dir>/state.json          files in review and their metadata
//! <dir>/objects/<sha256>    content-addressed texts (bases, previous texts)
//! ```
//!
//! Each entry stores `base_hash` (object), `current_hash` (hash of the text
//! the buffer had when saved, `null` for a deleted file), `status`,
//! `previous_hash`, `turn_id` and `created_at`. Hashes are SHA-256 of the
//! LF-normalized UTF-8 text.
//!
//! Restoring **never writes a file**: the current text is read as it is on
//! disk (through the host's `read_current`), normalized, hashed and compared
//! with `current_hash`. If it matches, the hunks are recomputed against the
//! stored base; if not, the entry is dropped and reported. Objects no longer
//! referenced are deleted on every save.
//!
//! Version 2 (spec 09 D15) adds `comments`: the unsent comments, each with
//! its rows, text, decision marks, the hash of the buffer text when saved
//! (`file_hash`) and the text of its rows as an object (`snippet_hash`).
//! On load a comment keeps its rows if the file is unchanged, otherwise it
//! is relocated to the nearest exact occurrence of its rows' text, or else
//! to one row at the same place (clamped). A missing or binary file drops
//! it. A version 1 file loads without comments.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use cincel_text::Rope;
use serde::{Deserialize, Serialize};

use crate::comments::{CommentDropReason, CommentEntry};
use crate::file::FileReview;
use crate::store::{ReviewStore, detached};
use crate::text::{normalize, rope_sha256, sha256_hex};
use crate::types::{FileStatus, ReviewError, TurnId};

/// Current `state.json` format.
pub const STATE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct StateFile {
    version: u32,
    files: Vec<Entry>,
    /// Unsent comments (version 2; absent in version 1).
    #[serde(default)]
    comments: Vec<CommentEntry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    base_hash: String,
    current_hash: Option<String>,
    status: EntryStatus,
    previous_hash: Option<String>,
    turn_id: u64,
    created_at: u64,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EntryStatus {
    Modified,
    Created,
    Deleted,
}

/// Why a persisted entry was not restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// The file changed on disk since the state was saved.
    Changed,
    /// The file no longer exists.
    Missing,
    /// A file recorded as deleted exists again.
    Reappeared,
    /// The file is not valid UTF-8 any more.
    NotUtf8,
    /// A stored object is missing or does not match its hash.
    BadObject,
}

/// What [`ReviewStore::load`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// Entries restored, with their hunks recomputed.
    pub restored: Vec<PathBuf>,
    /// Entries dropped (the UI tells the user).
    pub dropped: Vec<(PathBuf, DropReason)>,
    /// Comments restored (relocated ones included).
    pub comments_restored: usize,
    /// Comments whose file changed since the save: moved to their text, or
    /// to the nearest row when it is gone.
    pub comments_relocated: usize,
    /// Comments dropped (the UI tells the user).
    pub comments_dropped: Vec<(PathBuf, CommentDropReason)>,
}

impl ReviewStore {
    /// Writes the review state to `dir` (created if needed). Files with
    /// nothing pending are not saved; unreferenced objects are deleted.
    pub fn save(&self, dir: &Path) -> Result<(), ReviewError> {
        let objects = dir.join("objects");
        fs::create_dir_all(&objects)?;
        let mut referenced = BTreeSet::new();
        let mut entries = Vec::new();
        for file in self.files.values().filter(|f| f.has_pending_work()) {
            let base_hash = write_object(&objects, &file.base)?;
            referenced.insert(base_hash.clone());
            let (status, previous_hash, current_hash) = match &file.status {
                FileStatus::Modified => (
                    EntryStatus::Modified,
                    None,
                    Some(rope_sha256(file.current.rope())),
                ),
                FileStatus::Created { previous } => {
                    let previous_hash = match previous {
                        Some(previous) => {
                            let hash = write_object(&objects, previous)?;
                            referenced.insert(hash.clone());
                            Some(hash)
                        }
                        None => None,
                    };
                    (
                        EntryStatus::Created,
                        previous_hash,
                        Some(rope_sha256(file.current.rope())),
                    )
                }
                FileStatus::Deleted { .. } => (EntryStatus::Deleted, None, None),
            };
            entries.push(Entry {
                path: file.path.clone(),
                base_hash,
                current_hash,
                status,
                previous_hash,
                turn_id: file.turn_id.0,
                created_at: file.created_at,
            });
        }
        let (comments, snippets) = self.comment_entries(|rope| write_object(&objects, rope))?;
        referenced.extend(snippets);
        let state = StateFile {
            version: STATE_VERSION,
            files: entries,
            comments,
        };
        let json = serde_json::to_vec_pretty(&state)?;
        let tmp = dir.join("state.json.tmp");
        {
            let mut out = fs::File::create(&tmp)?;
            out.write_all(&json)?;
            out.sync_all()?;
        }
        fs::rename(&tmp, dir.join("state.json"))?;

        for entry in fs::read_dir(&objects)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !referenced.contains(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }

    /// Restores the state saved in `dir`. `read_current` returns the bytes of
    /// a file as it is on disk now (`None` if it does not exist). Paths
    /// already in review are left alone, and so are comments whose id is
    /// already in the store. A missing `state.json` restores nothing.
    pub fn load(
        &mut self,
        dir: &Path,
        read_current: impl Fn(&Path) -> Option<Vec<u8>>,
    ) -> Result<LoadReport, ReviewError> {
        let mut report = LoadReport::default();
        let state_path = dir.join("state.json");
        let bytes = match fs::read(&state_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(error) => return Err(error.into()),
        };
        let state: StateFile = serde_json::from_slice(&bytes)?;
        let objects = dir.join("objects");
        let comments = state.comments;
        for entry in state.files {
            if self.files.contains_key(&entry.path) {
                continue;
            }
            match self.restore(&objects, &entry, &read_current) {
                Ok(()) => report.restored.push(entry.path),
                Err(reason) => {
                    tracing::warn!(path = %entry.path.display(), ?reason, "review entry dropped");
                    report.dropped.push((entry.path, reason));
                }
            }
        }
        self.restore_comment_entries(
            comments,
            |hash| {
                read_object(&objects, hash)
                    .ok()
                    .map(|rope| rope.to_string())
            },
            &read_current,
            &mut report,
        );
        for (path, reason) in &report.comments_dropped {
            tracing::warn!(path = %path.display(), ?reason, "review comment dropped");
        }
        Ok(report)
    }

    fn restore(
        &mut self,
        objects: &Path,
        entry: &Entry,
        read_current: &impl Fn(&Path) -> Option<Vec<u8>>,
    ) -> Result<(), DropReason> {
        let base = read_object(objects, &entry.base_hash)?;
        let previous = match &entry.previous_hash {
            Some(hash) => Some(read_object(objects, hash)?),
            None => None,
        };
        let on_disk = read_current(&entry.path);
        let (status, current) = match entry.status {
            EntryStatus::Deleted => {
                if on_disk.is_some() {
                    return Err(DropReason::Reappeared);
                }
                (
                    FileStatus::Deleted {
                        previous: base.clone(),
                    },
                    None,
                )
            }
            status => {
                let bytes = on_disk.ok_or(DropReason::Missing)?;
                let text = std::str::from_utf8(&bytes).map_err(|_| DropReason::NotUtf8)?;
                let text = normalize(text);
                if Some(sha256_hex(text.as_bytes())) != entry.current_hash {
                    return Err(DropReason::Changed);
                }
                let status = if status == EntryStatus::Created {
                    FileStatus::Created { previous }
                } else {
                    FileStatus::Modified
                };
                (status, Some(text))
            }
        };
        let snapshot = current.as_deref().map(detached).unwrap_or_default();
        let mut file = FileReview::new(
            entry.path.clone(),
            base,
            snapshot,
            false,
            status,
            TurnId(entry.turn_id),
            &mut self.next_id,
        );
        file.created_at = entry.created_at;
        if let Some(data) =
            crate::recompute::compute_diff(&file.base, file.current.rope(), &|| false)
        {
            file.too_large = data.too_large;
            file.binary = data.binary;
            file.whole_stats = data.stats;
            if file.file_level_only() {
                file.hunks.clear();
            } else {
                file.install(&data.hunks, &mut self.next_id);
            }
            file.needs_recompute = false;
        }
        self.files.insert(entry.path.clone(), file);
        Ok(())
    }
}

fn write_object(objects: &Path, rope: &Rope) -> Result<String, ReviewError> {
    let hash = rope_sha256(rope);
    let path = objects.join(&hash);
    if !path.exists() {
        let tmp = objects.join(format!("{hash}.tmp"));
        {
            let mut out = fs::File::create(&tmp)?;
            for chunk in rope.chunks() {
                out.write_all(chunk.as_bytes())?;
            }
            out.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
    }
    Ok(hash)
}

fn read_object(objects: &Path, hash: &str) -> Result<Rope, DropReason> {
    let bytes = fs::read(objects.join(hash)).map_err(|_| DropReason::BadObject)?;
    if sha256_hex(&bytes) != hash {
        return Err(DropReason::BadObject);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| DropReason::BadObject)?;
    Ok(Rope::from_str(text))
}
