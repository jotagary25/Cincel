//! The chat's drop-down menus close by themselves
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §4): a click
//! outside, the focus going to another element (the editor, `Ctrl+L`), `Esc`,
//! and the window losing the focus. Choosing a row keeps working, and a click
//! on the button that opened the menu closes it and leaves it closed.
//!
//! The panel sits in a window next to a real `cincel_editor::EditorView`, the
//! stand-in for "the editor" of the workspace.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cincel_acp::acp::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
    SessionConfigSelectOption, SessionMode, SessionModeState,
};
use cincel_acp::{AgentEvent, acp::schema::v1::AvailableCommand};
use cincel_editor::{EditorSettings, EditorView};
use cincel_syntax::LanguageRegistry;
use cincel_text::Buffer;
use gpui::Focusable as _;
use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, Context, Entity, FocusHandle, KeyBinding, Modifiers, Render, TestAppContext,
    VisualTestContext, Window, div, px, size,
};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::{ChatPanel, Popover};
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

gpui::actions!(menu_dismiss_tests, [FocusEditor]);

/// The chat on the left and an editor on the right, like the workspace.
struct Harness {
    chat: Entity<ChatPanel>,
    editor: Entity<EditorView>,
    editor_focus: FocusHandle,
}

impl Harness {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let chat =
            cx.new(|cx| ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx));
        let registry = Arc::new(LanguageRegistry::new());
        let editor = cx.new(|cx| {
            EditorView::new(
                cincel_editor::shared(Buffer::new("fn main() {}\n")),
                None,
                registry,
                EditorSettings::default(),
                ChatTheme::default().composer_theme(),
                window,
                cx,
            )
        });
        let editor_focus = editor.read(cx).focus_handle(cx);
        Self {
            chat,
            editor,
            editor_focus,
        }
    }
}

impl Render for Harness {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor_focus = self.editor_focus.clone();
        div()
            .key_context("Harness")
            .on_action(cx.listener(move |_, _: &FocusEditor, window, cx| {
                window.focus(&editor_focus, cx);
            }))
            .flex()
            .size_full()
            .child(div().w(px(420.)).h_full().child(self.chat.clone()))
            .child(
                div()
                    .id("editor-area")
                    .debug_selector(|| "editor-area".to_string())
                    .flex_1()
                    .h_full()
                    .child(self.editor.clone()),
            )
    }
}

/// Every [`crate::ChatEvent`] the panel emitted, as its `Debug` string.
type Recorder = Rc<RefCell<Vec<String>>>;

struct Setup {
    chat: Entity<ChatPanel>,
    harness: Entity<Harness>,
    visual: VisualTestContext,
    recorder: Recorder,
}

fn model_option() -> SessionConfigOption {
    let mut option = SessionConfigOption::new(
        "model",
        "Modelo",
        SessionConfigKind::Select(SessionConfigSelect::new(
            "sonnet",
            vec![
                SessionConfigSelectOption::new("sonnet", "Sonnet"),
                SessionConfigSelectOption::new("opus", "Opus"),
            ],
        )),
    );
    option.category = Some(SessionConfigOptionCategory::Model);
    option
}

fn connection(id: &str, agent_id: &str, label: &str) -> ChatConnection {
    ChatConnection {
        id: id.into(),
        agent_id: agent_id.into(),
        label: label.into(),
        agent_name: provider_name(agent_id).to_string(),
        identity: None,
        last_used: String::new(),
        badge: ConnectionBadge::Connected,
    }
}

