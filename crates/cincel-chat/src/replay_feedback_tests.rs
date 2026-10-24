//! The review feedback block is never shown nor stored
//! (`docs/specs/12-etapa7-ronda4.md` §3.3): the pure cleaning of D2, the
//! replay filter (A1, A2, A4) and the stored transcripts (A3), with the real
//! panel. Synthetic data only.

use std::path::PathBuf;

use cincel_acp::acp::schema::v1::{
    ContentBlock, ContentChunk, SessionId, SessionUpdate, TextContent,
};
use cincel_acp::protocol::wrap_review_feedback;
use cincel_acp::{AgentEvent, REVIEW_FEEDBACK_CLOSE, REVIEW_FEEDBACK_OPEN};
use gpui::{AnyWindowHandle, Context, Entity, TestAppContext, VisualTestContext, Window};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::review_feedback::strip_review_feedback;
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

/// A feedback block as the workspace sends it, with diff lines.
fn feedback() -> String {
    wrap_review_feedback(
        "--- a/src/parser.rs\n+++ b/src/parser.rs\n@@ -1,2 +1,2 @@\n-fn viejo() {}\n+fn nuevo() {}",
    )
}

// ------------------------------------------------------------ D2, pure

#[test]
fn a_lone_block_leaves_nothing() {
    assert_eq!(strip_review_feedback(&feedback()), None);
    assert_eq!(
        strip_review_feedback(&format!("\n\n{}\n  \n", feedback())),
        None
    );
}

#[test]
fn a_block_at_the_start_goes_with_its_line_breaks() {
    let text = format!("{}\n\nArreglá el parser", feedback());
    assert_eq!(
        strip_review_feedback(&text).as_deref(),
        Some("Arreglá el parser")
    );
}

#[test]
fn a_block_at_the_end_goes_with_its_line_breaks() {
    let text = format!("Arreglá el parser\n\n{}", feedback());
    assert_eq!(
        strip_review_feedback(&text).as_deref(),
        Some("Arreglá el parser")
    );
    let crlf = format!("Arreglá el parser\r\n{}\r\n", feedback());
    assert_eq!(
        strip_review_feedback(&crlf).as_deref(),
        Some("Arreglá el parser")
    );
}

#[test]
fn two_blocks_both_go() {
    let text = format!(
        "{}\nPrimero esto\n\n{}\n\ny después aquello",
        feedback(),
        feedback()
    );
    assert_eq!(
        strip_review_feedback(&text).as_deref(),
        Some("Primero esto\n\ny después aquello")
    );
    let back_to_back = format!("Hola\n\n{}\n{}", feedback(), feedback());
    assert_eq!(
        strip_review_feedback(&back_to_back).as_deref(),
        Some("Hola")
    );
}

#[test]
fn an_opening_tag_without_its_closing_tag_stays() {
    let text = format!("Escribí {REVIEW_FEEDBACK_OPEN} a mano");
    assert_eq!(strip_review_feedback(&text).as_deref(), Some(text.as_str()));
    let alone = REVIEW_FEEDBACK_OPEN.to_string();
    assert_eq!(
        strip_review_feedback(&alone).as_deref(),
        Some(alone.as_str())
    );
    // A stray opening tag before a real block is the user's text.
    let mixed = format!("Ojo con {REVIEW_FEEDBACK_OPEN}\n\n{}", feedback());
    assert_eq!(
        strip_review_feedback(&mixed).as_deref(),
        Some(format!("Ojo con {REVIEW_FEEDBACK_OPEN}").as_str())
    );
}

#[test]
fn a_block_whose_lines_start_with_minus_and_plus_goes_whole() {
    let block = wrap_review_feedback("- quitado\n+ agregado\n-\n+\n--- a\n+++ b");
    let text = format!("{block}\n\nSeguí");
    let left = strip_review_feedback(&text).expect("queda texto");
    assert_eq!(left, "Seguí");
    assert!(!left.contains(REVIEW_FEEDBACK_CLOSE));
}

#[test]
fn text_without_the_block_is_unchanged() {
    let text = "Hola,\n\n- un punto\n+ otro";
    assert_eq!(strip_review_feedback(text).as_deref(), Some(text));
}

