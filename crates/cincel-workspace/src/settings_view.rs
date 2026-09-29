//! The settings tab (`Ctrl+,`, `docs/specs/07-etapa5-productividad.md` §4).
//!
//! [`SettingsView`] is a tab of the center area like a file tab, drawn with
//! gpui-kit's primitives (`Switch`, `Select`, `NumberInput`, `Input`,
//! `Button`) rather than gpui-kit's `Settings` component, whose fixed
//! English texts Cincel cannot translate (D2). Every visible string is
//! Spanish and passed explicitly, placeholders included.
//!
//! # Two ways to change a setting
//!
//! Each control writes its key straight into `settings.json` with
//! [`cincel_settings::write_edit`], which keeps comments, key order and
//! unknown keys byte for byte, and then emits [`SettingsViewEvent::Written`]:
//! the workspace re-reads the file and applies it *without* the
//! "Configuración recargada" toast (`Workspace::apply_reloaded`). Editing the
//! file by hand keeps working as before: the watcher reloads it and this view,
//! which observes [`AppSettings`], redraws with the values in force. When the
//! file has a syntax error nothing is written: a banner says where the error
//! is and the controls are disabled, showing the values in force.
//!
//! # Connections
//!
//! The Connections section never touches `cincel_connections` itself: it
//! shows the [`ConnectionsInfo`] that [`crate::agents::Agents`] pushes into
//! it and emits [`SettingsViewEvent`]s (rename, reconnect, repair, delete,
//! new connection, update the adapter or Node, cancel), which `Agents`
//! decides on — the same rule the chat follows since Etapa 4.

use cincel_chat::{ChatConnection, ConnectionBadge};
use cincel_connections::AgentKind;
use cincel_settings::{EditError, Paths, Settings, SettingsEdit};
use gpui::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, SharedString, Subscription, Window,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Escape, Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction,
};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, div, px};
use serde_json::Value;
use uuid::Uuid;

use crate::settings::AppSettings;
use crate::theme::{ThemeColors, Typography};

/// The GPUI key context of the settings tab (§11.2).
pub const KEY_CONTEXT: &str = "SettingsPage";

/// The title of the tab, in the tab bar and the status bar.
pub const TAB_TITLE: &str = "Configuración";

/// Width of the column of sections (§4.1).
const SECTIONS_WIDTH: f32 = 180.;
/// Height of one entry of the column of sections.
const SECTION_ROW_HEIGHT: f32 = 28.;
/// Width of a select control.
const SELECT_WIDTH: f32 = 240.;
/// Width of a number control.
const NUMBER_WIDTH: f32 = 130.;
/// Widest the search field gets.
const SEARCH_MAX_WIDTH: f32 = 360.;

/// Placeholder of the search field.
pub const SEARCH_PLACEHOLDER: &str = "Buscar ajustes…";
/// The button that opens `settings.json` in a tab.
pub const OPEN_FILE_LABEL: &str = "Abrir settings.json";
/// Tooltip of the per-row reset button.
pub const RESET_LABEL: &str = "Restablecer";
/// Placeholder of the "add a pattern" field of the list rows.
pub const ADD_PATTERN_PLACEHOLDER: &str = "Agregar patrón";
/// The button that removes one pattern of a list row.
pub const REMOVE_PATTERN_LABEL: &str = "Quitar";
/// Placeholder of every select, shown only when nothing is selected.
const SELECT_PLACEHOLDER: &str = "Elegí una opción";
/// Placeholder of the search field inside a long select.
const SELECT_SEARCH_PLACEHOLDER: &str = "Buscar…";
/// What a select shows when its search matches nothing.
const SELECT_EMPTY: &str = "Sin resultados";
/// Placeholder of the inline rename field of a connection.
const RENAME_PLACEHOLDER: &str = "Nombre de la conexión";
/// The rename error of §4.4.
pub const EMPTY_NAME_ERROR: &str = "El nombre no puede quedar vacío";
/// The note under the Connections section (§4.2).
pub const CONNECTIONS_FOOTER: &str =
    "Los servidores MCP y la versión de Node se configuran en settings.json";
/// "Sin red" line of the installed agents (§4.4).
pub const OFFLINE_CATALOG: &str = "No se pudo consultar el catálogo de agentes: sin conexión";
/// The Node row when nothing is installed (§4.4).
pub const NODE_NOT_INSTALLED: &str = "No instalado (se descarga al conectar Claude o Codex)";

/// One section of the column on the left (§4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    /// Theme, text rendering, window decorations.
    Appearance,
    /// Families and sizes.
    Fonts,
    /// Editor behaviour.
    Editor,
    /// Autosave and excluded files.
    Files,
    /// Agent change review.
    Review,
    /// Saved connections, installed agents and the private Node (§4.4).
    Connections,
}

impl SettingsSection {
    /// Every section, in the order of the column.
    pub const ALL: [SettingsSection; 6] = [
        SettingsSection::Appearance,
        SettingsSection::Fonts,
        SettingsSection::Editor,
        SettingsSection::Files,
        SettingsSection::Review,
        SettingsSection::Connections,
    ];

    /// The Spanish title of the section.
    pub fn title(self) -> &'static str {
        match self {
            SettingsSection::Appearance => "Apariencia",
            SettingsSection::Fonts => "Fuentes",
            SettingsSection::Editor => "Editor",
            SettingsSection::Files => "Archivos",
            SettingsSection::Review => "Revisión",
            SettingsSection::Connections => "Conexiones",
        }
    }
}

/// What the settings tab tells the rest of the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsViewEvent {
    /// A key was written to `settings.json`: re-read it and apply it without
    /// the "Configuración recargada" toast (§4.3).
    Written,
    /// "Abrir settings.json": the center opens it in a pinned tab, creating
    /// it with `{}` when it does not exist.
    OpenSettingsFile,
    /// "Renombrar" saved a non-empty label.
    Rename {
        /// The connection.
        id: Uuid,
        /// Its new label, already trimmed.
        label: String,
    },
    /// "Volver a conectar" (expired sessions only).
    Reconnect {
        /// The connection.
        id: Uuid,
    },
    /// "Reparar" (unavailable connections only).
    Repair {
        /// The connection.
        id: Uuid,
    },
    /// "Eliminar…": the existing F3 of the connections modal, already on
    /// this connection.
    Delete {
        /// The connection.
        id: Uuid,
    },
    /// "Conectar nuevo agente…" (F2).
    NewConnection,
    /// "Actualizar a X.Y.Z" of an installed agent (§10.4).
    UpdateAdapter(AgentKind),
    /// "Actualizar a vX.Y.Z" of the private Node (§10.4).
    UpdateNode,
    /// "Buscar actualizaciones": ask the ACP registry and nodejs.org again.
    CheckUpdates,
    /// "Cancelar" of an update in progress (§10.3).
    CancelUpdate,
    /// The Connections section came on screen for the first time in this
    /// tab: `Agents` asks the registry once per session (§4.4).
    ConnectionsShown,
}

impl EventEmitter<SettingsViewEvent> for SettingsView {}

/// Whether the ACP registry and nodejs.org have been asked (§4.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogState {
    /// Not yet.
    #[default]
    NotChecked,
    /// The question is in flight.
    Checking,
    /// Answered: the update buttons reflect it.
    Checked,
    /// No network: "No se pudo consultar el catálogo de agentes: sin
    /// conexión", and no update button.
    Offline,
}

/// What an update is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UpdateTarget {
    /// The adapter (or binary) of one agent.
    Adapter(AgentKind),
    /// The private Node runtime.
    Node,
}

/// An update in progress: the bar and its text.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateOperation {
    /// What is being updated.
    pub target: UpdateTarget,
    /// Percentage, when the download says how big it is.
    pub percent: Option<f32>,
    /// "Actualizando el adaptador de Claude… 45 %".
    pub text: String,
}

/// How the last update of a target ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// "Actualizado a 0.9.0 · se usará al volver a conectar".
    Updated(String),
    /// "No se pudo actualizar: <motivo>", with "Reintentar".
    Failed(String),
}

/// One installed agent of "Agentes instalados".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledAgent {
    /// Which agent.
    pub kind: AgentKind,
    /// The installed version.
    pub version: String,
    /// The registry's newer version, when there is one.
    pub update: Option<String>,
    /// How its last update ended, if it ran in this session.
    pub outcome: Option<UpdateOutcome>,
}

/// The "Node privado" row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeInfo {
    /// Installed version (`v24.1.0`), if any.
    pub installed: Option<String>,
    /// The newer version the policy offers, when there is one.
    pub update: Option<String>,
    /// How its last update ended, if it ran in this session.
    pub outcome: Option<UpdateOutcome>,
}

/// Everything the Connections section shows, as `Agents` summarised it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectionsInfo {
    /// The saved connections, most recently used first (the same rows the
    /// chat's popover shows).
    pub connections: Vec<ChatConnection>,
    /// The installed agents.
    pub agents: Vec<InstalledAgent>,
    /// The private Node runtime.
    pub node: NodeInfo,
    /// Whether the registry has been asked.
    pub catalog: CatalogState,
    /// The update in progress, if any (one at a time).
    pub operation: Option<UpdateOperation>,
}

