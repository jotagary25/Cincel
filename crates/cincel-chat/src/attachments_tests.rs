//! Images in the chat over the GPUI test harness
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.4, §5.5):
//! the three ways in (`Ctrl+V`, a drop from the file manager, the paperclip),
//! the thumbnails and their `×`, the cap of ten, the reduction to 2 000 px,
//! the refusals, the capability notices, what the prompt carries — checked
//! against the real fake agent's `FAKE_PROMPT_LOG` — and the sent thumbnails.
//!
//! Every image is generated here with `image`; nothing binary is in the repo.
//! The fake-agent test needs `cargo build -p cincel-acp --bin
//! cincel-acp-fake-agent` first, like the workspace's end-to-end tests.

use std::cell::RefCell;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use cincel_acp::acp::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, ImageContent, PromptCapabilities, SessionId,
    SessionUpdate,
};
use cincel_acp::protocol::PromptBlock;
use cincel_acp::{AgentCommand, AgentConnection, AgentEvent, LaunchSpec};
use gpui::{
    AnyWindowHandle, ClipboardItem, Context, Entity, ExternalPaths, FileDropEvent, Modifiers,
    TestAppContext, VisualTestContext, Window, px, size,
};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

use crate::actions::bind_default_keys;
use crate::attachments::{
    MAX_IMAGES_PER_MESSAGE, NO_CONNECTION_FOR_IMAGES, PASTED_IMAGE_NAME, TOO_MANY_IMAGES,
    sha256_hex,
};
use crate::model::*;
use crate::panel::ChatPanel;
use crate::panel_media::{AttachmentSource, AttachmentState};
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

/// One `AgentCommand::Prompt` the panel emitted.
pub(crate) type SentPrompt = (SessionId, Vec<PromptBlock>);

/// What the panel emitted: every event as its `Debug` string, and the
/// prompts as they were.
#[derive(Clone, Default)]
pub(crate) struct Recorded {
    pub(crate) events: Rc<RefCell<Vec<String>>>,
    pub(crate) prompts: Rc<RefCell<Vec<SentPrompt>>>,
}

impl Recorded {
    pub(crate) fn log(&self) -> String {
        self.events.borrow().join("\n")
    }

    pub(crate) fn clear(&self) {
        self.events.borrow_mut().clear();
        self.prompts.borrow_mut().clear();
    }
}

/// The connection every test uses.
pub(crate) const LABEL: &str = "Claude · personal";

/// Opens a 400 × 720 window with a panel and records what it emits.
pub(crate) fn open(cx: &mut TestAppContext) -> (Entity<ChatPanel>, VisualTestContext, Recorded) {
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
    let recorded = Recorded::default();
    let sink = recorded.clone();
    cx.update(|cx| {
        cx.subscribe(&panel, move |_panel, event: &crate::ChatEvent, _cx| {
            sink.events.borrow_mut().push(format!("{event:?}"));
            if let crate::ChatEvent::Command(AgentCommand::Prompt {
                session_id, blocks, ..
            }) = event
            {
                sink.prompts
                    .borrow_mut()
                    .push((session_id.clone(), blocks.clone()));
            }
        })
        .detach();
    });
    visual.simulate_resize(size(px(400.), px(720.)));
    visual.run_until_parked();
    (panel, visual, recorded)
}

/// Runs `f` with both a `Window` and the panel's `Context`.
pub(crate) fn with_window<R>(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    f: impl FnOnce(&mut ChatPanel, &mut Window, &mut Context<ChatPanel>) -> R,
) -> R {
    visual.update(|window, cx| panel.update(cx, |panel, cx| f(panel, window, cx)))
}

/// `initialize`'s answer, with or without `promptCapabilities.image`.
pub(crate) fn connected(images: bool) -> AgentEvent {
    AgentEvent::Connected {
        agent_info: None,
        auth_methods: Vec::new(),
        capabilities: Box::new(
            AgentCapabilities::new().prompt_capabilities(PromptCapabilities::new().image(images)),
        ),
    }
}