// ------------------------------------------------------------ the panel

fn open(cx: &mut TestAppContext) -> (Entity<ChatPanel>, VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_default_keys(cx);
    });
    let window = cx.add_window(|window, cx| {
        ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
    });
    let panel = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let handle: AnyWindowHandle = window.into();
    let visual = VisualTestContext::from_window(handle, cx);
    (panel, visual)
}

fn with_window<R>(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    f: impl FnOnce(&mut ChatPanel, &mut Window, &mut Context<ChatPanel>) -> R,
) -> R {
    visual.update(|window, cx| panel.update(cx, |panel, cx| f(panel, window, cx)))
}

fn user_chunk(text: &str) -> AgentEvent {
    AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    }
}

fn agent_chunk(text: &str) -> AgentEvent {
    AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    }
}

fn user(text: &str) -> Entry {
    Entry::UserMessage(UserMessage {
        blocks: vec![MessageBlock::Text(text.to_string())],
    })
}

fn answer(text: &str) -> Entry {
    Entry::AgentText(AgentText {
        markdown: text.to_string(),
        streaming: false,
        segments: Vec::new(),
    })
}

/// A stored conversation with one exchange.
fn conversation(entries: Vec<Entry>) -> Conversation {
    let mut conversation =
        Conversation::new("c1", "claude-acp", "Claude", PathBuf::from("/proyecto"), 10);
    conversation.session_id = Some("s-vieja".to_string());
    conversation.entries = entries;
    conversation
}

/// Each entry as a short line, to compare transcripts.
fn describe(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry {
            Entry::UserMessage(message) => {
                let blocks: Vec<String> = message
                    .blocks
                    .iter()
                    .map(|block| match block {
                        MessageBlock::Text(text) => text.clone(),
                        other => format!("{other:?}"),
                    })
                    .collect();
                format!("user: {}", blocks.join(" | "))
            }
            Entry::AgentText(text) => format!("agent: {}", text.markdown),
            Entry::Notice(notice) => format!("notice: {}", notice.text),
            other => format!("{other:?}"),
        })
        .collect()
}

fn replaying_panel(cx: &mut TestAppContext) -> (Entity<ChatPanel>, VisualTestContext) {
    let (panel, mut visual) = open(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(
            conversation(vec![user("Arreglá el parser"), answer("Listo.")]),
            window,
            cx,
        );
        panel.begin_replay(cx);
    });
    (panel, visual)
}

#[gpui::test]
fn a1_a_replayed_message_that_is_only_the_block_adds_nothing(cx: &mut TestAppContext) {
    let (panel, mut visual) = replaying_panel(cx);
    with_window(&panel, &mut visual, |panel, _window, cx| {
        panel.handle_event(user_chunk(&feedback()), cx);
        assert_eq!(
            describe(panel.entries()),
            vec!["user: Arreglá el parser", "agent: Listo."]
        );
    });
}

#[gpui::test]
fn a2_block_and_text_in_one_chunk_shows_only_the_new_text(cx: &mut TestAppContext) {
    let (panel, mut visual) = replaying_panel(cx);
    with_window(&panel, &mut visual, |panel, _window, cx| {
        // Already on screen: nothing at all.
        panel.handle_event(
            user_chunk(&format!("{}\n\nArreglá el parser", feedback())),
            cx,
        );
        assert_eq!(panel.entries().len(), 2, "{:?}", describe(panel.entries()));
        // The other order, with a turn that never reached disk: only its text.
        panel.handle_event(
            user_chunk(&format!("Ahora los tests\n\n{}", feedback())),
            cx,
        );
        assert_eq!(
            describe(panel.entries()),
            vec![
                "user: Arreglá el parser",
                "agent: Listo.",
                "user: Ahora los tests"
            ]
        );
    });
}

