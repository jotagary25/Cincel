//! asteroid-editor: ver `docs/specs/modulos/editor.md`.
//!
//! E0 vertical slice: a GPUI element that renders a buffer with read-only
//! phantom rows (the deleted lines of a review hunk) spliced between the real
//! lines, following the "text splice" design of Zed described in
//! `docs/research/04-repos-zed-lapce.md` (adenda 04-c).

pub mod display_map;
pub mod element;
pub mod theme;
pub mod view;

#[cfg(all(test, feature = "test-support"))]
mod tests;

pub use display_map::{
    BufferRow, DiffTransformMap, DisplayCell, DisplayMap, DisplayPoint, DisplayRow, PhantomHunk,
    RowKind,
};
pub use element::EditorElement;
pub use view::{EditorStyle, EditorView, bind_default_keys, editor};
