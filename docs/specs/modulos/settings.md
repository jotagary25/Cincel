# Módulo `cincel-settings`

Tipos y carga de configuración. Sin GPUI.

## Archivos
- `~/.config/cincel/settings.json` (JSON con comentarios y comas finales). Claves v1:
```jsonc
{
  "theme": { "mode": "system" | "dark" | "light", "dark": "Cincel Dark", "light": "Cincel Light" },
  "ui_font_family": "Inter", "ui_font_size": 13,
  "buffer_font_family": "JetBrains Mono", "buffer_font_size": 14, "buffer_line_height": 1.5,
  "text_rendering": "subpixel" | "grayscale",
  "editor": { "soft_wrap": false, "tab_size": 4, "insert_spaces": true, "show_whitespace": false, "ruler": 100, "cursor_blink": true },
  "files": { "exclude": ["**/.git", "**/target", "**/node_modules"], "autosave": "off" | "on_focus_change" | "after_delay", "autosave_delay_ms": 1000,
             "trim_trailing_whitespace_on_save": true, "ensure_final_newline_on_save": true },
  "review": { "jump_to_next_on_decide": false, "max_file_size_kb": 2048, "max_lines": 50000,
              "snapshot_max_total_mb": 300,
              "sensitive_paths": ["**/.env*", "**/.git/**", "**/Cargo.lock", "**/package-lock.json"] },
  "connections": { "default_label": null,
                    "runtime": { "node_version": "lts" },
                    "mcp_servers": [] },
  "window": { "decorations": "client" | "server" }
}
```
- La sección `agents` (Etapa 2/3) se renombró a `connections` en la Etapa 4 (`docs/specs/06-etapa4-conexiones-y-cincel.md` §9): `default`, `custom` y `registry_url` desaparecieron (el sistema de conexiones que los reemplaza es un workstream posterior); `mcp_servers` se mudó tal cual. `registry_url` no se movió a `connections` porque el registro (`cincel-acp::registry`) usa una URL fija, no lee ese ajuste.
- Un archivo que todavía trae la sección vieja `agents` (en cualquier forma, incluido un `agents.autonomy` suelto) carga igual y avisa una sola vez, en `agents`: "ajuste retirado: la sección «agents» ahora se llama «connections» … se ignora, podés borrarlo". `agents.autonomy` en sí se había retirado en la Etapa 3 (la política de permisos es fija, `01-producto.md §F5`).
- `~/.config/cincel/keymap.json`: lista de `{ context, bindings }` (`modulos/workspace.md`). Desde la Etapa 6 (D14), el `DEFAULT_KEYMAP_JSONC` incluye, después de la sección `Editor`, las secciones `Editor && searching` (`tab`/`shift-tab`: cambiar de campo) y `Editor && searching && replacing` (`enter`/`ctrl-enter`: reemplazar uno o todos); antes, esas cuatro teclas funcionaban pero no figuraban en el archivo por defecto ni en el modal de atajos (`F1`).
- `~/.config/cincel/themes/*.json`: tokens de `02-visual.md §2` + colores de sintaxis; desde la Etapa 5 suma `git.added`, `git.modified`, `git.deleted` (un tema de usuario sin esas claves usa los del tema incluido de su apariencia).

## Responsabilidades
- Tipos `Settings`, `Keymap`, `Theme` con `Default` completo y `serde` con `#[serde(default)]`.
- `load()` tolerante: un valor inválido se reporta (ruta JSON + motivo) y se usa el predeterminado; nunca impide arrancar.
- Watcher de los tres archivos → `SettingsEvent::Changed`.
- `cincel --print-default-settings` imprime el JSON predeterminado con comentarios.

## Etapa 6: tope de memoria de la foto en Configuración

Detalle, verificación y desviaciones en `docs/etapas/etapa-6.md`. Spec:
`docs/specs/08-etapa6-cierre-1-0.md` §5.1.

`review.snapshot_max_total_mb` (16–4096, por defecto 300; fuera de rango se
reporta y se usa 300) ya existía como clave, pero sin fila en la pestaña de
configuración: solo se podía cambiar escribiendo `settings.json` a mano.
Desde esta etapa, `crates/cincel-workspace/src/settings_view.rs` (tabla
`ROWS`, sección Revisión, después de `review.max_lines`) tiene la fila
**"Memoria para la foto del proyecto (MB)"** (control `number(16., 4096.,
16., 0)`), con la descripción "Antes de cada mensaje al agente, Cincel
guarda una copia de tus archivos para poder mostrarte y deshacer lo que
cambie. Este es el máximo que ocupa esa copia; pasado el tope, los archivos
que no entran solo se pueden aceptar enteros." Aparece al buscar "memoria" o
"foto" en la pestaña; el turno siguiente usa el valor nuevo.

## Etapa 5: edición del archivo desde la interfaz y keymap efectivo

Detalle, verificación y desviaciones en `docs/etapas/etapa-5.md`. Spec:
`docs/specs/07-etapa5-productividad.md` §4.3 y §5.2.

- **`edit.rs`**: `SettingsEdit::{Set, Remove}`, `apply_edit(text, edit) ->
  Result<String, EditError>` (puro, sobre el CST de `jsonc-parser` 0.33,
  feature `cst`) y `write_edit(paths, edit) -> Result<(), EditError>`
  (lectura-modificación-escritura atómica: archivo temporal en el mismo
  directorio + `rename`). Conserva comentarios, orden de claves, comas
  finales y claves desconocidas byte a byte fuera del valor tocado. Un
  archivo vacío o inexistente arranca del documento con el comentario
  explicativo (`cincel --print-default-settings` te muestra todos), no de
  `{}` a secas. `EditError::Syntax { line, column, message }` da la posición
  1-indexada del error para el banner de la pestaña de configuración
  (`modulos/workspace.md`).
- **`Keymap::effective_bindings()`**: `KeymapSection` gana
  `origin: KeymapOrigin { Default, User }` (no se serializa); esta función da
  una fila por (tecla, contexto) con la de nivel más alto ganando, más
  `replaces` (qué comando de nivel inferior tenía esa misma tecla), para el
  modal de atajos.

## Ronda 2 de la Etapa 7: limpieza al guardar

Spec: `docs/specs/10-etapa7-ronda2.md` §7.8 (R16). `FilesSettings` suma dos claves (`#[serde(default)]`, bool, `true` por defecto), con estos comentarios en `DEFAULT_SETTINGS_JSONC` (`crates/cincel-settings/src/defaults.rs`):

```jsonc
    // Al guardar, quitar los espacios del final de cada línea (no en Markdown ni en los cambios del agente sin decidir).
    "trim_trailing_whitespace_on_save": true,
    // Al guardar, terminar el archivo con un salto de línea si no lo tiene.
    "ensure_final_newline_on_save": true
```

- `files.trim_trailing_whitespace_on_save`: quita los espacios y tabulaciones del final de cada línea al guardar (no en Markdown, ni en los cambios del agente sin decidir).
- `files.ensure_final_newline_on_save`: agrega un salto de línea final si el archivo no vacío no lo tiene.
- Las aplica `Center::save_path` en el workspace (`modulos/workspace.md`); tienen fila en Configuración → Archivos.

## Criterios de aceptación
- [ ] Un `settings.json` vacío, inexistente o con un error de sintaxis produce los valores por defecto y un aviso.
- [ ] Tests de ida y vuelta serde para los tres tipos.
