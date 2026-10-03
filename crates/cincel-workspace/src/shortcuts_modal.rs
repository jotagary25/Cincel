//! The keyboard shortcuts modal (`F1`, the status bar button and the menu's
//! "Atajos de teclado"; `docs/specs/07-etapa5-productividad.md` §5).
//!
//! [`ShortcutsModal`] shows every shortcut of the running keymap, grouped by
//! [`ShortcutCategory`] and searchable. Its data is never cached: every
//! render reads the keymap in force
//! ([`crate::settings::keymap`]) fresh, merges it with the widgets' own
//! Rust-side defaults ([`cincel_editor::default_key_bindings`],
//! [`cincel_chat::default_key_bindings`], [`crate::keymap::built_in_bindings`])
//! and recomputes the rows, so a `keymap.json` reload (which calls
//! `cx.refresh_windows()`, `crate::settings::reload`) shows up the next time
//! the window paints without any extra plumbing here — the same trick
//! `crate::file_finder::FileFinder` relies on for its own floating panel.
//!
//! The merge itself reuses [`cincel_settings::Keymap::effective_bindings`]
//! rather than re-implementing layering: the widget defaults become a
//! bottom [`KeymapSection`] layer, the keymap in force (already the built-in
//! JSONC layered with the user's file) goes on top, and
//! [`build_rows`] turns the resulting per-(keystroke, context) rows into
//! per-command display rows (a command with more than one active key, like
//! `workspace::next_change`, shows every key as its own chip).

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, MouseButton, MouseDownEvent,
    Subscription, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, SharedString, div, px};

use cincel_settings::{
    ContextExpr, KeyBinding as SettingsKeyBinding, Keymap, KeymapOrigin, KeymapSection, Keystroke,
};

use crate::center::CenterPanel;
use crate::theme::ThemeColors;
use crate::workspace::Workspace;

/// The GPUI key context of the floating modal; `crate::keymap` binds `Esc`
/// against it (`built_in_bindings`, like the file finder's own navigation).
pub const KEY_CONTEXT: &str = "ShortcutsModal";

/// §5.2 geometry.
const MAX_WIDTH: f32 = 640.;
const WINDOW_MARGIN: f32 = 48.;
const HEIGHT_FRACTION: f32 = 0.7;
const HEADER_HEIGHT: f32 = 32.;

/// `shortcuts::dismiss` (`Esc`): closes the modal without changing anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = shortcuts, name = "dismiss")]
pub struct Dismiss;

/// The five groups of §5.1, in the order the modal lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutCategory {
    /// Everything that is not one of the other four: window, panels, focus,
    /// zoom, the file finder, the modal itself.
    General,
    /// `editor::*`.
    Editor,
    /// `review::*` plus the workspace-level review navigation commands.
    Review,
    /// `chat::*` plus the workspace commands that act on the chat panel.
    Chat,
    /// The connections section of the settings tab.
    Connections,
}

impl ShortcutCategory {
    /// Display order of §5.1: "Generales, Editor, Revisión, Chat,
    /// Conexiones".
    pub const ORDER: [ShortcutCategory; 5] = [
        ShortcutCategory::General,
        ShortcutCategory::Editor,
        ShortcutCategory::Review,
        ShortcutCategory::Chat,
        ShortcutCategory::Connections,
    ];

    /// The Spanish heading shown above the group.
    pub fn label(self) -> &'static str {
        match self {
            ShortcutCategory::General => "Generales",
            ShortcutCategory::Editor => "Editor",
            ShortcutCategory::Review => "Revisión",
            ShortcutCategory::Chat => "Chat",
            ShortcutCategory::Connections => "Conexiones",
        }
    }
}

/// `docs/specs/07-etapa5-productividad.md` §5.2: which category a command
/// name belongs to, by an explicit table (not every `workspace::*` command is
/// "Generales").
pub fn category(command: &str) -> ShortcutCategory {
    const REVIEW_WORKSPACE_COMMANDS: &[&str] = &[
        "workspace::next_change",
        "workspace::prev_change",
        "workspace::next_file_with_changes",
        "workspace::accept_turn",
        "workspace::reject_turn",
        "workspace::undo_last_reject",
        "workspace::open_review_panel",
    ];
    const CHAT_WORKSPACE_COMMANDS: &[&str] = &[
        "workspace::toggle_chat",
        "workspace::focus_chat",
        "workspace::mention_in_chat",
    ];

    if command.starts_with("review::") || REVIEW_WORKSPACE_COMMANDS.contains(&command) {
        ShortcutCategory::Review
    } else if command.starts_with("chat::") || CHAT_WORKSPACE_COMMANDS.contains(&command) {
        ShortcutCategory::Chat
    } else if command == "workspace::open_connections" {
        ShortcutCategory::Connections
    } else if command.starts_with("editor::") {
        ShortcutCategory::Editor
    } else {
        ShortcutCategory::General
    }
}

