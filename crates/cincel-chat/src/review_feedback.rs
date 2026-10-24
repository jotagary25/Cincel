//! The review feedback block is never shown nor stored
//! (`docs/specs/12-etapa7-ronda4.md` §3).
//!
//! When the user rejects or edits what the agent wrote, the workspace sends
//! the difference back as a `<user_review_feedback>` text block in front of
//! the next prompt (`cincel_acp::protocol::wrap_review_feedback`). The agent
//! stores it with the prompt, so a `session/load` replays it as a
//! `user_message_chunk`; builds before 0.2.3 painted it as a user bubble and
//! saved it with the conversation. The tags come from `cincel-acp`
//! ([`REVIEW_FEEDBACK_OPEN`], [`REVIEW_FEEDBACK_CLOSE`]), never a copy (D1).

use cincel_acp::acp::schema::v1::{ContentBlock, SessionUpdate};
use cincel_acp::{REVIEW_FEEDBACK_CLOSE, REVIEW_FEEDBACK_OPEN};

use crate::model::{Entry, MessageBlock};

/// Removes every complete `<user_review_feedback>…</user_review_feedback>`
/// block from `text`, with the line breaks that separate it from the rest
/// (D2).
///
/// Returns what is left, or `None` when nothing but whitespace is left. An
/// opening tag without its closing tag is not the block and stays as it is.
#[must_use]
pub fn strip_review_feedback(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(REVIEW_FEEDBACK_OPEN) {
        let body_start = open + REVIEW_FEEDBACK_OPEN.len();
        let Some(close) = rest[body_start..].find(REVIEW_FEEDBACK_CLOSE) else {
            break;
        };
        let close = body_start + close;
        // With two opening tags before one closing tag, the block is the one
        // the closing tag ends; the first tag is the user's own text.
        let start = rest[body_start..close]
            .rfind(REVIEW_FEEDBACK_OPEN)
            .map_or(open, |inner| body_start + inner);
        let after = &rest[close + REVIEW_FEEDBACK_CLOSE.len()..];
        let after = after.trim_start_matches(['\r', '\n']);
        out.push_str(&rest[..start]);
        if after.is_empty() {
            // The block closed the text: the breaks before it go too.
            let kept = out.trim_end_matches(['\r', '\n']).len();
            out.truncate(kept);
        }
        rest = after;
    }
    out.push_str(rest);
    (!out.trim().is_empty()).then_some(out)
}

/// Whether `text` may hold the block: the cheap test before
/// [`strip_review_feedback`], so ordinary text is never copied.
fn may_hold_feedback(text: &str) -> bool {
    text.contains(REVIEW_FEEDBACK_OPEN) && text.contains(REVIEW_FEEDBACK_CLOSE)
}

/// `update` without the review feedback block (R1, R2): `None` when a user
/// text chunk was only that block, the chunk with the rest of its text when
/// it held more, and any other update unchanged.
pub(crate) fn without_review_feedback(mut update: SessionUpdate) -> Option<SessionUpdate> {
    if let SessionUpdate::UserMessageChunk(chunk) = &mut update
        && let ContentBlock::Text(text) = &mut chunk.content
        && may_hold_feedback(&text.text)
    {
        text.text = strip_review_feedback(&text.text)?;
    }
    Some(update)
}

/// Cleans a stored transcript (R3): every text block of every user message
/// goes through [`strip_review_feedback`], and a message that is left
/// without blocks is removed. Entries without the block are not touched.
pub(crate) fn strip_from_entries(entries: &mut Vec<Entry>) {
    entries.retain_mut(|entry| {
        let Entry::UserMessage(message) = entry else {
            return true;
        };
        let mut stripped = false;
        message.blocks.retain_mut(|block| match block {
            MessageBlock::Text(text) if may_hold_feedback(text) => {
                stripped = true;
                match strip_review_feedback(text) {
                    Some(rest) => {
                        *text = rest;
                        true
                    }
                    None => false,
                }
            }
            _ => true,
        });
        !(stripped && message.blocks.is_empty())
    });
}