/// Makes `LABEL` the active connection with a session; `images` is what its
/// agent announced.
pub(crate) fn connect(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, images: bool) {
    panel.update(visual, |panel, cx| {
        panel.set_connections(
            vec![ChatConnection {
                id: "c-claude".into(),
                agent_id: "claude-acp".into(),
                label: LABEL.into(),
                agent_name: "Claude".into(),
                identity: None,
                last_used: String::new(),
                badge: ConnectionBadge::Connected,
            }],
            cx,
        );
        panel.set_active_connection(Some("c-claude".into()), cx);
        panel.handle_event(connected(images), cx);
        panel.handle_event(
            AgentEvent::SessionCreated {
                session_id: SessionId::new("s1"),
                modes: None,
                config_options: Vec::new(),
                commands: Vec::new(),
            },
            cx,
        );
    });
    visual.run_until_parked();
}

pub(crate) fn gradient(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, ((x * 3 + y) % 256) as u8])
    }))
}

pub(crate) fn encode(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).expect("encode");
    out.into_inner()
}

/// A PNG of `width × height` written to `dir/name`.
pub(crate) fn png_file(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, encode(&gradient(width, height), ImageFormat::Png)).expect("write");
    path
}

fn ready_count(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> usize {
    panel.read_with(visual, |panel, _| {
        panel
            .attachments()
            .iter()
            .filter(|attachment| matches!(attachment.state, AttachmentState::Ready(_)))
            .count()
    })
}

fn focus_composer(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) {
    with_window(panel, visual, |panel, window, cx| {
        panel.focus_input(window, cx)
    });
    visual.run_until_parked();
}

fn send(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, text: &str) {
    with_window(panel, visual, |panel, window, cx| {
        panel.set_input_text(text, window, cx);
        panel.send(window, cx);
    });
    visual.run_until_parked();
}

/// The images of the one prompt sent so far: `(mime, bytes)`.
fn prompt_images(recorded: &Recorded) -> Vec<(String, Vec<u8>)> {
    let prompts = recorded.prompts.borrow();
    assert_eq!(prompts.len(), 1, "un solo prompt");
    prompts[0]
        .1
        .iter()
        .filter_map(|block| match block {
            PromptBlock::Image { mime_type, data } => Some((mime_type.clone(), data.to_vec())),
            _ => None,
        })
        .collect()
}

fn drop_files(visual: &mut VisualTestContext, paths: Vec<PathBuf>) {
    let target = visual
        .debug_bounds("chat-input-box")
        .expect("la caja se pintó")
        .center();
    // GPUI ignores a drop while the last input was the keyboard (a file drop
    // does not switch the input modality back to the mouse); a real drag
    // reaches the window after the pointer moved, which is what this mimics.
    visual.simulate_mouse_move(target, None, Modifiers::default());
    visual.simulate_event(FileDropEvent::Entered {
        position: target,
        paths: ExternalPaths(paths.into()),
    });
    visual.simulate_event(FileDropEvent::Pending { position: target });
    visual.simulate_event(FileDropEvent::Submit { position: target });
    visual.run_until_parked();
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let spot = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} no se pintó"))
        .center();
    visual.simulate_click(spot, Modifiers::default());
    visual.run_until_parked();
}

// ------------------------------------------------------------------ tests

