//! The two documents Cincel ships with: the commented default
//! `settings.json` and the default `keymap.json`.
//!
//! Both are plain JSONC text rather than generated from the types, because
//! the comments *are* the documentation the user reads. Tests keep them
//! honest: the settings document must parse back to [`crate::Settings::default`]
//! with no issues, and the keymap must contain every shortcut of
//! `docs/specs/02-visual.md` §8.

/// The default keymap, `docs/specs/02-visual.md` §8.
///
/// Command names come from `docs/specs/modulos/editor.md` and
/// `docs/specs/modulos/workspace.md`. Sections are ordered from least to most
/// specific: a later section wins, which is how `ctrl-backspace` deletes a
/// word in the editor but rejects the hunk when there is one under the
/// cursor.
pub const DEFAULT_KEYMAP_JSONC: &str = r##"[
  // Global. No context: these work wherever the focus is.
  {
    "bindings": {
      "ctrl-o": "workspace::open_folder",
      "ctrl-shift-a": "workspace::toggle_chat",
      "ctrl-shift-e": "workspace::toggle_tree",
      // Siguiente zona de foco: chat → editor → archivos → chat. Salta los
      // paneles ocultos y nunca abre nada. "workspace::focus_chat" sigue
      // existiendo, sin atajo.
      "ctrl-l": "workspace::focus_next_zone",
      // Buscador de archivos, configuración, atajos, nuevo archivo y salir.
      "ctrl-p": "workspace::toggle_file_finder",
      "ctrl-,": "workspace::open_settings",
      "f1": "workspace::show_shortcuts",
      "ctrl-n": "workspace::new_file",
      "ctrl-q": "workspace::quit",
      "ctrl-shift-r": "workspace::open_review_panel",
      // Siguiente / anterior cambio, con las dos teclas de la spec.
      "alt-j": "workspace::next_change",
      "alt-k": "workspace::prev_change",
      "f7": "workspace::next_change",
      "shift-f7": "workspace::prev_change",
      "alt-l": "workspace::next_file_with_changes",
      // Aceptar / rechazar todo el turno.
      "ctrl-alt-enter": "workspace::accept_turn",
      "ctrl-alt-backspace": "workspace::reject_turn",
      "alt-shift-u": "workspace::undo_last_reject",
      // Zoom de la interfaz.
      "ctrl-=": "workspace::zoom_in",
      "ctrl--": "workspace::zoom_out",
      "ctrl-0": "workspace::zoom_reset"
    }
  },

  // El editor de código.
  {
    "context": "Editor",
    "bindings": {
      "ctrl-s": "editor::save",
      "ctrl-alt-s": "editor::save_all",
      "ctrl-w": "workspace::close_tab",
      "ctrl-f": "editor::find",
      "ctrl-h": "editor::find_replace",
      "f3": "editor::find_next",
      "shift-f3": "editor::find_prev",
      "ctrl-z": "editor::undo",
      "ctrl-shift-z": "editor::redo",
      "ctrl-g": "editor::go_to_line",
      "ctrl-a": "editor::select_all",
      "ctrl-c": "editor::copy",
      "ctrl-x": "editor::cut",
      "ctrl-v": "editor::paste",
      "alt-z": "editor::toggle_soft_wrap",
      "enter": "editor::insert_newline",
      "tab": "editor::tab",
      "shift-tab": "editor::backtab",
      // Sin un segmento bajo el cursor, estos son sus significados normales.
      "ctrl-enter": "editor::insert_newline",
      "ctrl-backspace": "editor::delete_word_left",
      "ctrl-delete": "editor::delete_word_right",
      // Aceptar / rechazar el archivo entero.
      "ctrl-shift-enter": "review::accept_file",
      "ctrl-shift-backspace": "review::reject_file",
      // Vista previa de Markdown (también en el contexto "Center", más abajo,
      // para cuando la pestaña ya está en modo vista previa y el editor no
      // tiene el foco).
      "ctrl-shift-v": "workspace::toggle_markdown_preview"
    }
  },

  // El área de pestañas, tanto con el editor como con la vista previa de
  // Markdown enfocados.
  {
    "context": "Center",
    "bindings": {
      "ctrl-shift-v": "workspace::toggle_markdown_preview"
    }
  },

  // Con un segmento pendiente bajo el cursor, Ctrl+Enter y Ctrl+Backspace
  // deciden ese segmento en lugar de editar.
  {
    "context": "Editor && review_hunk_under_cursor",
    "bindings": {
      "ctrl-enter": "review::accept_hunk",
      "ctrl-backspace": "review::reject_hunk",
      "alt-enter": "review::accept_line",
      "alt-backspace": "review::reject_line"
    }
  },

  // La barra de búsqueda del editor. "tab"/"shift-tab" mueven el foco entre
  // sus campos en lugar de tabular el archivo; como esta sección va después
  // de "Editor", gana ella mientras la barra tiene el foco (si reasignás
  // "tab" en tu propia sección "Editor" sin "searching", esa gana también
  // acá, igual que en Zed).
  {
    "context": "Editor && searching",
    "bindings": {
      "enter": "editor::find_next",
      "shift-enter": "editor::find_prev",
      "escape": "editor::dismiss_find",
      "tab": "editor::search_next_field",
      "shift-tab": "editor::search_prev_field"
    }
  },

  // El campo "Reemplazar…" de la barra de búsqueda, cuando tiene el foco:
  // "enter" reemplaza la coincidencia actual y "ctrl-enter" reemplaza todas.
  {
    "context": "Editor && searching && replacing",
    "bindings": {
      "enter": "editor::replace_next",
      "ctrl-enter": "editor::replace_all"
    }
  },

  // El chat.
  {
    "context": "Chat",
    "bindings": {
      "enter": "chat::send",
      "shift-enter": "chat::newline",
      "escape": "chat::cancel_turn"
    }
  }
]
"##;