/// The Spanish description of `command`, for the modal's rows. `None` means
/// no crate documented it yet, in which case the modal falls back to the raw
/// command name (§5.1) — and
/// [`crate::shortcuts_modal::tests::every_default_command_has_a_spanish_description`]
/// (in `shortcuts_modal_tests.rs`) makes sure that never happens for a
/// command the default keymap or the widgets actually bind.
pub fn description(command: &str) -> Option<&'static str> {
    Some(match command {
        // workspace::*
        "workspace::open_folder" => "Abrir una carpeta como proyecto",
        "workspace::toggle_chat" => "Mostrar u ocultar el chat",
        "workspace::toggle_tree" => "Mostrar u ocultar los archivos",
        "workspace::focus_chat" => "Llevar el foco al chat",
        "workspace::focus_next_zone" => "Saltar a la próxima zona de foco",
        "workspace::toggle_file_finder" => "Buscar archivos",
        "workspace::open_settings" => "Abrir la configuración",
        "workspace::show_shortcuts" => "Ver los atajos de teclado",
        "workspace::new_file" => "Crear un archivo nuevo",
        "workspace::quit" => "Salir de Cincel",
        "workspace::open_connections" => "Abrir la configuración en Conexiones",
        "workspace::close_tab" => "Cerrar la pestaña activa",
        "workspace::toggle_markdown_preview" => "Alternar la vista previa de Markdown",
        "workspace::zoom_in" => "Agrandar la interfaz",
        "workspace::zoom_out" => "Achicar la interfaz",
        "workspace::zoom_reset" => "Restablecer el zoom de la interfaz",
        "workspace::next_change" => "Ir al próximo cambio pendiente",
        "workspace::prev_change" => "Ir al cambio pendiente anterior",
        "workspace::next_file_with_changes" => "Ir al próximo archivo con cambios pendientes",
        "workspace::accept_turn" => "Aceptar todos los cambios del último turno",
        "workspace::reject_turn" => "Rechazar todos los cambios del último turno",
        "workspace::undo_last_reject" => "Deshacer el último rechazo",
        "workspace::open_review_panel" => "Abrir o cerrar el panel de revisión",
        "workspace::open_selected" => "Abrir el archivo seleccionado en el árbol",
        "workspace::reveal_in_folder" => "Revelar en el administrador de archivos",
        "workspace::copy_path" => "Copiar la ruta absoluta",
        "workspace::copy_relative_path" => "Copiar la ruta relativa al proyecto",
        "workspace::mention_in_chat" => "Mencionar el archivo en el chat",
        "workspace::confirm_close" => "Confirmar el diálogo de cambios sin guardar",
        "workspace::cancel_close" => "Cancelar el diálogo de cambios sin guardar",
        "workspace::close_image_viewer" => "Cerrar la imagen abierta",

        // editor::*
        "editor::move_left" => "Mover el cursor un carácter a la izquierda",
        "editor::move_right" => "Mover el cursor un carácter a la derecha",
        "editor::move_up" => "Mover el cursor una fila arriba",
        "editor::move_down" => "Mover el cursor una fila abajo",
        "editor::move_word_left" => "Mover el cursor a la palabra anterior",
        "editor::move_word_right" => "Mover el cursor a la palabra siguiente",
        "editor::move_to_line_start" => "Ir al inicio de la línea",
        "editor::move_to_line_end" => "Ir al final de la línea",
        "editor::move_page_up" => "Subir una pantalla",
        "editor::move_page_down" => "Bajar una pantalla",
        "editor::move_to_document_start" => "Ir al inicio del documento",
        "editor::move_to_document_end" => "Ir al final del documento",
        "editor::select_left" => "Extender la selección un carácter a la izquierda",
        "editor::select_right" => "Extender la selección un carácter a la derecha",
        "editor::select_up" => "Extender la selección una fila arriba",
        "editor::select_down" => "Extender la selección una fila abajo",
        "editor::select_word_left" => "Extender la selección a la palabra anterior",
        "editor::select_word_right" => "Extender la selección a la palabra siguiente",
        "editor::select_to_line_start" => "Extender la selección al inicio de la línea",
        "editor::select_to_line_end" => "Extender la selección al final de la línea",
        "editor::select_page_up" => "Extender la selección una pantalla arriba",
        "editor::select_page_down" => "Extender la selección una pantalla abajo",
        "editor::select_to_document_start" => "Extender la selección al inicio del documento",
        "editor::select_to_document_end" => "Extender la selección al final del documento",
        "editor::select_all" => "Seleccionar todo",
        "editor::backspace" => "Borrar el carácter anterior",
        "editor::delete" => "Borrar el carácter siguiente",
        "editor::delete_word_left" => "Borrar la palabra anterior",
        "editor::delete_word_right" => "Borrar la palabra siguiente",
        "editor::insert_newline" => "Insertar un salto de línea",
        "editor::tab" => "Indentar o insertar una tabulación",
        "editor::backtab" => "Quitar un nivel de indentación",
        "editor::undo" => "Deshacer",
        "editor::redo" => "Rehacer",
        "editor::copy" => "Copiar la selección",
        "editor::cut" => "Cortar la selección",
        "editor::paste" => "Pegar el portapapeles",
        "editor::find" => "Buscar en el archivo",
        "editor::find_next" => "Ir a la coincidencia siguiente",
        "editor::find_prev" => "Ir a la coincidencia anterior",
        "editor::find_replace" => "Buscar y reemplazar en el archivo",
        "editor::dismiss_find" => "Cerrar la búsqueda",
        "editor::replace_next" => "Reemplazar la coincidencia actual",
        "editor::replace_all" => "Reemplazar todas las coincidencias",
        "editor::search_next_field" => "Mover el foco al otro campo del buscador",
        "editor::search_prev_field" => "Mover el foco al otro campo del buscador (hacia atrás)",
        "editor::toggle_search_regex" => "Alternar expresiones regulares en la búsqueda",
        "editor::toggle_search_case" => "Alternar mayúsculas y minúsculas en la búsqueda",
        "editor::go_to_line" => "Ir a una línea",
        "editor::toggle_soft_wrap" => "Alternar el ajuste de línea",
        "editor::toggle_whitespace" => "Mostrar u ocultar los espacios en blanco",
        "editor::scroll_line_up" => "Desplazar una línea hacia arriba",
        "editor::scroll_line_down" => "Desplazar una línea hacia abajo",
        "editor::save" => "Guardar el archivo",
        "editor::save_all" => "Guardar todos los archivos",
        "editor::cancel" => "Cerrar la búsqueda o el diálogo abierto",
        "editor::confirm" => "Confirmar el diálogo abierto",
        "editor::move_line_up" => "Mover las líneas seleccionadas hacia arriba",
        "editor::move_line_down" => "Mover las líneas seleccionadas hacia abajo",
        "editor::duplicate_line_down" => "Duplicar las líneas seleccionadas debajo",
        "editor::duplicate_line_up" => "Duplicar las líneas seleccionadas arriba",
        "editor::delete_line" => "Borrar las líneas seleccionadas",
        "editor::toggle_comments" => "Comentar o descomentar las líneas seleccionadas",
        "editor::join_lines" => "Unir la línea con la siguiente",
        "editor::select_line" => "Seleccionar la línea del cursor",
        "editor::select_next" => {
            "Seleccionar la palabra bajo el cursor o la siguiente coincidencia"
        }
        "editor::move_to_matching_bracket" => "Ir al paréntesis o llave que corresponde",
        "editor::uppercase" => "Pasar la selección a mayúsculas",
        "editor::lowercase" => "Pasar la selección a minúsculas",
        "editor::sort_lines" => "Ordenar las líneas seleccionadas",
        "editor::comment_selection" => "Comentar la selección",
        "editor::save_comment" => "Guardar el comentario",
        "editor::cancel_comment" => "Cancelar el comentario",

        // review::*
        "review::accept_hunk" => "Aceptar el segmento bajo el cursor",
        "review::reject_hunk" => "Rechazar el segmento bajo el cursor",
        "review::accept_line" => "Aceptar la línea bajo el cursor",
        "review::reject_line" => "Rechazar la línea bajo el cursor",
        "review::accept_file" => "Aceptar todos los cambios del archivo",
        "review::reject_file" => "Rechazar todos los cambios del archivo",
        "review::next_hunk" => "Ir al segmento pendiente siguiente",
        "review::prev_hunk" => "Ir al segmento pendiente anterior",
        "review::next_file" => "Ir al próximo archivo con cambios pendientes",
        "review::open_review_panel" => "Abrir o cerrar el panel de revisión",
        "review::undo_last_reject" => "Deshacer el último rechazo",
        "review::comment_hunk" => "Comentar el segmento",

        // chat::*
        "chat::send" => "Enviar el mensaje",
        "chat::newline" => "Insertar un salto de línea en el mensaje",
        "chat::cancel_turn" => "Cancelar el turno en curso",
        "chat::focus_input" => "Llevar el foco al compositor del chat",
        "chat::new_session" => "Iniciar una sesión nueva",
        "chat::accept_permission" => "Aceptar el permiso pendiente con la primera opción",
        "chat::reject_permission" => "Rechazar el permiso pendiente",
        "chat::close_popover" => "Cerrar el menú emergente abierto",
        "chat::popover_next" => "Mover la selección hacia abajo en el menú emergente",
        "chat::popover_prev" => "Mover la selección hacia arriba en el menú emergente",
        "chat::popover_confirm" => "Confirmar la fila seleccionada del menú emergente",

        // file_finder::*
        "file_finder::select_next" => "Mover la selección hacia abajo",
        "file_finder::select_prev" => "Mover la selección hacia arriba",
        "file_finder::page_down" => "Bajar diez filas",
        "file_finder::page_up" => "Subir diez filas",
        "file_finder::confirm" => "Abrir el archivo seleccionado",
        "file_finder::dismiss" => "Cerrar el buscador de archivos",

        // shortcuts::*
        "shortcuts::dismiss" => "Cerrar el modal de atajos",

        _ => return None,
    })
}

