//! The order of what a message carries over the GPUI test harness
//! (`docs/specs/10-etapa7-ronda2.md` §5): in the sent bubble the images are
//! above the text and the comment cards under it; in the composer the row of
//! thumbnails is above the text field.
//!
//! Every image is generated here with `image`; nothing binary is in the repo.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use image::ImageFormat;

use crate::attachments_tests::{LABEL, connect, encode, gradient, open, png_file, with_window};
use crate::comments::{SentCommentCard, SentCommentKind};
use crate::model::*;
use crate::panel::ChatPanel;

fn card() -> SentCommentCard {
    SentCommentCard {
        display_path: "src/calc.py".into(),
        path: PathBuf::from("/p/src/calc.py"),
        first_line: 24,
        last_line: 25,
        kind: SentCommentKind::Lines,
        state_label: "rechazado".into(),
        code: Some("a = 1\nb = 2".into()),
        removed: None,
        truncated_lines: 0,
        lang: Some("py".into()),
        text: "Hacelo con un diccionario".into(),
        not_sent: false,
    }
}

/// An image of the sent message with its bytes in memory.
fn image_ref(name: &str, width: u32, height: u32) -> ImageRef {
    let bytes = encode(&gradient(width, height), ImageFormat::Png);
    ImageRef {
        file: String::new(),
        name: name.into(),
        mime_type: "image/png".into(),
        width,
        height,
        bytes_len: bytes.len() as u64,
        data: Some(Arc::from(bytes)),
    }
}

/// Opens a conversation whose only message has `blocks`.
fn load_message(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    blocks: Vec<MessageBlock>,
) {
    let mut conversation = Conversation::new("c1", "claude-acp", LABEL, PathBuf::from("/p"), 0);
    conversation
        .entries
        .push(Entry::UserMessage(UserMessage { blocks }));
    with_window(panel, visual, |panel, window, cx| {
        panel.load_conversation(conversation, window, cx);
    });
    visual.run_until_parked();
}

/// §5.3: with text and images, the bottom edge of `user-images-0` is above the
/// top edge of `user-text-0`; the cards stay under the text.
#[gpui::test]
fn the_images_of_a_sent_message_are_above_the_text_and_the_cards_below(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    load_message(
        &panel,
        &mut visual,
        vec![
            // The text comes first in the blocks, the images after it: the
            // painted order does not depend on the order they were typed.
            MessageBlock::Text("mirá estas capturas".into()),
            MessageBlock::Image(image_ref("a.png", 120, 80)),
            MessageBlock::Image(image_ref("b.png", 60, 90)),
            MessageBlock::Comment(card()),
            MessageBlock::Comment(card()),
        ],
    );
    let bubble = visual.debug_bounds("user-bubble-0").expect("burbuja");
    let images = visual.debug_bounds("user-images-0").expect("imágenes");
    let text = visual.debug_bounds("user-text-0").expect("texto");
    let first = visual.debug_bounds("comment-card-0-0").expect("tarjeta 0");
    let second = visual.debug_bounds("comment-card-0-1").expect("tarjeta 1");
    assert!(
        images.bottom() <= text.top(),
        "las imágenes arriba del texto: {images:?} / {text:?}"
    );
    assert!(
        first.top() >= text.bottom(),
        "las tarjetas debajo del texto: {text:?} / {first:?}"
    );
    assert!(second.top() >= first.bottom());
    assert!(
        bubble.contains(&images.origin) && bubble.contains(&second.origin),
        "todo dentro de la burbuja"
    );
    assert!(
        bubble.top() <= images.top(),
        "las imágenes abren la burbuja"
    );
    // Both thumbnails are in the row, side by side.
    let one = visual.debug_bounds("sent-image-0-0").expect("miniatura 0");
    let two = visual.debug_bounds("sent-image-0-1").expect("miniatura 1");
    assert!(images.contains(&one.origin) && images.contains(&two.origin));
    assert!(one.right() <= two.left(), "una al lado de la otra");
    // The 6 px gap of the bubble separates the three parts.
    assert!(
        text.top() - images.bottom() >= gpui::px(5.5),
        "hueco de 6 px"
    );
}

/// The same through the real path: attach, type, send.
#[gpui::test]
fn a_message_sent_from_the_composer_paints_its_images_above_the_text(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let png = png_file(dir.path(), "a.png", 90, 60);
    panel.update(&mut visual, |panel, cx| panel.attach_paths(&[png], cx));
    visual.run_until_parked();
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("¿qué ves?", window, cx);
        panel.send(window, cx);
    });
    visual.run_until_parked();
    let images = visual.debug_bounds("user-images-0").expect("imágenes");
    let text = visual.debug_bounds("user-text-0").expect("texto");
    assert!(images.bottom() <= text.top(), "{images:?} / {text:?}");
}

/// §5.1: a message without text shows only the thumbnails (and the cards);
/// one without images shows no row of thumbnails.
#[gpui::test]
fn a_message_without_text_shows_only_the_thumbnails_and_one_without_images_none(
    cx: &mut TestAppContext,
) {
    let (panel, mut visual, _) = open(cx);
    load_message(
        &panel,
        &mut visual,
        vec![
            MessageBlock::Image(image_ref("a.png", 50, 50)),
            MessageBlock::Comment(card()),
        ],
    );
    assert!(visual.debug_bounds("user-images-0").is_some());
    assert!(visual.debug_bounds("user-text-0").is_none(), "sin texto");
    let images = visual.debug_bounds("user-images-0").unwrap();
    let cards = visual.debug_bounds("comment-card-0-0").expect("tarjeta");
    assert!(cards.top() >= images.bottom(), "la tarjeta sigue debajo");

    load_message(
        &panel,
        &mut visual,
        vec![
            MessageBlock::Text("solo texto".into()),
            MessageBlock::Comment(card()),
        ],
    );
    assert!(
        visual.debug_bounds("user-images-0").is_none(),
        "sin imágenes"
    );
    let text = visual.debug_bounds("user-text-0").expect("texto");
    let cards = visual.debug_bounds("comment-card-0-0").expect("tarjeta");
    assert!(cards.top() >= text.bottom());
}

/// §5.1, §5.3: in the composer box the row of thumbnails is above the text
/// field (`chat-input`), under the comment tags, and everything is inside the
/// box. Fixed here so it does not change.
#[gpui::test]
fn in_the_composer_the_thumbnails_are_above_the_text_field(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let png = png_file(dir.path(), "a.png", 40, 40);
    panel.update(&mut visual, |panel, cx| panel.attach_paths(&[png], cx));
    visual.run_until_parked();
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("con una captura", window, cx);
    });
    visual.run_until_parked();

    let boxed = visual.debug_bounds("chat-input-box").expect("caja");
    let row = visual
        .debug_bounds("chat-attachments")
        .expect("fila de miniaturas");
    let thumb = visual.debug_bounds("attachment-0").expect("miniatura");
    let input = visual.debug_bounds("chat-input").expect("campo de texto");
    assert!(
        row.bottom() <= input.top(),
        "la fila de miniaturas está sobre el campo: {row:?} / {input:?}"
    );
    assert!(thumb.bottom() <= input.top());
    assert!(
        boxed.contains(&row.origin) && boxed.contains(&input.origin),
        "las dos cosas dentro de la caja"
    );
}