/// §5.4: `Ctrl+V` with a PNG capture adds a thumbnail and pastes no text;
/// with text it pastes the text as always.
#[gpui::test]
fn ctrl_v_with_an_image_attaches_it_and_with_text_pastes_text(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    focus_composer(&panel, &mut visual);

    let bytes = encode(&gradient(64, 40), ImageFormat::Png);
    visual.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        bytes.clone(),
    )));
    visual.simulate_keystrokes("ctrl-v");
    visual.run_until_parked();
    panel.read_with(&visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "", "no se pega texto");
        let [attachment] = panel.attachments() else {
            panic!("una miniatura: {:?}", panel.attachments());
        };
        assert_eq!(attachment.name, PASTED_IMAGE_NAME);
        let AttachmentState::Ready(image) = &attachment.state else {
            panic!("lista");
        };
        assert_eq!(&*image.bytes, bytes.as_slice());
    });
    let thumb = visual
        .debug_bounds("attachment-0")
        .expect("la miniatura se pintó");
    assert_eq!(thumb.size.width, px(56.));
    assert_eq!(thumb.size.height, px(56.));
    let row = visual
        .debug_bounds("chat-attachments")
        .expect("fila propia");
    let input = visual.debug_bounds("chat-input-box").expect("caja");
    assert!(
        input.contains(&row.origin) && row.bottom() <= input.bottom(),
        "la fila de miniaturas está dentro de la caja: {row:?} en {input:?}"
    );

    visual.write_to_clipboard(ClipboardItem::new_string("hola".to_string()));
    visual.simulate_keystrokes("ctrl-v");
    visual.run_until_parked();
    panel.read_with(&visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "hola");
        assert_eq!(panel.attachments().len(), 1);
    });
    assert!(!recorded.log().contains("Notify"), "{}", recorded.log());
}

/// Wayland hands files copied in the file manager over as `file://` text:
/// those that are images are attached; any other text is pasted.
#[gpui::test]
fn ctrl_v_with_copied_image_files_attaches_them(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    connect(&panel, &mut visual, true);
    focus_composer(&panel, &mut visual);
    let dir = tempfile::tempdir().expect("tempdir");
    let a = png_file(dir.path(), "captura uno.png", 30, 30);
    let uri = format!("file://{}", a.display()).replace(' ', "%20");
    visual.write_to_clipboard(ClipboardItem::new_string(format!("{uri}\n")));
    visual.simulate_keystrokes("ctrl-v");
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), 1);
    panel.read_with(&visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "");
        assert_eq!(panel.attachments()[0].name, "captura uno.png");
    });

    // Files offered as `ExternalPaths` (X11, other platforms): the same.
    let b = png_file(dir.path(), "b.png", 20, 20);
    visual.write_to_clipboard(ClipboardItem {
        entries: vec![gpui::ClipboardEntry::ExternalPaths(ExternalPaths(
            vec![b, dir.path().join("notas.txt")].into(),
        ))],
    });
    visual.simulate_keystrokes("ctrl-v");
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), 2, "solo las imágenes");
}

/// §5.4: two PNGs and a `.txt` dropped on the box: two thumbnails and the
/// notice about the `.txt` (the format is read from the content).
#[gpui::test]
fn dropping_two_pngs_and_a_txt_gives_two_thumbnails_and_a_notice(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let a = png_file(dir.path(), "a.png", 40, 30);
    let b = png_file(dir.path(), "b.png", 30, 40);
    let txt = dir.path().join("notas.txt");
    std::fs::write(&txt, "no soy una imagen").expect("write");

    drop_files(&mut visual, vec![a, txt, b]);
    assert_eq!(ready_count(&panel, &mut visual), 2);
    panel.read_with(&visual, |panel, _| {
        let names: Vec<&str> = panel
            .attachments()
            .iter()
            .map(|attachment| attachment.name.as_str())
            .collect();
        assert_eq!(names, ["a.png", "b.png"]);
    });
    assert!(
        recorded
            .log()
            .contains("«notas.txt» no es una imagen PNG, JPEG, GIF o WebP"),
        "{}",
        recorded.log()
    );
    assert!(visual.debug_bounds("attachment-1").is_some());
}

