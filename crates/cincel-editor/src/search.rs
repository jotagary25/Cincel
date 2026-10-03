//! In-editor search and replace: the state behind the bar `editor::find`
//! (`Ctrl+F`) and `editor::find_replace` (`Ctrl+H`) open.
//!
//! Matching covers the buffer **and** the phantom rows (the lines the agent
//! removed, still on screen until the user decides;
//! `docs/specs/07-etapa5-productividad.md` §10.2). Matches are kept in screen
//! order: the phantom rows of a hunk come before the real row they are spliced
//! in front of. A phantom match is highlighted, counted and visited like any
//! other, but it is **never replaced** (decision D12): the phantom text is the
//! review's base text, which "Rechazar" restores, not part of the file.
//!
//! Matching is case-insensitive by default and can switch to a regular
//! expression with the `.*` toggle; with a regular expression the replacement
//! expands `$1` and `${name}` exactly like [`regex::Regex::replace`]. All
//! matches are highlighted; the current one gets a stronger background
//! (`docs/specs/02-visual.md` §5).

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// Hard cap on the number of matches kept (buffer and phantom rows together),
/// so a query like `.` on a huge file cannot make the frame budget explode.
/// "Reemplazar todo" is not bound by it: it rescans the whole text.
pub const MAX_MATCHES: usize = 10_000;

/// What the bar says when "Reemplazar" lands on a phantom match.
pub const PHANTOM_REPLACE_NOTICE: &str =
    "Esa coincidencia está en una línea que el agente quitó: no se puede reemplazar";

/// Where a match is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MatchLocation {
    /// A byte range of the buffer.
    Buffer(Range<usize>),
    /// A range inside a phantom row: line `line_ix` of the deleted text of
    /// hunk `hunk_ix` (an index into [`crate::DiffTransformMap::hunks`]).
    Phantom {
        /// Index of the hunk in [`crate::DiffTransformMap::hunks`].
        hunk_ix: usize,
        /// Index of the line inside the hunk's deleted text.
        line_ix: usize,
        /// Byte range inside that line.
        range: Range<usize>,
    },
}

impl MatchLocation {
    /// Whether the match is on a phantom row.
    pub fn is_phantom(&self) -> bool {
        matches!(self, MatchLocation::Phantom { .. })
    }

    /// The buffer range of a real match, `None` for a phantom one.
    pub fn buffer_range(&self) -> Option<Range<usize>> {
        match self {
            MatchLocation::Buffer(range) => Some(range.clone()),
            MatchLocation::Phantom { .. } => None,
        }
    }
}

/// The deleted lines of one hunk, as the search sees them.
#[derive(Clone, Copy, Debug)]
pub struct PhantomLines<'a> {
    /// Buffer offset of the real row the phantom rows are spliced in front
    /// of (the buffer length when they sit after the last row).
    pub insert_offset: usize,
    /// The deleted lines, without newlines.
    pub lines: &'a [String],
}

/// Which field of the bar gets the keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchField {
    /// "Buscar…".
    #[default]
    Find,
    /// "Reemplazar…" (only while [`SearchState::replace_open`]).
    Replace,
}

/// The state of the search bar.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    /// What the user typed in "Buscar…".
    pub query: String,
    /// Whether the query is a regular expression.
    pub regex: bool,
    /// Whether matching is case-sensitive (off by default).
    pub case_sensitive: bool,
    /// What the user typed in "Reemplazar…".
    pub replacement: String,
    /// The field that gets the keyboard.
    pub field: SearchField,
    /// Whether the bar is in replace mode (both fields side by side in the
    /// same 28 px strip).
    pub replace_open: bool,
    /// Every match, in screen order.
    matches: Vec<MatchLocation>,
    /// Buffer offset each hunk's phantom rows sit at, by hunk index, from the
    /// last refresh: the position of a phantom match for "at or after".
    phantom_offsets: Vec<usize>,
    /// Index of the current match inside `matches`.
    current: Option<usize>,
    /// Whether the query failed to compile as a regular expression.
    invalid: bool,
    /// A message that takes the counter's place (a phantom match that cannot
    /// be replaced, the tally of "Reemplazar todo").
    status: Option<String>,
}

