//! Test harness: a [`ReviewStore`] wired to real `cincel_text::Buffer`s the
//! way the host does it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use cincel_review::{FileOp, Hunk, LinePair, Revert, ReviewStore, TurnId};
use cincel_text::{Buffer, EditSource, minimal_edits};

pub struct Harness {
    pub store: ReviewStore,
    pub buffers: BTreeMap<PathBuf, Buffer>,
    /// Files on "disk" (what the host saved or deleted).
    pub disk: BTreeMap<PathBuf, String>,
    pub turn: u64,
}

pub fn p(path: &str) -> PathBuf {
    PathBuf::from(path)
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    pub fn new() -> Self {
        Self {
            store: ReviewStore::new(),
            buffers: BTreeMap::new(),
            disk: BTreeMap::new(),
            turn: 1,
        }
    }

    pub fn open(&mut self, path: &str, text: &str) {
        let mut buffer = Buffer::new(text);
        buffer.record_events(true);
        self.buffers.insert(p(path), buffer);
        self.disk.insert(p(path), text.to_owned());
    }

    pub fn text(&self, path: &str) -> String {
        self.buffers[&p(path)].text()
    }

    pub fn base(&self, path: &str) -> String {
        self.store.file(&p(path)).expect("tracked").base.to_string()
    }

    /// Opens `path` with `text`, captures its base and starts a turn.
    pub fn start(&mut self, path: &str, text: &str) {
        self.open(path, text);
        self.store.begin_turn(TurnId(self.turn));
        self.capture(path);
    }

    pub fn capture(&mut self, path: &str) -> bool {
        let snapshot = self.buffers[&p(path)].snapshot();
        self.store.capture_base(&p(path), &snapshot)
    }

    /// Feeds every queued event of `path` to the store with the current
    /// snapshot (exact when one event is queued, the fallback otherwise).
    pub fn pump(&mut self, path: &str) {
        let buffer = self.buffers.get_mut(&p(path)).expect("open");
        let events = buffer.drain_events();
        let snapshot = buffer.snapshot();
        for event in events {
            let cincel_text::BufferEvent::Edited { source, .. } = &event;
            let source = *source;
            self.store
                .buffer_edited(&p(path), &event, &snapshot, source);
            self.store.comment_buffer_event(&p(path), &event, &snapshot);
        }
    }

    /// The agent writes the whole text, landing as minimal edits with one
    /// event per edit (the fast path).
    pub fn agent_write(&mut self, path: &str, text: &str) {
        let turn = self.turn;
        let current = self.text(path);
        let edits = minimal_edits(&current, text);
        let buffer = self.buffers.get_mut(&p(path)).expect("open");
        buffer.start_transaction(EditSource::Agent { turn_id: turn });
        for (range, new_text) in edits.iter().rev() {
            let buffer = self.buffers.get_mut(&p(path)).expect("open");
            buffer.edit(std::slice::from_ref(range), new_text);
            self.pump(path);
        }
        self.buffers
            .get_mut(&p(path))
            .expect("open")
            .end_transaction();
        self.disk.insert(p(path), text.to_owned());
    }

    /// The agent writes, but the host drains all events at once (fallback).
    pub fn agent_write_batched(&mut self, path: &str, text: &str) {
        let turn = self.turn;
        let buffer = self.buffers.get_mut(&p(path)).expect("open");
        buffer.set_text_minimal(text, EditSource::Agent { turn_id: turn });
        self.pump(path);
        self.disk.insert(p(path), text.to_owned());
    }

    pub fn user_edit(&mut self, path: &str, range: Range<usize>, text: &str) {
        let buffer = self.buffers.get_mut(&p(path)).expect("open");
        buffer.start_transaction(EditSource::User);
        buffer.edit(&[range], text);
        buffer.end_transaction();
        self.pump(path);
    }

    pub fn undo(&mut self, path: &str) {
        let buffer = self.buffers.get_mut(&p(path)).expect("open");
        buffer.undo();
        self.pump(path);
    }

    /// Applies a revert like the host: Review transaction + save, or a file
    /// operation.
    pub fn apply(&mut self, revert: Revert) {
        match revert {
            Revert::Edits { path, edits } => {
                let key = path.to_string_lossy().into_owned();
                let buffer = self.buffers.get_mut(&path).expect("open");
                buffer.start_transaction(EditSource::Review);
                for edit in edits {
                    let buffer = self.buffers.get_mut(&path).expect("open");
                    buffer.edit(std::slice::from_ref(&edit.range), &edit.text);
                    self.pump(&key);
                }
                let buffer = self.buffers.get_mut(&path).expect("open");
                buffer.end_transaction();
                let text = buffer.text();
                self.disk.insert(path, text);
            }
            Revert::File(FileOp::DeleteFile(path)) => {
                self.buffers.remove(&path);
                self.disk.remove(&path);
            }
            Revert::File(FileOp::WriteFile(path, text)) => {
                self.disk.insert(path.clone(), text.clone());
                if let Some(buffer) = self.buffers.get_mut(&path) {
                    buffer.set_text_minimal(&text, EditSource::Load);
                    let key = path.to_string_lossy().into_owned();
                    self.pump(&key);
                } else {
                    let mut buffer = Buffer::new(&text);
                    buffer.record_events(true);
                    let snapshot = buffer.snapshot();
                    self.buffers.insert(path.clone(), buffer);
                    self.store.file_written(&path, &snapshot, EditSource::Load);
                }
            }
        }
    }

    pub fn apply_all(&mut self, reverts: Vec<Revert>) {
        for revert in reverts {
            self.apply(revert);
        }
    }

    /// Runs the background recompute the way the host does.
    pub fn settle(&mut self, path: &str) {
        if let Some(job) = self.store.recompute_job(&p(path)) {
            let result = job.run();
            self.store.apply_recompute(result);
        }
    }

    pub fn settle_all(&mut self) {
        let paths: Vec<PathBuf> = self.store.files().map(|f| f.path.clone()).collect();
        for path in paths {
            let key = path.to_string_lossy().into_owned();
            self.settle(&key);
        }
    }

    pub fn hunks(&self, path: &str) -> Vec<Hunk> {
        self.store.hunks(&p(path)).into_iter().cloned().collect()
    }

    pub fn line(&self, path: &str, hunk: usize, line: usize) -> LinePair {
        self.hunks(path)[hunk].lines[line]
    }

    /// Checks the segment/line invariants of `path` against its buffer.
    pub fn check(&self, path: &str) {
        let Some(file) = self.store.file(&p(path)) else {
            return;
        };
        if file.file_level_only() {
            return;
        }
        let buffer = self.text(path);
        assert_eq!(
            file.current().text(),
            buffer,
            "store copy of {path} out of sync"
        );
        let base = file.base.to_string();
        let dump = format!("buffer={buffer:?}\nbase={base:?}\nhunks={:#?}", file.hunks);
        let (mut b, mut c) = (0usize, 0usize);
        for hunk in &file.hunks {
            let range = hunk.buffer_byte_range();
            assert!(range.start >= c, "hunks out of order in {path}");
            assert!(hunk.base_byte_range.start >= b, "base ranges out of order");
            assert_eq!(
                &buffer[c..range.start],
                &base[b..hunk.base_byte_range.start],
                "equal segment differs in {path}\n{dump}"
            );
            let at_start = |t: &str, o: usize| o == 0 || t.as_bytes()[o - 1] == b'\n';
            let at_end = |t: &str, o: usize| o == 0 || o == t.len() || t.as_bytes()[o - 1] == b'\n';
            assert!(
                at_start(&buffer, range.start) && at_end(&buffer, range.end),
                "buffer side not line aligned\n{dump}"
            );
            assert!(
                at_start(&base, hunk.base_byte_range.start)
                    && at_end(&base, hunk.base_byte_range.end),
                "base side not line aligned\n{dump}"
            );
            assert_ne!(
                &buffer[range.clone()],
                &base[hunk.base_byte_range.clone()],
                "equal hunk left in {path}\n{dump}"
            );
            let start_row = base[..hunk.base_byte_range.start].matches('\n').count() as u32;
            assert_eq!(
                hunk.base_rows.start, start_row,
                "base_rows start wrong\n{dump}"
            );
            let lines = base[hunk.base_byte_range.clone()]
                .split_inclusive('\n')
                .count() as u32;
            assert_eq!(
                hunk.base_rows.end - hunk.base_rows.start,
                lines,
                "base_rows len wrong\n{dump}"
            );
            if hunk.is_fresh() {
                for pair in &hunk.lines {
                    if let Some(row) = pair.base_row {
                        assert!(hunk.base_rows.contains(&row));
                    }
                    if let Some(anchor) = pair.buffer_row {
                        assert!(range.contains(&anchor.offset));
                        assert!(at_start(&buffer, anchor.offset));
                    }
                }
            }
            b = hunk.base_byte_range.end;
            c = range.end;
        }
        assert_eq!(
            &buffer[c..],
            &base[b..],
            "trailing segment differs in {path}"
        );
    }

    pub fn file_exists(&self, path: &str) -> bool {
        self.disk.contains_key(Path::new(path))
    }
}