/// §5.4: the `×` of a thumbnail (a real click) removes it and it does not
/// travel; the rest goes after the text, in order (D4).
#[gpui::test]
fn the_x_of_a_thumbnail_removes_it_and_the_rest_travels_after_the_text(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let first = png_file(dir.path(), "uno.png", 10, 10);
    let second = png_file(dir.path(), "dos.png", 12, 12);
    let third = png_file(dir.path(), "tres.png", 14, 14);
    panel.update(&mut visual, |panel, cx| {
        panel.attach_paths(&[first, second.clone(), third.clone()], cx);
    });
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), 3);

    click(&mut visual, "attachment-remove-0");
    panel.read_with(&visual, |panel, _| {
        let names: Vec<&str> = panel
            .attachments()
            .iter()
            .map(|attachment| attachment.name.as_str())
            .collect();
        assert_eq!(names, ["dos.png", "tres.png"]);
    });

    recorded.clear();
    send(&panel, &mut visual, "mirá estas");
    let prompts = recorded.prompts.borrow();
    let blocks = &prompts[0].1;
    assert_eq!(blocks.len(), 3, "{blocks:?}");
    assert_eq!(blocks[0], PromptBlock::Text("mirá estas".into()));
    for (block, path) in blocks[1..].iter().zip([&second, &third]) {
        let PromptBlock::Image { mime_type, data } = block else {
            panic!("imagen: {block:?}");
        };
        assert_eq!(mime_type, "image/png");
        assert_eq!(&**data, std::fs::read(path).unwrap().as_slice());
    }
    drop(prompts);
    panel.read_with(&visual, |panel, _| {
        assert!(panel.attachments().is_empty(), "la caja queda vacía");
    });
}

/// §5.4: the eleventh image is refused with its notice; the ten that fit
/// are attached.
#[gpui::test]
fn the_eleventh_image_is_refused(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let bytes = encode(&gradient(8, 8), ImageFormat::Png);
    let sources: Vec<AttachmentSource> = (0..11)
        .map(|n| AttachmentSource::Bytes {
            name: format!("img{n}.png"),
            bytes: bytes.clone(),
        })
        .collect();
    panel.update(&mut visual, |panel, cx| panel.attach_images(sources, cx));
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), MAX_IMAGES_PER_MESSAGE);
    assert!(
        recorded.log().contains(TOO_MANY_IMAGES),
        "{}",
        recorded.log()
    );

    recorded.clear();
    let one_more = vec![AttachmentSource::Bytes {
        name: "otra.png".into(),
        bytes,
    }];
    panel.update(&mut visual, |panel, cx| panel.attach_images(one_more, cx));
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), MAX_IMAGES_PER_MESSAGE);
    assert!(recorded.log().contains(TOO_MANY_IMAGES));
}

/// §5.4: a 4 000 × 3 000 image reaches the agent at 2 000 × 1 500; an
/// 800 × 600 one with the very bytes of its file.
#[gpui::test]
fn a_4000_px_image_reaches_the_agent_at_2000_and_a_small_one_unchanged(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let big = png_file(dir.path(), "grande.png", 4_000, 3_000);
    let small = dir.path().join("chica.jpg");
    let small_bytes = encode(&gradient(800, 600), ImageFormat::Jpeg);
    std::fs::write(&small, &small_bytes).expect("write");
    panel.update(&mut visual, |panel, cx| {
        panel.attach_paths(&[big, small], cx)
    });
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), 2);

    send(&panel, &mut visual, "");
    let images = prompt_images(&recorded);
    assert_eq!(images.len(), 2, "un mensaje puede ser solo imágenes");
    let reduced = image::load_from_memory(&images[0].1).expect("decodifica");
    assert_eq!((reduced.width(), reduced.height()), (2_000, 1_500));
    assert_eq!(images[0].0, "image/png");
    assert_eq!(images[1], ("image/jpeg".to_string(), small_bytes));
}

/// §5.4: what still weighs more than 10 MB after reducing, and what is not an
/// image, are refused with their notices and not attached.
#[gpui::test]
fn too_heavy_and_not_an_image_are_refused(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let mut state = 7u32;
    let noise = DynamicImage::ImageRgb8(RgbImage::from_fn(2_400, 2_400, |_, _| {
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        };
        Rgb([next(), next(), next()])
    }));
    let sources = vec![
        AttachmentSource::Bytes {
            name: "ruido.png".into(),
            bytes: encode(&noise, ImageFormat::Png),
        },
        AttachmentSource::Bytes {
            name: "notas.png".into(),
            bytes: b"texto con nombre de imagen".to_vec(),
        },
    ];
    panel.update(&mut visual, |panel, cx| panel.attach_images(sources, cx));
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| assert!(panel.attachments().is_empty()));
    let log = recorded.log();
    assert!(log.contains("«ruido.png» pesa "), "{log}");
    assert!(
        log.contains(" MB aun reducida; el máximo es 10 MB"),
        "{log}"
    );
    assert!(
        log.contains("«notas.png» no es una imagen PNG, JPEG, GIF o WebP"),
        "{log}"
    );
}

