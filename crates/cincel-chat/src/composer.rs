//! The composer's Markdown decorations: what the chat hands its
//! `cincel_editor::EditorView` through [`cincel_editor::Decorator`].
//!
//! The composer highlights the **structure** of the Markdown being typed, not
//! a grammar's captures (the Markdown grammar `cincel-syntax` ships is the
//! block grammar only: emphasis, inline code and links are opaque to it). It
//! uses the twelve syntax tokens of `02-visual.md` §2 and nothing else:
//!
//! | Markdown | Token | Cincel Dark |
//! |---|---|---|
//! | heading line (`# …`, marker and text) | `keyword` | `#c678dd` |
//! | list marker (`-`, `*`, `+`, `1.`, `1)`), quote marker (`>`) | `keyword` | `#c678dd` |
//! | emphasis delimiters (`**`, `__`, `*`, `_`, `~~`) | `keyword` | `#c678dd` |
//! | inline code, backticks included | `string` | `#98c379` |
//! | fence lines (```` ``` ```` + info string), thematic break, link URL | `comment` | `#5c6370` |
//! | link text, brackets included; `@archivo` mentions | `function` | `#61afef` |
//! | code inside a fence whose info string names a known language | that language's captures | — |
//!
//! Every row of a fenced block, fences included, gets a full-width `bg.editor`
//! background (the composer itself sits on `bg.surface`) and is painted with
//! the code font; the rest uses the interface font. An unclosed fence runs to
//! the end of the text, which is what typing one looks like.

use std::ops::Range;
use std::sync::Arc;

use cincel_editor::{Decorator, TextDecorations};
use cincel_syntax::{CancelFlag, HighlightId, LanguageRegistry, SyntaxState};
use cincel_text::Buffer;
use gpui::Rgba;

use crate::markdown::MAX_HIGHLIGHTED_CODE_BYTES;

/// Builds the composer's decorator: `fence_background` behind code blocks and
/// `mentions` (the `@archivo` tokens of the draft) painted as links.
pub fn decorator(
    registry: Arc<LanguageRegistry>,
    fence_background: Rgba,
    mentions: Vec<String>,
) -> Decorator {
    Arc::new(move |text: &str| decorate(text, &registry, fence_background, &mentions))
}

/// The decorations of `text` (see the module docs for the mapping).
pub fn decorate(
    text: &str,
    registry: &Arc<LanguageRegistry>,
    fence_background: Rgba,
    mentions: &[String],
) -> TextDecorations {
    let mut spans: Vec<(Range<usize>, HighlightId)> = Vec::new();
    let mut code_rows: Vec<Range<u32>> = Vec::new();
    let mut fence: Option<OpenFence> = None;

    let mut offset = 0usize;
    for (row, line) in text.split('\n').enumerate() {
        let row = row as u32;
        let line_range = offset..offset + line.len();
        offset += line.len() + 1;

        if let Some(open) = fence.as_ref() {
            if closes_fence(line, open.marker, open.count) {
                spans.push((line_range.clone(), HighlightId::Comment));
                let open = fence.take().expect("open fence");
                highlight_code(text, &open, line_range.start, registry, &mut spans);
                code_rows.push(open.row..row + 1);
            }
            continue;
        }

        if let Some((marker, count, prefix)) = opens_fence(line) {
            spans.push((line_range.clone(), HighlightId::Comment));
            let info = line[prefix..].trim();
            fence = Some(OpenFence {
                row,
                marker,
                count,
                lang: info.split_whitespace().next().unwrap_or("").to_string(),
                content_start: (line_range.end + 1).min(text.len()),
            });
            continue;
        }

        block_line(line, line_range.start, mentions, &mut spans);
    }
    if let Some(open) = fence.take() {
        let end = text.len();
        highlight_code(text, &open, end, registry, &mut spans);
        let last_row = text.split('\n').count() as u32;
        code_rows.push(open.row..last_row);
    }

    spans.sort_by_key(|(range, _)| (range.start, range.end));
    let mut clean: Vec<(Range<usize>, HighlightId)> = Vec::with_capacity(spans.len());
    for (range, id) in spans {
        if range.is_empty() {
            continue;
        }
        if clean.last().is_some_and(|(last, _)| range.start < last.end) {
            continue;
        }
        clean.push((range, id));
    }

    TextDecorations {
        highlights: clean,
        row_backgrounds: code_rows
            .iter()
            .map(|rows| (rows.clone(), fence_background))
            .collect(),
        monospace_rows: code_rows,
    }
}

/// A fence that has been opened and not closed yet.
struct OpenFence {
    row: u32,
    marker: char,
    /// How many markers opened it.
    count: usize,
    lang: String,
    content_start: usize,
}

