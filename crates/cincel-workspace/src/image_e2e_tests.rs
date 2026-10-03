//! The chat's images inside the workspace
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.3.2 to
//! §5.3.5, §5.4, E7-G): the capability of each connection, the notices, the
//! file dialog's answer, what reaches the agent (the fake agent's
//! `FAKE_PROMPT_LOG`), the `conversations/<id>/images/` folder, the viewer and
//! the drag from the tree.
//!
//! Every image is generated here with the `image` crate; nothing binary lives
//! in the repository. The fake agent is a real process, so the waits for it
//! are bounded polls (`CINCEL_PERF_BUDGET_FACTOR` widens the bound), never a
//! single `run_until_parked`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cincel_acp::{AgentEvent, acp::schema::v1::SessionId};
use cincel_chat::attachments::sha256_hex;
use cincel_chat::{
    AttachmentState, ChatEvent, ChatImageSource, ChatPanel, Conversation, Entry, ImageRef,
    MessageBlock, NoticeLevel, UserMessage,
};
use cincel_settings::Config;
use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use uuid::Uuid;

use crate::conversations::ConversationStore;
use crate::image_viewer::fitted_size;
use crate::project::ProjectOptions;
use crate::test_support::{FakeEnv, isolate_state};
use crate::toast::ToastKind;
use crate::workspace::{Workspace, WorkspaceEvent, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

// ------------------------------------------------------------------ helpers

/// How long a wait on a real thread or process may take.
fn budget() -> Duration {
    let factor = std::env::var("CINCEL_PERF_BUDGET_FACTOR")
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|factor| *factor >= 1.0)
        .unwrap_or(1.0);
    Duration::from_secs_f64(30.0 * factor)
}

/// Runs the scheduler until `done` holds, polling with a bound.
fn wait_until(
    cx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + budget();
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "se agotó la espera de: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A valid PNG of `width` × `height` whose pixels depend on `seed`.
fn png_bytes(width: u32, height: u32, seed: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([
            (x as u8).wrapping_mul(3).wrapping_add(seed),
            (y as u8).wrapping_mul(5),
            seed,
            255,
        ])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("el PNG se codifica");
    out.into_inner()
}

/// Everything one test needs: a workspace window over a project with one
/// saved fake Claude connection called "Fake" (not connected yet).
struct Rig {
    workspace: Entity<Workspace>,
    /// Keep alive: owns the profiles and the fake adapters.
    env: FakeEnv,
    id: Uuid,
    root: tempfile::TempDir,
    /// `FAKE_PROMPT_LOG`: one JSON line per prompt the fake agent received.
    log: PathBuf,
    _log_dir: tempfile::TempDir,
}

impl Rig {
    fn chat(&self, cx: &mut VisualTestContext) -> Entity<ChatPanel> {
        self.workspace
            .read_with(cx, |workspace, _| workspace.chat().clone())
    }

    fn agents(&self, cx: &mut VisualTestContext) -> Entity<crate::agents::Agents> {
        self.workspace
            .read_with(cx, |workspace, _| workspace.agents().clone())
    }

    fn toasts(&self, cx: &mut VisualTestContext) -> Entity<crate::toast::Toasts> {
        self.workspace
            .read_with(cx, |workspace, _| workspace.toasts().clone())
    }

    fn store(&self) -> ConversationStore {
        ConversationStore::for_project(self.root.path()).expect("hay directorio de estado")
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }
}

/// The project: `src/main.rs`, `notes.txt` and `logo.png`.
fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "no soy una imagen\n").unwrap();
    std::fs::write(dir.path().join("logo.png"), png_bytes(32, 32, 9)).unwrap();
    dir
}