/// §5.4: without a connection the three ways say "Conectá un agente para
/// adjuntar imágenes"; with one whose agent does not take images (the fake
/// agent's `FAKE_NO_IMAGES`), "«Nombre» no acepta imágenes". Nothing is
/// attached and the button opens no dialog.
#[gpui::test]
fn the_three_ways_notify_when_images_cannot_be_attached(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    let dir = tempfile::tempdir().expect("tempdir");
    let png = png_file(dir.path(), "a.png", 10, 10);
    let bytes = std::fs::read(&png).unwrap();

    for (images, expected) in [
        (None, NO_CONNECTION_FOR_IMAGES.to_string()),
        (Some(false), format!("«{LABEL}» no acepta imágenes")),
    ] {
        if let Some(images) = images {
            connect(&panel, &mut visual, images);
        }
        focus_composer(&panel, &mut visual);
        recorded.clear();
        // 1. Ctrl+V.
        visual.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            bytes.clone(),
        )));
        visual.simulate_keystrokes("ctrl-v");
        visual.run_until_parked();
        // 2. A drop.
        drop_files(&mut visual, vec![png.clone()]);
        // 3. The paperclip (a real click) and `chat::attach_image`.
        click(&mut visual, "chat-attach");
        // `chat::attach_image` from a keymap, with the focus in the chat.
        focus_composer(&panel, &mut visual);
        visual.dispatch_action(crate::actions::AttachImage);
        visual.run_until_parked();

        let notices = recorded
            .events
            .borrow()
            .iter()
            .filter(|event| event.contains(&format!("{expected:?}")))
            .count();
        assert_eq!(notices, 4, "{}", recorded.log());
        assert!(!recorded.log().contains("PickImages"), "{}", recorded.log());
        panel.read_with(&visual, |panel, cx| {
            assert!(panel.attachments().is_empty());
            assert_eq!(panel.input_text(cx), "", "tampoco pega texto");
        });
    }

    // With images the button asks the workspace for the dialog, and what it
    // picks comes back through `attach_paths`.
    connect(&panel, &mut visual, true);
    recorded.clear();
    click(&mut visual, "chat-attach");
    assert!(recorded.log().contains("PickImages"), "{}", recorded.log());
    panel.update(&mut visual, |panel, cx| panel.attach_paths(&[png], cx));
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), 1);
}

/// §5.1: `Enter` does not send while an image is being prepared, and the
/// hint says "Preparando 1 imagen…".
#[gpui::test]
fn enter_waits_while_an_image_is_being_prepared(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    recorded.clear();
    let bytes = encode(&gradient(16, 16), ImageFormat::Png);
    let sent = with_window(&panel, &mut visual, |panel, window, cx| {
        panel.attach_images(
            vec![AttachmentSource::Bytes {
                name: "a.png".into(),
                bytes,
            }],
            cx,
        );
        assert!(panel.is_preparing_images());
        assert_eq!(panel.composer_hint().as_ref(), "Preparando 1 imagen…");
        panel.set_input_text("hola", window, cx);
        panel.send(window, cx);
        panel.entries().len()
    });
    assert_eq!(sent, 0, "no se envió");
    assert!(recorded.prompts.borrow().is_empty());
    visual.run_until_parked();
    send(&panel, &mut visual, "hola");
    assert_eq!(prompt_images(&recorded).len(), 1);
}