/// Deterministic xorshift for property tests.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

const WORDS: &[&str] = &[
    "fn", "let", "x", "y", "  ", "\t", "ñ", "é", "{", "}", "foo", "bar", "0",
];

pub fn random_line(rng: &mut Rng) -> String {
    let n = rng.below(4);
    let mut line = String::new();
    for _ in 0..n {
        line.push_str(WORDS[rng.below(WORDS.len())]);
        if rng.chance(50) {
            line.push(' ');
        }
    }
    line
}

pub fn random_text(rng: &mut Rng) -> String {
    let lines = rng.below(12);
    let mut text = String::new();
    for _ in 0..lines {
        text.push_str(&random_line(rng));
        text.push('\n');
    }
    if rng.chance(30) {
        text.push_str(&random_line(rng));
    }
    text
}

/// A random mutation of `text` at line granularity.
pub fn mutate(rng: &mut Rng, text: &str) -> String {
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(lines.len() + 1);
        match rng.below(4) {
            0 => lines.insert(at, format!("{}\n", random_line(rng))),
            1 if at < lines.len() => {
                lines.remove(at);
            }
            _ if at < lines.len() => {
                let had_newline = lines[at].ends_with('\n');
                lines[at] = random_line(rng);
                if had_newline {
                    lines[at].push('\n');
                }
            }
            _ => lines.push(random_line(rng)),
        }
    }
    let mut out: String = lines.concat();
    if rng.chance(15) {
        if out.ends_with('\n') {
            out.pop();
        } else {
            out.push('\n');
        }
    }
    out
}

/// A random byte range on char boundaries.
pub fn random_range(rng: &mut Rng, text: &str) -> Range<usize> {
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let a = bounds[rng.below(bounds.len())];
    let b = if rng.chance(40) {
        a
    } else {
        bounds[rng.below(bounds.len())]
    };
    a.min(b)..a.max(b)
}