/// A workspace window with `agent_env` set in the fake Claude agent's
/// environment (on top of `FAKE_PROMPT_LOG`).
fn rig<'a>(
    agent_env: &[(&str, &str)],
    cx: &'a mut TestAppContext,
) -> (Rig, &'a mut VisualTestContext) {
    init_test(cx);
    let log_dir = tempfile::tempdir().unwrap();
    let log = log_dir.path().join("prompts.jsonl");
    let log_text = log.to_string_lossy().to_string();
    let mut vars: Vec<(&str, &str)> = vec![("FAKE_PROMPT_LOG", log_text.as_str())];
    vars.extend_from_slice(agent_env);
    let env = FakeEnv::with_agent_env(&vars);
    let id = env.add_connection("claude-acp", "Fake");
    let root = sample_project();
    let options = WorkspaceOptions {
        project: Some(root.path().to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.simulate_resize(size(px(1100.), px(760.)));
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| {
        agents.set_connections(env.connections.clone(), cx)
    });
    cx.run_until_parked();
    (
        Rig {
            workspace,
            env,
            id,
            root,
            log,
            _log_dir: log_dir,
        },
        cx,
    )
}

/// Feeds the active connection's events to `Agents` (the tests run with the
/// background draining off) until `stop` matches.
fn drain_until(rig: &Rig, cx: &mut VisualTestContext, stop: impl Fn(&AgentEvent) -> bool) {
    let agents = rig.agents(cx);
    let events = agents
        .read_with(cx, |agents, _| agents.connection_events())
        .expect("hay una conexión en marcha");
    let deadline = Instant::now() + budget();
    loop {
        match events.try_recv() {
            Ok(event) => {
                let done = stop(&event);
                agents.update(cx, |agents, cx| agents.handle_agent_event(event, cx));
                cx.run_until_parked();
                if done {
                    return;
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "el agente falso no respondió a tiempo"
                );
                cx.run_until_parked();
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => panic!("el agente falso se cerró"),
        }
    }
}

/// Picks the "Fake" connection and waits for its session.
fn connect(rig: &Rig, cx: &mut VisualTestContext) -> SessionId {
    let chat = rig.chat(cx);
    chat.update(cx, |chat, cx| {
        chat.select_connection(&rig.id.to_string(), cx)
    });
    cx.run_until_parked();
    drain_until(rig, cx, |event| {
        matches!(event, AgentEvent::SessionCreated { .. })
    });
    chat.read_with(cx, |chat, _| {
        chat.session_id().cloned().expect("hay sesión")
    })
}

fn toast_texts(rig: &Rig, cx: &mut VisualTestContext) -> Vec<String> {
    let toasts = rig.toasts(cx);
    toasts.read_with(cx, |toasts, _| {
        toasts
            .items()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

/// Waits until every image of the draft is ready (and there are `count`).
fn wait_for_attachments(chat: &Entity<ChatPanel>, count: usize, cx: &mut VisualTestContext) {
    wait_until(cx, "la preparación de las imágenes", |cx| {
        chat.read_with(cx, |chat, _| {
            chat.attachments().len() == count
                && chat
                    .attachments()
                    .iter()
                    .all(|attachment| matches!(attachment.state, AttachmentState::Ready(_)))
        })
    });
}

/// Types `text` and sends it.
fn send(chat: &Entity<ChatPanel>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        chat.update(cx, |chat, cx| {
            chat.set_input_text(text, window, cx);
            chat.send(window, cx);
        });
    });
    cx.run_until_parked();
}

/// The lines of `FAKE_PROMPT_LOG`, parsed.
fn prompt_log(rig: &Rig) -> Vec<serde_json::Value> {
    std::fs::read_to_string(&rig.log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("una línea JSON del registro"))
        .collect()
}

/// Connects, attaches two PNGs by path through the dialog's answer, sends
/// them with a text and waits for the turn to end. Returns the conversation's
/// id and the two files' bytes, in the order they were attached.
fn send_two_images(rig: &Rig, cx: &mut VisualTestContext) -> (String, [Vec<u8>; 2]) {
    connect(rig, cx);
    let first = png_bytes(64, 48, 1);
    let second = png_bytes(40, 40, 2);
    let paths = [rig.path("primera.png"), rig.path("segunda.png")];
    std::fs::write(&paths[0], &first).unwrap();
    std::fs::write(&paths[1], &second).unwrap();

    let agents = rig.agents(cx);
    let chat = rig.chat(cx);
    agents.update(cx, |agents, cx| agents.attach_picked(paths.to_vec(), cx));
    wait_for_attachments(&chat, 2, cx);
    send(&chat, "mirá estas dos imágenes", cx);
    drain_until(rig, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    let id = agents
        .read_with(cx, |agents, _| agents.conversation_binding())
        .expect("hay conversación")
        .0;
    (id, [first, second])
}

// ------------------------------------------------------- capacity (§5.3.2)

/// `Connected` gives the chat the capability the agent announced, and a
/// connection that stops (or is not usable) takes it away again.
#[gpui::test]
fn the_capability_follows_the_active_connection(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let chat = rig.chat(cx);
    assert_eq!(chat.read_with(cx, |chat, _| chat.image_support()), None);

    connect(&rig, cx);
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.image_support()),
        Some(true),
        "el agente falso anuncia imágenes"
    );

    // Choosing a connection that cannot run (its session expired) stops the
    // process: no connection, no capability.
    let other = rig.env.add_connection("claude-acp", "Vencida");
    rig.env.expire(other);
    let agents = rig.agents(cx);
    agents.update(cx, |agents, cx| agents.refresh_connections(cx));
    chat.update(cx, |chat, cx| {
        chat.select_connection(&other.to_string(), cx)
    });
    cx.run_until_parked();
    assert!(!agents.read_with(cx, |agents, _| agents.is_agent_running()));
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.image_support()),
        None,
        "sin proceso no hay capacidad"
    );
}