/// A chat with two connections, a history row, a model selector and a slash
/// command, next to an editor; the composer holds the focus.
fn setup(cx: &mut TestAppContext) -> Setup {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_default_keys(cx);
        cx.bind_keys([KeyBinding::new("ctrl-l", FocusEditor, Some("Harness"))]);
    });
    let window = cx.add_window(Harness::new);
    let harness = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let chat = harness.read_with(cx, |harness, _| harness.chat.clone());
    let handle: AnyWindowHandle = window.into();
    let mut visual = VisualTestContext::from_window(handle, cx);

    let recorder: Recorder = Rc::new(RefCell::new(Vec::new()));
    let sink = recorder.clone();
    cx.update(|cx| {
        cx.subscribe(&chat, move |_chat, event: &crate::ChatEvent, _cx| {
            sink.borrow_mut().push(format!("{event:?}"));
        })
        .detach();
    });

    chat.update(&mut visual, |chat, cx| {
        chat.set_connections(
            vec![
                connection("c-claude", "claude-acp", "Claude · personal"),
                connection("c-codex", "codex-acp", "Codex · trabajo"),
            ],
            cx,
        );
        chat.set_active_connection(Some("c-claude".into()), cx);
        let conversation = Conversation::new(
            "c1",
            "claude-acp",
            "Claude · personal",
            std::path::PathBuf::from("/p"),
            1,
        )
        .with_connection("c-claude");
        chat.set_conversations(vec![conversation.summary("ayer")], cx);
        chat.handle_event(
            AgentEvent::SessionCreated {
                session_id: cincel_acp::acp::schema::v1::SessionId::new("s1"),
                modes: None,
                config_options: vec![model_option()],
                commands: vec![AvailableCommand::new("review", "Revisar el diff")],
            },
            cx,
        );
    });
    // The window is in the foreground, as in the app: GPUI only reports focus
    // changes (and `observe_window_activation` only has something to observe)
    // for an active window.
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.simulate_resize(size(px(900.), px(720.)));
    visual.run_until_parked();
    // The composer is where the user is typing.
    with_window(&chat, &mut visual, |chat, window, cx| {
        chat.focus_input(window, cx)
    });
    visual.run_until_parked();
    Setup {
        chat,
        harness,
        visual,
        recorder,
    }
}

fn with_window<R>(
    chat: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    f: impl FnOnce(&mut ChatPanel, &mut Window, &mut Context<ChatPanel>) -> R,
) -> R {
    visual.update(|window, cx| chat.update(cx, |chat, cx| f(chat, window, cx)))
}

/// The menus under test: the way to open each and how it is painted.
#[derive(Clone, Copy, Debug)]
enum Menu {
    Connections,
    RowActions,
    Conversations,
    Config,
    Files,
    Commands,
}

impl Menu {
    const ALL: [Menu; 6] = [
        Menu::Connections,
        Menu::RowActions,
        Menu::Conversations,
        Menu::Config,
        Menu::Files,
        Menu::Commands,
    ];

    /// The selector of the button that opens it (none for `@` and `/`, which
    /// open by typing).
    fn button(self) -> Option<&'static str> {
        match self {
            Menu::Connections => Some("chat-connection"),
            Menu::Conversations => Some("chat-sessions"),
            Menu::Config => Some("chat-selector-config-0"),
            _ => None,
        }
    }
}

fn open_menu(setup: &mut Setup, menu: Menu) {
    let chat = setup.chat.clone();
    with_window(&chat, &mut setup.visual, |chat, window, cx| match menu {
        Menu::Connections => chat.toggle_popover(Popover::Connections, cx),
        Menu::RowActions => chat.open_connection_menu("c-claude", cx),
        Menu::Conversations => chat.toggle_popover(Popover::Conversations, cx),
        Menu::Config => chat.toggle_popover(Popover::Config(0), cx),
        Menu::Files => {
            chat.set_file_candidates(vec!["/p/main.rs".into(), "/p/lib.rs".into()], cx);
            chat.set_input_text("@", window, cx);
        }
        Menu::Commands => chat.set_input_text("/", window, cx),
    });
    // One frame to paint the menu and hand it the focus, one to settle.
    setup.visual.simulate_resize(size(px(900.), px(721.)));
    setup.visual.run_until_parked();
    setup.visual.simulate_resize(size(px(900.), px(720.)));
    setup.visual.run_until_parked();
    assert_eq!(popover_of(setup), menu.popover_kind(), "{menu:?} se abrió");
}

impl Menu {
    /// The popover variant without its payload, to compare after typing.
    fn popover_kind(self) -> &'static str {
        match self {
            Menu::Connections => "connections",
            Menu::RowActions => "connection-menu",
            Menu::Conversations => "conversations",
            Menu::Config => "config",
            Menu::Files => "files",
            Menu::Commands => "commands",
        }
    }
}

fn popover_of(setup: &Setup) -> &'static str {
    let popover = setup
        .chat
        .read_with(&setup.visual, |chat, _| chat.popover().clone());
    match popover {
        Popover::Closed => "closed",
        Popover::Connections => "connections",
        Popover::ConnectionMenu(_) => "connection-menu",
        Popover::Conversations => "conversations",
        Popover::Config(_) => "config",
        Popover::Modes => "modes",
        Popover::Files { .. } => "files",
        Popover::Commands { .. } => "commands",
    }
}