/// A human label for a context expression (§5.1: "en el editor", "buscando",
/// …). Falls back to the raw expression text for one this table does not
/// know, so a future context never renders blank.
pub fn context_label(context: &ContextExpr) -> String {
    let text = context.to_string();
    match text.as_str() {
        "" => "global".to_string(),
        "Editor" => "en el editor".to_string(),
        "Editor && searching" => "buscando".to_string(),
        "Editor && searching && replacing" => "reemplazando".to_string(),
        "Editor && review_hunk_under_cursor" => "con un segmento bajo el cursor".to_string(),
        "Center" => "en el área de pestañas".to_string(),
        "Chat" => "en el chat".to_string(),
        "Chat && permission" => "con un permiso pendiente".to_string(),
        "Tree" => "en el árbol de archivos".to_string(),
        "Workspace" => "en la ventana".to_string(),
        "FileFinder" => "buscando archivos".to_string(),
        "ShortcutsModal" => "en el modal de atajos".to_string(),
        "ImageViewer" => "con una imagen abierta".to_string(),
        "SettingsPage" => "en la configuración".to_string(),
        other => other.to_string(),
    }
}

/// §5.2: `ctrl-shift-a` → `Ctrl+Shift+A`, with the Spanish key names of the
/// spec (`Esc`, `Supr`, `RePág`/`AvPág`, the arrows as glyphs).
pub fn format_keystroke(keystroke: &Keystroke) -> String {
    let mut modifiers = Vec::new();
    if keystroke.ctrl {
        modifiers.push("Ctrl");
    }
    if keystroke.alt {
        modifiers.push("Alt");
    }
    if keystroke.shift {
        modifiers.push("Shift");
    }
    if keystroke.cmd {
        modifiers.push("Cmd");
    }
    let mut out = modifiers.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&format_key(&keystroke.key));
    out
}

