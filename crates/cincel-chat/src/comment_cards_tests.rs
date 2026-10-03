//! Review comments in the chat over the GPUI test harness
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.6, §6.11):
//! the tags of the unsent comments in a row of their own inside the composer
//! box, their `×` (a real click) and their click, the cards of a sent
//! message, `not_sent`, and the persistence of `MessageBlock::Comment` and
//! `MessageBlock::Image` in a version 2 conversation, with the migration of a
//! version 1 file.

use std::path::PathBuf;

use cincel_acp::protocol::PromptBlock;
use gpui::{Modifiers, TestAppContext, VisualTestContext, px};
use image::ImageFormat;

use crate::attachments_tests::{
    LABEL, Recorded, connect, encode, gradient, open, png_file, with_window,
};
use crate::comments::*;
use crate::model::*;

fn pending(id: u64, label: &str, path: &str, line: u32) -> PendingComment {
    PendingComment {
        id,
        label: label.into(),
        tooltip: format!("{path}, línea {line}\nnota {id}"),
        path: PathBuf::from(path),
        line,
    }
}

fn card(code_lines: usize, state: &str) -> SentCommentCard {
    SentCommentCard {
        display_path: "src/calc.py".into(),
        path: PathBuf::from("/p/src/calc.py"),
        first_line: 24,
        last_line: 24 + code_lines as u32 - 1,
        kind: SentCommentKind::Lines,
        state_label: state.into(),
        code: Some(
            (0..code_lines)
                .map(|n| format!("linea_{n} = {n}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        removed: None,
        truncated_lines: 0,
        lang: Some("py".into()),
        text: "Hacelo con un diccionario".into(),
        not_sent: false,
    }
}

fn click_at(visual: &mut VisualTestContext, selector: &'static str) {
    let spot = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} no se pintó"))
        .center();
    visual.simulate_click(spot, Modifiers::default());
    visual.run_until_parked();
}

fn log(recorded: &Recorded) -> String {
    recorded.log()
}

/// §6.6, D13: one tag per unsent comment, ordered by file and line, in a row
/// of its own inside the box, above the thumbnails and the text.
#[gpui::test]
fn comment_tags_sit_in_their_own_row_inside_the_box(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let png = png_file(dir.path(), "a.png", 10, 10);
    panel.update(&mut visual, |panel, cx| {
        panel.set_pending_comments(
            vec![
                pending(2, "util.py:7", "/p/util.py", 7),
                pending(1, "calc.py:24-28", "/p/calc.py", 24),
            ],
            cx,
        );
        panel.attach_paths(&[png], cx);
    });
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| {
        let labels: Vec<&str> = panel
            .pending_comments()
            .iter()
            .map(|comment| comment.label.as_str())
            .collect();
        assert_eq!(
            labels,
            ["calc.py:24-28", "util.py:7"],
            "por archivo y línea"
        );
    });

    let input_box = visual.debug_bounds("chat-input-box").expect("caja");
    let tags = visual
        .debug_bounds("chat-comment-tags")
        .expect("fila de etiquetas");
    let thumbs = visual
        .debug_bounds("chat-attachments")
        .expect("fila de miniaturas");
    // The paperclip sits in the text's row, at its bottom left.
    let text_row = visual.debug_bounds("chat-attach").expect("el clip");
    assert!(
        input_box.contains(&tags.origin),
        "{tags:?} en {input_box:?}"
    );
    assert!(
        tags.bottom() <= thumbs.top(),
        "etiquetas arriba de las miniaturas"
    );
    assert!(
        thumbs.bottom() <= text_row.top(),
        "miniaturas arriba del texto"
    );
    let first = visual.debug_bounds("comment-tag-0").expect("etiqueta");
    let remove = visual.debug_bounds("comment-tag-remove-0").expect("su ×");
    assert!(
        first.contains(&remove.center()),
        "la × va dentro de la etiqueta"
    );
    assert!(remove.size.width <= px(16.));
}

/// §6.6: the `×` (a real click) emits `RemoveComment` and the tag goes; a
/// click on the tag emits `OpenLocation` with its file and line.
#[gpui::test]
fn the_x_of_a_tag_removes_the_comment_and_a_click_opens_the_file(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_pending_comments(
            vec![
                pending(5, "calc.py:24-28", "/p/calc.py", 24),
                pending(9, "util.py:7", "/p/util.py", 7),
            ],
            cx,
        );
    });
    visual.run_until_parked();

    click_at(&mut visual, "comment-tag-remove-0");
    assert!(
        log(&recorded).contains("RemoveComment { id: 5 }"),
        "{}",
        log(&recorded)
    );
    assert!(
        !log(&recorded).contains("OpenLocation"),
        "la × no abre el archivo"
    );
    panel.read_with(&visual, |panel, _| {
        assert_eq!(panel.pending_comments().len(), 1);
        assert_eq!(panel.pending_comments()[0].id, 9);
    });

    recorded.clear();
    click_at(&mut visual, "comment-tag-0");
    assert!(
        log(&recorded).contains("OpenLocation { path: \"/p/util.py\", line: 7 }"),
        "{}",
        log(&recorded)
    );
}

