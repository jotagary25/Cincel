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
  "files": { "exclude": ["**/.git", "**/target", "**/node_modules"], "autosave": "off" | "on_focus_change" },
  "review": { "jump_to_next_on_decide": true, "max_file_size_kb": 2048, "max_lines": 50000,
              "sensitive_paths": ["**/.env*", "**/.git/**", "**/Cargo.lock", "**/package-lock.json"] },
  "connections": { "default_label": null,
                    "runtime": { "node_version": "lts" },
                    "mcp_servers": [] },
  "window": { "decorations": "client" | "server" }
}
```
- La sección `agents` (Etapa 2/3) se renombró a `connections` en la Etapa 4 (`docs/specs/06-etapa4-conexiones-y-cincel.md` §9): `default`, `custom` y `registry_url` desaparecieron (el sistema de conexiones que los reemplaza es un workstream posterior); `mcp_servers` se mudó tal cual. `registry_url` no se movió a `connections` porque el registro (`cincel-acp::registry`) usa una URL fija, no lee ese ajuste.
- Un archivo que todavía trae la sección vieja `agents` (en cualquier forma, incluido un `agents.autonomy` suelto) carga igual y avisa una sola vez, en `agents`: "ajuste retirado: la sección «agents» ahora se llama «connections» … se ignora, podés borrarlo". `agents.autonomy` en sí se había retirado en la Etapa 3 (la política de permisos es fija, `01-producto.md §F5`).
- `~/.config/cincel/keymap.json`: lista de `{ context, bindings }` (`modulos/workspace.md`).
- `~/.config/cincel/themes/*.json`: tokens de `02-visual.md §2` + colores de sintaxis.

## Responsabilidades
- Tipos `Settings`, `Keymap`, `Theme` con `Default` completo y `serde` con `#[serde(default)]`.
- `load()` tolerante: un valor inválido se reporta (ruta JSON + motivo) y se usa el predeterminado; nunca impide arrancar.
- Watcher de los tres archivos → `SettingsEvent::Changed`.
- `cincel --print-default-settings` imprime el JSON predeterminado con comentarios.

## Criterios de aceptación
- [ ] Un `settings.json` vacío, inexistente o con un error de sintaxis produce los valores por defecto y un aviso.
- [ ] Tests de ida y vuelta serde para los tres tipos.