/// An agent that does not announce images (`FAKE_NO_IMAGES`) leaves the chat
/// with `Some(false)`, and every way of attaching says so and attaches
/// nothing.
#[gpui::test]
fn a_connection_without_images_refuses_them_with_a_notice(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[("FAKE_NO_IMAGES", "1")], cx);
    connect(&rig, cx);
    let chat = rig.chat(cx);
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.image_support()),
        Some(false)
    );
    let dialogs_before = crate::agents::dialog_requests();

    // 1. The files the dialog (or a drop) returns.
    let agents = rig.agents(cx);
    agents.update(cx, |agents, cx| {
        agents.attach_picked(vec![rig.path("logo.png")], cx)
    });
    cx.run_until_parked();
    // 2. The paperclip itself: no dialog opens.
    chat.update(cx, |chat, cx| chat.request_pick_images(cx));
    cx.run_until_parked();
    // 3. The tree's "Mencionar en el chat" on an image.
    let files = rig
        .workspace
        .read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |_files, cx| {
        cx.emit(WorkspaceEvent::MentionFile {
            path: rig.path("logo.png"),
        });
    });
    cx.run_until_parked();

    let texts = toast_texts(&rig, cx);
    let refused = texts
        .iter()
        .filter(|text| text.as_str() == "«Fake» no acepta imágenes")
        .count();
    assert_eq!(refused, 3, "un aviso por cada vía: {texts:?}");
    assert!(chat.read_with(cx, |chat, _| chat.attachments().is_empty()));
    assert_eq!(
        crate::agents::dialog_requests(),
        dialogs_before,
        "con un agente sin imágenes el clip no abre el diálogo"
    );
}

/// Without an active connection the notice asks for one.
#[gpui::test]
fn without_a_connection_attaching_asks_to_connect(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let chat = rig.chat(cx);
    let agents = rig.agents(cx);
    agents.update(cx, |agents, cx| {
        agents.attach_picked(vec![rig.path("logo.png")], cx)
    });
    cx.run_until_parked();

    assert!(
        toast_texts(&rig, cx)
            .iter()
            .any(|text| text == "Conectá un agente para adjuntar imágenes"),
        "{:?}",
        toast_texts(&rig, cx)
    );
    assert!(chat.read_with(cx, |chat, _| chat.attachments().is_empty()));
}

/// The chat's `Notify` becomes a pop-up in the tone it asks for.
#[gpui::test]
fn the_chat_notices_become_toasts_in_their_tone(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let chat = rig.chat(cx);
    for (level, text) in [
        (NoticeLevel::Info, "uno"),
        (NoticeLevel::Warning, "dos"),
        (NoticeLevel::Error, "tres"),
    ] {
        chat.update(cx, |_, cx| {
            cx.emit(ChatEvent::Notify {
                level,
                text: text.to_string(),
            });
        });
    }
    cx.run_until_parked();
    let toasts = rig.toasts(cx);
    let seen: Vec<(String, ToastKind)> = toasts.read_with(cx, |toasts, _| {
        toasts
            .items()
            .iter()
            .map(|toast| (toast.message.to_string(), toast.kind))
            .collect()
    });
    assert_eq!(
        seen,
        vec![
            ("uno".to_string(), ToastKind::Info),
            ("dos".to_string(), ToastKind::Warning),
            ("tres".to_string(), ToastKind::Error),
        ]
    );
}

