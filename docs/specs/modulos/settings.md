# Módulo `asteroid-settings`

Tipos y carga de configuración. Sin GPUI.

## Archivos
- `~/.config/asteroid/settings.json` (JSON con comentarios y comas finales). Claves v1:
```jsonc
{
  "theme": { "mode": "system" | "dark" | "light", "dark": "Asteroid Dark", "light": "Asteroid Light" },
  "ui_font_family": "Inter", "ui_font_size": 13,
  "buffer_font_family": "JetBrains Mono", "buffer_font_size": 14, "buffer_line_height": 1.5,
  "text_rendering": "subpixel" | "grayscale",
  "editor": { "soft_wrap": false, "tab_size": 4, "insert_spaces": true, "show_whitespace": false, "ruler": 100, "cursor_blink": true },
  "files": { "exclude": ["**/.git", "**/target", "**/node_modules"], "autosave": "off" | "on_focus_change" },
  "review": { "jump_to_next_on_decide": true, "max_file_size_kb": 2048, "max_lines": 50000,
              "sensitive_paths": ["**/.env*", "**/.git/**", "**/Cargo.lock", "**/package-lock.json"] },
  "agents": { "default": "claude-acp", "autonomy": "review_after" | "ask_before" | "always_apply",
              "registry_url": "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json",
              "custom": [ { "id": "mi-agente", "name": "Mi agente", "command": "mi-agente", "args": ["acp"], "env": {} } ],
              "mcp_servers": [] },
  "window": { "decorations": "client" | "server" }
}
```
- `~/.config/asteroid/keymap.json`: lista de `{ context, bindings }` (`modulos/workspace.md`).
- `~/.config/asteroid/themes/*.json`: tokens de `02-visual.md §2` + colores de sintaxis.

## Responsabilidades
- Tipos `Settings`, `Keymap`, `Theme` con `Default` completo y `serde` con `#[serde(default)]`.
- `load()` tolerante: un valor inválido se reporta (ruta JSON + motivo) y se usa el predeterminado; nunca impide arrancar.
- Watcher de los tres archivos → `SettingsEvent::Changed`.
- `asteroid --print-default-settings` imprime el JSON predeterminado con comentarios.

## Criterios de aceptación
- [ ] Un `settings.json` vacío, inexistente o con un error de sintaxis produce los valores por defecto y un aviso.
- [ ] Tests de ida y vuelta serde para los tres tipos.