/// ```` ``` ```` or `~~~` (three or more, at most three spaces of indent):
/// the marker, how many of it, and the byte where the info string starts.
fn opens_fence(line: &str) -> Option<(char, usize, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let marker = rest.chars().next().filter(|ch| *ch == '`' || *ch == '~')?;
    let len = rest.chars().take_while(|ch| *ch == marker).count();
    if len < 3 {
        return None;
    }
    // A backtick fence's info string may not contain backticks.
    if marker == '`' && rest[len..].contains('`') {
        return None;
    }
    Some((marker, len, indent + len))
}

/// Whether `line` closes a fence opened with `count` × `marker`: at least as
/// many markers and nothing else.
fn closes_fence(line: &str, marker: char, count: usize) -> bool {
    let trimmed = line.trim();
    trimmed.chars().take_while(|ch| *ch == marker).count() >= count
        && trimmed.chars().all(|ch| ch == marker)
}

/// The captures of the fenced code, when its info string names a language.
fn highlight_code(
    text: &str,
    open: &OpenFence,
    content_end: usize,
    registry: &Arc<LanguageRegistry>,
    spans: &mut Vec<(Range<usize>, HighlightId)>,
) {
    if open.lang.is_empty() || open.content_start >= content_end {
        return;
    }
    let Some(language) = registry.language(&open.lang) else {
        return;
    };
    // `content_end` is the start of the closing fence line (or the end of the
    // text); the newline before it belongs to no row of code.
    let end = content_end.min(text.len());
    let code = &text[open.content_start..end];
    let code = code.strip_suffix('\n').unwrap_or(code);
    if code.is_empty() || code.len() > MAX_HIGHLIGHTED_CODE_BYTES {
        return;
    }
    let buffer = Buffer::new(code);
    let mut state = SyntaxState::new(registry.clone(), language, buffer.snapshot());
    state.reparse(&CancelFlag::new());
    for (range, id) in state.highlights(0..code.len()) {
        spans.push((
            open.content_start + range.start..open.content_start + range.end,
            id,
        ));
    }
}

/// One line outside a fence: its block marker, then its inline markup.
fn block_line(
    line: &str,
    base: usize,
    mentions: &[String],
    spans: &mut Vec<(Range<usize>, HighlightId)>,
) {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];

    // ATX heading: the whole line.
    let hashes = rest.chars().take_while(|ch| *ch == '#').count();
    if indent <= 3
        && (1..=6).contains(&hashes)
        && rest[hashes..].chars().next().is_none_or(|ch| ch == ' ')
    {
        spans.push((base..base + line.len(), HighlightId::Keyword));
        return;
    }

    // Thematic break: three or more of the same `-`, `*` or `_`.
    let compact: String = rest.chars().filter(|ch| !ch.is_whitespace()).collect();
    if indent <= 3
        && compact.len() >= 3
        && compact.chars().next().is_some_and(|first| {
            matches!(first, '-' | '*' | '_') && compact.chars().all(|ch| ch == first)
        })
    {
        spans.push((base..base + line.len(), HighlightId::Comment));
        return;
    }

    let mut start = indent;
    // Quote markers, possibly nested (`> > texto`).
    while line[start..].starts_with('>') {
        spans.push((base + start..base + start + 1, HighlightId::Keyword));
        start += 1;
        start += line[start..].len() - line[start..].trim_start_matches(' ').len();
    }
    // List marker: `-`, `*`, `+`, or digits and `.` / `)`, then a space.
    let item = &line[start..];
    let bullet = item
        .chars()
        .next()
        .filter(|ch| matches!(ch, '-' | '*' | '+'))
        .map(|_| 1);
    let digits = item.chars().take_while(char::is_ascii_digit).count();
    let ordered = (1..=9).contains(&digits) && item[digits..].starts_with(['.', ')']);
    let marker_len = bullet.or(ordered.then_some(digits + 1));
    if let Some(len) = marker_len
        && item[len..].is_empty_or_space()
    {
        spans.push((base + start..base + start + len, HighlightId::Keyword));
        start += len;
    }

    inline(line, base, start, mentions, spans);
}

/// Small helper for "followed by a space or nothing".
trait EmptyOrSpace {
    fn is_empty_or_space(&self) -> bool;
}

impl EmptyOrSpace for str {
    fn is_empty_or_space(&self) -> bool {
        self.is_empty() || self.starts_with([' ', '\t'])
    }
}