// ----------------------------------------------- the paperclip's dialog

/// The paperclip opens the dialog once the connection takes images, and what
/// the dialog returns is attached by content (a `.txt` is refused).
#[gpui::test]
fn the_paperclip_opens_the_dialog_and_its_files_are_attached(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    connect(&rig, cx);
    let chat = rig.chat(cx);
    let agents = rig.agents(cx);
    let before = crate::agents::dialog_requests();

    chat.update(cx, |chat, cx| chat.request_pick_images(cx));
    cx.run_until_parked();
    assert_eq!(crate::agents::dialog_requests(), before + 1);

    agents.update(cx, |agents, cx| {
        agents.attach_picked(vec![rig.path("logo.png"), rig.path("notes.txt")], cx)
    });
    wait_until(cx, "el aviso del .txt", |cx| {
        let texts = toast_texts(&rig, cx);
        texts
            .iter()
            .any(|text| text == "«notes.txt» no es una imagen PNG, JPEG, GIF o WebP")
    });
    wait_for_attachments(&chat, 1, cx);
}

// ----------------------------------------------------- sending (§5.4)

/// The agent receives the text and then one `image` block per image, in
/// order, with the right MIME type, the original bytes (they fit, so they
/// are not re-encoded) and no `uri`.
#[gpui::test]
fn the_agent_receives_the_text_then_the_images(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let (_, [first, second]) = send_two_images(&rig, cx);

    let log = prompt_log(&rig);
    assert_eq!(log.len(), 1, "un solo prompt: {log:?}");
    let blocks = log[0]["prompt"].as_array().expect("el arreglo prompt");
    let kinds: Vec<&str> = blocks
        .iter()
        .map(|block| block["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["text", "image", "image"], "{blocks:?}");
    assert_eq!(blocks[0]["text"], "mirá estas dos imágenes");
    for (block, bytes) in blocks[1..].iter().zip([&first, &second]) {
        assert_eq!(block["mimeType"], "image/png");
        assert_eq!(
            block["sha256"],
            sha256_hex(bytes),
            "los datos son los del archivo"
        );
        assert_eq!(block["bytes"], bytes.len());
        assert!(block["uri"].is_null(), "sin uri: {block:?}");
        assert_eq!(block["decodeError"], false);
    }
}

// ------------------------------------------------------ storage (§5.3.4)

/// After sending, the images are in `conversations/<id>/images/<sha256>.png`
/// and the JSON references them without carrying their bytes.
#[gpui::test]
fn sent_images_are_stored_with_the_conversation(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let (id, [first, second]) = send_two_images(&rig, cx);
    let store = rig.store();

    let images = store.image_dir(&id);
    assert_eq!(images, store.dir().join(&id).join("images"));
    for bytes in [&first, &second] {
        let stored = images.join(format!("{}.png", sha256_hex(bytes)));
        assert_eq!(
            std::fs::read(&stored).unwrap_or_default(),
            *bytes,
            "falta {}",
            stored.display()
        );
    }
    let leftovers: Vec<_> = std::fs::read_dir(&images)
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(leftovers.is_empty(), "quedó un temporal: {leftovers:?}");

    let json = std::fs::read_to_string(store.dir().join(format!("{id}.json"))).unwrap();
    assert!(
        json.contains(&sha256_hex(&first)),
        "el JSON referencia el archivo"
    );
    assert!(!json.contains("iVBOR"), "el JSON no lleva base64");
    assert!(json.len() < 16 * 1024, "el JSON es chico: {} B", json.len());
}

/// Closing the conversation and opening it again from the history shows the
/// same images, read from their folder; a click on one opens the viewer on
/// the stored file.
#[gpui::test]
fn a_reopened_conversation_shows_its_images_from_disk(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let (id, _) = send_two_images(&rig, cx);
    let store = rig.store();
    let chat = rig.chat(cx);

    // Another conversation takes the screen, then the old one comes back.
    cx.update(|window, cx| chat.update(cx, |chat, cx| chat.new_session(window, cx)));
    cx.run_until_parked();
    assert!(
        chat.read_with(cx, |chat, _| chat.entries().is_empty()),
        "la conversación nueva empieza vacía"
    );
    chat.update(cx, |chat, cx| chat.open_conversation(&id, cx));
    cx.run_until_parked();

    let (entry, blocks) = chat.read_with(cx, |chat, _| {
        let dump = chat.export_transcript();
        dump.entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| match entry {
                Entry::UserMessage(message) => {
                    let images: Vec<(usize, ImageRef)> = message
                        .blocks
                        .iter()
                        .enumerate()
                        .filter_map(|(block, item)| match item {
                            MessageBlock::Image(image) => Some((block, image.clone())),
                            _ => None,
                        })
                        .collect();
                    (!images.is_empty()).then_some((index, images))
                }
                _ => None,
            })
            .expect("el mensaje con imágenes volvió")
    });
    assert_eq!(blocks.len(), 2);
    for (_, image) in &blocks {
        assert!(image.data.is_none(), "reabierta: sin bytes en memoria");
        assert!(
            store.image_dir(&id).join(&image.file).is_file(),
            "{}",
            image.file
        );
    }
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.conversation_dir().map(Path::to_path_buf)),
        Some(store.conversation_dir(&id)),
        "el chat lee las imágenes de la carpeta de la conversación"
    );
    // Painted: both thumbnails are on screen.
    for ordinal in 0..2 {
        // `debug_bounds` wants a `'static` name; a test may leak one.
        let selector: &'static str =
            Box::leak(format!("sent-image-{entry}-{ordinal}").into_boxed_str());
        assert!(
            cx.debug_bounds(selector).is_some(),
            "no se pintó {selector}"
        );
    }

    // A click opens the viewer on the stored file.
    chat.update(cx, |chat, cx| chat.open_image(entry, blocks[0].0, cx));
    cx.run_until_parked();
    let viewer = rig
        .workspace
        .read_with(cx, |workspace, _| workspace.image_viewer().clone());
    assert!(viewer.read_with(cx, |viewer, _| viewer.is_open() && viewer.is_showing_file()));
}