impl SearchState {
    /// Every match, in screen order.
    pub fn matches(&self) -> &[MatchLocation] {
        &self.matches
    }

    /// The buffer ranges of the real matches, ascending.
    pub fn buffer_matches(&self) -> Vec<Range<usize>> {
        self.matches
            .iter()
            .filter_map(MatchLocation::buffer_range)
            .collect()
    }

    /// Number of matches on phantom rows.
    pub fn phantom_match_count(&self) -> usize {
        self.matches
            .iter()
            .filter(|found| found.is_phantom())
            .count()
    }

    /// The current match, if any.
    pub fn current(&self) -> Option<MatchLocation> {
        self.current.and_then(|ix| self.matches.get(ix).cloned())
    }

    /// Index of the current match inside [`SearchState::matches`].
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// Makes `ix` the current match (ignored when out of range).
    pub fn set_current(&mut self, ix: usize) {
        if ix < self.matches.len() {
            self.current = Some(ix);
        }
    }

    /// Whether the query is a regular expression that does not compile.
    pub fn is_invalid(&self) -> bool {
        self.invalid
    }

    /// The message that replaces the counter, if any.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Sets (or clears) the message that replaces the counter.
    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    /// The `3/12` counter of the bar, or `None` when there is nothing to
    /// count. With phantom matches it reads `3/12 (2 en líneas quitadas)`.
    pub fn counter(&self) -> Option<String> {
        if self.query.is_empty() {
            return None;
        }
        if self.invalid {
            return Some("regex inválida".to_string());
        }
        if self.matches.is_empty() {
            return Some("0/0".to_string());
        }
        let current = self.current.map_or(0, |ix| ix + 1);
        let counter = format!("{current}/{}", self.matches.len());
        match self.phantom_match_count() {
            0 => Some(counter),
            phantom => Some(format!("{counter} ({phantom} en líneas quitadas)")),
        }
    }

    /// What the bar shows in the counter's place: the status message if there
    /// is one, otherwise [`SearchState::counter`].
    pub fn status_or_counter(&self) -> Option<String> {
        self.status.clone().or_else(|| self.counter())
    }

    /// Screen position of a match, in buffer offsets: its start for a real
    /// match, the offset its phantom rows sit at for a phantom one.
    pub fn position(&self, found: &MatchLocation) -> usize {
        match found {
            MatchLocation::Buffer(range) => range.start,
            MatchLocation::Phantom { hunk_ix, .. } => {
                self.phantom_offsets.get(*hunk_ix).copied().unwrap_or(0)
            }
        }
    }

    /// Recomputes every match over the buffer `text` alone (no phantom rows).
    pub fn refresh(&mut self, text: &str, from: usize) {
        self.refresh_with_phantoms(text, &[], from);
    }

    /// Recomputes every match over the buffer `text` and the phantom rows of
    /// `phantoms` (one entry per hunk, in hunk order, which is screen order),
    /// keeping the current match if it is still there, otherwise selecting
    /// the first match at or after the buffer offset `from`.
    ///
    /// Phantom rows are searched line by line: a match never crosses from one
    /// phantom row to the next, nor into the buffer.
    pub fn refresh_with_phantoms(
        &mut self,
        text: &str,
        phantoms: &[PhantomLines<'_>],
        from: usize,
    ) {
        let previous = self.current();
        let buffer = find_matches(text, &self.query, self.regex, self.case_sensitive);
        let mut merged = Vec::with_capacity(buffer.len());
        let mut buffer = buffer.into_iter().peekable();
        self.phantom_offsets = phantoms.iter().map(|hunk| hunk.insert_offset).collect();
        for (hunk_ix, hunk) in phantoms.iter().enumerate() {
            while let Some(range) = buffer.next_if(|range| range.start < hunk.insert_offset) {
                merged.push(MatchLocation::Buffer(range));
            }
            for (line_ix, line) in hunk.lines.iter().enumerate() {
                for range in find_matches(line, &self.query, self.regex, self.case_sensitive) {
                    merged.push(MatchLocation::Phantom {
                        hunk_ix,
                        line_ix,
                        range,
                    });
                }
            }
        }
        merged.extend(buffer.map(MatchLocation::Buffer));
        merged.truncate(MAX_MATCHES);
        self.matches = merged;
        self.invalid = !self.query.is_empty()
            && self.regex
            && compile(&self.query, self.case_sensitive).is_none();
        self.current = match previous {
            Some(previous) => self
                .matches
                .iter()
                .position(|found| *found == previous)
                .or_else(|| self.index_at_or_after(from)),
            None => self.index_at_or_after(from),
        };
    }

    fn index_at_or_after(&self, offset: usize) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        Some(
            self.matches
                .iter()
                .position(|found| self.position(found) >= offset)
                .unwrap_or(0),
        )
    }