/// Inline markup of `line[from..]`: code spans, links, mentions and emphasis
/// delimiters, in that order of precedence.
fn inline(
    line: &str,
    base: usize,
    from: usize,
    mentions: &[String],
    spans: &mut Vec<(Range<usize>, HighlightId)>,
) {
    let bytes = line.as_bytes();
    // Bytes already claimed by a code span, a link or a mention.
    let mut taken = vec![false; line.len()];

    // Code spans: a run of N backticks up to the next run of exactly N.
    let mut ix = from;
    while ix < line.len() {
        if bytes[ix] != b'`' {
            ix += 1;
            continue;
        }
        let run = bytes[ix..].iter().take_while(|byte| **byte == b'`').count();
        let mut close = None;
        let mut jx = ix + run;
        while jx < line.len() {
            if bytes[jx] == b'`' {
                let other = bytes[jx..].iter().take_while(|byte| **byte == b'`').count();
                if other == run {
                    close = Some(jx);
                    break;
                }
                jx += other;
            } else {
                jx += 1;
            }
        }
        match close {
            Some(close) => {
                let end = close + run;
                spans.push((base + ix..base + end, HighlightId::String));
                claim(ix..end, &mut taken);
                ix = end;
            }
            None => ix += run,
        }
    }

    // Links and images: `[text](url)`.
    let mut ix = from;
    while ix < line.len() {
        if bytes[ix] != b'[' || taken[ix] {
            ix += 1;
            continue;
        }
        let Some(close) = line[ix + 1..].find(']').map(|at| ix + 1 + at) else {
            break;
        };
        if taken[ix..=close].iter().any(|taken| *taken) || !line[close + 1..].starts_with('(') {
            ix += 1;
            continue;
        }
        let Some(paren) = line[close + 1..].find(')').map(|at| close + 1 + at) else {
            ix += 1;
            continue;
        };
        let start = if ix > from && bytes[ix - 1] == b'!' {
            ix - 1
        } else {
            ix
        };
        spans.push((base + start..base + close + 1, HighlightId::Function));
        spans.push((base + close + 1..base + paren + 1, HighlightId::Comment));
        claim(start..paren + 1, &mut taken);
        ix = paren + 1;
    }

    // Mentions: `@archivo` tokens of the draft, at the start of a word.
    for token in mentions {
        if token.is_empty() {
            continue;
        }
        let mut search = from;
        while let Some(at) = line[search..].find(token.as_str()).map(|at| search + at) {
            let end = at + token.len();
            let word_start = at == 0 || line[..at].ends_with(char::is_whitespace);
            if word_start && !taken[at..end].iter().any(|taken| *taken) {
                spans.push((base + at..base + end, HighlightId::Function));
                claim(at..end, &mut taken);
            }
            search = end;
        }
    }

    // Emphasis delimiters, longest first: only the markers are coloured.
    for delimiter in ["**", "__", "~~", "*", "_"] {
        let len = delimiter.len();
        let mut ix = from;
        while let Some(open) = find_free(line, delimiter, ix, &taken) {
            let after = open + len;
            let opens = line[after..]
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_whitespace())
                && (delimiter.starts_with('*')
                    || delimiter.starts_with('~')
                    || !line[..open]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_alphanumeric));
            if !opens {
                ix = after;
                continue;
            }
            let mut search = after + 1;
            let mut closed = None;
            while let Some(close) = find_free(line, delimiter, search.min(line.len()), &taken) {
                let closes = line[..close]
                    .chars()
                    .next_back()
                    .is_some_and(|ch| !ch.is_whitespace())
                    && (delimiter.starts_with('*')
                        || delimiter.starts_with('~')
                        || !line[close + len..]
                            .chars()
                            .next()
                            .is_some_and(char::is_alphanumeric));
                if closes {
                    closed = Some(close);
                    break;
                }
                search = close + len;
            }
            match closed {
                Some(close) => {
                    spans.push((base + open..base + after, HighlightId::Keyword));
                    spans.push((base + close..base + close + len, HighlightId::Keyword));
                    claim(open..after, &mut taken);
                    claim(close..close + len, &mut taken);
                    ix = close + len;
                }
                None => ix = after,
            }
        }
    }
}

/// Marks `range` as claimed.
fn claim(range: Range<usize>, taken: &mut [bool]) {
    for byte in &mut taken[range] {
        *byte = true;
    }
}