/// Deleting the conversation deletes its folder; the undo of the toast puts
/// the images back.
#[gpui::test]
fn deleting_a_conversation_deletes_its_images_and_the_undo_restores_them(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let (id, [first, _]) = send_two_images(&rig, cx);
    let store = rig.store();
    let stored = store
        .image_dir(&id)
        .join(format!("{}.png", sha256_hex(&first)));
    assert!(stored.is_file());

    let chat = rig.chat(cx);
    chat.update(cx, |chat, cx| chat.delete_conversation(&id, cx));
    cx.run_until_parked();
    assert!(
        !store.conversation_dir(&id).exists(),
        "la carpeta de la conversación se borró"
    );
    assert!(!store.dir().join(format!("{id}.json")).exists());

    // The 5 s undo: the toast's "Deshacer" saves the conversation again,
    // which writes its images back.
    let toasts = rig.toasts(cx);
    let undo = toasts.read_with(cx, |toasts, _| {
        toasts
            .items()
            .iter()
            .find(|toast| toast.actions.iter().any(|(label, _)| label == "Deshacer"))
            .map(|toast| toast.id)
            .expect("hay una oferta de deshacer")
    });
    toasts.update(cx, |toasts, cx| toasts.run_action(undo, 0, cx));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(&stored).unwrap_or_default(),
        first,
        "las imágenes volvieron con la conversación"
    );
}

/// Deleting a connection's conversations deletes their folders too.
#[test]
fn deleting_the_conversations_of_a_connection_deletes_their_folders() {
    isolate_state();
    let root = tempfile::tempdir().unwrap();
    let store = ConversationStore::for_project(root.path()).unwrap();
    let bytes = png_bytes(8, 8, 3);
    let mut conversation = conversation_with_image("conv-con-imagen", &bytes);
    conversation.connection_id = Some("conexion-1".to_string());
    store.save(&conversation).unwrap();
    assert!(store.image_dir("conv-con-imagen").is_dir());

    assert_eq!(store.delete_for_connection("conexion-1"), 1);
    assert!(!store.conversation_dir("conv-con-imagen").exists());
    assert!(store.index().is_empty());
}