fn format_key(key: &str) -> String {
    match key {
        "enter" => "Enter".to_string(),
        "escape" => "Esc".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Supr".to_string(),
        "tab" => "Tab".to_string(),
        "space" => "Espacio".to_string(),
        "home" => "Inicio".to_string(),
        "end" => "Fin".to_string(),
        "insert" => "Insert".to_string(),
        "pageup" => "RePág".to_string(),
        "pagedown" => "AvPág".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        other => {
            if let Some(digits) = other.strip_prefix('f')
                && !digits.is_empty()
                && digits.chars().all(|c| c.is_ascii_digit())
            {
                return format!("F{digits}");
            }
            if other.chars().count() == 1 {
                other.to_uppercase()
            } else {
                capitalize(other)
            }
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Case- and accent-insensitive normalization for the search field (§5.1).
fn normalize(text: &str) -> String {
    text.chars()
        .map(strip_accent)
        .collect::<String>()
        .to_lowercase()
}

fn strip_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
        'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
        'ñ' | 'Ñ' => 'n',
        other => other,
    }
}

/// One display row of the modal: every active keystroke bound to `identity`
/// under one context, plus what the user's `keymap.json` changed about it
/// (§5.1, §5.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutRow {
    /// The command name the row is about (the winning one when the user
    /// rebound the keystroke, the disabled one when they set it to `null`).
    pub identity: String,
    /// [`description`] of `identity`, or `identity` itself when the table has
    /// none (§5.1: "todo comando sin descripción aparece con su nombre
    /// técnico").
    pub description: String,
    pub category: ShortcutCategory,
    /// The human label of the context every key in this row shares.
    pub context_label: String,
    /// Formatted, active keys (`Ctrl+Shift+A`, …).
    pub keys: Vec<String>,
    /// Whether at least one active key is the user's own
    /// (`docs/specs/07-etapa5-productividad.md` §5.1's "Personalizado"
    /// badge).
    pub custom: bool,
    /// Gray notes under the description: "antes: …" or "reemplaza a «…»",
    /// one per customized key that has something to say.
    pub notes: Vec<String>,
    /// Formatted keys the user's `keymap.json` set to `null` for this
    /// command, shown dimmed.
    pub disabled_keys: Vec<String>,
    /// Set exactly when `disabled_keys` is not empty.
    pub disabled_note: Option<String>,
}