    /// Moves to the next match, wrapping around. Returns it.
    pub fn next_match(&mut self) -> Option<MatchLocation> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = Some(match self.current {
            Some(ix) => (ix + 1) % self.matches.len(),
            None => 0,
        });
        self.current()
    }

    /// Moves to the previous match, wrapping around. Returns it.
    pub fn previous_match(&mut self) -> Option<MatchLocation> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = Some(match self.current {
            Some(0) | None => self.matches.len() - 1,
            Some(ix) => ix - 1,
        });
        self.current()
    }

    /// Index of the first real (buffer) match after `ix` in screen order,
    /// wrapping around; `None` when every match is on a phantom row.
    pub fn next_buffer_match_after(&self, ix: usize) -> Option<usize> {
        let len = self.matches.len();
        (1..=len)
            .map(|step| (ix + step) % len)
            .find(|candidate| !self.matches[*candidate].is_phantom())
    }

    /// Selects the match at or after `offset` without moving past it.
    pub fn select_at_or_after(&mut self, offset: usize) {
        self.current = self.index_at_or_after(offset);
    }

    /// The replacement for the text a match covers, taken on its own: the
    /// literal replacement, or with a regular expression its expansion
    /// (`$1`, `${name}`, as [`regex::Regex::replace`] does).
    pub fn replacement_for(&self, matched: &str) -> String {
        self.replacement_at(matched, 0..matched.len())
    }

    /// The replacement for the match at `range` of `text`. With a regular
    /// expression the captures are taken in context (so `\b` and friends see
    /// the neighbouring characters); without one it is the literal text.
    pub fn replacement_at(&self, text: &str, range: Range<usize>) -> String {
        if !self.regex {
            return self.replacement.clone();
        }
        let Some(regex) = compile(&self.query, self.case_sensitive) else {
            return self.replacement.clone();
        };
        let in_context = regex
            .captures_at(text, range.start)
            .filter(|captures| captures.get(0).is_some_and(|all| all.range() == range));
        let captures = match in_context {
            Some(captures) => Some(captures),
            None => text
                .get(range.clone())
                .and_then(|matched| regex.captures(matched)),
        };
        match captures {
            Some(captures) => {
                let mut expanded = String::new();
                captures.expand(&self.replacement, &mut expanded);
                expanded
            }
            None => self.replacement.clone(),
        }
    }

    /// The edits "Reemplazar todo" applies to `text`, ascending and
    /// non-overlapping (every real match of the whole text, not capped by
    /// [`MAX_MATCHES`]), and the number of phantom matches it leaves alone.
    pub fn replace_all_edits(&self, text: &str) -> (Vec<(Range<usize>, String)>, usize) {
        let phantom = self.phantom_match_count();
        if self.query.is_empty() || self.invalid {
            return (Vec::new(), phantom);
        }
        let edits = find_matches_limited(
            text,
            &self.query,
            self.regex,
            self.case_sensitive,
            usize::MAX,
        )
        .into_iter()
        .map(|range| {
            let replacement = self.replacement_at(text, range.clone());
            (range, replacement)
        })
        .collect();
        (edits, phantom)
    }

    /// Forgets the query, the matches and the status (the replacement text and
    /// the toggles stay for the next time the bar opens).
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.phantom_offsets.clear();
        self.current = None;
        self.invalid = false;
        self.status = None;
    }
}

/// The tally "Reemplazar todo" leaves in the counter's place, e.g.
/// `5 reemplazadas · 2 en líneas quitadas por el agente sin tocar`.
pub fn replace_all_message(replaced: usize, phantom: usize) -> String {
    let replaced = if replaced == 1 {
        "1 reemplazada".to_string()
    } else {
        format!("{replaced} reemplazadas")
    };
    if phantom == 0 {
        replaced
    } else {
        format!("{replaced} · {phantom} en líneas quitadas por el agente sin tocar")
    }
}

