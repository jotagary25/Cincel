//! asteroid-syntax: tree-sitter highlighting. See `docs/specs/modulos/syntax.md`.
//!
//! No GPUI, no async runtime: a [`LanguageRegistry`] plus one [`SyntaxState`]
//! per open buffer.
//!
//! ```no_run
//! use std::sync::Arc;
//! use asteroid_syntax::{CancelFlag, LanguageRegistry, SyntaxState};
//! use asteroid_text::Buffer;
//!
//! let registry = Arc::new(LanguageRegistry::new());
//! let language = registry.language_for_path("src/main.rs").unwrap();
//! let buffer = Buffer::new("fn main() {}\n");
//!
//! let mut state = SyntaxState::new(registry, language, buffer.snapshot());
//! state.reparse(&CancelFlag::new());                  // background executor
//! let spans = state.highlights(0..buffer.len_bytes()); // visible range only
//! ```
//!
//! # Threading and cancellation
//!
//! [`SyntaxState::apply_event`] is cheap and runs on the UI thread for every
//! [`BufferEvent`](asteroid_text::BufferEvent): it derives the tree-sitter
//! `InputEdit`s and takes a [`BufferSnapshot`](asteroid_text::BufferSnapshot),
//! which is a rope clone, not a copy of the text. The expensive part,
//! [`SyntaxState::reparse`], is meant for `cx.background_executor()`; the UI
//! keeps a [`CancelFlag`] and raises it when a newer edit makes the running
//! parse pointless. [`PARSE_BUDGET_HINT`] is the 5 ms the spec mentions:
//! [`ParseOutcome::Parsed`] reports whether the parse fitted in it, and
//! [`SyntaxState::reparse_within`] turns it into a hard deadline.
//!
//! # Deviations from the spec
//!
//! - The crate uses `tree-sitter` directly instead of `tree-sitter-highlight`.
//!   `Highlighter::highlight` always re-parses the whole document from scratch
//!   and cannot be restricted to a byte range, so it can satisfy neither the
//!   incremental-reparse nor the visible-range requirement. The capture
//!   name -> [`HighlightId`] mapping reproduces
//!   `HighlightConfiguration::configure`'s rule, and injections use the same
//!   `@injection.content` / `@injection.language` conventions, so the bundled
//!   `injections.scm` files work unchanged.
//! - `Parser::set_cancellation_flag` no longer exists in tree-sitter 0.27;
//!   [`CancelFlag`] is checked from the parse progress callback instead.
//! - Injections go one level deep (Markdown fences, HTML `<script>`/`<style>`)
//!   and are rebuilt by `reparse`, not incrementally.

mod highlight;
mod language;
mod state;

#[cfg(test)]
mod tests;

pub use highlight::{HighlightId, HighlightTheme, Rgb};
pub use language::{Language, LanguageRegistry};
pub use state::{CancelFlag, PARSE_BUDGET_HINT, ParseOutcome, SyntaxState};