/// The kind of control a row has.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    /// An on/off switch.
    Switch,
    /// A select over a fixed or computed list.
    Choice(ChoiceKind),
    /// A number field with its +/- buttons.
    Number(NumberSpec),
    /// An editable list of glob patterns.
    Patterns,
}

/// The selects of §4.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ChoiceKind {
    ThemeMode,
    DarkTheme,
    LightTheme,
    TextRendering,
    Decorations,
    UiFont,
    BufferFont,
    Autosave,
}

/// Range and precision of a number row.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NumberSpec {
    min: f64,
    max: f64,
    step: f64,
    /// `0` writes integers, `1` one decimal (shown with a comma).
    decimals: u8,
}

const fn number(min: f64, max: f64, step: f64, decimals: u8) -> Control {
    Control::Number(NumberSpec {
        min,
        max,
        step,
        decimals,
    })
}

/// One row of the table of §4.2.
#[derive(Clone, Copy, Debug)]
struct Row {
    section: SettingsSection,
    /// The JSON key as a user reads it (`editor.soft_wrap`).
    key: &'static str,
    /// The same key as `cincel_settings::edit` wants it.
    path: &'static [&'static str],
    title: &'static str,
    description: &'static str,
    control: Control,
    /// A warning under the row ("Se aplica al reiniciar Cincel").
    note: Option<&'static str>,
}

/// Every setting the tab edits, in display order (§4.2).
const ROWS: &[Row] = &[
    Row {
        section: SettingsSection::Appearance,
        key: "theme.mode",
        path: &["theme", "mode"],
        title: "Modo del tema",
        description: "Seguir al escritorio o fijar el modo oscuro o el claro.",
        control: Control::Choice(ChoiceKind::ThemeMode),
        note: None,
    },
    Row {
        section: SettingsSection::Appearance,
        key: "theme.dark",
        path: &["theme", "dark"],
        title: "Tema oscuro",
        description: "El tema que se usa en modo oscuro.",
        control: Control::Choice(ChoiceKind::DarkTheme),
        note: None,
    },
    Row {
        section: SettingsSection::Appearance,
        key: "theme.light",
        path: &["theme", "light"],
        title: "Tema claro",
        description: "El tema que se usa en modo claro.",
        control: Control::Choice(ChoiceKind::LightTheme),
        note: None,
    },
    Row {
        section: SettingsSection::Appearance,
        key: "text_rendering",
        path: &["text_rendering"],
        title: "Suavizado del texto",
        description: "Cómo se dibujan los bordes de las letras.",
        control: Control::Choice(ChoiceKind::TextRendering),
        note: None,
    },
    Row {
        section: SettingsSection::Appearance,
        key: "window.decorations",
        path: &["window", "decorations"],
        title: "Barra de título",
        description: "Quién dibuja la barra de título de la ventana.",
        control: Control::Choice(ChoiceKind::Decorations),
        note: Some("Se aplica al reiniciar Cincel"),
    },
    Row {
        section: SettingsSection::Fonts,
        key: "ui_font_family",
        path: &["ui_font_family"],
        title: "Fuente de la interfaz",
        description: "La familia tipográfica de menús, paneles y chat.",
        control: Control::Choice(ChoiceKind::UiFont),
        note: None,
    },
    Row {
        section: SettingsSection::Fonts,
        key: "ui_font_size",
        path: &["ui_font_size"],
        title: "Tamaño de la interfaz",
        description: "En píxeles, de 6 a 72.",
        control: number(6., 72., 1., 0),
        note: None,
    },
    Row {
        section: SettingsSection::Fonts,
        key: "buffer_font_family",
        path: &["buffer_font_family"],
        title: "Fuente del editor",
        description: "La familia tipográfica del código.",
        control: Control::Choice(ChoiceKind::BufferFont),
        note: None,
    },
    Row {
        section: SettingsSection::Fonts,
        key: "buffer_font_size",
        path: &["buffer_font_size"],
        title: "Tamaño del editor",
        description: "En píxeles, de 6 a 72.",
        control: number(6., 72., 1., 0),
        note: None,
    },
    Row {
        section: SettingsSection::Fonts,
        key: "buffer_line_height",
        path: &["buffer_line_height"],
        title: "Interlineado del editor",
        description: "Múltiplo del tamaño de letra, de 1,0 a 4,0.",
        control: number(1., 4., 0.1, 1),
        note: None,
    },
    Row {
        section: SettingsSection::Editor,
        key: "editor.soft_wrap",
        path: &["editor", "soft_wrap"],
        title: "Ajustar líneas largas",
        description: "Parte las líneas que no entran en el ancho del editor.",
        control: Control::Switch,
        note: None,
    },
    Row {
        section: SettingsSection::Editor,
        key: "editor.tab_size",
        path: &["editor", "tab_size"],
        title: "Tamaño de tabulación",
        description: "Columnas de cada tabulación, de 1 a 16.",
        control: number(1., 16., 1., 0),
        note: None,
    },
    Row {
        section: SettingsSection::Editor,
        key: "editor.insert_spaces",
        path: &["editor", "insert_spaces"],
        title: "Insertar espacios al tabular",
        description: "Tab escribe espacios en lugar de un carácter de tabulación.",
        control: Control::Switch,
        note: None,
    },
    Row {
        section: SettingsSection::Editor,
        key: "editor.auto_close_pairs",
        path: &["editor", "auto_close_pairs"],
        title: "Cerrar pares automáticamente",
        description: "Al abrir un paréntesis, un corchete, una llave o unas comillas se agrega el cierre.",
        control: Control::Switch,
        note: None,
    },
    Row {
        section: SettingsSection::Editor,
        key: "editor.show_whitespace",
        path: &["editor", "show_whitespace"],
        title: "Mostrar espacios en blanco",
        description: "Dibuja los espacios y las tabulaciones.",
        control: Control::Switch,
        note: None,
    },
    Row {
        section: SettingsSection::Editor,
        key: "editor.cursor_blink",
        path: &["editor", "cursor_blink"],
        title: "Parpadeo del cursor",
        description: "El cursor parpadea mientras escribís.",
        control: Control::Switch,
        note: None,
    },
    Row {
        section: SettingsSection::Editor,
        key: "editor.ruler",
        path: &["editor", "ruler"],
        title: "Guía vertical",
        description: "Columna de la guía; 0 oculta la guía.",
        control: number(0., 400., 1., 0),
        note: None,
    },
    Row {
        section: SettingsSection::Files,
        key: "files.autosave",
        path: &["files", "autosave"],
        title: "Autoguardado",
        description: "Cuándo se guardan los archivos sin que lo pidas.",
        control: Control::Choice(ChoiceKind::Autosave),
        note: None,
    },
    Row {
        section: SettingsSection::Files,
        key: "files.autosave_delay_ms",
        path: &["files", "autosave_delay_ms"],
        title: "Pausa del autoguardado",
        description: "Milisegundos sin escribir antes de guardar, de 100 a 60 000. Solo con «Tras una pausa».",
        control: number(100., 60_000., 100., 0),
        note: None,
    },
    Row {
        section: SettingsSection::Files,
        key: "files.exclude",
        path: &["files", "exclude"],
        title: "Archivos excluidos",
        description: "Patrones que no se muestran en el árbol ni en el buscador de archivos.",
        control: Control::Patterns,
        note: None,
    },
    Row {
        section: SettingsSection::Review,
        key: "review.jump_to_next_on_decide",
        path: &["review", "jump_to_next_on_decide"],
        title: "Saltar al siguiente cambio al decidir",
        description: "Después de aceptar o rechazar, el cursor va al próximo cambio pendiente.",
        control: Control::Switch,
        note: None,
    },
    Row {
        section: SettingsSection::Review,
        key: "review.sensitive_paths",
        path: &["review", "sensitive_paths"],
        title: "Rutas sensibles",
        description: "Rutas que siempre piden permiso antes de que el agente las toque.",
        control: Control::Patterns,
        note: None,
    },
    Row {
        section: SettingsSection::Review,
        key: "review.max_file_size_kb",
        path: &["review", "max_file_size_kb"],
        title: "Tamaño máximo por archivo (KB)",
        description: "No se puede superar el límite fijo de 2 MB / 50 000 líneas.",
        control: number(1., 2048., 1., 0),
        note: None,
    },
    Row {
        section: SettingsSection::Review,
        key: "review.max_lines",
        path: &["review", "max_lines"],
        title: "Líneas máximas por archivo",
        description: "No se puede superar el límite fijo de 2 MB / 50 000 líneas.",
        control: number(1., 50_000., 1., 0),
        note: None,
    },
];

/// Words that make the Connections section match a search.
const CONNECTIONS_KEYWORDS: &[&str] = &[
    "Conexiones",
    "Conectar nuevo agente",
    "Agentes instalados",
    "adaptador",
    "Actualizar",
    "Node privado",
    "connections.mcp_servers",
    "connections.runtime.node_version",
];

/// One entry of a select.
#[derive(Clone, Debug, PartialEq)]
struct Choice {
    /// What is written to `settings.json`.
    value: SharedString,
    /// What the user reads.
    title: SharedString,
}

impl Choice {
    fn new(value: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            title: title.into(),
        }
    }
}

impl SelectItem for Choice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &SharedString {
        &self.value
    }
}