impl ShortcutRow {
    fn matches(&self, normalized_query: &str) -> bool {
        if normalized_query.is_empty() {
            return true;
        }
        if normalize(&self.description).contains(normalized_query) {
            return true;
        }
        if normalize(&self.identity).contains(normalized_query) {
            return true;
        }
        self.keys
            .iter()
            .chain(self.disabled_keys.iter())
            .any(|key| normalize(key).contains(normalized_query))
    }
}

fn describe(command: &str) -> String {
    description(command)
        .map(str::to_owned)
        .unwrap_or_else(|| command.to_string())
}

/// The widget-side default bindings
/// (`cincel_editor::default_key_bindings`, `cincel_chat::default_key_bindings`,
/// `crate::keymap::built_in_bindings`) as a bottom [`Keymap`] layer, origin
/// [`KeymapOrigin::Default`] (`KeymapSection::new`'s default), so
/// [`Keymap::effective_bindings`] can merge them with the keymap in force
/// exactly like it merges the built-in JSONC with the user's file.
///
/// gpui-kit's own internal bindings (lists, inputs) are excluded by
/// construction: only these three explicit, Cincel-owned functions are read.
fn widget_only_keymap() -> Keymap {
    let mut sections = Vec::new();
    sections.extend(widget_sections(cincel_editor::default_key_bindings()));
    sections.extend(widget_sections(cincel_chat::default_key_bindings()));
    sections.extend(widget_sections(crate::keymap::built_in_bindings()));
    Keymap::from_sections(sections)
}

/// Converts GPUI [`gpui::KeyBinding`]s into [`KeymapSection`]s, one per
/// binding, skipping anything this crate's small [`ContextExpr`] grammar
/// cannot parse (`Chat > Composer`'s descendant `>` operator, which GPUI
/// supports and this modal does not need to: the command it names,
/// `chat::newline`, already has a describable row through its plain `Chat`
/// binding) or whose keystroke is a multi-key chord (none of the three
/// sources uses one today).
fn widget_sections(bindings: Vec<gpui::KeyBinding>) -> Vec<KeymapSection> {
    bindings
        .into_iter()
        .filter_map(|binding| {
            let keystrokes = binding.keystrokes();
            if keystrokes.len() != 1 {
                return None;
            }
            let keystroke = Keystroke::parse(&keystrokes[0].unparse()).ok()?;
            let context_text = binding
                .predicate()
                .map(|predicate| predicate.to_string())
                .unwrap_or_default();
            let context = ContextExpr::parse(&context_text).ok()?;
            let command = binding.action().name().to_string();
            Some(KeymapSection::new(
                context,
                vec![SettingsKeyBinding {
                    keystroke,
                    command: Some(command),
                }],
            ))
        })
        .collect()
}