/// §5.1, §5.4: switching to a connection that does not take images with
/// images in the draft keeps the thumbnails, shows the line above the box and
/// holds `Enter` until they are removed.
#[gpui::test]
fn a_connection_without_images_holds_back_a_draft_with_images(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let png = png_file(dir.path(), "a.png", 10, 10);
    panel.update(&mut visual, |panel, cx| panel.attach_paths(&[png], cx));
    visual.run_until_parked();
    panel.update(&mut visual, |panel, cx| {
        panel.set_image_support(Some(false), cx)
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("chat-images-blocked").is_some());
    assert!(visual.debug_bounds("attachment-0").is_some(), "se quedan");
    recorded.clear();
    send(&panel, &mut visual, "hola");
    assert!(recorded.prompts.borrow().is_empty(), "Enter no envía");
    panel.read_with(&visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "hola", "el borrador queda");
    });

    click(&mut visual, "attachment-remove-0");
    assert!(visual.debug_bounds("chat-images-blocked").is_none());
    send(&panel, &mut visual, "hola");
    assert_eq!(recorded.prompts.borrow().len(), 1);
}

/// §5.4: the sent message shows the thumbnails (≤ 96 px high, ≤ 160 px wide,
/// proportional) and a click emits "abrir visor" with the bytes.
#[gpui::test]
fn sent_thumbnails_are_painted_and_a_click_opens_the_viewer(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let wide = png_file(dir.path(), "ancha.png", 400, 100);
    panel.update(&mut visual, |panel, cx| panel.attach_paths(&[wide], cx));
    visual.run_until_parked();
    send(&panel, &mut visual, "mirá");

    let thumb = visual
        .debug_bounds("sent-image-0-0")
        .expect("la miniatura del mensaje se pintó");
    assert_eq!(thumb.size.width, px(160.));
    assert_eq!(thumb.size.height, px(40.));
    let bubble = visual.debug_bounds("user-bubble-0").expect("burbuja");
    let text = visual.debug_bounds("user-text-0").expect("texto");
    // The images go above the text (`docs/specs/10-etapa7-ronda2.md` §5).
    assert!(bubble.contains(&thumb.origin) && thumb.bottom() <= text.top());

    recorded.clear();
    click(&mut visual, "sent-image-0-0");
    let log = recorded.log();
    assert!(log.contains("OpenImage"), "{log}");
    assert!(log.contains("Bytes(image/png"), "{log}");
    assert!(log.contains("name: \"ancha.png\""), "{log}");
    assert!(
        log.contains("width: 400") && log.contains("height: 100"),
        "{log}"
    );

    // The message is in the conversation as a reference to its file.
    panel.read_with(&visual, |panel, _| {
        let Entry::UserMessage(message) = &panel.entries()[0] else {
            panic!("mensaje");
        };
        let MessageBlock::Image(image) = &message.blocks[1] else {
            panic!("imagen: {:?}", message.blocks);
        };
        let data = image.data.clone().expect("en memoria");
        assert_eq!(image.file, format!("{}.png", sha256_hex(&data)));
    });
}

/// §5.1: reopening a conversation paints its images from
/// `<conversation dir>/images/`, and "Imagen no disponible" for a missing
/// file; the click on a stored one opens the viewer on the file.
#[gpui::test]
fn a_reopened_conversation_paints_its_images_from_the_folder(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    let dir = tempfile::tempdir().expect("tempdir");
    let images = dir.path().join("images");
    std::fs::create_dir_all(&images).unwrap();
    let bytes = encode(&gradient(50, 50), ImageFormat::Png);
    let sha = sha256_hex(&bytes);
    std::fs::write(images.join(format!("{sha}.png")), &bytes).unwrap();
    let stored = |file: String| ImageRef {
        file,
        name: "captura.png".into(),
        mime_type: "image/png".into(),
        width: 50,
        height: 50,
        bytes_len: bytes.len() as u64,
        data: None,
    };
    let mut conversation = Conversation::new("c1", "claude-acp", LABEL, dir.path().into(), 0);
    conversation.entries.push(Entry::UserMessage(UserMessage {
        blocks: vec![
            MessageBlock::Text("dos capturas".into()),
            MessageBlock::Image(stored(format!("{sha}.png"))),
            MessageBlock::Image(stored("falta.png".into())),
        ],
    }));
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(conversation, window, cx);
        panel.set_conversation_dir(Some(dir.path().to_path_buf()), cx);
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sent-image-0-0").is_some());
    assert!(visual.debug_bounds("sent-image-missing-0-1").is_some());
    assert!(visual.debug_bounds("sent-image-0-1").is_none());

    click(&mut visual, "sent-image-0-0");
    let expected = images.join(format!("{sha}.png"));
    assert!(
        recorded
            .log()
            .contains(&format!("File({})", expected.display())),
        "{}",
        recorded.log()
    );
}