/// A conversation with one user message made of one stored image.
fn conversation_with_image(id: &str, bytes: &[u8]) -> Conversation {
    let mut conversation = Conversation::new(id, "claude-acp", "Fake", PathBuf::from("/tmp"), 1);
    conversation.entries.push(Entry::UserMessage(UserMessage {
        blocks: vec![MessageBlock::Image(ImageRef {
            file: format!("{}.png", sha256_hex(bytes)),
            name: "captura.png".to_string(),
            mime_type: "image/png".to_string(),
            width: 8,
            height: 8,
            bytes_len: bytes.len() as u64,
            data: Some(std::sync::Arc::from(bytes.to_vec())),
        })],
    }));
    conversation
}

/// Saving writes each image once, with no leftovers, never outside its
/// folder, and saving again changes nothing.
#[test]
fn saving_writes_each_image_once_and_only_inside_its_folder() {
    isolate_state();
    let root = tempfile::tempdir().unwrap();
    let store = ConversationStore::for_project(root.path()).unwrap();
    let bytes = png_bytes(8, 8, 4);
    let mut conversation = conversation_with_image("conv-uno", &bytes);
    // A hostile name never leaves the folder.
    if let Entry::UserMessage(message) = &mut conversation.entries[0] {
        message.blocks.push(MessageBlock::Image(ImageRef {
            file: "../escapa.png".to_string(),
            name: "x".to_string(),
            mime_type: "image/png".to_string(),
            width: 1,
            height: 1,
            bytes_len: 1,
            data: Some(std::sync::Arc::from(vec![1u8])),
        }));
    }
    store.save(&conversation).unwrap();
    store.save(&conversation).unwrap();

    let names: Vec<String> = std::fs::read_dir(store.image_dir("conv-uno"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec![format!("{}.png", sha256_hex(&bytes))]);
    assert!(
        !store
            .conversation_dir("conv-uno")
            .join("escapa.png")
            .exists()
    );
    assert!(!store.conversation_dir("escapa.png").exists());
}

// ------------------------------------------------- version 1 (§5.3.4)

/// A conversation saved as version 1 loads as it is (no images, no comment
/// cards), reads as version 2 and opens in the chat.
#[gpui::test]
fn a_version_1_conversation_loads_and_opens(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    let store = rig.store();
    let mut conversation = Conversation::new(
        "conv-v1",
        "claude-acp",
        "Fake",
        rig.root.path().to_path_buf(),
        1_000,
    );
    conversation.title = "Charla vieja".to_string();
    conversation.entries.push(Entry::UserMessage(UserMessage {
        blocks: vec![MessageBlock::Text(
            "hola, esto es de la versión 1".to_string(),
        )],
    }));
    store.save(&conversation).unwrap();
    // The file as the 0.1.0 wrote it: `"version": 1`.
    let path = store.dir().join("conv-v1.json");
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["version"] = serde_json::json!(1);
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"version\": 1")
    );

    let loaded = store.load("conv-v1").expect("la versión 1 se lee");
    assert_eq!(loaded.version, cincel_chat::CONVERSATION_VERSION);
    assert!(loaded.images_to_store().is_empty());
    assert_eq!(loaded.entries.len(), 1);

    // And it opens from the history like any other.
    let chat = rig.chat(cx);
    chat.update(cx, |chat, cx| chat.open_conversation("conv-v1", cx));
    cx.run_until_parked();
    let shown = chat.read_with(cx, |chat, _| chat.entries().len());
    assert_eq!(shown, 1, "la conversación v1 se ve en el chat");
}

// ------------------------------------------------------------ the viewer

/// A workspace window over a project, with nothing connected.
fn plain_window(cx: &mut TestAppContext) -> (Rig, &mut VisualTestContext) {
    rig(&[], cx)
}

