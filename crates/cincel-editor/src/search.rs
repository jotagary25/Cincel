//! In-editor search: the state behind the bar `editor::find` opens.
//!
//! Matching happens over the buffer text only (phantom rows are not searched in
//! v1), is case-insensitive by default, and can switch to a regular expression
//! with the `.*` toggle. All matches are highlighted; the current one gets a
//! stronger background (`docs/specs/02-visual.md` §5).

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// Hard cap on the number of matches kept, so a query like `.` on a huge file
/// cannot make the frame budget explode.
pub const MAX_MATCHES: usize = 10_000;

/// The state of the search bar.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    /// What the user typed.
    pub query: String,
    /// Whether the query is a regular expression.
    pub regex: bool,
    /// Whether matching is case-sensitive (off by default).
    pub case_sensitive: bool,
    /// Byte ranges of every match, ascending.
    matches: Vec<Range<usize>>,
    /// Index of the current match inside `matches`.
    current: Option<usize>,
    /// Whether the query failed to compile as a regular expression.
    invalid: bool,
}

impl SearchState {
    /// Every match, ascending.
    pub fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    /// The current match, if any.
    pub fn current(&self) -> Option<Range<usize>> {
        self.current.and_then(|ix| self.matches.get(ix).cloned())
    }

    /// Index of the current match inside [`SearchState::matches`].
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// Whether the query is a regular expression that does not compile.
    pub fn is_invalid(&self) -> bool {
        self.invalid
    }

    /// The `3/12` counter of the bar, or `None` when there is nothing to count.
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
        Some(format!("{current}/{}", self.matches.len()))
    }

    /// Recomputes every match over `text`, keeping the current one if it is
    /// still there, otherwise selecting the first match at or after `from`.
    pub fn refresh(&mut self, text: &str, from: usize) {
        let previous = self.current();
        self.matches = find_matches(text, &self.query, self.regex, self.case_sensitive);
        self.invalid = !self.query.is_empty()
            && self.regex
            && compile(&self.query, self.case_sensitive).is_none();
        self.current = match previous {
            Some(previous) => self
                .matches
                .iter()
                .position(|range| *range == previous)
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
                .position(|range| range.start >= offset)
                .unwrap_or(0),
        )
    }

    /// Moves to the next match, wrapping around. Returns it.
    pub fn next_match(&mut self) -> Option<Range<usize>> {
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
    pub fn previous_match(&mut self) -> Option<Range<usize>> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = Some(match self.current {
            Some(0) | None => self.matches.len() - 1,
            Some(ix) => ix - 1,
        });
        self.current()
    }

    /// Selects the match at or after `offset` without moving past it.
    pub fn select_at_or_after(&mut self, offset: usize) {
        self.current = self.index_at_or_after(offset);
    }

    /// Forgets the query and the matches.
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current = None;
        self.invalid = false;
    }
}

fn compile(query: &str, case_sensitive: bool) -> Option<Regex> {
    RegexBuilder::new(query)
        .case_insensitive(!case_sensitive)
        .build()
        .ok()
}

/// Every match of `query` in `text`, ascending and non-overlapping.
pub fn find_matches(
    text: &str,
    query: &str,
    regex: bool,
    case_sensitive: bool,
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
            .take(MAX_MATCHES)
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
            if matches.len() >= MAX_MATCHES {
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
            if matches.len() >= MAX_MATCHES {
                break;
            }
        } else {
            ix += 1;
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut state = SearchState {
            query: "a".to_string(),
            ..Default::default()
        };
        state.refresh("a b a b a", 0);
        assert_eq!(state.matches().len(), 3);
        assert_eq!(state.counter().as_deref(), Some("1/3"));
        assert_eq!(state.next_match(), Some(4..5));
        assert_eq!(state.next_match(), Some(8..9));
        assert_eq!(state.next_match(), Some(0..1), "wraps to the first match");
        assert_eq!(
            state.previous_match(),
            Some(8..9),
            "wraps to the last match"
        );
        assert_eq!(state.counter().as_deref(), Some("3/3"));
    }

    #[test]
    fn refresh_keeps_the_current_match_when_it_survives() {
        let mut state = SearchState {
            query: "b".to_string(),
            ..Default::default()
        };
        state.refresh("a b c b", 0);
        state.next_match();
        let current = state.current().expect("a match");
        state.refresh("a b c b", 0);
        assert_eq!(state.current(), Some(current));
    }

    #[test]
    fn refresh_selects_the_match_after_the_cursor() {
        let mut state = SearchState {
            query: "x".to_string(),
            ..Default::default()
        };
        state.refresh("x---x---x", 4);
        assert_eq!(state.current(), Some(4..5));
    }

    #[test]
    fn an_invalid_regex_is_reported() {
        let mut state = SearchState {
            query: "a(".to_string(),
            regex: true,
            ..Default::default()
        };
        state.refresh("abc", 0);
        assert!(state.is_invalid());
        assert_eq!(state.counter().as_deref(), Some("regex inválida"));
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut state = SearchState {
            query: "a".to_string(),
            ..Default::default()
        };
        state.refresh("aaa", 0);
        state.clear();
        assert!(state.matches().is_empty());
        assert_eq!(state.counter(), None);
    }
}