fn compile(query: &str, case_sensitive: bool) -> Option<Regex> {
    RegexBuilder::new(query)
        .case_insensitive(!case_sensitive)
        .build()
        .ok()
}

/// Every match of `query` in `text`, ascending and non-overlapping, at most
/// [`MAX_MATCHES`] of them.
pub fn find_matches(
    text: &str,
    query: &str,
    regex: bool,
    case_sensitive: bool,
) -> Vec<Range<usize>> {
    find_matches_limited(text, query, regex, case_sensitive, MAX_MATCHES)
}

/// Every match of `query` in `text`, ascending and non-overlapping, at most
/// `limit` of them.
fn find_matches_limited(
    text: &str,
    query: &str,
    regex: bool,
    case_sensitive: bool,
    limit: usize,
) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if regex {
        let Some(regex) = compile(query, case_sensitive) else {
            return Vec::new();
        };
        return regex
            .find_iter(text)
            .filter(|found| !found.is_empty())
            .take(limit)
            .map(|found| found.start()..found.end())
            .collect();
    }

    let mut matches = Vec::new();
    if case_sensitive {
        let mut start = 0;
        while let Some(found) = text[start..].find(query) {
            let at = start + found;
            matches.push(at..at + query.len());
            start = at + query.len();
            if matches.len() >= limit {
                break;
            }
        }
        return matches;
    }

    // Case-insensitive plain search. Lowercasing can change byte lengths
    // (`İ`), so the haystack is walked by `char` and compared in place.
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    let haystack: Vec<(usize, char)> = text
        .char_indices()
        .flat_map(|(byte, ch)| {
            ch.to_lowercase()
                .map(move |lower| (byte, lower))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut ix = 0;
    while ix + needle.len() <= haystack.len() {
        if (0..needle.len()).all(|offset| haystack[ix + offset].1 == needle[offset]) {
            let start = haystack[ix].0;
            let end = haystack
                .get(ix + needle.len())
                .map(|(byte, _)| *byte)
                .unwrap_or(text.len());
            matches.push(start..end);
            ix += needle.len();
            if matches.len() >= limit {
                break;
            }
        } else {
            ix += 1;
        }
    }
    matches
}

// -- occurrences of the word under the cursor (spec 10 §7.1, E1) -------------

/// Longest selection, in bytes, whose occurrences are marked.
pub const MAX_OCCURRENCE_BYTES: usize = 256;

/// What the editor marks the other occurrences of: the word under the cursor
/// or the text of a one-row selection (`docs/specs/10-etapa7-ronda2.md` §7.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccurrenceQuery {
    /// The exact text looked for (case-sensitive).
    pub text: String,
    /// The character before an occurrence must not be a word character.
    pub whole_word_start: bool,
    /// The character after an occurrence must not be a word character.
    pub whole_word_end: bool,
    /// Buffer range of the occurrence the query comes from, never marked.
    pub own: Range<usize>,
}

/// A word character: alphanumeric or `_` (the class `1` of the word
/// movement in `view.rs`).
pub fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

impl OccurrenceQuery {
    /// The query for a cursor at byte `column` of `line` (which starts at
    /// buffer offset `line_start`) with nothing selected: the word the cursor
    /// is inside of or right at the end of, as a whole word.
    pub fn for_cursor(line: &str, line_start: usize, column: usize) -> Option<Self> {
        let column = column.min(line.len());
        if !line.is_char_boundary(column) {
            return None;
        }
        let inside = line[column..].chars().next().is_some_and(is_word_char);
        let at_end = line[..column].chars().next_back().is_some_and(is_word_char);
        if !inside && !at_end {
            return None;
        }
        let start = line[..column]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| is_word_char(*ch))
            .last()
            .map_or(column, |(ix, _)| ix);
        let end = line[column..]
            .char_indices()
            .find(|(_, ch)| !is_word_char(*ch))
            .map_or(line.len(), |(ix, _)| column + ix);
        Some(Self {
            text: line[start..end].to_string(),
            whole_word_start: true,
            whole_word_end: true,
            own: line_start + start..line_start + end,
        })
    }

    /// The query for a selection inside one buffer row: `text` is what is
    /// selected and `range` its buffer range. `None` when it is empty, longer
    /// than [`MAX_OCCURRENCE_BYTES`], only whitespace or crosses a line.
    pub fn for_selection(text: &str, range: Range<usize>) -> Option<Self> {
        if text.is_empty()
            || text.len() > MAX_OCCURRENCE_BYTES
            || text.contains('\n')
            || text.trim().is_empty()
        {
            return None;
        }
        Some(Self {
            text: text.to_string(),
            whole_word_start: text.chars().next().is_some_and(is_word_char),
            whole_word_end: text.chars().next_back().is_some_and(is_word_char),
            own: range,
        })
    }
}