/// Opens a stored-size PNG through the chat's own event, the way a click on a
/// thumbnail does.
fn open_from_chat(rig: &Rig, source: ChatImageSource, cx: &mut VisualTestContext) {
    let chat = rig.chat(cx);
    chat.update(cx, |_, cx| {
        cx.emit(ChatEvent::OpenImage {
            source,
            name: "captura.png".to_string(),
            width: 64,
            height: 48,
            bytes_len: 2048,
        });
    });
    cx.run_until_parked();
}

fn bytes_source(seed: u8) -> ChatImageSource {
    ChatImageSource::Bytes {
        mime_type: "image/png".to_string(),
        data: std::sync::Arc::from(png_bytes(64, 48, seed)),
    }
}

fn viewer_open(rig: &Rig, cx: &mut VisualTestContext) -> bool {
    rig.workspace
        .read_with(cx, |workspace, cx| workspace.is_image_viewer_open(cx))
}

fn composer_focused(rig: &Rig, cx: &mut VisualTestContext) -> bool {
    let chat = rig.chat(cx);
    cx.update(|window, cx| {
        chat.read(cx)
            .composer()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    })
}

fn focus_composer(rig: &Rig, cx: &mut VisualTestContext) {
    let chat = rig.chat(cx);
    cx.update(|window, cx| chat.update(cx, |chat, cx| chat.focus_input(window, cx)));
    cx.run_until_parked();
}

/// `OpenImage` shows the image over the window with its caption, takes the
/// keyboard, and `Esc` closes it and gives the keyboard back to the box.
#[gpui::test]
fn the_viewer_opens_over_the_window_and_escape_closes_it(cx: &mut TestAppContext) {
    let (rig, cx) = plain_window(cx);
    focus_composer(&rig, cx);
    assert!(composer_focused(&rig, cx));

    open_from_chat(&rig, bytes_source(5), cx);
    assert!(viewer_open(&rig, cx));
    let viewer = rig
        .workspace
        .read_with(cx, |workspace, _| workspace.image_viewer().clone());
    assert_eq!(
        viewer.read_with(cx, |viewer, _| viewer.caption()),
        Some("captura.png · 64 × 48 · 2,0 KB".to_string())
    );
    let window = cx.debug_bounds("image-viewer").expect("la capa se pinta");
    let picture = cx
        .debug_bounds("image-viewer-image")
        .expect("la imagen se pinta");
    assert!(window.contains(&picture.origin), "la imagen cae dentro");
    // 64 × 48 fits: painted at its real size.
    assert_eq!(picture.size.width, px(64.));
    assert_eq!(picture.size.height, px(48.));
    let handle = viewer.read_with(cx, |viewer, cx| viewer.focus_handle(cx));
    assert!(cx.update(|window, _| handle.is_focused(window)));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!viewer_open(&rig, cx), "Esc cierra el visor");
    assert!(
        composer_focused(&rig, cx),
        "el foco vuelve a la caja del chat"
    );
}