type ChoiceState = Entity<SelectState<SearchableVec<Choice>>>;

/// Why the tab cannot write `settings.json` right now (§4.3).
#[derive(Clone, Debug, PartialEq, Eq)]
enum FileProblem {
    /// A syntax error, 1-indexed.
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },
    /// The top level is not an object.
    NotAnObject,
}

/// The inline "Renombrar" of a connection row.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Renaming {
    id: Uuid,
    error: Option<String>,
}

/// The settings tab.
pub struct SettingsView {
    focus_handle: FocusHandle,
    section: SettingsSection,
    search: Entity<InputState>,
    selects: Vec<(ChoiceKind, ChoiceState)>,
    /// One field per number row, by index into [`ROWS`].
    numbers: Vec<(usize, Entity<InputState>)>,
    /// One "Agregar patrón" field per list row, by index into [`ROWS`].
    pattern_inputs: Vec<(usize, Entity<InputState>)>,
    rename_input: Entity<InputState>,
    renaming: Option<Renaming>,
    connections: ConnectionsInfo,
    problem: Option<FileProblem>,
    write_error: Option<String>,
    /// `Settings::default()` as JSON, for "Restablecer".
    defaults: Value,
    /// The installed font families, sorted, read once.
    font_names: Vec<String>,
    /// Whether [`SettingsViewEvent::ConnectionsShown`] went out already.
    announced_connections: bool,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsView {
    /// Builds the tab on its first section, with every control reflecting
    /// the settings in force.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = crate::settings::settings(cx);
        let json = settings_json(&settings);
        let mut font_names = cx.text_system().all_font_names();
        font_names.sort();
        font_names.dedup();
        let mut subscriptions = Vec::new();

        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(SEARCH_PLACEHOLDER.to_string()));
        subscriptions.push(cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.announce_connections_if_visible(cx);
                    cx.notify();
                }
            },
        ));

        let mut selects = Vec::new();
        for row in ROWS {
            let Control::Choice(kind) = row.control else {
                continue;
            };
            let items = choices(kind, &settings, &font_names, cx);
            let selected = selected_value(kind, &settings);
            let index = items
                .iter()
                .position(|choice| choice.value == selected)
                .map(|row| gpui_kit::component::IndexPath::default().row(row));
            // The long lists (fonts, themes) get a search field.
            let searchable = matches!(
                kind,
                ChoiceKind::UiFont
                    | ChoiceKind::BufferFont
                    | ChoiceKind::DarkTheme
                    | ChoiceKind::LightTheme
            );
            let state = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), index, window, cx)
                    .searchable(searchable)
            });
            let path = row.path;
            subscriptions.push(cx.subscribe_in(
                &state,
                window,
                move |this, _, event: &SelectEvent<SearchableVec<Choice>>, _, cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    this.write(
                        SettingsEdit::Set {
                            path,
                            value: Value::String(value.to_string()),
                        },
                        cx,
                    );
                },
            ));
            selects.push((kind, state));
        }

        let mut numbers = Vec::new();
        let mut pattern_inputs = Vec::new();
        for (index, row) in ROWS.iter().enumerate() {
            match row.control {
                Control::Number(spec) => {
                    let text = format_number(json_at(&json, row.path), spec);
                    let state = cx.new(|cx| InputState::new(window, cx).default_value(text));
                    subscriptions.push(cx.subscribe_in(
                        &state,
                        window,
                        move |this, _, event: &InputEvent, window, cx| {
                            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                                this.commit_number(index, window, cx);
                            }
                        },
                    ));
                    subscriptions.push(cx.subscribe_in(
                        &state,
                        window,
                        move |this, _, event: &NumberInputEvent, window, cx| {
                            let NumberInputEvent::Step(action) = event;
                            this.step_number(index, *action, window, cx);
                        },
                    ));
                    numbers.push((index, state));
                }
                Control::Patterns => {
                    let state = cx.new(|cx| {
                        InputState::new(window, cx).placeholder(ADD_PATTERN_PLACEHOLDER.to_string())
                    });
                    subscriptions.push(cx.subscribe_in(
                        &state,
                        window,
                        move |this, _, event: &InputEvent, window, cx| {
                            if matches!(event, InputEvent::PressEnter { .. }) {
                                this.add_pattern(index, window, cx);
                            }
                        },
                    ));
                    pattern_inputs.push((index, state));
                }
                Control::Switch | Control::Choice(_) => {}
            }
        }

        let rename_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(RENAME_PLACEHOLDER.to_string()));
        subscriptions.push(cx.subscribe_in(
            &rename_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_rename(window, cx);
                }
            },
        ));

        // Hand edits and the tab's own writes both land here, through the
        // global the reload replaces.
        subscriptions.push(
            cx.observe_global_in::<AppSettings>(window, |this, window, cx| {
                this.sync_from_settings(window, cx);
            }),
        );

        Self {
            focus_handle: cx.focus_handle(),
            section: SettingsSection::Appearance,
            search,
            selects,
            numbers,
            pattern_inputs,
            rename_input,
            renaming: None,
            connections: ConnectionsInfo::default(),
            problem: file_problem(cx),
            write_error: None,
            defaults: settings_json(&Settings::default()),
            font_names,
            announced_connections: false,
            _subscriptions: subscriptions,
        }
    }

    // ------------------------------------------------------------ getters

    /// The section on screen (without a search).
    pub fn section(&self) -> SettingsSection {
        self.section
    }

    /// What the Connections section shows.
    pub fn connections_info(&self) -> &ConnectionsInfo {
        &self.connections
    }

    /// Forces an update in progress, as if `FakeEnv`'s installer were still
    /// running. Used by the tests: with no controllable slow installer in
    /// this crate (`FakeEnv` has no registry, so a real update resolves
    /// almost instantly and races the test's single threaded executor, the
    /// same limitation `connection_cancel_tests.rs` documents for the
    /// connect flow), this is the only deterministic way to exercise the
    /// §10.4 progress row's layout.
    pub fn set_operation_for_test(&mut self, operation: UpdateOperation, cx: &mut Context<Self>) {
        self.connections.operation = Some(operation);
        cx.notify();
    }

    /// The text of the search field.
    pub fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    /// The sections on screen: the selected one, or every one with a match
    /// while the search field has text.
    pub fn visible_sections(&self, cx: &App) -> Vec<SettingsSection> {
        let query = fold(self.query(cx).trim());
        if query.is_empty() {
            return vec![self.section];
        }
        SettingsSection::ALL
            .into_iter()
            .filter(|section| self.section_matches(*section, &query))
            .collect()
    }

    /// The keys of the rows on screen, in order.
    pub fn visible_keys(&self, cx: &App) -> Vec<&'static str> {
        let query = fold(self.query(cx).trim());
        let sections = self.visible_sections(cx);
        ROWS.iter()
            .filter(|row| sections.contains(&row.section) && row_matches(row, &query))
            .map(|row| row.key)
            .collect()
    }

    /// The banner above the controls, if any (§4.3).
    pub fn banner(&self) -> Option<String> {
        self.problem
            .as_ref()
            .map(problem_text)
            .or_else(|| self.write_error.clone())
    }

    /// Whether the controls are disabled (the file has a syntax error).
    pub fn is_read_only(&self) -> bool {
        self.problem.is_some()
    }

    /// The text of the number field of `key`, as the user sees it.
    pub fn number_text(&self, key: &str, cx: &App) -> Option<String> {
        let index = ROWS.iter().position(|row| row.key == key)?;
        self.numbers
            .iter()
            .find(|(row, _)| *row == index)
            .map(|(_, state)| state.read(cx).value().to_string())
    }

    /// What the select of `key` shows as selected.
    pub fn selected_title(&self, key: &str, cx: &App) -> Option<String> {
        let row = ROWS.iter().find(|row| row.key == key)?;
        let Control::Choice(kind) = row.control else {
            return None;
        };
        let (_, state) = self.selects.iter().find(|(k, _)| *k == kind)?;
        let value = state.read(cx).selected_value()?.clone();
        let settings = crate::settings::settings(cx);
        choices(kind, &settings, &self.font_names, cx)
            .into_iter()
            .find(|choice| choice.value == value)
            .map(|choice| choice.title.to_string())
    }

    /// Whether `key` differs from its default, which is when the row shows
    /// "Restablecer".
    pub fn is_modified(&self, key: &str, cx: &App) -> bool {
        let Some(row) = ROWS.iter().find(|row| row.key == key) else {
            return false;
        };
        let json = settings_json(&crate::settings::settings(cx));
        json_at(&json, row.path) != json_at(&self.defaults, row.path)
    }

    /// The rename error on screen, if any.
    pub fn rename_error(&self) -> Option<&str> {
        self.renaming
            .as_ref()
            .and_then(|renaming| renaming.error.as_deref())
    }

    /// Every string the tab can draw in its current state: section titles,
    /// row texts, the labels of the controls and the placeholders of the
    /// fields. The test against English leftovers of gpui-kit reads it
    /// (§4.5).
    pub fn visible_texts(&self, cx: &App) -> Vec<String> {
        let mut texts: Vec<String> = SettingsSection::ALL
            .iter()
            .map(|section| section.title().to_string())
            .collect();
        texts.extend(
            [
                TAB_TITLE,
                SEARCH_PLACEHOLDER,
                OPEN_FILE_LABEL,
                RESET_LABEL,
                REMOVE_PATTERN_LABEL,
                SELECT_PLACEHOLDER,
                SELECT_SEARCH_PLACEHOLDER,
                SELECT_EMPTY,
                CONNECTIONS_FOOTER,
                OFFLINE_CATALOG,
                NODE_NOT_INSTALLED,
                EMPTY_NAME_ERROR,
            ]
            .map(str::to_string),
        );
        texts.extend(CONNECTIONS_LABELS.iter().map(|text| text.to_string()));
        let settings = crate::settings::settings(cx);
        for row in ROWS {
            texts.push(row.title.to_string());
            texts.push(row.description.to_string());
            texts.push(row.key.to_string());
            texts.extend(row.note.map(str::to_string));
            // Font families are the system's own names, not Cincel's texts.
            if let Control::Choice(kind) = row.control
                && !matches!(kind, ChoiceKind::UiFont | ChoiceKind::BufferFont)
            {
                texts.extend(
                    choices(kind, &settings, &self.font_names, cx)
                        .into_iter()
                        .map(|choice| choice.title.to_string()),
                );
            }
        }
        // The placeholders every field was built with (the number fields
        // have none).
        texts.extend(
            [
                SEARCH_PLACEHOLDER,
                ADD_PATTERN_PLACEHOLDER,
                RENAME_PLACEHOLDER,
            ]
            .map(str::to_string),
        );
        texts.extend(self.banner());
        for connection in &self.connections.connections {
            texts.push(connection.label.clone());
            texts.push(connection.last_used.clone());
            texts.push(badge_label(&connection.badge).to_string());
        }
        texts.extend(self.connections.agents.iter().map(agent_line));
        texts.push(node_line(&self.connections.node));
        texts.retain(|text| !text.is_empty());
        texts
    }

    // ------------------------------------------------------------ changes

    /// Shows `section` (clearing the search, which would hide it).
    pub fn set_section(
        &mut self,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.section = section;
        if !self.query(cx).is_empty() {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
        }
        self.announce_connections_if_visible(cx);
        cx.notify();
    }

    /// Types `query` in the search field.
    pub fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| {
            search.set_value(query.to_string(), window, cx)
        });
        self.announce_connections_if_visible(cx);
        cx.notify();
    }

    /// Replaces what the Connections section shows (`Agents` calls it).
    pub fn set_connections_info(&mut self, info: ConnectionsInfo, cx: &mut Context<Self>) {
        if self.connections != info {
            self.connections = info;
            cx.notify();
        }
    }

    /// Sets `key` to `value` in `settings.json`, as its control would.
    ///
    /// Returns `false` when `key` is not a row of the tab.
    pub fn set_value(&mut self, key: &str, value: Value, cx: &mut Context<Self>) -> bool {
        let Some(row) = ROWS.iter().find(|row| row.key == key) else {
            return false;
        };
        self.write(
            SettingsEdit::Set {
                path: row.path,
                value,
            },
            cx,
        );
        true
    }

    /// "Restablecer" of `key`: removes it from `settings.json`.
    pub fn reset(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(row) = ROWS.iter().find(|row| row.key == key) else {
            return false;
        };
        self.write(SettingsEdit::Remove { path: row.path }, cx);
        true
    }

    /// Writes one edit, unless the file cannot be edited right now.
    fn write(&mut self, edit: SettingsEdit, cx: &mut Context<Self>) {
        self.problem = file_problem(cx);
        if self.problem.is_some() {
            tracing::debug!("settings.json tiene un error: la pestaña no escribe");
            cx.notify();
            return;
        }
        let Some(paths) = config_paths(cx) else {
            self.write_error = Some(
                "No se encontró la carpeta de configuración: los ajustes no se pueden guardar."
                    .to_string(),
            );
            cx.notify();
            return;
        };
        match cincel_settings::write_edit(&paths, &edit) {
            Ok(()) => {
                tracing::info!(?edit, "ajuste guardado desde la pestaña de configuración");
                self.write_error = None;
                cx.emit(SettingsViewEvent::Written);
            }
            Err(EditError::Syntax { .. } | EditError::NotAnObject) => {
                self.problem = file_problem(cx);
            }
            Err(EditError::Io(error)) => {
                tracing::warn!(%error, "no se pudo escribir settings.json");
                self.write_error = Some(format!("No se pudo guardar settings.json: {error}"));
            }
        }
        cx.notify();
    }

    /// The value of a number row as typed: clamped and written, or put back
    /// when it is not a number.
    fn commit_number(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let row = &ROWS[index];
        let Control::Number(spec) = row.control else {
            return;
        };
        let Some(state) = self.number_state(index) else {
            return;
        };
        let json = settings_json(&crate::settings::settings(cx));
        let current = json_at(&json, row.path).and_then(Value::as_f64);
        // A disabled field keeps showing the value in force.
        let typed = if self.row_enabled(row, cx) {
            parse_number(&state.read(cx).value(), spec)
        } else {
            None
        };
        match typed {
            Some(value) => {
                let value = clamp(value, spec);
                let text = format_value(value, spec);
                state.update(cx, |state, cx| {
                    if state.value().as_ref() != text.as_str() {
                        state.set_value(text, window, cx);
                    }
                });
                if current.is_none_or(|current| (current - value).abs() > f64::EPSILON * 16.) {
                    self.write(
                        SettingsEdit::Set {
                            path: row.path,
                            value: number_value(value, spec),
                        },
                        cx,
                    );
                }
            }
            None => {
                let text = format_number(json_at(&json, row.path), spec);
                state.update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
    }

    /// The +/- buttons of a number row.
    fn step_number(
        &mut self,
        index: usize,
        action: StepAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = &ROWS[index];
        let Control::Number(spec) = row.control else {
            return;
        };
        if !self.row_enabled(row, cx) {
            return;
        }
        let json = settings_json(&crate::settings::settings(cx));
        let current = json_at(&json, row.path)
            .and_then(Value::as_f64)
            .unwrap_or(spec.min);
        let next = match action {
            StepAction::Increment => current + spec.step,
            StepAction::Decrement => current - spec.step,
        };
        let next = clamp(next, spec);
        if let Some(state) = self.number_state(index) {
            let text = format_value(next, spec);
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if (next - current).abs() > f64::EPSILON * 16. {
            self.write(
                SettingsEdit::Set {
                    path: row.path,
                    value: number_value(next, spec),
                },
                cx,
            );
        }
    }

    /// Types `text` in the number field of `key` and presses `Enter`.
    pub fn type_number(
        &mut self,
        key: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = ROWS.iter().position(|row| row.key == key) else {
            return;
        };
        let Some(state) = self.number_state(index) else {
            return;
        };
        state.update(cx, |state, cx| {
            state.set_value(text.to_string(), window, cx)
        });
        self.commit_number(index, window, cx);
    }

    fn number_state(&self, index: usize) -> Option<Entity<InputState>> {
        self.numbers
            .iter()
            .find(|(row, _)| *row == index)
            .map(|(_, state)| state.clone())
    }

    /// `Enter` in "Agregar patrón".
    fn add_pattern(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let row = &ROWS[index];
        let Some((_, input)) = self.pattern_inputs.iter().find(|(row, _)| *row == index) else {
            return;
        };
        let input = input.clone();
        let pattern = input.read(cx).value().trim().to_string();
        if pattern.is_empty() || !self.row_enabled(row, cx) {
            return;
        }
        let mut patterns = self.patterns(row, cx);
        if !patterns.contains(&pattern) {
            patterns.push(pattern);
            self.write(
                SettingsEdit::Set {
                    path: row.path,
                    value: Value::from(patterns),
                },
                cx,
            );
        }
        input.update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// "Quitar" of one pattern.
    fn remove_pattern(&mut self, index: usize, position: usize, cx: &mut Context<Self>) {
        let row = &ROWS[index];
        let mut patterns = self.patterns(row, cx);
        if position >= patterns.len() {
            return;
        }
        patterns.remove(position);
        self.write(
            SettingsEdit::Set {
                path: row.path,
                value: Value::from(patterns),
            },
            cx,
        );
    }

    /// Adds `pattern` to the list row `key`, as typing it and pressing
    /// `Enter` would.
    pub fn add_pattern_to(
        &mut self,
        key: &str,
        pattern: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = ROWS.iter().position(|row| row.key == key) else {
            return;
        };
        if let Some((_, input)) = self.pattern_inputs.iter().find(|(row, _)| *row == index) {
            let input = input.clone();
            input.update(cx, |input, cx| {
                input.set_value(pattern.to_string(), window, cx)
            });
        }
        self.add_pattern(index, window, cx);
    }

    fn patterns(&self, row: &Row, cx: &App) -> Vec<String> {
        let json = settings_json(&crate::settings::settings(cx));
        json_at(&json, row.path)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the control of `row` accepts changes.
    fn row_enabled(&self, row: &Row, cx: &App) -> bool {
        if self.problem.is_some() {
            return false;
        }
        if row.key == "files.autosave_delay_ms" {
            return crate::settings::settings(cx).files.autosave
                == cincel_settings::Autosave::AfterDelay;
        }
        true
    }

    /// Re-reads the settings in force into every control (a write of the
    /// tab or a hand edit, through the watcher).
    fn sync_from_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.problem = file_problem(cx);
        let settings = crate::settings::settings(cx);
        let json = settings_json(&settings);
        for (kind, state) in &self.selects {
            let items = choices(*kind, &settings, &self.font_names, cx);
            let value = selected_value(*kind, &settings);
            state.update(cx, |state, cx| {
                let same = state.selected_value() == Some(&value);
                state.set_items(SearchableVec::new(items), window, cx);
                if !same {
                    state.set_selected_value(&value, window, cx);
                }
            });
        }
        for (index, state) in &self.numbers {
            let Control::Number(spec) = ROWS[*index].control else {
                continue;
            };
            let text = format_number(json_at(&json, ROWS[*index].path), spec);
            let focused = state.read(cx).focus_handle(cx).is_focused(window);
            state.update(cx, |state, cx| {
                if !focused && state.value().as_ref() != text.as_str() {
                    state.set_value(text, window, cx);
                }
            });
        }
        cx.notify();
    }

    /// Emits [`SettingsViewEvent::ConnectionsShown`] the first time the
    /// Connections section is on screen.
    fn announce_connections_if_visible(&mut self, cx: &mut Context<Self>) {
        if self.announced_connections {
            return;
        }
        if self
            .visible_sections(cx)
            .contains(&SettingsSection::Connections)
        {
            self.announced_connections = true;
            cx.emit(SettingsViewEvent::ConnectionsShown);
        }
    }

    fn section_matches(&self, section: SettingsSection, query: &str) -> bool {
        if fold(section.title()).contains(query) {
            return true;
        }
        if section == SettingsSection::Connections {
            return CONNECTIONS_KEYWORDS
                .iter()
                .any(|keyword| fold(keyword).contains(query))
                || self
                    .connections
                    .connections
                    .iter()
                    .any(|connection| fold(&connection.label).contains(query));
        }
        ROWS.iter()
            .any(|row| row.section == section && row_matches(row, query))
    }

    // ---------------------------------------------------------- renaming

    /// "Renombrar" of a connection row: the inline field.
    pub fn start_rename(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(label) = self
            .connections
            .connections
            .iter()
            .find(|connection| connection.id == id.to_string())
            .map(|connection| connection.label.clone())
        else {
            return;
        };
        self.renaming = Some(Renaming { id, error: None });
        self.rename_input
            .update(cx, |input, cx| input.set_value(label, window, cx));
        let handle = self.rename_input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Types `label` in the inline rename field.
    pub fn set_rename_text(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_input.update(cx, |input, cx| {
            input.set_value(label.to_string(), window, cx)
        });
    }

    /// `Enter` in the rename field: an empty name is refused.
    pub fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.as_mut() else {
            return;
        };
        let label = self.rename_input.read(cx).value().trim().to_string();
        if label.is_empty() {
            renaming.error = Some(EMPTY_NAME_ERROR.to_string());
            cx.notify();
            return;
        }
        let id = renaming.id;
        self.renaming = None;
        cx.emit(SettingsViewEvent::Rename { id, label });
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// `Esc` in the rename field.
    pub fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// Emits one of the Connections events, as its button would.
    pub fn request(&mut self, event: SettingsViewEvent, cx: &mut Context<Self>) {
        cx.emit(event);
    }
}

/// The fixed labels of the Connections section.
const CONNECTIONS_LABELS: &[&str] = &[
    "Conectar nuevo agente…",
    "Renombrar",
    "Volver a conectar",
    "Reparar",
    "Eliminar…",
    "Guardar",
    "Cancelar",
    "Agentes instalados",
    "Buscar actualizaciones",
    "Buscando actualizaciones…",
    "Node privado",
    "Reintentar",
    "Ningún agente instalado todavía.",
    "Todavía no hay conexiones.",
];

// -------------------------------------------------------------- helpers

/// The configuration directory in force, if any.
fn config_paths(cx: &App) -> Option<Paths> {
    cx.try_global::<AppSettings>()
        .and_then(|state| state.config.paths.clone())
}

/// What is wrong with `settings.json` as it is on disk, if anything.
fn file_problem(cx: &App) -> Option<FileProblem> {
    let paths = config_paths(cx)?;
    let text = std::fs::read_to_string(&paths.settings).ok()?;
    // An empty path touches nothing: this only parses and checks the root.
    match cincel_settings::apply_edit(&text, &SettingsEdit::Remove { path: &[] }) {
        Err(EditError::Syntax {
            line,
            column,
            message,
        }) => Some(FileProblem::Syntax {
            line,
            column,
            message,
        }),
        Err(EditError::NotAnObject) => Some(FileProblem::NotAnObject),
        Ok(_) | Err(EditError::Io(_)) => None,
    }
}

/// The banner of §4.3.
fn problem_text(problem: &FileProblem) -> String {
    match problem {
        FileProblem::Syntax {
            line,
            column,
            message,
        } => format!(
            "settings.json tiene un error en la línea {line}, columna {column}: {}. \
             Corregilo para poder cambiar ajustes desde acá.",
            spanish_syntax_message(message)
        ),
        FileProblem::NotAnObject => "settings.json no es un objeto JSON. Corregilo para poder \
                                     cambiar ajustes desde acá."
            .to_string(),
    }
}

/// `jsonc-parser` explains its errors in English; the banner speaks Spanish.
fn spanish_syntax_message(message: &str) -> &'static str {
    let message = message.split(" on line ").next().unwrap_or(message);
    const TABLE: &[(&str, &str)] = &[
        ("Comments are not allowed", "no se admiten comentarios"),
        (
            "Expected colon after",
            "falta «:» después del nombre de una clave",
        ),
        (
            "Expected digit following negative sign",
            "falta un dígito después del signo menos",
        ),
        ("Expected digit", "falta un dígito"),
        ("Expected plus, minus, or digit", "número mal escrito"),
        (
            "Expected value after colon",
            "falta el valor después de «:»",
        ),
        (
            "Expected string for object property",
            "falta el nombre de una clave entre comillas",
        ),
        (
            "Hexadecimal numbers are not allowed",
            "no se admiten números hexadecimales",
        ),
        ("Expected comma", "falta una coma"),
        (
            "Text cannot contain more than one JSON value",
            "hay más de un valor en el archivo",
        ),
        (
            "Single-quoted strings are not allowed",
            "las cadenas van entre comillas dobles",
        ),
        ("Trailing commas are not allowed", "sobra una coma al final"),
        ("Unary plus", "no se admite «+» delante de un número"),
        ("Unexpected close brace", "sobra una «}»"),
        ("Unexpected close bracket", "sobra un «]»"),
        ("Unexpected colon", "sobra un «:»"),
        ("Unexpected comma", "sobra una coma"),
        ("Unexpected word", "hay una palabra fuera de lugar"),
        (
            "Unexpected token in object",
            "hay algo fuera de lugar dentro de un objeto",
        ),
        ("Unexpected token", "hay algo fuera de lugar"),
        ("Unterminated array", "falta cerrar una lista con «]»"),
        (
            "Unterminated comment block",
            "falta cerrar un comentario con «*/»",
        ),
        ("Unterminated object", "falta cerrar un objeto con «}»"),
        ("Unterminated string", "falta cerrar una cadena con «\"»"),
        ("Maximum nesting depth", "hay demasiados niveles anidados"),
        ("Invalid escape", "hay un escape inválido en una cadena"),
        ("Expected four hex digits", "hay un escape «\\u» incompleto"),
    ];
    TABLE
        .iter()
        .find(|(english, _)| message.starts_with(english))
        .map(|(_, spanish)| *spanish)
        .unwrap_or("hay un error de sintaxis")
}

/// Lowercase without accents, for a search that ignores both (§4.1).
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|character| match character {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// Whether `row` matches a folded query: its title, description or key.
fn row_matches(row: &Row, query: &str) -> bool {
    query.is_empty()
        || fold(row.title).contains(query)
        || fold(row.description).contains(query)
        || fold(row.key).contains(query)
}

/// The settings as JSON, which gives every row its value by path.
fn settings_json(settings: &Settings) -> Value {
    serde_json::to_value(settings).unwrap_or(Value::Null)
}

/// The value at `path` in `json`.
fn json_at<'a>(json: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(json, |value, segment| value.get(*segment))
}

/// The entries of a select.
fn choices(kind: ChoiceKind, settings: &Settings, font_names: &[String], cx: &App) -> Vec<Choice> {
    match kind {
        ChoiceKind::ThemeMode => vec![
            Choice::new("system", "Seguir al escritorio"),
            Choice::new("dark", "Oscuro"),
            Choice::new("light", "Claro"),
        ],
        ChoiceKind::TextRendering => vec![
            Choice::new("subpixel", "Subpíxel"),
            Choice::new("grayscale", "Escala de grises"),
        ],
        ChoiceKind::Decorations => vec![
            Choice::new("client", "Integrada (Cincel)"),
            Choice::new("server", "Del escritorio"),
        ],
        ChoiceKind::Autosave => vec![
            Choice::new("off", "Apagado"),
            Choice::new("on_focus_change", "Al perder el foco"),
            Choice::new("after_delay", "Tras una pausa"),
        ],
        ChoiceKind::DarkTheme | ChoiceKind::LightTheme => {
            let current = if kind == ChoiceKind::DarkTheme {
                &settings.theme.dark
            } else {
                &settings.theme.light
            };
            let mut names: Vec<String> = cx
                .try_global::<AppSettings>()
                .map(|state| state.config.themes.names().map(str::to_string).collect())
                .unwrap_or_else(|| {
                    vec![
                        cincel_settings::DARK_THEME_NAME.to_string(),
                        cincel_settings::LIGHT_THEME_NAME.to_string(),
                    ]
                });
            names.sort();
            names.dedup();
            let mut items: Vec<Choice> = names
                .iter()
                .map(|name| Choice::new(name.clone(), name.clone()))
                .collect();
            if !names.contains(current) {
                items.insert(
                    0,
                    Choice::new(current.clone(), format!("{current} (no encontrado)")),
                );
            }
            items
        }
        ChoiceKind::UiFont | ChoiceKind::BufferFont => {
            let current = if kind == ChoiceKind::UiFont {
                &settings.ui_font_family
            } else {
                &settings.buffer_font_family
            };
            let mut items: Vec<Choice> = font_names
                .iter()
                .map(|name| Choice::new(name.clone(), name.clone()))
                .collect();
            if !font_names.contains(current) {
                // Spelled with another case, the family still resolves
                // (`Typography::first_installed`); otherwise say so.
                let installed = font_names
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(current));
                let title = if installed {
                    current.clone()
                } else {
                    format!("{current} (no instalada)")
                };
                items.insert(0, Choice::new(current.clone(), title));
            }
            items
        }
    }
}

/// The value of `settings` a select shows.
fn selected_value(kind: ChoiceKind, settings: &Settings) -> SharedString {
    let json = settings_json(settings);
    let path: &[&str] = match kind {
        ChoiceKind::ThemeMode => &["theme", "mode"],
        ChoiceKind::DarkTheme => &["theme", "dark"],
        ChoiceKind::LightTheme => &["theme", "light"],
        ChoiceKind::TextRendering => &["text_rendering"],
        ChoiceKind::Decorations => &["window", "decorations"],
        ChoiceKind::UiFont => &["ui_font_family"],
        ChoiceKind::BufferFont => &["buffer_font_family"],
        ChoiceKind::Autosave => &["files", "autosave"],
    };
    let value = json_at(&json, path)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    SharedString::from(value)
}

/// A number as its field shows it: an integer, or one decimal with a comma.
fn format_value(value: f64, spec: NumberSpec) -> String {
    if spec.decimals == 0 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}").replace('.', ",")
    }
}

