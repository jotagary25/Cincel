//! Long code blocks, folded (`docs/specs/10-etapa7-ronda2.md` §10, R7–R9).
//!
//! An answer (or a Markdown piece of a user bubble) is cut into **segments**:
//! runs of ordinary Markdown ([`SegmentKind::Prose`]) and top-level fenced
//! code blocks with more than [`CODE_FOLD_MIN_LINES`]` - 1` lines of code
//! ([`SegmentKind::LongCode`]). Each segment gets its own `gpui-kit`
//! `TextViewState`, so a long block can be clipped by its own style without
//! touching the rest of the answer. No GPUI here: this is plain text work.

use std::ops::Range;

/// A fenced block with at least this many lines of code (fences not counted)
/// is folded.
pub const CODE_FOLD_MIN_LINES: usize = 21;

/// Lines of code a folded block shows.
pub const CODE_FOLD_VISIBLE_LINES: usize = 12;

/// Height of the fade over the bottom of a folded block, in px before the
/// interface zoom (`docs/specs/10-etapa7-ronda2.md` §10.2).
pub const CODE_FOLD_FADE_HEIGHT: f32 = 40.;

/// What a segment of Markdown is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    /// Ordinary Markdown, short code blocks and indented ones included.
    Prose,
    /// A top-level fenced block long enough to fold.
    LongCode {
        /// Lines of code, without the fences.
        lines: usize,
        /// Whether its closing fence has arrived.
        closed: bool,
    },
}

impl SegmentKind {
    /// Whether two kinds are the same variant (a growing block keeps its kind
    /// while its line count changes).
    #[must_use]
    pub fn same_variant(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Prose, Self::Prose) | (Self::LongCode { .. }, Self::LongCode { .. })
        )
    }

    /// Lines hidden while folded (`total - 12`), or `None` for prose.
    #[must_use]
    pub fn hidden_lines(self) -> Option<usize> {
        match self {
            Self::Prose => None,
            Self::LongCode { lines, .. } => Some(lines.saturating_sub(CODE_FOLD_VISIBLE_LINES)),
        }
    }

    /// The text of the button at the foot of a long block: "Ver más (N
    /// líneas)" while folded, "Ver menos" unfolded; `None` for prose.
    #[must_use]
    pub fn toggle_label(self, expanded: bool) -> Option<String> {
        let hidden = self.hidden_lines()?;
        Some(if expanded {
            "Ver menos".to_string()
        } else {
            format!("Ver más ({hidden} líneas)")
        })
    }
}

/// A byte range of the Markdown and what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdSegment {
    /// Bytes of the source text.
    pub range: Range<usize>,
    /// Prose or a long code block.
    pub kind: SegmentKind,
}

/// Which long block of the transcript: entry, piece of a user bubble (0 for
/// an answer) and segment. The panel keeps the unfolded ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CodeBlockKey {
    /// Index of the [`crate::Entry`].
    pub entry: usize,
    /// Markdown piece of a user bubble; 0 in an answer.
    pub piece: usize,
    /// Segment inside the entry (or the piece).
    pub segment: usize,
}

impl CodeBlockKey {
    /// The suffix of the block's selectors: `{entry}-{piece}-{segment}`.
    #[must_use]
    pub fn selector_suffix(self) -> String {
        format!("{}-{}-{}", self.entry, self.piece, self.segment)
    }
}

/// The fence a line opens or closes with: its character and how many.
fn fence_run(line: &str) -> Option<(u8, usize)> {
    let bytes = line.as_bytes();
    let first = *bytes.first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let count = bytes.iter().take_while(|byte| **byte == first).count();
    (count >= 3).then_some((first, count))
}

/// Whether `line` closes a fence of `count` × `ch`: at most three spaces of
/// indentation, the same character at least `count` times and nothing else
/// but spaces.
fn closes(line: &str, ch: u8, count: usize) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let run = rest.bytes().take_while(|byte| *byte == ch).count();
    run >= count && rest[run..].trim_matches([' ', '\t']).is_empty()
}