/// A click outside the image, and the `×`, close it; a click on the image
/// does not.
#[gpui::test]
fn a_click_outside_the_image_and_the_close_button_close_the_viewer(cx: &mut TestAppContext) {
    let (rig, cx) = plain_window(cx);

    open_from_chat(&rig, bytes_source(6), cx);
    let picture = cx.debug_bounds("image-viewer-image").unwrap();
    cx.simulate_click(picture.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        viewer_open(&rig, cx),
        "un clic sobre la imagen no la cierra"
    );

    cx.simulate_click(point(px(6.), px(40.)), Modifiers::none());
    cx.run_until_parked();
    assert!(
        !viewer_open(&rig, cx),
        "un clic fuera de la imagen la cierra"
    );

    open_from_chat(&rig, bytes_source(7), cx);
    let close = cx
        .debug_bounds("image-viewer-close")
        .expect("el botón × se pinta");
    cx.simulate_click(close.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(!viewer_open(&rig, cx), "la × cierra el visor");
}

/// With the viewer up, `Ctrl+L` moves nothing (it counts as a modal), and
/// opening another image keeps the first one's way back.
#[gpui::test]
fn ctrl_l_does_nothing_while_the_viewer_is_open(cx: &mut TestAppContext) {
    let (rig, cx) = plain_window(cx);
    focus_composer(&rig, cx);
    open_from_chat(&rig, bytes_source(8), cx);
    assert!(
        rig.workspace
            .read_with(cx, |workspace, cx| workspace.is_modal_open(cx)),
        "el visor cuenta como modal"
    );
    let viewer = rig
        .workspace
        .read_with(cx, |workspace, _| workspace.image_viewer().clone());
    let handle = viewer.read_with(cx, |viewer, cx| viewer.focus_handle(cx));

    cx.simulate_keystrokes("ctrl-l");
    cx.run_until_parked();
    assert!(viewer_open(&rig, cx));
    assert!(
        cx.update(|window, _| handle.is_focused(window)),
        "Ctrl+L no movió el foco"
    );
    let zone = cx.update(|window, cx| rig.workspace.read(cx).focus_zone(window, cx));
    assert_eq!(
        zone, None,
        "el foco no está en ninguna zona del chat o del editor"
    );

    // A second image while the first is up: one Esc still ends on the box.
    open_from_chat(&rig, bytes_source(9), cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!viewer_open(&rig, cx));
    assert!(composer_focused(&rig, cx));
}

/// A viewer opened from a file on disk shows it (and says so).
#[gpui::test]
fn the_viewer_opens_a_stored_file(cx: &mut TestAppContext) {
    let (rig, cx) = plain_window(cx);
    open_from_chat(&rig, ChatImageSource::File(rig.path("logo.png")), cx);
    let viewer = rig
        .workspace
        .read_with(cx, |workspace, _| workspace.image_viewer().clone());
    assert!(viewer.read_with(cx, |viewer, _| viewer.is_open() && viewer.is_showing_file()));
    assert!(cx.debug_bounds("image-viewer-image").is_some());
}

/// The size an image is painted at: its own when it fits, scaled down on the
/// tighter axis (keeping the proportions) when it does not.
#[test]
fn the_viewer_fits_the_image_inside_the_window() {
    // Fits: unchanged.
    assert_eq!(fitted_size((900., 600.), (800, 600)), (800., 600.));
    // Wider than the window: scaled by the width.
    let (width, height) = fitted_size((900., 600.), (1800, 900));
    assert_eq!((width, height), (900., 450.));
    // Taller than the window: scaled by the height.
    let (width, height) = fitted_size((900., 600.), (600, 1200));
    assert_eq!((width, height), (300., 600.));
    // Never enlarged.
    assert_eq!(fitted_size((900., 600.), (10, 10)), (10., 10.));
    // Unknown size: the whole box.
    assert_eq!(fitted_size((900., 600.), (0, 0)), (900., 600.));
}

// ------------------------------------------------------ the tree's drag

/// "Mencionar en el chat" on an image attaches it; on a source file it is the
/// `@` mention it always was.
#[gpui::test]
fn dragging_from_the_tree_attaches_an_image_and_mentions_a_source_file(cx: &mut TestAppContext) {
    let (rig, cx) = rig(&[], cx);
    connect(&rig, cx);
    let chat = rig.chat(cx);
    let files = rig
        .workspace
        .read_with(cx, |workspace, _| workspace.files().clone());

    files.update(cx, |_files, cx| {
        cx.emit(WorkspaceEvent::MentionFile {
            path: rig.path("logo.png"),
        });
    });
    wait_for_attachments(&chat, 1, cx);
    assert!(
        chat.read_with(cx, |chat, _| chat.mentions().is_empty()),
        "una imagen no deja una mención"
    );
    assert_eq!(chat.read_with(cx, |chat, cx| chat.input_text(cx)), "");

    files.update(cx, |_files, cx| {
        cx.emit(WorkspaceEvent::MentionFile {
            path: rig.path("src/main.rs"),
        });
    });
    cx.run_until_parked();
    let mentions = chat.read_with(cx, |chat, _| {
        chat.mentions()
            .iter()
            .map(|mention| mention.path.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(mentions, vec![rig.path("src/main.rs")]);
    assert_eq!(
        chat.read_with(cx, |chat, cx| chat.input_text(cx)),
        "@main.rs "
    );
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.attachments().len()),
        1,
        "la imagen sigue adjunta"
    );
}