/// The value at a path as its field shows it.
fn format_number(value: Option<&Value>, spec: NumberSpec) -> String {
    value
        .and_then(Value::as_f64)
        .map(|value| format_value(value, spec))
        .unwrap_or_default()
}

/// What the user typed, as a number: `1,5` and `1.5` both work, and so do
/// spaces between thousands.
fn parse_number(text: &str, spec: NumberSpec) -> Option<f64> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '\u{a0}')
        .collect();
    let cleaned = if spec.decimals == 0 {
        cleaned.replace(['.', ','], "")
    } else {
        cleaned.replace(',', ".")
    };
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// `value` inside the row's range, at its precision.
fn clamp(value: f64, spec: NumberSpec) -> f64 {
    let value = value.clamp(spec.min, spec.max);
    if spec.decimals == 0 {
        value.round()
    } else {
        (value * 10.).round() / 10.
    }
}

/// The JSON a number row writes: an integer, or a one-decimal number that
/// `serde_json` prints as typed (`1.6`, never `1.6000000000000001`).
fn number_value(value: f64, spec: NumberSpec) -> Value {
    if spec.decimals == 0 {
        Value::from(value.round() as i64)
    } else {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

/// The badge of a connection row.
fn badge_label(badge: &ConnectionBadge) -> &'static str {
    match badge {
        ConnectionBadge::Connected => "Conectada",
        ConnectionBadge::SessionExpired => "Sesión vencida",
        ConnectionBadge::Unavailable { .. } => "No disponible",
    }
}

/// "Claude · adaptador 0.8.2" (§4.4).
fn agent_line(agent: &InstalledAgent) -> String {
    if agent.kind.needs_node() {
        format!(
            "{} · adaptador {}",
            agent.kind.display_name(),
            agent.version
        )
    } else {
        format!("{} · {}", agent.kind.full_name(), agent.version)
    }
}

/// The "Node privado" line.
fn node_line(node: &NodeInfo) -> String {
    node.installed
        .clone()
        .unwrap_or_else(|| NODE_NOT_INSTALLED.to_string())
}

/// Numeric semver comparison (`x.y.z`, a `-prerelease` suffix always sorting
/// below its plain release). Mirrors
/// `cincel_connections::Adapters::update_available`'s own rule: this is a
/// second, independent check kept at the display layer so a row never shows
/// "Actualizar a…" for a version that is not actually newer than what is
/// installed, whatever computed the `update` field it was handed.
fn is_newer_version(candidate: &str, installed: &str) -> bool {
    fn split(version: &str) -> (Vec<u64>, Option<&str>) {
        let version = version.trim().trim_start_matches('v');
        let version = version.split('+').next().unwrap_or(version);
        let (core, prerelease) = match version.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (version, None),
        };
        let parts = core
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect();
        (parts, prerelease)
    }
    let (core_a, pre_a) = split(candidate);
    let (core_b, pre_b) = split(installed);
    let len = core_a.len().max(core_b.len());
    for index in 0..len {
        let x = core_a.get(index).copied().unwrap_or(0);
        let y = core_b.get(index).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    match (pre_a, pre_b) {
        (None, None) => false,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a > b,
    }
}

/// `update`, only when it is numerically newer than `installed` (defence in
/// depth, see [`is_newer_version`]).
fn newer_update<'a>(update: Option<&'a String>, installed: &str) -> Option<&'a String> {
    update.filter(|candidate| is_newer_version(candidate, installed))
}