/// The next `needle` at or after `from` none of whose bytes is taken.
fn find_free(line: &str, needle: &str, from: usize, taken: &[bool]) -> Option<usize> {
    let mut search = from;
    while search <= line.len() {
        let at = line.get(search..)?.find(needle).map(|at| search + at)?;
        if !taken[at..at + needle.len()].iter().any(|taken| *taken) {
            return Some(at);
        }
        search = at + 1;
        while search < line.len() && !line.is_char_boundary(search) {
            search += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans_of(text: &str) -> Vec<(String, HighlightId)> {
        decorate(
            text,
            &Arc::new(LanguageRegistry::new()),
            gpui::rgb(0x282c34),
            &[],
        )
        .highlights
        .into_iter()
        .map(|(range, id)| (text[range].to_string(), id))
        .collect()
    }

    fn has(spans: &[(String, HighlightId)], text: &str, id: HighlightId) -> bool {
        spans
            .iter()
            .any(|(span, span_id)| span == text && *span_id == id)
    }

    #[test]
    fn headings_and_list_markers_are_keywords() {
        let spans = spans_of("## Plan\n- uno\n* dos\n+ tres\n12. cuatro\n3) cinco\n> cita");
        assert!(has(&spans, "## Plan", HighlightId::Keyword));
        for marker in ["-", "*", "+", "12.", "3)", ">"] {
            assert!(
                has(&spans, marker, HighlightId::Keyword),
                "{marker}: {spans:?}"
            );
        }
        // A `-` glued to a word is not a list marker.
        assert!(spans_of("-no").is_empty());
    }

    #[test]
    fn emphasis_colours_only_its_delimiters() {
        let spans = spans_of("un **fuerte** y *leve* y ~~tachado~~");
        let delimiters: Vec<&str> = spans
            .iter()
            .filter(|(_, id)| *id == HighlightId::Keyword)
            .map(|(text, _)| text.as_str())
            .collect();
        assert_eq!(delimiters, ["**", "**", "*", "*", "~~", "~~"]);
        assert!(!spans.iter().any(|(text, _)| text.contains("fuerte")));
        // snake_case is not emphasis.
        assert!(spans_of("mi_variable_larga").is_empty());
    }

    #[test]
    fn inline_code_links_and_mentions() {
        let spans = spans_of("usá `cargo test` y [la guía](https://x.y) sin *énfasis* `a*b*c`");
        assert!(has(&spans, "`cargo test`", HighlightId::String));
        assert!(has(&spans, "[la guía]", HighlightId::Function));
        assert!(has(&spans, "(https://x.y)", HighlightId::Comment));
        assert!(
            has(&spans, "`a*b*c`", HighlightId::String),
            "no emphasis inside code"
        );

        let text = "mirá @main.rs, ¿qué tal? y no@main.rs";
        let decorations = decorate(
            text,
            &Arc::new(LanguageRegistry::new()),
            gpui::rgb(0),
            &["@main.rs".to_string()],
        );
        let at = text.find('@').unwrap();
        assert_eq!(
            decorations.highlights,
            vec![(at..at + "@main.rs".len(), HighlightId::Function)],
            "only the token at the start of a word"
        );
    }

    #[test]
    fn a_fenced_block_gets_the_background_the_code_font_and_its_language() {
        let text = "Mirá:\n```python\ndef suma(a, b):\n    return a + b\n```\nListo";
        let decorations = decorate(
            text,
            &Arc::new(LanguageRegistry::new()),
            gpui::rgb(0x282c34),
            &[],
        );
        assert_eq!(
            decorations.row_backgrounds,
            vec![(1..5, gpui::rgb(0x282c34))]
        );
        assert_eq!(decorations.monospace_rows, vec![1..5]);
        let spans: Vec<(String, HighlightId)> = decorations
            .highlights
            .iter()
            .map(|(range, id)| (text[range.clone()].to_string(), *id))
            .collect();
        assert!(has(&spans, "```python", HighlightId::Comment));
        assert!(has(&spans, "```", HighlightId::Comment));
        assert!(has(&spans, "def", HighlightId::Keyword), "{spans:?}");
        assert!(has(&spans, "return", HighlightId::Keyword), "{spans:?}");
        // Sorted and non-overlapping, as the editor wants them.
        assert!(
            decorations
                .highlights
                .windows(2)
                .all(|pair| pair[0].0.end <= pair[1].0.start)
        );
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_end_while_typing() {
        let text = "hola\n```rust\nfn main() {";
        let decorations = decorate(text, &Arc::new(LanguageRegistry::new()), gpui::rgb(0), &[]);
        assert_eq!(decorations.monospace_rows, vec![1..3]);
        assert!(decorations.is_monospace(2));
        assert!(!decorations.is_monospace(0));
    }

    #[test]
    fn markup_inside_a_fence_is_left_alone() {
        let spans = spans_of("```\n# no es título\n- ni lista\n```");
        assert!(
            !spans.iter().any(|(_, id)| *id == HighlightId::Keyword),
            "{spans:?}"
        );
    }

    #[test]
    fn a_thematic_break_is_a_comment_not_a_list() {
        let spans = spans_of("---\n* * *");
        assert!(has(&spans, "---", HighlightId::Comment));
        assert!(has(&spans, "* * *", HighlightId::Comment));
    }
}