fn is_closed(setup: &Setup) -> bool {
    popover_of(setup) == "closed"
}

fn settle(setup: &mut Setup) {
    setup.visual.simulate_resize(size(px(900.), px(721.)));
    setup.visual.run_until_parked();
    setup.visual.simulate_resize(size(px(900.), px(720.)));
    setup.visual.run_until_parked();
}

fn composer_focused(setup: &mut Setup) -> bool {
    let composer = setup.chat.read_with(&setup.visual, |chat, cx| {
        chat.composer().read(cx).focus_handle(cx)
    });
    setup.visual.update(|window, _| composer.is_focused(window))
}

fn editor_focused(setup: &mut Setup) -> bool {
    let focus = setup
        .harness
        .read_with(&setup.visual, |harness, _| harness.editor_focus.clone());
    setup.visual.update(|window, _| focus.is_focused(window))
}

#[gpui::test]
fn opening_a_header_menu_hands_it_the_focus(cx: &mut TestAppContext) {
    for menu in [Menu::Connections, Menu::Conversations, Menu::Config] {
        let mut setup = setup(cx);
        assert!(
            composer_focused(&mut setup),
            "{menu:?}: parte del compositor"
        );
        open_menu(&mut setup, menu);
        assert!(
            !composer_focused(&mut setup),
            "{menu:?}: el menú tomó el foco"
        );
        let chat = setup.chat.clone();
        let inside = setup
            .visual
            .update(|window, cx| chat.read(cx).contains_focus(window, cx));
        assert!(inside, "{menu:?}: el foco sigue dentro del panel");
    }
}

#[gpui::test]
fn a_click_on_the_editor_closes_every_menu(cx: &mut TestAppContext) {
    for menu in Menu::ALL {
        let mut setup = setup(cx);
        open_menu(&mut setup, menu);
        let editor = setup
            .visual
            .debug_bounds("editor-area")
            .expect("el editor se pinta");
        setup
            .visual
            .simulate_click(editor.center(), Modifiers::default());
        setup.visual.run_until_parked();
        assert!(is_closed(&setup), "{menu:?}: clic en el editor lo cierra");
        assert!(
            editor_focused(&mut setup),
            "{menu:?}: el foco quedó en el editor"
        );
        settle(&mut setup);
        assert!(is_closed(&setup), "{menu:?}: sigue cerrado");
    }
}

#[gpui::test]
fn ctrl_l_closes_every_menu(cx: &mut TestAppContext) {
    for menu in [
        Menu::Connections,
        Menu::RowActions,
        Menu::Conversations,
        Menu::Config,
    ] {
        let mut setup = setup(cx);
        open_menu(&mut setup, menu);
        setup.visual.simulate_keystrokes("ctrl-l");
        settle(&mut setup);
        assert!(is_closed(&setup), "{menu:?}: Ctrl+L lo cierra");
        assert!(
            editor_focused(&mut setup),
            "{menu:?}: el foco pasó al editor"
        );
    }
}

#[gpui::test]
fn moving_the_focus_elsewhere_closes_the_lists_of_the_composer(cx: &mut TestAppContext) {
    // `@` and `/` do not take the focus: it is the composer's focus that
    // leaving closes them (`Ctrl+L` is a focus move, whoever makes it).
    for menu in [Menu::Files, Menu::Commands] {
        let mut setup = setup(cx);
        open_menu(&mut setup, menu);
        assert!(
            composer_focused(&mut setup),
            "{menu:?}: el compositor sigue escribiendo"
        );
        let focus = setup
            .harness
            .read_with(&setup.visual, |harness, _| harness.editor_focus.clone());
        setup.visual.update(|window, cx| window.focus(&focus, cx));
        setup.visual.run_until_parked();
        settle(&mut setup);
        assert!(is_closed(&setup), "{menu:?}: el foco se fue del compositor");
    }
}

#[gpui::test]
fn escape_closes_every_menu_and_gives_the_focus_back(cx: &mut TestAppContext) {
    for menu in Menu::ALL {
        let mut setup = setup(cx);
        open_menu(&mut setup, menu);
        setup.visual.simulate_keystrokes("escape");
        setup.visual.run_until_parked();
        assert!(is_closed(&setup), "{menu:?}: Esc lo cierra");
        settle(&mut setup);
        assert!(is_closed(&setup), "{menu:?}: sigue cerrado");
        assert!(
            composer_focused(&mut setup),
            "{menu:?}: Esc devuelve el foco a donde estaba"
        );
    }
}