#[gpui::test]
fn a2_block_and_text_in_two_chunks_shows_only_the_new_text(cx: &mut TestAppContext) {
    let (panel, mut visual) = replaying_panel(cx);
    with_window(&panel, &mut visual, |panel, _window, cx| {
        // Block first, then the text already on screen.
        panel.handle_event(user_chunk(&feedback()), cx);
        panel.handle_event(user_chunk("Arreglá el parser"), cx);
        assert_eq!(panel.entries().len(), 2, "{:?}", describe(panel.entries()));
        // A new turn: text first, then the block.
        panel.handle_event(user_chunk("Ahora los tests"), cx);
        panel.handle_event(user_chunk(&feedback()), cx);
        panel.handle_event(agent_chunk("Hecho."), cx);
        let entries = describe(panel.entries());
        assert_eq!(
            entries,
            vec![
                "user: Arreglá el parser",
                "agent: Listo.",
                "user: Ahora los tests",
                "agent: Hecho."
            ]
        );
        assert!(
            entries
                .iter()
                .all(|line| !line.contains(REVIEW_FEEDBACK_OPEN)),
            "el adjunto no llega a la conversación guardada"
        );
    });
}

#[gpui::test]
fn r2_the_block_is_not_shown_outside_a_replay_either(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    with_window(&panel, &mut visual, |panel, _window, cx| {
        assert!(!panel.is_replaying());
        panel.handle_event(user_chunk(&feedback()), cx);
        assert!(
            panel.entries().is_empty(),
            "{:?}",
            describe(panel.entries())
        );
        panel.handle_event(user_chunk(&format!("{}\nHola", feedback())), cx);
        assert_eq!(describe(panel.entries()), vec!["user: Hola"]);
    });
}

#[gpui::test]
fn a4_a_hand_written_opening_tag_is_shown_as_is(cx: &mut TestAppContext) {
    let (panel, mut visual) = replaying_panel(cx);
    with_window(&panel, &mut visual, |panel, _window, cx| {
        panel.handle_event(user_chunk(REVIEW_FEEDBACK_OPEN), cx);
        assert_eq!(
            describe(panel.entries()),
            vec![
                "user: Arreglá el parser".to_string(),
                "agent: Listo.".to_string(),
                format!("user: {REVIEW_FEEDBACK_OPEN}"),
            ]
        );
    });
}

/// A transcript as builds before the fix stored it: the replayed block as a
/// message of its own, and glued to a message.
fn dirty_entries() -> Vec<Entry> {
    vec![
        user("Arreglá el parser"),
        answer("Listo."),
        user(&feedback()),
        Entry::UserMessage(UserMessage {
            blocks: vec![
                MessageBlock::Text(feedback()),
                MessageBlock::Text("Ahora los tests".to_string()),
            ],
        }),
        user(&format!("{}\n\nY la documentación", feedback())),
        Entry::Notice(Notice {
            level: NoticeLevel::Info,
            text: "Aviso".to_string(),
        }),
        // Not the block: an empty message and a hand-written tag stay.
        Entry::UserMessage(UserMessage { blocks: Vec::new() }),
        user(REVIEW_FEEDBACK_OPEN),
        answer(&format!(
            "Cité {REVIEW_FEEDBACK_OPEN}{REVIEW_FEEDBACK_CLOSE}"
        )),
    ]
}

fn clean_entries() -> Vec<String> {
    vec![
        "user: Arreglá el parser".to_string(),
        "agent: Listo.".to_string(),
        "user: Ahora los tests".to_string(),
        "user: Y la documentación".to_string(),
        "notice: Aviso".to_string(),
        "user: ".to_string(),
        format!("user: {REVIEW_FEEDBACK_OPEN}"),
        format!("agent: Cité {REVIEW_FEEDBACK_OPEN}{REVIEW_FEEDBACK_CLOSE}"),
    ]
}

#[gpui::test]
fn a3_importing_a_transcript_drops_the_block(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    with_window(&panel, &mut visual, |panel, _window, cx| {
        panel.import_transcript(
            TranscriptDump {
                version: TRANSCRIPT_VERSION,
                agent_id: Some("claude-acp".to_string()),
                session_id: None,
                entries: dirty_entries(),
            },
            cx,
        );
        assert_eq!(describe(panel.entries()), clean_entries());
        // What the next save writes is the clean transcript.
        assert_eq!(
            describe(&panel.export_transcript().entries),
            clean_entries()
        );
    });
}

#[gpui::test]
fn a3_loading_a_conversation_drops_the_block(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(conversation(dirty_entries()), window, cx);
        assert_eq!(describe(panel.entries()), clean_entries());
    });
}