/// §5.1: images a `session/load` replays are shown from their data, and a
/// replay of one already on screen is dropped.
#[gpui::test]
fn replayed_images_are_shown_from_their_data(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    let bytes = encode(&gradient(24, 12), ImageFormat::Png);
    let chunk = || AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Image(ImageContent::new(
                base64::engine::general_purpose::STANDARD.encode(&bytes),
                "image/png",
            )),
        ))),
    };
    panel.update(&mut visual, |panel, cx| panel.handle_event(chunk(), cx));
    panel.read_with(&visual, |panel, _| {
        let Entry::UserMessage(message) = &panel.entries()[0] else {
            panic!("mensaje");
        };
        let [MessageBlock::Image(image)] = message.blocks.as_slice() else {
            panic!("{:?}", message.blocks);
        };
        assert!(image.file.is_empty(), "no se guarda aparte");
        assert_eq!((image.width, image.height), (24, 12));
        assert_eq!(image.data.as_deref(), Some(bytes.as_slice()));
    });
    panel.update(&mut visual, |panel, cx| {
        panel.begin_replay(cx);
        panel.handle_event(chunk(), cx);
    });
    panel.read_with(&visual, |panel, _| {
        let Entry::UserMessage(message) = &panel.entries()[0] else {
            panic!("mensaje");
        };
        assert_eq!(message.blocks.len(), 1, "la repetición se descarta");
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sent-image-0-0").is_some());
}

/// The author's answer to §10: a file from Cincel's tree that is an image is
/// attached as an image; any other file is still a mention.
#[gpui::test]
fn an_image_from_the_tree_is_attached_and_other_files_are_mentioned(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    connect(&panel, &mut visual, true);
    let dir = tempfile::tempdir().expect("tempdir");
    let png = png_file(dir.path(), "diagrama.png", 10, 10);
    let code = dir.path().join("main.rs");
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.mention_or_attach(png, window, cx);
        panel.mention_or_attach(code.clone(), window, cx);
    });
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), 1);
    panel.read_with(&visual, |panel, cx| {
        assert_eq!(panel.mentions().len(), 1);
        assert_eq!(panel.mentions()[0].path, code);
        assert!(panel.input_text(cx).contains("@main.rs"));
    });
}

// ------------------------------------------------------------- fake agent

/// The fake agent built next to the tests (`cargo build -p cincel-acp --bin
/// cincel-acp-fake-agent`).
fn fake_agent() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/cincel-chat tiene dos ancestros");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    for profile in ["debug", "release"] {
        let candidate = target.join(profile).join("cincel-acp-fake-agent");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "no se encontró cincel-acp-fake-agent bajo {}; corré `cargo build -p cincel-acp \
         --bin cincel-acp-fake-agent` primero",
        target.display()
    );
}

/// How long a real agent process may take, scaled like every time limit of
/// the tests (`CINCEL_PERF_BUDGET_FACTOR`).
fn deadline() -> Instant {
    let factor = std::env::var("CINCEL_PERF_BUDGET_FACTOR")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|factor| *factor >= 1.)
        .unwrap_or(1.);
    Instant::now() + Duration::from_secs_f64(60. * factor)
}