/// §6.2.7, §6.7: the comments do not travel in the prompt blocks (the
/// workspace puts them in the review feedback), and neither sending nor a new
/// conversation drops them from the box: the workspace does, after taking
/// them.
#[gpui::test]
fn the_tags_stay_until_the_workspace_takes_them(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    panel.update(&mut visual, |panel, cx| {
        panel.set_pending_comments(vec![pending(1, "calc.py:3", "/p/calc.py", 3)], cx);
    });
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("atendé mis comentarios", window, cx);
        panel.send(window, cx);
    });
    visual.run_until_parked();
    let prompts = recorded.prompts.borrow();
    assert_eq!(
        prompts[0].1,
        vec![PromptBlock::Text("atendé mis comentarios".into())]
    );
    drop(prompts);
    with_window(&panel, &mut visual, |panel, window, cx| {
        assert_eq!(panel.pending_comments().len(), 1);
        panel.start_new_conversation(window, cx);
        assert_eq!(panel.pending_comments().len(), 1, "son del proyecto");
    });
}

/// §6.3, §6.6: the cards of a sent message, under its text and its images,
/// with file, lines, state, the code (six lines, then "… N líneas más", which
/// unfolds) and the text; the header opens the file; `not_sent` adds its line.
#[gpui::test]
fn sent_comment_cards_are_painted_under_the_text_and_the_images(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let png = png_file(dir.path(), "a.png", 30, 30);
    panel.update(&mut visual, |panel, cx| panel.attach_paths(&[png], cx));
    visual.run_until_parked();
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("atendé mis comentarios", window, cx);
        panel.send(window, cx);
        panel.attach_sent_comments(
            vec![
                card(10, COMMENT_STATE_REJECTED),
                card(2, COMMENT_STATE_NO_AGENT_CHANGE),
            ],
            cx,
        );
    });
    visual.run_until_parked();

    let bubble = visual.debug_bounds("user-bubble-0").expect("burbuja");
    let image = visual.debug_bounds("sent-image-0-0").expect("imagen");
    let first = visual.debug_bounds("comment-card-0-0").expect("tarjeta");
    let second = visual
        .debug_bounds("comment-card-0-1")
        .expect("segunda tarjeta");
    assert!(bubble.contains(&first.origin) && bubble.contains(&second.origin));
    assert!(first.top() >= image.bottom(), "debajo de las imágenes");
    assert!(second.top() >= first.bottom());
    assert!(visual.debug_bounds("comment-card-text-0-0").is_some());
    assert!(
        visual.debug_bounds("comment-card-more-0-0").is_some(),
        "10 líneas: «… 4 líneas más»"
    );
    assert!(
        visual.debug_bounds("comment-card-more-0-1").is_none(),
        "2 líneas entran"
    );
    panel.read_with(&visual, |panel, _| {
        let Entry::UserMessage(message) = &panel.entries()[0] else {
            panic!("mensaje");
        };
        let cards: Vec<&SentCommentCard> = message
            .blocks
            .iter()
            .filter_map(|block| match block {
                MessageBlock::Comment(card) => Some(card),
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].header_detail(), " · líneas 24-33 · rechazado");
    });

    let folded = visual
        .debug_bounds("comment-card-code-0-0")
        .expect("código");
    click_at(&mut visual, "comment-card-more-0-0");
    let unfolded = visual
        .debug_bounds("comment-card-code-0-0")
        .expect("código");
    assert!(unfolded.size.height > folded.size.height, "se despliega");

    recorded.clear();
    click_at(&mut visual, "comment-card-header-0-0");
    assert!(
        log(&recorded).contains("OpenLocation { path: \"/p/src/calc.py\", line: 24 }"),
        "{}",
        log(&recorded)
    );

    assert!(visual.debug_bounds("comment-card-not-sent-0-0").is_none());
    panel.update(&mut visual, |panel, cx| panel.mark_comments_not_sent(cx));
    visual.run_until_parked();
    assert!(visual.debug_bounds("comment-card-not-sent-0-0").is_some());
    assert!(visual.debug_bounds("comment-card-not-sent-0-1").is_some());
}