/// Every command the default keymap or the widgets bind, for the
/// completeness test (§5.4): "recorre `Keymap::default()` + los bindings de
/// widgets y exige `description(..).is_some()` para todos". Pure: needs no
/// `App`.
pub fn all_default_commands() -> Vec<String> {
    let mut commands: Vec<String> = Keymap::default()
        .commands()
        .into_iter()
        .map(str::to_owned)
        .collect();
    for binding in cincel_editor::default_key_bindings() {
        commands.push(binding.action().name().to_string());
    }
    for binding in cincel_chat::default_key_bindings() {
        commands.push(binding.action().name().to_string());
    }
    for binding in crate::keymap::built_in_bindings() {
        commands.push(binding.action().name().to_string());
    }
    commands.sort();
    commands.dedup();
    commands
}

/// What a lower layer bound at the same (keystroke, context) as `current`,
/// if the user moved `command` to a different key than its own default
/// (§5.1's "antes: Ctrl+X", as opposed to "reemplaza a «…»", which is
/// [`EffectiveBinding::replaces`] instead).
fn before_note(
    default_only: &Keymap,
    command: &str,
    context: &ContextExpr,
    current: &Keystroke,
) -> Option<String> {
    let contexts = context.identifiers();
    let previous = default_only
        .keystrokes_for(command, &contexts)
        .into_iter()
        .find(|keystroke| keystroke != current)?;
    Some(format!("antes: {}", format_keystroke(&previous)))
}

/// Groups `combined`'s [`cincel_settings::EffectiveBinding`]s by command,
/// producing the display rows of §5.1 ("Una acción con varias teclas muestra
/// todas"). `default_only` (the same widget layer under `Keymap::default()`,
/// without the user's file) backs [`before_note`].
///
/// `pub(crate)`: see [`default_only_keymap`].
pub(crate) fn build_rows(combined: &Keymap, default_only: &Keymap) -> Vec<ShortcutRow> {
    struct Group {
        identity: String,
        context: ContextExpr,
        active: Vec<(Keystroke, bool, Option<String>)>,
        disabled: Vec<Keystroke>,
    }

    fn group_index(groups: &mut Vec<Group>, identity: &str, context: &ContextExpr) -> usize {
        if let Some(index) = groups
            .iter()
            .position(|group| group.identity == identity && &group.context == context)
        {
            return index;
        }
        groups.push(Group {
            identity: identity.to_string(),
            context: context.clone(),
            active: Vec::new(),
            disabled: Vec::new(),
        });
        groups.len() - 1
    }

    let mut groups: Vec<Group> = Vec::new();
    for binding in combined.effective_bindings() {
        match (&binding.command, &binding.replaces) {
            (Some(command), replaces) => {
                let is_custom = binding.origin == KeymapOrigin::User;
                let note = if !is_custom {
                    None
                } else {
                    match replaces {
                        Some(replaced) if replaced != command => {
                            Some(format!("reemplaza a «{}»", describe(replaced)))
                        }
                        _ => {
                            before_note(default_only, command, &binding.context, &binding.keystroke)
                        }
                    }
                };
                let index = group_index(&mut groups, command, &binding.context);
                groups[index]
                    .active
                    .push((binding.keystroke.clone(), is_custom, note));
            }
            (None, Some(replaced)) => {
                let index = group_index(&mut groups, replaced, &binding.context);
                groups[index].disabled.push(binding.keystroke.clone());
            }
            (None, None) => {}
        }
    }

    groups
        .into_iter()
        .map(|group| ShortcutRow {
            description: describe(&group.identity),
            category: category(&group.identity),
            context_label: context_label(&group.context),
            keys: group
                .active
                .iter()
                .map(|(key, _, _)| format_keystroke(key))
                .collect(),
            custom: group.active.iter().any(|(_, custom, _)| *custom),
            notes: group
                .active
                .iter()
                .filter_map(|(_, _, note)| note.clone())
                .collect(),
            disabled_keys: group.disabled.iter().map(format_keystroke).collect(),
            disabled_note: (!group.disabled.is_empty())
                .then(|| "Desactivado en tu keymap.json".to_string()),
            identity: group.identity,
        })
        .collect()
}

/// The keymap in force (`crate::settings::keymap`) layered over the widget
/// defaults, for the modal's live rows.
fn combined_keymap(cx: &App) -> Keymap {
    widget_only_keymap().layered(crate::settings::keymap(cx))
}

/// The built-in keymap (no user file) layered over the widget defaults, for
/// [`before_note`]'s baseline. No `App` needed: [`Keymap::default`] only
/// parses the built-in JSONC.
///
/// `pub(crate)` so `user_docs_tests` can enumerate the exact same rows the
/// modal shows, with no user `keymap.json` in the way, to check
/// `docs/usuario/atajos.md` documents every one of them (spec 08 §8.2).
pub(crate) fn default_only_keymap() -> Keymap {
    widget_only_keymap().layered(Keymap::default())
}