#[gpui::test]
fn escape_returns_the_focus_to_the_editor_when_it_came_from_there(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    let focus = setup
        .harness
        .read_with(&setup.visual, |harness, _| harness.editor_focus.clone());
    setup.visual.update(|window, cx| window.focus(&focus, cx));
    settle(&mut setup);
    open_menu(&mut setup, Menu::Connections);
    assert!(!editor_focused(&mut setup), "el menú tomó el foco");
    setup.visual.simulate_keystrokes("escape");
    settle(&mut setup);
    assert!(is_closed(&setup));
    assert!(editor_focused(&mut setup), "el foco vuelve al editor");
}

#[gpui::test]
fn the_window_losing_the_focus_closes_every_menu(cx: &mut TestAppContext) {
    for menu in Menu::ALL {
        let mut setup = setup(cx);
        open_menu(&mut setup, menu);
        setup.visual.deactivate_window();
        settle(&mut setup);
        assert!(is_closed(&setup), "{menu:?}: la ventana perdió el foco");
    }
}

#[gpui::test]
fn a_click_on_the_conversation_text_closes_the_header_menus(cx: &mut TestAppContext) {
    for menu in [Menu::Connections, Menu::RowActions, Menu::Conversations] {
        let mut setup = setup(cx);
        open_menu(&mut setup, menu);
        // The transcript's lower edge, above the composer and below any menu.
        let composer = setup
            .visual
            .debug_bounds("chat-composer-box")
            .expect("el compositor se pinta");
        let spot = gpui::point(px(200.), composer.top() - px(12.));
        setup.visual.simulate_click(spot, Modifiers::default());
        setup.visual.run_until_parked();
        settle(&mut setup);
        assert!(
            is_closed(&setup),
            "{menu:?}: clic en la conversación lo cierra"
        );
    }
}

#[gpui::test]
fn a_click_on_the_button_that_opened_the_menu_closes_it_once(cx: &mut TestAppContext) {
    for menu in [Menu::Connections, Menu::Conversations, Menu::Config] {
        let mut setup = setup(cx);
        let button = menu.button().expect("tiene botón");
        open_menu(&mut setup, menu);
        let bounds = setup
            .visual
            .debug_bounds(button)
            .expect("el botón se pinta");
        setup
            .visual
            .simulate_click(bounds.center(), Modifiers::default());
        setup.visual.run_until_parked();
        settle(&mut setup);
        assert!(
            is_closed(&setup),
            "{menu:?}: el clic en su botón lo cierra y no lo vuelve a abrir"
        );
        // And the next click opens it again.
        let bounds = setup
            .visual
            .debug_bounds(button)
            .expect("el botón se pinta");
        setup
            .visual
            .simulate_click(bounds.center(), Modifiers::default());
        setup.visual.run_until_parked();
        settle(&mut setup);
        assert_eq!(
            popover_of(&setup),
            menu.popover_kind(),
            "{menu:?}: se reabre"
        );
    }
}

#[gpui::test]
fn a_press_on_another_header_button_swaps_the_menu(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    open_menu(&mut setup, Menu::Connections);
    let history = setup
        .visual
        .debug_bounds("chat-sessions")
        .expect("el reloj se pinta");
    setup
        .visual
        .simulate_click(history.center(), Modifiers::default());
    setup.visual.run_until_parked();
    settle(&mut setup);
    assert_eq!(popover_of(&setup), "conversations");
}

#[gpui::test]
fn the_conectar_button_without_a_connection_closes_its_menu_once(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    let chat = setup.chat.clone();
    chat.update(&mut setup.visual, |chat, cx| {
        chat.set_active_connection(None, cx);
    });
    settle(&mut setup);
    let button = setup
        .visual
        .debug_bounds("chat-connect")
        .expect("el botón «Conectar» se pinta");
    setup
        .visual
        .simulate_click(button.center(), Modifiers::default());
    settle(&mut setup);
    assert_eq!(popover_of(&setup), "connections", "el clic lo abre");
    setup
        .visual
        .simulate_click(button.center(), Modifiers::default());
    settle(&mut setup);
    assert!(
        is_closed(&setup),
        "el segundo clic lo cierra y sigue cerrado"
    );
}