// --------------------------------------------------------------- render

impl SettingsView {
    fn render_sections(
        &self,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let visible = self.visible_sections(cx);
        let searching = !self.query(cx).trim().is_empty();
        v_flex()
            .id("settings-sections")
            .w(px(SECTIONS_WIDTH * scale))
            .flex_none()
            .h_full()
            .py(px(8. * scale))
            .px(px(6. * scale))
            .gap(px(2. * scale))
            .bg(theme.bg_app)
            .border_r_1()
            .border_color(theme.border)
            .children(SettingsSection::ALL.into_iter().map(|section| {
                let selected = if searching {
                    visible.contains(&section)
                } else {
                    self.section == section
                };
                let hover = theme.bg_surface;
                div()
                    .id(ElementId::Name(
                        format!("settings-section-{}", section.title()).into(),
                    ))
                    .h(px(SECTION_ROW_HEIGHT * scale))
                    .px(px(10. * scale))
                    .flex()
                    .items_center()
                    .rounded(px(4. * scale))
                    .cursor_pointer()
                    .text_size(px(13. * scale))
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .when(selected, |this| this.bg(theme.bg_surface))
                    .hover(move |style| style.bg(hover))
                    .child(section.title())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.set_section(section, window, cx);
                    }))
            }))
    }

    fn render_row(
        &self,
        index: usize,
        json: &Value,
        theme: &ThemeColors,
        scale: f32,
        mono: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let row = &ROWS[index];
        let enabled = self.row_enabled(row, cx);
        let value = json_at(json, row.path);
        let modified = value != json_at(&self.defaults, row.path);

        let texts = v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(2. * scale))
            .child(
                div()
                    .text_size(px(13. * scale))
                    .text_color(theme.text)
                    .child(row.title),
            )
            .child(
                div()
                    .text_size(px(12. * scale))
                    .text_color(theme.text_muted)
                    .child(row.description),
            )
            .child(
                div()
                    .text_size(px(11. * scale))
                    .text_color(theme.text_muted)
                    .when_some(mono, |this, family| this.font_family(family))
                    .child(row.key),
            )
            .children(row.note.map(|note| {
                div()
                    .text_size(px(12. * scale))
                    .text_color(theme.status_warning)
                    .child(note)
            }));

        let reset = div()
            .w(px(28. * scale))
            .flex_none()
            .flex()
            .justify_center()
            .when(modified, |this| {
                this.child(
                    Button::new(("settings-reset", index))
                        .icon(IconName::RotateCcw)
                        .xsmall()
                        .ghost()
                        .tooltip(RESET_LABEL)
                        .disabled(self.problem.is_some())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.reset(ROWS[index].key, cx);
                        })),
                )
            });

        let control = match row.control {
            Control::Switch => {
                let checked = value.and_then(Value::as_bool).unwrap_or(false);
                let path = row.path;
                Switch::new(("settings-switch", index))
                    .checked(checked)
                    .disabled(!enabled)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.write(
                            SettingsEdit::Set {
                                path,
                                value: Value::Bool(*checked),
                            },
                            cx,
                        );
                    }))
                    .into_any_element()
            }
            Control::Choice(kind) => match self.selects.iter().find(|(k, _)| *k == kind) {
                Some((_, state)) => Select::new(state)
                    .id(("settings-select", index))
                    .small()
                    .w(px(SELECT_WIDTH * scale))
                    .placeholder(SELECT_PLACEHOLDER)
                    .search_placeholder(SELECT_SEARCH_PLACEHOLDER)
                    .empty(|_, _| div().p_2().child(SELECT_EMPTY))
                    .disabled(!enabled)
                    .into_any_element(),
                None => div().into_any_element(),
            },
            Control::Number(_) => match self.number_state(index) {
                Some(state) => div()
                    .w(px(NUMBER_WIDTH * scale))
                    .child(NumberInput::new(&state).small().disabled(!enabled))
                    .into_any_element(),
                None => div().into_any_element(),
            },
            Control::Patterns => div().into_any_element(),
        };

        let header = h_flex()
            .w_full()
            .gap(px(12. * scale))
            .items_center()
            .child(texts)
            .child(reset)
            .child(div().flex_none().child(control));

        let body = if row.control == Control::Patterns {
            let patterns: Vec<String> = value
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let input = self
                .pattern_inputs
                .iter()
                .find(|(row, _)| *row == index)
                .map(|(_, state)| state.clone());
            Some(
                v_flex()
                    .w_full()
                    .gap(px(4. * scale))
                    .pl(px(8. * scale))
                    .children(patterns.into_iter().enumerate().map(|(position, pattern)| {
                        h_flex()
                            .gap(px(8. * scale))
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12. * scale))
                                    .text_color(theme.text)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(pattern),
                            )
                            .child(
                                Button::new(ElementId::NamedInteger(
                                    format!("settings-pattern-remove-{index}").into(),
                                    position as u64,
                                ))
                                .label(REMOVE_PATTERN_LABEL)
                                .xsmall()
                                .outline()
                                .disabled(!enabled)
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, _, cx| {
                                        this.remove_pattern(index, position, cx);
                                    },
                                )),
                            )
                    }))
                    .children(input.map(|state| {
                        div()
                            .w(px(SELECT_WIDTH * scale))
                            .child(Input::new(&state).small().disabled(!enabled))
                    })),
            )
        } else {
            None
        };

        v_flex()
            .id(("settings-row", index))
            .w_full()
            .gap(px(8. * scale))
            .py(px(10. * scale))
            .border_b_1()
            .border_color(theme.border)
            .child(header)
            .children(body)
            .into_any_element()
    }

    fn render_connections(
        &self,
        theme: &ThemeColors,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let info = &self.connections;
        let small = |text: String, color| {
            div()
                .text_size(px(12. * scale))
                .text_color(color)
                .child(SharedString::from(text))
        };

        let mut list = v_flex().w_full().gap(px(2. * scale));
        if info.connections.is_empty() {
            list = list.child(small(
                "Todavía no hay conexiones.".to_string(),
                theme.text_muted,
            ));
        }
        for (position, connection) in info.connections.iter().enumerate() {
            let Ok(id) = Uuid::parse_str(&connection.id) else {
                continue;
            };
            let renaming = self.renaming.as_ref().filter(|renaming| renaming.id == id);
            let (badge_color, reason) = match &connection.badge {
                ConnectionBadge::Connected => (theme.status_ok, None),
                ConnectionBadge::SessionExpired => (theme.status_warning, None),
                ConnectionBadge::Unavailable { reason } => {
                    (theme.status_error, Some(SharedString::from(reason.clone())))
                }
            };
            let badge = div()
                .id(("settings-connection-badge", position))
                .px(px(6. * scale))
                .rounded(px(4. * scale))
                .bg(theme.bg_surface)
                .text_size(px(11. * scale))
                .text_color(badge_color)
                .child(badge_label(&connection.badge))
                .when_some(reason, |this, reason| {
                    this.tooltip(move |window, cx| Tooltip::new(reason.clone()).build(window, cx))
                });

            let details = if let Some(renaming) = renaming {
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2. * scale))
                    .on_action(cx.listener(|this, _: &Escape, window, cx| {
                        this.cancel_rename(window, cx);
                    }))
                    .child(Input::new(&self.rename_input).small())
                    .children(
                        renaming
                            .error
                            .clone()
                            .map(|error| small(error, theme.status_error)),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(13. * scale))
                            .text_color(theme.text)
                            .child(SharedString::from(connection.label.clone())),
                    )
                    .children(
                        connection
                            .identity
                            .clone()
                            .map(|identity| small(identity, theme.text_muted)),
                    )
                    .child(
                        div()
                            .text_size(px(11. * scale))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(connection.last_used.clone())),
                    )
                    .into_any_element()
            };

            let mut actions = h_flex().gap(px(6. * scale)).flex_none();
            if renaming.is_some() {
                actions = actions
                    .child(
                        Button::new(("settings-rename-save", position))
                            .label("Guardar")
                            .xsmall()
                            .outline()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.save_rename(window, cx)
                            })),
                    )
                    .child(
                        Button::new(("settings-rename-cancel", position))
                            .label("Cancelar")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_rename(window, cx)
                            })),
                    );
            } else {
                actions = actions.child(
                    Button::new(("settings-rename", position))
                        .label("Renombrar")
                        .xsmall()
                        .outline()
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.start_rename(id, window, cx)
                        })),
                );
                match connection.badge {
                    ConnectionBadge::SessionExpired => {
                        actions = actions.child(
                            Button::new(("settings-reconnect", position))
                                .label("Volver a conectar")
                                .xsmall()
                                .outline()
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(SettingsViewEvent::Reconnect { id })
                                })),
                        );
                    }
                    ConnectionBadge::Unavailable { .. } => {
                        actions = actions.child(
                            Button::new(("settings-repair", position))
                                .label("Reparar")
                                .xsmall()
                                .outline()
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(SettingsViewEvent::Repair { id })
                                })),
                        );
                    }
                    ConnectionBadge::Connected => {}
                }
                actions = actions.child(
                    Button::new(("settings-delete", position))
                        .label("Eliminar…")
                        .xsmall()
                        .outline()
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(SettingsViewEvent::Delete { id })
                        })),
                );
            }

            list = list.child(
                h_flex()
                    .w_full()
                    .gap(px(10. * scale))
                    .py(px(8. * scale))
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(cincel_chat::provider_icon(
                        &connection.agent_id,
                        px(20. * scale),
                        theme.text,
                        theme.bg_surface,
                        theme.border,
                    ))
                    .child(details)
                    .child(badge)
                    .child(actions),
            );
        }

        let busy = info.operation.is_some();
        let offline = info.catalog == CatalogState::Offline;
        let catalog_line = match info.catalog {
            CatalogState::Checking => Some(small(
                "Buscando actualizaciones…".to_string(),
                theme.text_muted,
            )),
            CatalogState::Offline => Some(small(OFFLINE_CATALOG.to_string(), theme.text_muted)),
            CatalogState::NotChecked | CatalogState::Checked => None,
        };

        let update_area = |target: UpdateTarget,
                           update: Option<&String>,
                           outcome: Option<&UpdateOutcome>,
                           position: usize,
                           cx: &mut Context<Self>|
         -> gpui::AnyElement {
            if let Some(operation) = info.operation.as_ref().filter(|op| op.target == target) {
                // §10.4 fix: this replaces "Actualizar a…" in the exact same
                // row slot, so the text must never push past its own
                // container's right edge (that used to overlap the row
                // above and clip "Cancelar" down to "Cancela"). `min_w_0` on
                // the flexed text lets it actually shrink instead of
                // keeping its unwrapped width as a hard floor, `.truncate()`
                // then ellipsizes it, and the cancel button is wrapped in
                // its own `flex_none` so it never shares that shrinkage. At
                // an extreme narrow width `flex_wrap` moves the text under
                // the row instead of overlapping it.
                return v_flex()
                    .w(px(260. * scale))
                    .min_w_0()
                    .gap(px(4. * scale))
                    .debug_selector(move || format!("settings-update-progress-area-{position}"))
                    .child(
                        Progress::new(("settings-update-progress", position))
                            .loading(operation.percent.is_none())
                            .value(operation.percent.unwrap_or(0.)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .flex_wrap()
                            .gap(px(8. * scale))
                            .items_center()
                            .debug_selector(move || {
                                format!("settings-update-progress-row-{position}")
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    // `.truncate()` already covers
                                    // `overflow_hidden` + `whitespace_nowrap`
                                    // + the ellipsis itself.
                                    .truncate()
                                    .text_size(px(12. * scale))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(operation.text.clone())),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .debug_selector(move || {
                                        format!("settings-update-cancel-{position}")
                                    })
                                    .child(
                                        Button::new(("settings-update-cancel", position))
                                            .label("Cancelar")
                                            .xsmall()
                                            .outline()
                                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                                cx.emit(SettingsViewEvent::CancelUpdate)
                                            })),
                                    ),
                            ),
                    )
                    .into_any_element();
            }
            let event = match target {
                UpdateTarget::Adapter(kind) => SettingsViewEvent::UpdateAdapter(kind),
                UpdateTarget::Node => SettingsViewEvent::UpdateNode,
            };
            let mut area = h_flex().gap(px(8. * scale)).items_center();
            match outcome {
                Some(UpdateOutcome::Updated(message)) => {
                    area = area.child(small(message.clone(), theme.status_ok));
                }
                Some(UpdateOutcome::Failed(message)) => {
                    let retry = event.clone();
                    area = area
                        .child(small(message.clone(), theme.status_error))
                        .child(
                            Button::new(("settings-update-retry", position))
                                .label("Reintentar")
                                .xsmall()
                                .outline()
                                .disabled(busy)
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(retry.clone())
                                })),
                        );
                }
                None => {}
            }
            if let Some(version) = update.filter(|_| !offline) {
                area = area.child(
                    Button::new(("settings-update", position))
                        .label(SharedString::from(format!("Actualizar a {version}")))
                        .xsmall()
                        .outline()
                        .disabled(busy)
                        .debug_selector(move || format!("settings-update-{position}"))
                        .on_click(
                            cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(event.clone())),
                        ),
                );
            }
            area.into_any_element()
        };

        let mut agents = v_flex().w_full().gap(px(2. * scale));
        if info.agents.is_empty() {
            agents = agents.child(small(
                "Ningún agente instalado todavía.".to_string(),
                theme.text_muted,
            ));
        }
        for (position, agent) in info.agents.iter().enumerate() {
            agents = agents.child(
                h_flex()
                    .w_full()
                    .gap(px(10. * scale))
                    .py(px(6. * scale))
                    .items_center()
                    .child(cincel_chat::provider_icon(
                        agent.kind.agent_id(),
                        px(18. * scale),
                        theme.text,
                        theme.bg_surface,
                        theme.border,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13. * scale))
                            .text_color(theme.text)
                            .child(SharedString::from(agent_line(agent))),
                    )
                    .child(update_area(
                        UpdateTarget::Adapter(agent.kind),
                        newer_update(agent.update.as_ref(), &agent.version),
                        agent.outcome.as_ref(),
                        position,
                        cx,
                    )),
            );
        }

        let node = h_flex()
            .w_full()
            .gap(px(10. * scale))
            .py(px(6. * scale))
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(13. * scale))
                            .text_color(theme.text)
                            .child("Node privado"),
                    )
                    .child(small(node_line(&info.node), theme.text_muted)),
            )
            .child(update_area(
                UpdateTarget::Node,
                info.node
                    .installed
                    .as_ref()
                    .and_then(|installed| newer_update(info.node.update.as_ref(), installed)),
                info.node.outcome.as_ref(),
                usize::MAX,
                cx,
            ));

        let subtitle = |text: &'static str| {
            div()
                .pt(px(12. * scale))
                .text_size(px(13. * scale))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(text)
        };

        v_flex()
            .w_full()
            .gap(px(6. * scale))
            .child(
                h_flex().child(
                    Button::new("settings-new-connection")
                        .label("Conectar nuevo agente…")
                        .small()
                        .outline()
                        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                            cx.emit(SettingsViewEvent::NewConnection)
                        })),
                ),
            )
            .child(list)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(subtitle("Agentes instalados"))
                    .child(
                        Button::new("settings-check-updates")
                            .label("Buscar actualizaciones")
                            .xsmall()
                            .ghost()
                            .disabled(busy || info.catalog == CatalogState::Checking)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(SettingsViewEvent::CheckUpdates)
                            })),
                    ),
            )
            .children(catalog_line)
            .child(agents)
            .child(node)
            .child(
                h_flex()
                    .w_full()
                    .pt(px(12. * scale))
                    .gap(px(8. * scale))
                    .items_center()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12. * scale))
                            .text_color(theme.text_muted)
                            .child(CONNECTIONS_FOOTER),
                    )
                    .child(
                        Button::new("settings-connections-open-file")
                            .label(OPEN_FILE_LABEL)
                            .xsmall()
                            .outline()
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(SettingsViewEvent::OpenSettingsFile)
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        // Every pixel size follows the zoom (`workspace::zoom_*`).
        let scale = crate::settings::ui_scale(cx);
        let settings = crate::settings::settings(cx);
        let json = settings_json(&settings);
        let mono = Typography::buffer_font_family(&settings, cx);
        let query_text = self.query(cx);
        let query = fold(query_text.trim());
        let sections = self.visible_sections(cx);

        let mut content = v_flex().w_full().gap(px(18. * scale));
        for section in &sections {
            let mut block = v_flex().w_full().child(
                div()
                    .pb(px(4. * scale))
                    .text_size(px(15. * scale))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(section.title()),
            );
            if *section == SettingsSection::Connections {
                block = block.child(self.render_connections(&theme, scale, cx));
            } else {
                for (index, row) in ROWS.iter().enumerate() {
                    if row.section == *section && row_matches(row, &query) {
                        block = block.child(self.render_row(
                            index,
                            &json,
                            &theme,
                            scale,
                            mono.clone(),
                            cx,
                        ));
                    }
                }
            }
            content = content.child(block);
        }
        if sections.is_empty() {
            content = content.child(
                div()
                    .text_size(px(13. * scale))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!(
                        "Ningún ajuste coincide con «{}»",
                        query_text.trim()
                    ))),
            );
        }

        let banner = self.banner().map(|text| {
            h_flex()
                .w_full()
                .gap(px(10. * scale))
                .p(px(10. * scale))
                .items_center()
                .rounded(px(4. * scale))
                .border_1()
                .border_color(theme.status_warning)
                .bg(theme.status_warning.alpha(0.12))
                .child(
                    div()
                        .text_color(theme.status_warning)
                        .child(IconName::TriangleAlert),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12. * scale))
                        .text_color(theme.text)
                        .child(SharedString::from(text)),
                )
                .child(
                    Button::new("settings-banner-open-file")
                        .label(OPEN_FILE_LABEL)
                        .xsmall()
                        .outline()
                        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                            cx.emit(SettingsViewEvent::OpenSettingsFile)
                        })),
                )
        });

        h_flex()
            .id("settings-view")
            .debug_selector(|| "settings-view".to_string())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .bg(theme.bg_editor)
            .text_color(theme.text)
            .child(self.render_sections(&theme, scale, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        h_flex()
                            .w_full()
                            .flex_none()
                            .gap(px(12. * scale))
                            .px(px(24. * scale))
                            .py(px(12. * scale))
                            .items_center()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .flex_1()
                                    .max_w(px(SEARCH_MAX_WIDTH * scale))
                                    .child(Input::new(&self.search).small()),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("settings-open-file")
                                    .label(OPEN_FILE_LABEL)
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                        cx.emit(SettingsViewEvent::OpenSettingsFile)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(px(24. * scale))
                            .py(px(16. * scale))
                            .child(
                                v_flex()
                                    .w_full()
                                    .max_w(px(820. * scale))
                                    .gap(px(12. * scale))
                                    .children(banner)
                                    .child(content),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_path_spells_its_key() {
        for row in ROWS {
            assert_eq!(row.path.join("."), row.key);
        }
    }

    #[test]
    fn every_row_key_exists_in_the_default_settings() {
        let json = settings_json(&Settings::default());
        for row in ROWS {
            assert!(json_at(&json, row.path).is_some(), "{}", row.key);
        }
    }

    #[test]
    fn numbers_parse_with_comma_or_dot_and_are_clamped() {
        let height = NumberSpec {
            min: 1.,
            max: 4.,
            step: 0.1,
            decimals: 1,
        };
        assert_eq!(parse_number("1,6", height), Some(1.6));
        assert_eq!(parse_number(" 1.6 ", height), Some(1.6));
        assert_eq!(parse_number("uno", height), None);
        assert_eq!(clamp(9., height), 4.);
        assert_eq!(format_value(1.5, height), "1,5");
        assert_eq!(number_value(1.6, height).to_string(), "1.6");

        let delay = NumberSpec {
            min: 100.,
            max: 60_000.,
            step: 100.,
            decimals: 0,
        };
        assert_eq!(parse_number("60 000", delay), Some(60_000.));
        assert_eq!(clamp(10., delay), 100.);
        assert_eq!(number_value(1000., delay).to_string(), "1000");
    }

    #[test]
    fn the_search_ignores_case_and_accents() {
        assert_eq!(fold("Revisión ÁRBOL"), "revision arbol");
        let row = ROWS
            .iter()
            .find(|row| row.key == "editor.soft_wrap")
            .unwrap();
        assert!(row_matches(row, &fold("LINEAS largas")));
        assert!(row_matches(row, &fold("soft_wrap")));
        assert!(!row_matches(row, &fold("autoguardado")));
    }

    #[test]
    fn syntax_errors_are_explained_in_spanish() {
        assert_eq!(
            spanish_syntax_message("Expected comma on line 3 column 4"),
            "falta una coma"
        );
        assert_eq!(
            spanish_syntax_message("Unexpected token on line 1 column 18"),
            "hay algo fuera de lugar"
        );
        assert_eq!(spanish_syntax_message("¿?"), "hay un error de sintaxis");
    }

    #[test]
    fn agent_lines_follow_the_spec() {
        let claude = InstalledAgent {
            kind: AgentKind::Claude,
            version: "0.8.2".to_string(),
            update: None,
            outcome: None,
        };
        assert_eq!(agent_line(&claude), "Claude · adaptador 0.8.2");
        assert_eq!(node_line(&NodeInfo::default()), NODE_NOT_INSTALLED);
    }
}