/// The floating modal itself (§5.1, §5.2): a centered dialog with a search
/// field and the grouped list, built once per window (like
/// `crate::file_finder::FileFinder`) so it does not need a project.
pub struct ShortcutsModal {
    center: Entity<CenterPanel>,
    query: Entity<InputState>,
    open: bool,
    previous_focus: Option<FocusHandle>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl ShortcutsModal {
    /// Builds a closed modal. `center` is only used by "Abrir keymap.json"
    /// (§5.1), to open the file as a tab the same way the file finder opens
    /// a project file.
    pub fn new(center: Entity<CenterPanel>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::build(center, window, cx))
    }

    fn build(center: Entity<CenterPanel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Buscar por acción o tecla…"));
        let subscriptions = vec![cx.subscribe_in(&query, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
            let _ = this;
        })];
        Self {
            center,
            query,
            open: false,
            previous_focus: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether the modal is on screen.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The search field, for the tests.
    pub fn query(&self) -> &Entity<InputState> {
        &self.query
    }

    /// The rows currently on screen: every group of §5.1, sorted by category
    /// (in [`ShortcutCategory::ORDER`]) then by description, filtered by the
    /// search field.
    pub fn visible_rows(&self, cx: &App) -> Vec<ShortcutRow> {
        let combined = combined_keymap(cx);
        let default_only = default_only_keymap();
        let mut rows = build_rows(&combined, &default_only);
        rows.sort_by(|a, b| {
            let order = |category: ShortcutCategory| {
                ShortcutCategory::ORDER
                    .iter()
                    .position(|candidate| *candidate == category)
                    .unwrap_or(usize::MAX)
            };
            order(a.category).cmp(&order(b.category)).then_with(|| {
                a.description
                    .to_lowercase()
                    .cmp(&b.description.to_lowercase())
            })
        });
        let query = normalize(&self.query.read(cx).value()).replace('-', "+");
        rows.into_iter().filter(|row| row.matches(&query)).collect()
    }

    /// Opens the modal, remembering `previous_focus` (restored by
    /// [`Self::dismiss`]) and moving the keyboard to the search field
    /// (§5.1: "Modal centrado... buscador... con el foco al abrir").
    pub fn open(
        &mut self,
        previous_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open = true;
        self.previous_focus = previous_focus;
        self.query
            .update(cx, |query, cx| query.select_all(window, cx));
        let handle = self.query.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Closes the modal, giving the keyboard back to whatever had it before
    /// (§5.1: "Esc o clic fuera cierra y devuelve el foco").
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    /// "Abrir keymap.json" (§5.1): creates the file with `[]` when it does
    /// not exist yet, opens it as a pinned tab and closes the modal.
    ///
    /// Without a project open this is a no-op: `CenterPanel::open_file`
    /// needs one (`crate::center`, out of scope for this sub-stage), the
    /// same limitation `workspace::open_settings` already has.
    fn open_keymap_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(paths) = cincel_settings::Paths::resolve() {
            if !paths.keymap.exists() {
                if let Some(parent) = paths.keymap.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&paths.keymap, "[]\n");
            }
            let path = paths.keymap.clone();
            self.open = false;
            self.previous_focus = None;
            self.center
                .clone()
                .update(cx, |center, cx| center.open_file(&path, true, window, cx));
        } else {
            self.open = false;
            self.previous_focus = None;
        }
        cx.notify();
    }

    fn on_dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }

    fn render_row(&self, row: &ShortcutRow, theme: &ThemeColors, scale: f32) -> impl IntoElement {
        h_flex()
            .id(format!("shortcut-row-{}", row.identity))
            .gap_3()
            .py_1p5()
            .px_3()
            .items_start()
            .justify_between()
            .child(
                v_flex()
                    .gap_0p5()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(13. * scale))
                                    .text_color(theme.text)
                                    .child(SharedString::from(row.description.clone())),
                            )
                            .when(row.custom, |el| {
                                el.child(
                                    div()
                                        .text_size(px(11. * scale))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.text_accent)
                                        .child("Personalizado"),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .text_size(px(11. * scale))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(row.identity.clone()))
                            .child(SharedString::from(row.context_label.clone())),
                    )
                    .children(row.notes.iter().map(|note| {
                        div()
                            .text_size(px(11. * scale))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(note.clone()))
                    }))
                    .children(row.disabled_note.as_ref().map(|note| {
                        div()
                            .text_size(px(11. * scale))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(note.clone()))
                    })),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_none()
                    .children(
                        row.keys
                            .iter()
                            .map(|key| key_chip(key, theme, scale, false)),
                    )
                    .children(
                        row.disabled_keys
                            .iter()
                            .map(|key| key_chip(key, theme, scale, true)),
                    ),
            )
    }
}