/// The default `settings.json`, with a comment on every key.
///
/// This is what `cincel --print-default-settings` prints; a user can
/// redirect it into `~/.config/cincel/settings.json` and edit from there.
pub fn default_settings_jsonc() -> String {
    DEFAULT_SETTINGS_JSONC.to_owned()
}

/// The default keymap file, for `cincel --print-default-keymap` and for
/// anyone who wants to start their own from it.
pub fn default_keymap_jsonc() -> String {
    DEFAULT_KEYMAP_JSONC.to_owned()
}

const DEFAULT_SETTINGS_JSONC: &str = r##"{
  // Ajustes de Cincel. Es JSON con comentarios y comas finales.
  // Todo lo que no escribas usa el valor que ves acá.

  "theme": {
    // "system" sigue al escritorio; "dark" o "light" lo fijan.
    "mode": "system",
    // Tema usado en apariencia oscura. Los temas viven en
    // ~/.config/cincel/themes/*.json
    "dark": "Cincel Dark",
    // Tema usado en apariencia clara.
    "light": "Cincel Light"
  },

  // Tipografía de la interfaz (paneles, pestañas, barra de estado).
  "ui_font_family": "Inter",
  "ui_font_size": 13,

  // Tipografía del editor de código.
  "buffer_font_family": "JetBrains Mono",
  "buffer_font_size": 14,
  // Alto de línea como múltiplo del tamaño de fuente.
  "buffer_line_height": 1.5,

  // Suavizado del texto: "subpixel" o "grayscale".
  "text_rendering": "subpixel",

  "editor": {
    // Ajustar las líneas largas al ancho del editor en lugar de desplazar a
    // lo ancho (activado por defecto). Alt+Z lo alterna en la pestaña actual.
    "soft_wrap": true,
    // Ancho de la tabulación, en columnas (1 a 16).
    "tab_size": 4,
    // Insertar espacios al presionar Tab.
    "insert_spaces": true,
    // Mostrar espacios y tabulaciones.
    "show_whitespace": false,
    // Columna de la guía vertical; 0 la oculta.
    "ruler": 100,
    // Parpadeo del cursor.
    "cursor_blink": true,
    // Cerrar automáticamente paréntesis, corchetes, llaves y comillas.
    "auto_close_pairs": true
  },

  "files": {
    // Patrones que no se muestran en el árbol ni se recorren.
    // Se suman a lo que diga .gitignore.
    "exclude": ["**/.git", "**/target", "**/node_modules"],
    // "off", "on_focus_change" o "after_delay" (guarda tras autosave_delay_ms
    // sin escribir).
    "autosave": "off",
    // Con "after_delay": milisegundos sin escribir antes de guardar (100 a 60000).
    "autosave_delay_ms": 1000
  },

  "review": {
    // Saltar al siguiente segmento pendiente al aceptar o rechazar uno.
    "jump_to_next_on_decide": false,
    // Archivos más grandes que esto se revisan enteros, no por segmento.
    "max_file_size_kb": 2048,
    // Ídem por cantidad de líneas.
    "max_lines": 50000,
    // Antes de cada mensaje al agente, Cincel guarda una copia de tus
    // archivos para poder mostrarte y deshacer lo que cambie. Este es el
    // máximo que ocupa esa copia (de 16 a 4096 MB); pasado el tope, los
    // archivos que no entran solo se pueden aceptar enteros.
    "snapshot_max_total_mb": 300,
    // Rutas que siempre piden confirmación antes de que el agente las toque.
    "sensitive_paths": ["**/.env*", "**/.git/**", "**/Cargo.lock", "**/package-lock.json"]
  },

  "connections": {
    // Etiqueta de la conexión preseleccionada en el popover "Conectar";
    // null (el valor por defecto) no preselecciona ninguna.
    "default_label": null,
    // Runtime de Node privado para los adaptadores ACP distribuidos por npm.
    "runtime": {
      // "lts" o una versión mayor exacta, por ejemplo "22".
      "node_version": "lts"
    },
    // Servidores MCP ofrecidos a cada sesión. Por ejemplo:
    // { "name": "fs", "command": "mcp-server-fs", "args": [], "env": {} }
    "mcp_servers": []
  },

  "window": {
    // "client": barra de título integrada (estilo Zed).
    // "server": que la dibuje el escritorio.
    "decorations": "client"
  }
}
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Keymap, Keystroke};
    use crate::settings::Settings;

    #[test]
    fn default_settings_document_parses_to_the_defaults() {
        let loaded = Settings::parse(&default_settings_jsonc());
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value, Settings::default());
    }

    #[test]
    fn default_settings_document_documents_every_key() {
        let text = default_settings_jsonc();
        for key in [
            "theme",
            "ui_font_family",
            "ui_font_size",
            "buffer_font_family",
            "buffer_font_size",
            "buffer_line_height",
            "text_rendering",
            "editor",
            "files",
            "review",
            "connections",
            "window",
        ] {
            assert!(text.contains(&format!("\"{key}\"")), "falta la clave {key}");
        }
        assert!(text.contains("//"), "el documento debe llevar comentarios");
    }

    #[test]
    fn default_keymap_parses_cleanly() {
        let loaded = Keymap::parse(DEFAULT_KEYMAP_JSONC);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value, Keymap::default());
    }

    /// Every row of `docs/specs/02-visual.md` §8, in order.
    #[test]
    fn default_keymap_has_every_shortcut_of_the_spec() {
        let keymap = Keymap::default();
        let editor = ["Editor"];
        let hunk = ["Editor", "review_hunk_under_cursor"];
        let searching = ["Editor", "searching"];
        let replacing = ["Editor", "searching", "replacing"];
        let chat = ["Chat"];
        let center = ["Center"];
        let global: [&str; 0] = [];
        let cases: &[(&str, &[&str], &str)] = &[
            ("ctrl-o", &global, "workspace::open_folder"),
            ("ctrl-s", &editor, "editor::save"),
            ("ctrl-alt-s", &editor, "editor::save_all"),
            ("ctrl-w", &editor, "workspace::close_tab"),
            ("ctrl-f", &editor, "editor::find"),
            ("ctrl-z", &editor, "editor::undo"),
            ("ctrl-shift-z", &editor, "editor::redo"),
            ("ctrl-g", &editor, "editor::go_to_line"),
            ("ctrl-shift-a", &global, "workspace::toggle_chat"),
            ("ctrl-shift-e", &global, "workspace::toggle_tree"),
            // `07-etapa5-productividad.md` §9.2 and §11.1.
            ("ctrl-l", &global, "workspace::focus_next_zone"),
            ("ctrl-p", &global, "workspace::toggle_file_finder"),
            ("ctrl-,", &global, "workspace::open_settings"),
            ("f1", &global, "workspace::show_shortcuts"),
            ("ctrl-n", &global, "workspace::new_file"),
            ("ctrl-q", &global, "workspace::quit"),
            ("ctrl-h", &editor, "editor::find_replace"),
            ("enter", &chat, "chat::send"),
            ("shift-enter", &chat, "chat::newline"),
            ("escape", &chat, "chat::cancel_turn"),
            ("ctrl-enter", &hunk, "review::accept_hunk"),
            ("ctrl-backspace", &hunk, "review::reject_hunk"),
            ("ctrl-enter", &editor, "editor::insert_newline"),
            ("ctrl-backspace", &editor, "editor::delete_word_left"),
            ("ctrl-shift-enter", &editor, "review::accept_file"),
            ("ctrl-shift-backspace", &editor, "review::reject_file"),
            (
                "ctrl-shift-v",
                &editor,
                "workspace::toggle_markdown_preview",
            ),
            (
                "ctrl-shift-v",
                &center,
                "workspace::toggle_markdown_preview",
            ),
            ("ctrl-alt-enter", &global, "workspace::accept_turn"),
            ("ctrl-alt-backspace", &global, "workspace::reject_turn"),
            ("alt-j", &global, "workspace::next_change"),
            ("alt-k", &global, "workspace::prev_change"),
            ("f7", &global, "workspace::next_change"),
            ("shift-f7", &global, "workspace::prev_change"),
            ("alt-l", &global, "workspace::next_file_with_changes"),
            ("ctrl-shift-r", &global, "workspace::open_review_panel"),
            ("alt-shift-u", &global, "workspace::undo_last_reject"),
            ("ctrl-=", &global, "workspace::zoom_in"),
            ("ctrl--", &global, "workspace::zoom_out"),
            ("ctrl-0", &global, "workspace::zoom_reset"),
            // `08-etapa6-cierre-1-0.md` §5.4 (D14): search-bar keys.
            ("tab", &searching, "editor::search_next_field"),
            ("shift-tab", &searching, "editor::search_prev_field"),
            ("enter", &replacing, "editor::replace_next"),
            ("ctrl-enter", &replacing, "editor::replace_all"),
        ];
        for (stroke, contexts, command) in cases {
            let keystroke = Keystroke::parse(stroke).unwrap();
            assert_eq!(
                keymap.resolve(&keystroke, contexts),
                Some(*command),
                "{stroke} en {contexts:?}"
            );
        }
        // `workspace::focus_chat` stays registered but loses `ctrl-l` (§9.2, D9).
        assert!(!keymap.commands().contains(&"workspace::focus_chat"));
    }

    #[test]
    fn default_keymap_avoids_the_forbidden_keys() {
        // 02-visual.md §8: ni Super, ni Alt+F* (los toma COSMIC/GNOME).
        for section in Keymap::default().sections() {
            for binding in &section.bindings {
                let stroke = binding.keystroke.to_string();
                assert!(!binding.keystroke.cmd, "{stroke} usa Super");
                assert!(
                    !(binding.keystroke.alt
                        && binding.keystroke.key.starts_with('f')
                        && binding.keystroke.key.len() > 1),
                    "{stroke} es un Alt+F*"
                );
            }
        }
    }
}
