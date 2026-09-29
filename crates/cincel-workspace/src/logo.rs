//! Cincel's own chisel mark, embedded so the empty state and the title bar
//! draw it instead of a generic gpui-kit icon.
//!
//! The mark is painted from the PNG renders of the app icon
//! (`assets/logo/dev.cincel.Cincel-128.png` and `-32.png`, byte-for-byte
//! copies of `packaging/icons/png/*/apps/dev.cincel.Cincel.png`), so it
//! shows in full color: GPUI paints SVGs as a single-color mask, which turned
//! the icon (background included) into a plain square. The SVG stays
//! embedded too, unmodified, as the reference of the mark;
//! `packaging/icons/dev.cincel.Cincel.svg` is the one place a human edits it
//! (it also feeds the desktop icon renders, `packaging/icons/render.sh`).

use std::sync::{Arc, LazyLock};

use gpui::{Image, ImageFormat, Img, Pixels, Styled as _, img, px};

/// The embedded bytes of Cincel's app icon (SVG), unmodified.
pub const SVG: &[u8] = include_bytes!("../assets/logo/dev.cincel.Cincel.svg");
/// The 128 px render, for the empty state.
pub const PNG_128: &[u8] = include_bytes!("../assets/logo/dev.cincel.Cincel-128.png");
/// The 32 px render, for the title bar.
pub const PNG_32: &[u8] = include_bytes!("../assets/logo/dev.cincel.Cincel-32.png");

static IMAGE_128: LazyLock<Arc<Image>> =
    LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, PNG_128.to_vec())));
static IMAGE_32: LazyLock<Arc<Image>> =
    LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, PNG_32.to_vec())));

/// Cincel's chisel mark at `size`, in its own colors. Callers add their own
/// `debug_selector` (`crate::workspace::render_empty_state`,
/// `crate::title_menu::render_title_bar`).
pub fn logo(size: Pixels) -> Img {
    let image = if size > px(40.) {
        Arc::clone(&IMAGE_128)
    } else {
        Arc::clone(&IMAGE_32)
    };
    img(image).flex_shrink_0().size(size)
}