/// The occurrences of `query` in `text`, which starts at buffer offset `base`:
/// exact (case-sensitive) and non-overlapping, ascending, with the word
/// boundaries the query asks for, without [`OccurrenceQuery::own`], as buffer
/// ranges. At most [`MAX_MATCHES`] of them.
pub fn find_occurrences(text: &str, base: usize, query: &OccurrenceQuery) -> Vec<Range<usize>> {
    let needle = query.text.as_str();
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let mut from = 0;
    while from <= text.len() {
        let Some(ix) = text[from..].find(needle) else {
            break;
        };
        let start = from + ix;
        let end = start + needle.len();
        let start_ok =
            !query.whole_word_start || !text[..start].chars().next_back().is_some_and(is_word_char);
        let end_ok = !query.whole_word_end || !text[end..].chars().next().is_some_and(is_word_char);
        if !(start_ok && end_ok) {
            // Candidates may overlap a rejected one: step one character.
            from = start + text[start..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let range = base + start..base + end;
        if range != query.own {
            found.push(range);
            if found.len() >= MAX_MATCHES {
                break;
            }
        }
        from = end;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(range: Range<usize>) -> MatchLocation {
        MatchLocation::Buffer(range)
    }

    fn phantom(hunk_ix: usize, line_ix: usize, range: Range<usize>) -> MatchLocation {
        MatchLocation::Phantom {
            hunk_ix,
            line_ix,
            range,
        }
    }

    fn query_state(query: &str) -> SearchState {
        SearchState {
            query: query.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn plain_search_is_case_insensitive_by_default() {
        let found = find_matches("Hola hola HOLA", "hola", false, false);
        assert_eq!(found, vec![0..4, 5..9, 10..14]);
    }

    #[test]
    fn plain_search_can_be_case_sensitive() {
        let found = find_matches("Hola hola HOLA", "hola", false, true);
        assert_eq!(found, vec![5..9]);
    }

    #[test]
    fn plain_search_handles_accents() {
        let text = "año AÑO";
        let found = find_matches(text, "año", false, false);
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].clone()], "año");
        assert_eq!(&text[found[1].clone()], "AÑO");
    }

    #[test]
    fn regex_search_finds_patterns() {
        let found = find_matches("a1 b22 c333", r"\d+", true, false);
        assert_eq!(found, vec![1..2, 4..6, 8..11]);
    }

    #[test]
    fn an_invalid_regex_yields_no_matches() {
        assert!(find_matches("abc", "a(", true, false).is_empty());
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let mut state = query_state("a");
        state.refresh("a b a b a", 0);
        assert_eq!(state.matches().len(), 3);
        assert_eq!(state.counter().as_deref(), Some("1/3"));
        assert_eq!(state.next_match(), Some(buffer(4..5)));
        assert_eq!(state.next_match(), Some(buffer(8..9)));
        assert_eq!(
            state.next_match(),
            Some(buffer(0..1)),
            "wraps to the first match"
        );
        assert_eq!(
            state.previous_match(),
            Some(buffer(8..9)),
            "wraps to the last match"
        );
        assert_eq!(state.counter().as_deref(), Some("3/3"));
    }

    #[test]
    fn refresh_keeps_the_current_match_when_it_survives() {
        let mut state = query_state("b");
        state.refresh("a b c b", 0);
        state.next_match();
        let current = state.current().expect("a match");
        state.refresh("a b c b", 0);
        assert_eq!(state.current(), Some(current));
    }

    #[test]
    fn refresh_selects_the_match_after_the_cursor() {
        let mut state = query_state("x");
        state.refresh("x---x---x", 4);
        assert_eq!(state.current(), Some(buffer(4..5)));
    }

    #[test]
    fn an_invalid_regex_is_reported() {
        let mut state = SearchState {
            regex: true,
            ..query_state("a(")
        };
        state.refresh("abc", 0);
        assert!(state.is_invalid());
        assert_eq!(state.counter().as_deref(), Some("regex inválida"));
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut state = query_state("a");
        state.refresh("aaa", 0);
        state.set_status(Some("algo".into()));
        state.clear();
        assert!(state.matches().is_empty());
        assert_eq!(state.counter(), None);
        assert_eq!(state.status(), None);
    }

    // -- phantom rows (07-etapa5 §10.2) -------------------------------------

    /// `uno foo\nfoo dos\nfoo\n` with a hunk of two deleted lines spliced in
    /// front of row 1 (offset 8) and one in front of row 2 (offset 16).
    fn with_phantoms(state: &mut SearchState, from: usize) {
        let first = vec!["foo viejo".to_string(), "otro foo foo".to_string()];
        let second = vec!["sin nada".to_string()];
        let phantoms = [
            PhantomLines {
                insert_offset: 8,
                lines: &first,
            },
            PhantomLines {
                insert_offset: 16,
                lines: &second,
            },
        ];
        state.refresh_with_phantoms("uno foo\nfoo dos\nfoo\n", &phantoms, from);
    }

    #[test]
    fn phantom_matches_sit_in_screen_order() {
        let mut state = query_state("foo");
        with_phantoms(&mut state, 0);
        assert_eq!(
            state.matches(),
            &[
                buffer(4..7),
                phantom(0, 0, 0..3),
                phantom(0, 1, 5..8),
                phantom(0, 1, 9..12),
                buffer(8..11),
                buffer(16..19),
            ],
            "the deleted rows of a hunk come before the real row below them"
        );
        assert_eq!(state.buffer_matches(), vec![4..7, 8..11, 16..19]);
        assert_eq!(state.phantom_match_count(), 3);
    }

    #[test]
    fn the_counter_includes_the_phantom_matches() {
        let mut state = query_state("foo");
        with_phantoms(&mut state, 0);
        assert_eq!(
            state.counter().as_deref(),
            Some("1/6 (3 en líneas quitadas)")
        );
        state.next_match();
        assert_eq!(
            state.counter().as_deref(),
            Some("2/6 (3 en líneas quitadas)")
        );
        assert!(state.current().unwrap().is_phantom());
    }

    #[test]
    fn the_status_takes_the_counters_place() {
        let mut state = query_state("foo");
        with_phantoms(&mut state, 0);
        state.set_status(Some(PHANTOM_REPLACE_NOTICE.to_string()));
        assert_eq!(
            state.status_or_counter().as_deref(),
            Some(PHANTOM_REPLACE_NOTICE)
        );
        // A refresh (an edit, a review update) does not clear it.
        with_phantoms(&mut state, 0);
        assert_eq!(state.status(), Some(PHANTOM_REPLACE_NOTICE));
    }

    #[test]
    fn at_or_after_counts_phantom_rows_at_their_insertion_point() {
        let mut state = query_state("foo");
        // Offset 8 is the start of the row the first hunk is spliced before,
        // so its phantom rows come first.
        with_phantoms(&mut state, 8);
        assert_eq!(state.current(), Some(phantom(0, 0, 0..3)));
        with_phantoms(&mut state, 9);
        assert_eq!(state.current(), Some(phantom(0, 0, 0..3)), "kept");
        let mut state = SearchState {
            query: "foo".into(),
            ..Default::default()
        };
        with_phantoms(&mut state, 9);
        assert_eq!(state.current(), Some(buffer(16..19)));
    }

    #[test]
    fn a_phantom_match_never_crosses_rows() {
        let lines = vec!["ab".to_string(), "cd".to_string()];
        let mut state = SearchState {
            regex: true,
            ..query_state(r"b\s*c")
        };
        state.refresh_with_phantoms(
            "",
            &[PhantomLines {
                insert_offset: 0,
                lines: &lines,
            }],
            0,
        );
        assert!(state.matches().is_empty());
    }

    #[test]
    fn the_next_real_match_skips_phantom_ones() {
        let mut state = query_state("foo");
        with_phantoms(&mut state, 0);
        assert_eq!(state.next_buffer_match_after(1), Some(4));
        assert_eq!(state.next_buffer_match_after(5), Some(0), "wraps around");
        let lines = vec!["foo".to_string()];
        let mut only_phantom = SearchState {
            query: "foo".into(),
            ..Default::default()
        };
        only_phantom.refresh_with_phantoms(
            "nada\n",
            &[PhantomLines {
                insert_offset: 0,
                lines: &lines,
            }],
            0,
        );
        assert_eq!(only_phantom.next_buffer_match_after(0), None);
    }

    #[test]
    fn max_matches_counts_both_kinds() {
        let lines = vec!["a".repeat(MAX_MATCHES)];
        let mut state = query_state("a");
        state.refresh_with_phantoms(
            "aaaa",
            &[PhantomLines {
                insert_offset: 4,
                lines: &lines,
            }],
            0,
        );
        assert_eq!(state.matches().len(), MAX_MATCHES);
        assert_eq!(state.buffer_matches().len(), 4);
    }

    // -- replacement --------------------------------------------------------

    #[test]
    fn a_literal_replacement_is_taken_as_is() {
        let state = SearchState {
            replacement: "$1 bar".into(),
            ..query_state("foo")
        };
        assert_eq!(state.replacement_for("FOO"), "$1 bar");
    }

    #[test]
    fn a_regex_replacement_expands_groups() {
        let state = SearchState {
            regex: true,
            replacement: "$1 en".into(),
            ..query_state(r"(\w+)@")
        };
        assert_eq!(state.replacement_for("ana@"), "ana en");
        let named = SearchState {
            regex: true,
            replacement: "<${nombre}>".into(),
            ..query_state(r"(?P<nombre>\w+)@")
        };
        assert_eq!(named.replacement_for("luis@"), "<luis>");
    }

    #[test]
    fn a_regex_replacement_sees_its_context() {
        // `\Bo` needs the character before the match, which the match alone
        // does not have.
        let state = SearchState {
            regex: true,
            replacement: "0".into(),
            ..query_state(r"\Bo")
        };
        let text = "foo";
        let found = find_matches(text, &state.query, true, false);
        assert_eq!(found, vec![1..2, 2..3]);
        assert_eq!(state.replacement_at(text, 1..2), "0");
    }

    #[test]
    fn replace_all_edits_skip_phantom_rows_and_ignore_the_cap() {
        let mut state = SearchState {
            replacement: "bar".into(),
            ..query_state("foo")
        };
        with_phantoms(&mut state, 0);
        let (edits, phantom) = state.replace_all_edits("uno foo\nfoo dos\nfoo\n");
        assert_eq!(
            edits,
            vec![
                (4..7, "bar".to_string()),
                (8..11, "bar".to_string()),
                (16..19, "bar".to_string()),
            ]
        );
        assert_eq!(phantom, 3);

        let huge = "a".repeat(MAX_MATCHES + 5);
        let mut state = SearchState {
            replacement: "b".into(),
            ..query_state("a")
        };
        state.refresh(&huge, 0);
        assert_eq!(state.replace_all_edits(&huge).0.len(), MAX_MATCHES + 5);
    }

    #[test]
    fn replace_all_with_regex_groups() {
        let mut state = SearchState {
            regex: true,
            replacement: "$1 en".into(),
            ..query_state(r"(\w+)@")
        };
        let text = "ana@ casa, luis@ campo";
        state.refresh(text, 0);
        let (edits, _) = state.replace_all_edits(text);
        assert_eq!(
            edits,
            vec![
                (0..4, "ana en".to_string()),
                (11..16, "luis en".to_string())
            ]
        );
    }

    #[test]
    fn the_replace_all_message() {
        assert_eq!(replace_all_message(5, 0), "5 reemplazadas");
        assert_eq!(replace_all_message(1, 0), "1 reemplazada");
        assert_eq!(
            replace_all_message(5, 2),
            "5 reemplazadas · 2 en líneas quitadas por el agente sin tocar"
        );
    }
}