fn key_chip(text: &str, theme: &ThemeColors, scale: f32, dimmed: bool) -> impl IntoElement {
    div()
        .px(px(6. * scale))
        .py(px(2. * scale))
        .rounded(px(4. * scale))
        .bg(theme.bg_surface)
        .border_1()
        .border_color(theme.border)
        .text_size(px(11. * scale))
        .text_color(if dimmed { theme.text_muted } else { theme.text })
        .when(dimmed, |el| el.opacity(0.6))
        .child(SharedString::from(text.to_string()))
}

impl Focusable for ShortcutsModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShortcutsModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let window_width: f32 = window.viewport_size().width.into();
        let window_height: f32 = window.viewport_size().height.into();
        let width = (MAX_WIDTH * scale).min((window_width - WINDOW_MARGIN * scale).max(0.));
        let max_height = window_height * HEIGHT_FRACTION;

        let rows = self.visible_rows(cx);
        let query_text = self.query.read(cx).value().to_string();

        let body: gpui::AnyElement = if rows.is_empty() {
            div()
                .p_3()
                .text_sm()
                .text_color(theme.text_muted)
                .child(SharedString::from(format!(
                    "Ningún atajo coincide con «{query_text}»"
                )))
                .into_any_element()
        } else {
            let mut list = v_flex().id("shortcuts-list").flex_1().overflow_y_scroll();
            for category in ShortcutCategory::ORDER {
                let group: Vec<&ShortcutRow> =
                    rows.iter().filter(|row| row.category == category).collect();
                if group.is_empty() {
                    continue;
                }
                list = list.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_size(px(11. * scale))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_muted)
                        .child(SharedString::from(category.label())),
                );
                for row in group {
                    list = list.child(self.render_row(row, &theme, scale));
                }
            }
            list.into_any_element()
        };

        div()
            .id("shortcuts-modal")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_dismiss))
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            // Keeps mouse and scroll-wheel events from reaching the editor
            // behind the modal (§3, §4.4, §5, §10.4): without this, the
            // wheel scrolled both the shortcuts list and the file open
            // underneath it.
            .occlude()
            .bg(theme.bg_app.alpha(0.4))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.dismiss(window, cx)),
            )
            .child(
                v_flex()
                    .id("shortcuts-dialog")
                    .debug_selector(|| "shortcuts-dialog".to_string())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .w(px(width))
                    .max_h(px(max_height))
                    .rounded(px(6. * scale))
                    .bg(theme.bg_elevated)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .flex_none()
                            .px_3()
                            .py_2()
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .h(px(HEADER_HEIGHT * scale))
                                    .flex()
                                    .items_center()
                                    .text_size(px(13. * scale))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child("Atajos de teclado"),
                            )
                            .child(Input::new(&self.query)),
                    )
                    .child(body)
                    .child(
                        h_flex()
                            .flex_none()
                            .justify_between()
                            .items_center()
                            .px_3()
                            .py_2()
                            .border_t_1()
                            .border_color(theme.border)
                            .text_size(px(11. * scale))
                            .text_color(theme.text_muted)
                            .child("Los atajos se cambian en keymap.json")
                            .child(
                                div()
                                    .id("shortcuts-open-keymap")
                                    .cursor_pointer()
                                    .text_color(theme.text_accent)
                                    .hover(|style| style.text_color(theme.text))
                                    .child("Abrir keymap.json")
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.open_keymap_file(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// `F1` / the status bar button / the menu's "Atajos de teclado"
    /// (`workspace::show_shortcuts`, §5.1): opens the modal with the search
    /// field focused, or closes it if it is already open.
    pub fn show_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let modal = self.shortcuts_modal().clone();
        if modal.read(cx).is_open() {
            modal.update(cx, |modal, cx| modal.dismiss(window, cx));
        } else {
            let previous = window.focused(cx);
            modal.update(cx, |modal, cx| modal.open(previous, window, cx));
        }
    }
}