/// Cuts `md` into segments (§10.3).
///
/// An opening fence starts in column 0 with three or more `` ` `` or `~`; it
/// closes on the first later line with the same character repeated at least
/// as many times and nothing else but spaces. A block with more than 20 lines
/// of code, closed or still open at the end of the text, is a
/// [`SegmentKind::LongCode`] whose range holds its fences; everything else is
/// joined into [`SegmentKind::Prose`] segments, and a prose segment that is
/// empty or only whitespace is not emitted. A fence with indentation (inside a
/// list or a quote) is never folded (R8): its lines are skipped as a whole, so
/// nothing inside it is taken for a top-level fence.
#[must_use]
pub fn split_markdown(md: &str) -> Vec<MdSegment> {
    // Every line with its start offset; the last one may lack its `\n`.
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0;
    for line in md.split_inclusive('\n') {
        lines.push((offset, line));
        offset += line.len();
    }

    let mut segments = Vec::new();
    let mut prose_start = 0;
    let push_prose = |segments: &mut Vec<MdSegment>, range: Range<usize>| {
        if !md[range.clone()].trim().is_empty() {
            segments.push(MdSegment {
                range,
                kind: SegmentKind::Prose,
            });
        }
    };

    let mut ix = 0;
    while ix < lines.len() {
        let (start, line) = lines[ix];
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        let Some((ch, count)) = fence_run(&line[indent..]) else {
            ix += 1;
            continue;
        };
        // A backtick fence's info string cannot hold a backtick (CommonMark):
        // "```a`b" is inline code, not a fence.
        if ch == b'`' && line[indent + count..].contains('`') {
            ix += 1;
            continue;
        }
        let close = (ix + 1..lines.len()).find(|&later| closes(lines[later].1, ch, count));
        let code_lines = match close {
            Some(close) => close - ix - 1,
            None => lines.len() - ix - 1,
        };
        let end = match close {
            Some(close) => lines[close].0 + lines[close].1.len(),
            None => md.len(),
        };
        if indent == 0 && code_lines >= CODE_FOLD_MIN_LINES {
            push_prose(&mut segments, prose_start..start);
            segments.push(MdSegment {
                range: start..end,
                kind: SegmentKind::LongCode {
                    lines: code_lines,
                    closed: close.is_some(),
                },
            });
            prose_start = end;
        }
        ix = match close {
            Some(close) => close + 1,
            None => lines.len(),
        };
    }
    push_prose(&mut segments, prose_start..md.len());
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(fence: &str, lines: usize) -> String {
        let mut text = format!("{fence}rust\n");
        for n in 0..lines {
            text.push_str(&format!("let x{n} = {n};\n"));
        }
        text.push_str(fence);
        text.push('\n');
        text
    }

    fn kinds(md: &str) -> Vec<SegmentKind> {
        split_markdown(md).into_iter().map(|s| s.kind).collect()
    }

    #[test]
    fn text_without_blocks_is_one_prose_segment() {
        let md = "Hola.\n\nUna lista:\n- uno\n- dos\n";
        assert_eq!(
            split_markdown(md),
            vec![MdSegment {
                range: 0..md.len(),
                kind: SegmentKind::Prose
            }]
        );
        assert!(split_markdown("").is_empty());
        assert!(split_markdown("  \n\n ").is_empty(), "solo espacios");
    }

    #[test]
    fn backtick_and_tilde_fences_fold() {
        for fence in ["```", "~~~"] {
            let code = block(fence, 30);
            let md = format!("Antes.\n\n{code}\nDespués.\n");
            let segments = split_markdown(&md);
            assert_eq!(segments.len(), 3, "{fence}: {segments:?}");
            assert_eq!(segments[0].kind, SegmentKind::Prose);
            assert_eq!(
                segments[1].kind,
                SegmentKind::LongCode {
                    lines: 30,
                    closed: true
                }
            );
            assert_eq!(
                &md[segments[1].range.clone()],
                code,
                "las vallas van dentro"
            );
            assert_eq!(&md[segments[2].range.clone()], "\nDespués.\n");
        }
    }

    #[test]
    fn twenty_lines_do_not_fold_and_twenty_one_do() {
        assert_eq!(kinds(&block("```", 20)), vec![SegmentKind::Prose]);
        assert_eq!(
            kinds(&block("```", 21)),
            vec![SegmentKind::LongCode {
                lines: 21,
                closed: true
            }]
        );
        assert_eq!(
            SegmentKind::LongCode {
                lines: 21,
                closed: true
            }
            .hidden_lines(),
            Some(9)
        );
    }

    #[test]
    fn a_longer_closing_fence_closes_and_a_shorter_one_does_not() {
        let mut md = String::from("````\n");
        for n in 0..25 {
            md.push_str(&format!("línea {n}\n"));
        }
        md.push_str("```\n"); // shorter: still code
        md.push_str("`````  \n"); // longer, trailing spaces: closes
        md.push_str("fin\n");
        let segments = split_markdown(&md);
        assert_eq!(
            segments[0].kind,
            SegmentKind::LongCode {
                lines: 26,
                closed: true
            }
        );
        assert_eq!(&md[segments[1].range.clone()], "fin\n");
        // A different character does not close either.
        let mut tilde = String::from("~~~\n");
        for _ in 0..22 {
            tilde.push_str("x\n");
        }
        tilde.push_str("```\nmás\n");
        assert_eq!(
            kinds(&tilde),
            vec![SegmentKind::LongCode {
                lines: 24,
                closed: false
            }]
        );
    }

    #[test]
    fn a_block_still_open_at_the_end_folds_once_it_is_long() {
        let mut md = String::from("Mirá:\n```python\n");
        for n in 0..20 {
            md.push_str(&format!("print({n})\n"));
        }
        assert_eq!(kinds(&md), vec![SegmentKind::Prose], "20 líneas, abierto");
        md.push_str("print(20)"); // the 21st line, without its newline yet
        let segments = split_markdown(&md);
        assert_eq!(
            segments[1].kind,
            SegmentKind::LongCode {
                lines: 21,
                closed: false
            }
        );
        assert_eq!(segments[1].range.end, md.len());
        assert_eq!(&md[segments[0].range.clone()], "Mirá:\n");
    }

    #[test]
    fn several_blocks_each_get_their_segment() {
        let md = format!(
            "Uno:\n{}\n{}\nDos:\n{}",
            block("```", 22),
            block("```", 3),
            block("~~~", 40)
        );
        assert_eq!(
            kinds(&md),
            vec![
                SegmentKind::Prose,
                SegmentKind::LongCode {
                    lines: 22,
                    closed: true
                },
                SegmentKind::Prose,
                SegmentKind::LongCode {
                    lines: 40,
                    closed: true
                },
            ]
        );
        // Two long blocks back to back: no empty prose between them.
        let md = format!("{}\n{}", block("```", 25), block("```", 25));
        assert_eq!(split_markdown(&md).len(), 2);
    }

    #[test]
    fn an_indented_block_is_not_folded() {
        let mut md = String::from("1. Paso uno:\n\n   ```rust\n");
        for n in 0..30 {
            md.push_str(&format!("   let x{n} = {n};\n"));
        }
        md.push_str("   ```\n2. Paso dos\n");
        assert_eq!(kinds(&md), vec![SegmentKind::Prose]);
        // In a quote, too.
        let mut quote = String::from("> ```\n");
        for _ in 0..25 {
            quote.push_str("> x\n");
        }
        quote.push_str("> ```\n");
        assert_eq!(kinds(&quote), vec![SegmentKind::Prose]);
    }

    #[test]
    fn a_backtick_line_with_more_backticks_is_not_a_fence() {
        let mut md = String::from("```a`b\n");
        for _ in 0..25 {
            md.push_str("x\n");
        }
        assert_eq!(kinds(&md), vec![SegmentKind::Prose]);
    }
}
