//! asteroid-text: the text buffer. See `docs/specs/modulos/text.md`.
//!
//! No graphics, no GPUI, no async: a [`Buffer`] is a plain value that can be
//! unit-tested with `cargo test`.
//!
//! # What it gives you
//!
//! - a rope ([`ropey`]) with byte-offset edits and a monotonic [`Buffer::version`],
//! - [`Anchor`]s and [`AnchorMap`]/[`AnchorSet`] containers that survive edits,
//! - transactions ([`Buffer::start_transaction`], [`Buffer::edit`],
//!   [`Buffer::edit_many`], [`Buffer::end_transaction`]) with [`EditSource`],
//!   undo/redo per transaction (also [`Buffer::undo_with_ranges`], which says
//!   *what* changed) and 300 ms grouping of consecutive `User` transactions,
//! - [`BufferEvent::Edited`] delivered through [`Buffer::subscribe`],
//! - minimal diffs ([`diff::minimal_edits`], [`Buffer::set_text_minimal`]) so an
//!   agent write or a reload from disk lands as the few edits that actually
//!   changed instead of replacing the whole rope,
//! - line-ending and BOM detection on load, LF in memory, re-applied by
//!   [`Buffer::to_bytes`]; invalid UTF-8 either fails ([`Buffer::from_bytes`])
//!   or loads read-only with replacement characters
//!   ([`Buffer::from_bytes_lossy`]),
//! - saved-state tracking ([`Buffer::mark_saved`], [`Buffer::is_dirty`]) and a
//!   read-only flag ([`Buffer::set_read_only`], [`Buffer::try_edit`]),
//! - [`Point`] ⇄ offset and UTF-8 ⇄ UTF-16 conversions, all `char`-boundary safe,
//! - [`BufferSnapshot`]: a cheap, `Send` copy for background work, comparable
//!   with [`BufferSnapshot::ptr_eq`], [`BufferSnapshot::same_text_as`] and a
//!   lazily cached [`BufferSnapshot::content_hash`].
//!
//! # Events
//!
//! One [`BufferEvent::Edited`] is emitted per *edit step* (one `edit` call, or
//! one op of an `undo`/`redo`), not per transaction, so that a consumer can
//! turn each event into exactly one tree-sitter `InputEdit`.
//!
//! There are two ways to receive them, and they can be used together:
//!
//! - **push**: [`Buffer::subscribe`] takes a
//!   `Box<dyn FnMut(&BufferEvent) + Send>` that runs synchronously right after
//!   each step. It receives the event only, so it cannot re-enter the buffer;
//!   it is meant to forward the event wherever the owner needs it (a GPUI
//!   `cx.emit`, a channel, an [`AnchorMap`]). The `Send` bound is what keeps
//!   [`Buffer`] itself `Send`, so a `BufferStore` can hold one behind a mutex.
//! - **pull**: [`Buffer::record_events`] turns on an internal queue that the
//!   owner empties with [`Buffer::drain_events`]. Off by default, and free
//!   while it is off.

mod anchor;
mod buffer;
pub mod diff;

#[cfg(test)]
mod tests;

pub use anchor::{Anchor, AnchorId, AnchorMap, AnchorSet, Bias};
pub use buffer::{
    Buffer, BufferEvent, BufferSnapshot, DEFAULT_GROUP_INTERVAL, EditError, EditSource, LineEnding,
    LoadError, Point, SubscriptionId,
};
pub use diff::minimal_edits;
pub use ropey::{Rope, RopeSlice};