#[gpui::test]
fn choosing_a_row_still_works_and_gives_the_focus_back(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    open_menu(&mut setup, Menu::Connections);
    let row = setup
        .visual
        .debug_bounds("connection-row-1")
        .expect("la fila se pinta");
    setup
        .visual
        .simulate_click(row.center(), Modifiers::default());
    setup.visual.run_until_parked();
    settle(&mut setup);
    let log = setup.recorder.borrow().join("\n");
    assert!(
        log.contains("ConnectionSelected") && log.contains("c-codex"),
        "el clic dentro del menú elige la fila: {log}"
    );
    assert!(is_closed(&setup));
    assert!(composer_focused(&mut setup), "elegir devuelve el foco");
}

#[gpui::test]
fn right_clicking_a_row_keeps_the_menu_open_and_swaps_it_for_the_context_menu(
    cx: &mut TestAppContext,
) {
    let mut setup = setup(cx);
    open_menu(&mut setup, Menu::Connections);
    let row = setup
        .visual
        .debug_bounds("connection-row-0")
        .expect("la fila se pinta");
    setup
        .visual
        .simulate_mouse_down(row.center(), gpui::MouseButton::Right, Modifiers::default());
    setup
        .visual
        .simulate_mouse_up(row.center(), gpui::MouseButton::Right, Modifiers::default());
    settle(&mut setup);
    assert_eq!(popover_of(&setup), "connection-menu");
    assert!(
        !composer_focused(&mut setup),
        "el menú contextual conserva el foco"
    );
}

#[gpui::test]
fn choosing_a_selector_value_still_works_with_the_focus_in_the_menu(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    open_menu(&mut setup, Menu::Config);
    let row = setup
        .visual
        .debug_bounds("chat-popover")
        .expect("el selector se pinta");
    // The second value ("Opus") sits one row below the first.
    let second = gpui::point(
        row.center().x,
        row.top() + px(5.) + px(crate::settings::POPOVER_ROW_HEIGHT * 1.5),
    );
    setup.visual.simulate_click(second, Modifiers::default());
    settle(&mut setup);
    let log = setup.recorder.borrow().join("\n");
    assert!(log.contains("SetConfigOption"), "{log}");
    assert!(is_closed(&setup));
}

#[gpui::test]
fn enter_with_a_menu_open_does_not_send_the_draft(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    let chat = setup.chat.clone();
    with_window(&chat, &mut setup.visual, |chat, window, cx| {
        chat.set_input_text("hola", window, cx);
    });
    open_menu(&mut setup, Menu::Connections);
    setup.visual.simulate_keystrokes("enter");
    setup.visual.run_until_parked();
    let log = setup.recorder.borrow().join("\n");
    assert!(
        !log.contains("Prompt"),
        "Enter no envía desde el menú: {log}"
    );
    chat.read_with(&setup.visual, |chat, cx| {
        assert_eq!(chat.input_text(cx), "hola");
        assert!(chat.entries().is_empty());
    });
}

#[gpui::test]
fn the_legacy_modes_selector_closes_like_the_others(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_default_keys(cx);
    });
    let window = cx.add_window(Harness::new);
    let harness = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let chat = harness.read_with(cx, |harness, _| harness.chat.clone());
    let handle: AnyWindowHandle = window.into();
    let mut visual = VisualTestContext::from_window(handle, cx);
    chat.update(&mut visual, |chat, cx| {
        chat.set_connections(vec![connection("c", "claude-acp", "Claude")], cx);
        chat.set_active_connection(Some("c".into()), cx);
        chat.handle_event(
            AgentEvent::SessionCreated {
                session_id: cincel_acp::acp::schema::v1::SessionId::new("s1"),
                modes: Some(SessionModeState::new(
                    "ask",
                    vec![
                        SessionMode::new("ask", "Preguntar"),
                        SessionMode::new("code", "Código"),
                    ],
                )),
                config_options: Vec::new(),
                commands: Vec::new(),
            },
            cx,
        );
    });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.simulate_resize(size(px(900.), px(720.)));
    visual.run_until_parked();
    let chip = visual
        .debug_bounds("chat-selector-modes")
        .expect("el selector de modos se pinta");
    visual.simulate_click(chip.center(), Modifiers::default());
    visual.simulate_resize(size(px(900.), px(721.)));
    visual.run_until_parked();
    assert_eq!(
        chat.read_with(&visual, |chat, _| chat.popover().clone()),
        Popover::Modes
    );
    let editor = visual
        .debug_bounds("editor-area")
        .expect("el editor se pinta");
    visual.simulate_click(editor.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        chat.read_with(&visual, |chat, _| chat.popover().clone()),
        Popover::Closed
    );
}