/// Forwards the agent's events to the panel until `stop` matches one.
fn pump(
    connection: &AgentConnection,
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    mut stop: impl FnMut(&AgentEvent) -> bool,
) {
    let limit = deadline();
    loop {
        match connection.events().try_recv() {
            Ok(event) => {
                let done = stop(&event);
                panel.update(visual, |panel, cx| panel.handle_event(event, cx));
                visual.run_until_parked();
                if done {
                    return;
                }
            }
            Err(_) => {
                assert!(
                    Instant::now() < limit,
                    "el agente falso no respondió a tiempo"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// §5.4: the agent receives one `image` block per image, after the text, with
/// the right `mimeType`, base64 `data` that decodes to the attached bytes and
/// no `uri` — read from the real fake agent's `FAKE_PROMPT_LOG`.
#[gpui::test]
fn the_fake_agent_receives_the_text_and_then_the_images(cx: &mut TestAppContext) {
    let (panel, mut visual, recorded) = open(cx);
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("prompts.jsonl");
    let mut connection = AgentConnection::start(dir.path());
    connection
        .commands()
        .send_blocking(AgentCommand::Spawn {
            launch: LaunchSpec::new(fake_agent().display().to_string(), Vec::new())
                .with_env("FAKE_PROMPT_LOG", log.display().to_string()),
            cwd: dir.path().to_path_buf(),
        })
        .expect("spawn");
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![ChatConnection {
                id: "c-fake".into(),
                agent_id: "claude-acp".into(),
                label: LABEL.into(),
                agent_name: "Claude".into(),
                identity: None,
                last_used: String::new(),
                badge: ConnectionBadge::Connected,
            }],
            cx,
        );
        panel.set_active_connection(Some("c-fake".into()), cx);
    });
    let commands = connection.commands().clone();
    let cwd = dir.path().to_path_buf();
    pump(&connection, &panel, &mut visual, |event| match event {
        AgentEvent::Connected { .. } => {
            commands
                .send_blocking(AgentCommand::NewSession {
                    cwd: cwd.clone(),
                    mcp_servers: Vec::new(),
                })
                .expect("new session");
            false
        }
        AgentEvent::SessionCreated { .. } => true,
        _ => false,
    });
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.image_support()),
        Some(true),
        "la capacidad sale de `Connected`"
    );

    let big = png_file(dir.path(), "grande.png", 4_000, 3_000);
    let small = dir.path().join("chica.jpg");
    std::fs::write(&small, encode(&gradient(800, 600), ImageFormat::Jpeg)).unwrap();
    panel.update(&mut visual, |panel, cx| {
        panel.attach_paths(&[big, small], cx)
    });
    visual.run_until_parked();
    assert_eq!(ready_count(&panel, &mut visual), 2);
    let expected: Vec<(String, String)> = panel.read_with(&visual, |panel, _| {
        panel
            .attachments()
            .iter()
            .map(|attachment| match &attachment.state {
                AttachmentState::Ready(image) => {
                    (image.mime_type.to_string(), image.sha256.clone())
                }
                AttachmentState::Processing => panic!("lista"),
            })
            .collect()
    });

    send(&panel, &mut visual, "echo-images");
    let (session_id, blocks) = recorded.prompts.borrow()[0].clone();
    commands
        .send_blocking(AgentCommand::Prompt {
            session_id,
            blocks,
            feedback: None,
        })
        .expect("prompt");
    pump(&connection, &panel, &mut visual, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    connection.shutdown();

    let line = std::fs::read_to_string(&log).expect("el agente registró el prompt");
    let logged: serde_json::Value =
        serde_json::from_str(line.lines().next().expect("una línea")).expect("json");
    let prompt = logged["prompt"].as_array().expect("arreglo");
    assert_eq!(prompt.len(), 3, "{prompt:?}");
    assert_eq!(prompt[0]["type"], "text");
    assert_eq!(prompt[0]["text"], "echo-images");
    for (block, (mime, sha)) in prompt[1..].iter().zip(&expected) {
        assert_eq!(block["type"], "image");
        assert_eq!(block["mimeType"], mime.as_str());
        assert_eq!(block["sha256"], sha.as_str());
        assert_eq!(block["decodeError"], false);
        assert!(block["uri"].is_null(), "sin uri: {block}");
    }
    let answer = panel.read_with(&visual, |panel, _| {
        panel.entries().iter().find_map(|entry| match entry {
            Entry::AgentText(text) => Some(text.markdown.clone()),
            _ => None,
        })
    });
    assert_eq!(
        answer.as_deref(),
        Some("2 imágenes: image/png 2000x1500, image/jpeg 800x600")
    );
}