/// D7: a version 2 conversation keeps its comment cards and the image
/// references; the JSON carries no image bytes, and `images_to_store` hands
/// the workspace what it has to write.
#[gpui::test]
fn comments_and_image_references_persist_without_base64(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    connect(&panel, &mut visual, true);
    let bytes = encode(&gradient(20, 20), ImageFormat::Png);
    panel.update(&mut visual, |panel, cx| {
        panel.attach_images(
            vec![crate::AttachmentSource::Bytes {
                name: "captura.png".into(),
                bytes: bytes.clone(),
            }],
            cx,
        );
    });
    visual.run_until_parked();
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("mirá", window, cx);
        panel.send(window, cx);
        panel.attach_sent_comments(vec![card(3, COMMENT_STATE_ACCEPTED)], cx);
    });
    let entries = panel.read_with(&visual, |panel, _| panel.entries().to_vec());
    let mut conversation = Conversation::new("c1", "claude-acp", LABEL, PathBuf::from("/p"), 1);
    conversation.entries = entries;
    assert_eq!(conversation.version, 2);

    let stored = conversation.images_to_store();
    let sha = crate::attachments::sha256_hex(&bytes);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0, format!("{sha}.png"));
    assert_eq!(&*stored[0].1, bytes.as_slice());

    let json = serde_json::to_string(&conversation).expect("json");
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    assert!(!json.contains(&encoded[..40]), "sin base64 en el JSON");
    assert!(!json.contains("\"data\""), "{json}");
    let back: Conversation = serde_json::from_str(&json).expect("se lee");
    let Entry::UserMessage(message) = &back.entries[0] else {
        panic!("mensaje");
    };
    assert_eq!(message.blocks.len(), 3, "{:?}", message.blocks);
    let MessageBlock::Image(image) = &message.blocks[1] else {
        panic!("imagen");
    };
    assert_eq!(image.file, format!("{sha}.png"));
    assert_eq!(
        (image.width, image.height, image.bytes_len),
        (20, 20, bytes.len() as u64)
    );
    assert!(image.data.is_none(), "los bytes no viajan en el JSON");
    assert_eq!(
        message.blocks[2],
        MessageBlock::Comment(card(3, COMMENT_STATE_ACCEPTED))
    );
    assert!(back.images_to_store().is_empty());
}

/// D7: a conversation saved by the 0.1.0 (version 1, no image or comment
/// blocks) loads unchanged and migrates to version 2.
#[gpui::test]
fn a_version_1_conversation_loads_and_migrates(cx: &mut TestAppContext) {
    let v1 = r#"{
        "version": 1,
        "id": "20260901-aaaa",
        "agent_id": "claude-acp",
        "agent_name": "Claude · personal",
        "connection_id": "c-claude",
        "session_id": "s-viejo",
        "cwd": "/p",
        "created_at": 1790000000,
        "updated_at": 1790000100,
        "title": "Extraer el parser",
        "entries": [
            {"UserMessage": {"blocks": [{"Text": "Extraer el parser de "}, {"File": "/p/src/main.rs"}]}},
            {"AgentText": {"markdown": "Listo.", "streaming": false}},
            {"TurnSeparator": {"stop_reason": "turno terminado", "duration_secs": 3}}
        ]
    }"#;
    let conversation: Conversation = serde_json::from_str(v1).expect("la versión 1 se lee");
    assert_eq!(conversation.version, 1);
    let conversation = conversation.migrate();
    assert_eq!(conversation.version, CONVERSATION_VERSION);
    assert_eq!(CONVERSATION_VERSION, 2);
    assert_eq!(conversation.entries.len(), 3);
    let Entry::UserMessage(message) = &conversation.entries[0] else {
        panic!("mensaje");
    };
    assert!(
        message
            .blocks
            .iter()
            .all(|block| matches!(block, MessageBlock::Text(_) | MessageBlock::File(_))),
        "sin imágenes ni comentarios"
    );
    assert!(conversation.images_to_store().is_empty());

    // It paints like any other conversation.
    let (panel, mut visual, _) = open(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(conversation, window, cx);
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("user-bubble-0").is_some());
    assert!(visual.debug_bounds("user-images-0").is_none());
    assert!(visual.debug_bounds("comment-card-0-0").is_none());
}
