# Changelog

All notable changes to Cincel are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.0] - 2026-10-01

Resumen en español (etapa 7 del plan, `docs/specs/09-etapa7-conexiones-imagenes-comentarios.md`, y su ronda 2, `docs/specs/10-etapa7-ronda2.md`):

- **Comentarios para el agente**: junto a Aceptar y Rechazar hay un tercer
  botón, Comentar, y `Ctrl+Shift+M` comenta cualquier selección. La nota se
  escribe debajo de las líneas sin mover el código, aparece como etiqueta en
  la caja del chat y viaja una sola vez con el próximo mensaje, con las
  líneas a las que apunta y el estado del segmento; el mensaje enviado la
  muestra como tarjeta. Los comentarios se guardan con la revisión.
- **Imágenes en el chat**: `Ctrl+V`, arrastrar desde el explorador o botón
  Adjuntar; miniaturas con `×`, hasta 10 por mensaje, reducción a 2 000 px y
  tope de 10 MB; visor a tamaño completo; aviso si el agente no acepta
  imágenes; se guardan con la conversación.
- **Conexiones**: arriba del chat, solo el nombre completo de la conexión y
  una etiqueta con el estado de la conexión ("conectada", "desconectada",
  "sesión vencida", "no disponible", "autenticación requerida"); el menú de
  conexiones muestra cada una en dos líneas (nombre y tipo de agente) con su
  estado; el correo y el plan quedan en Configuración.
- **Menús**: todos los desplegables se cierran con clic fuera, cambio de
  foco, `Esc` o ventana inactiva.
- **Conversación**: una fila al pie dice qué hace el agente ("Pensando…",
  "Trabajando…", "Escribiendo…", "Esperando permiso…"); si subís a leer, el
  chat no te mueve, ni al terminar la respuesta, y una flecha redonda te
  lleva al final; los bloques de código de más de 20 líneas se muestran
  plegados a 12 con "Ver más (N líneas)" / "Ver menos"; las imágenes del
  mensaje enviado van arriba del texto.
- **Editor**: las demás apariciones de la palabra bajo el cursor (o de la
  selección de una línea) se marcan con un fondo suave; `Ctrl+C` / `Ctrl+X`
  sin selección copian o cortan la línea entera y `Ctrl+V` la pega como
  línea; `}` `]` `)` en una línea de solo sangría quitan un nivel, `Enter` no
  deja espacios sueltos y `Backspace` borra un nivel de sangría; se puede
  desplazar hasta dejar la última línea a media pantalla; `Ctrl+↑` / `Ctrl+↓`
  desplazan sin mover el cursor y `Ctrl+Alt+W` muestra los espacios en
  blanco; al guardar se quitan los espacios del final de cada línea y se
  asegura el salto de línea final (ajustes `files.*`, encendidos; no en
  Markdown ni en cambios del agente sin decidir).
- **Arreglo**: al retomar una conversación ya no aparece un mensaje suelto
  con las menciones `@archivo`.

### Added
- Review comments on hunks and selections (`Comentar`, `Ctrl+Shift+M`),
  sent once with the next prompt inside the review feedback block and shown
  as cards in the chat; persisted with the review.
- Images in the chat: paste, drop or attach (PNG, JPEG, GIF, WebP), up to 10
  per message, downscaled to 2 000 px, 10 MB cap; full-size viewer; stored
  with the conversation; sent as ACP `image` blocks when the agent supports
  them.

- Agent activity row at the bottom of the conversation ("Pensando…",
  "Trabajando…", "Escribiendo…", "Esperando permiso…").
- Floating "go to end" button; the transcript no longer follows new content
  once the user scrolls up (not even when the turn ends).
- Fenced code blocks longer than 20 lines are folded to 12 with
  "Ver más (N líneas)" / "Ver menos"; "Copiar" always copies the whole block.
- Editor: highlight of the other occurrences of the word under the cursor or
  of a single-line selection; whole-line copy/cut/paste without a selection;
  outdent on `}` `]` `)` typed on an indentation-only line; `Enter` leaves no
  trailing indentation; `Backspace` deletes one indent level; scroll beyond
  the last line (half a screen); `Ctrl+↑` / `Ctrl+↓` scroll without moving the
  cursor; `Ctrl+Alt+W` toggles whitespace rendering per tab.
- Settings `files.trim_trailing_whitespace_on_save` and
  `files.ensure_final_newline_on_save` (both on; trimming skips Markdown,
  pending agent hunks and files with a running turn).

### Changed
- Chat header shows only the full connection name and one connection-state
  badge; the connection menu lists each connection on two lines (name, agent
  kind) with its state; identity and plan live in Settings.
- Images of a sent message are rendered above its text.
- Every dropdown closes on click outside, focus change, `Esc` or window blur.

### Fixed
- Resuming a conversation no longer shows the replayed `@file` mentions as a
  stray user message.

## [0.1.0] - 2026-09-29

Primera versión. Resumen en español de lo que trae, por área (etapas 1 a 6
del plan de trabajo, `docs/specs/05-plan-etapas.md`):

- **Editor**: edición completa sobre un buffer propio (rope), deshacer y
  rehacer agrupados, resaltado de sintaxis incremental para más de una
  docena de lenguajes, mapa de líneas virtualizado con ajuste de línea,
  búsqueda y reemplazo (con regex opcional), ir a línea, sangría detectada,
  portapapeles y método de entrada (IME) para acentos y otros alfabetos.
- **Chat y agentes**: panel de chat conectado por el protocolo abierto ACP a
  Claude Code, Codex, Google Antigravity y cualquier agente compatible, con
  descarga y actualización propia de cada adaptador y de Node, sin depender
  de lo que ya esté instalado en el sistema ni de sesiones del sistema
  operativo.
- **Revisión de cambios**: el rasgo distintivo del editor. Cada cambio que
  hace un agente se muestra segmento por segmento dentro del editor mismo
  (nunca en el chat); se acepta o se rechaza por segmento, por línea, por
  archivo o de una vez; los archivos binarios (imágenes y demás) quedan
  fuera de la revisión y se aplican sin preguntar; un solo atajo
  (`Alt+Shift+U`) deshace el último rechazo.
- **Conexiones**: gestión de cuentas y adaptadores de agentes aislada por
  proyecto, con reintentos, reparación y desconexión, y actualización de
  Node y de cada adaptador desde la configuración.
- **Productividad**: buscador de archivos difuso (`Ctrl+P`), pantalla de
  configuración con controles para lo que antes solo se cambiaba a mano en
  `settings.json`, modal de atajos con buscador (`F1`), git en el margen del
  editor y autoguardado opcional.
- **Rendimiento**: metas medidas y comparadas contra Zed y un editor de la
  familia VS Code con un banco de medición propio (`docs/rendimiento.md`);
  fuentes Inter y JetBrains Mono embebidas en el binario para que la
  interfaz se vea igual en cualquier equipo.
- **Distribución**: paquete comprimido con instalador (`install.sh`, sin
  root) y paquete `.deb`, ambos compilados sobre Ubuntu 22.04 y sin
  bibliotecas vendorizadas; icono propio; avisos de licencias de terceros
  incluidos.

### Added
- Editor with incremental syntax highlighting, virtualized line wrapping,
  search and replace, undo/redo grouping, IME support and a full clipboard.
- Chat panel over the Agent Client Protocol (ACP), with self-managed
  download and update of official agent adapters and their Node runtime.
- Inline, segment-by-segment review of every change an agent makes,
  including binary files, with unified undo (`Alt+Shift+U`).
- Per-project connection management for agent accounts and adapters.
- Fuzzy file finder, a settings screen backed by `settings.json`, a
  searchable keybinding modal, git status in the editor gutter and optional
  autosave.
- Embedded Inter and JetBrains Mono fonts so the UI renders identically on
  any machine.
- A measurement bench and internal `--bench` scenarios used to compare
  Cincel's performance against Zed and a VS Code-family editor
  (`docs/rendimiento.md`).
- A tarball installer (`install.sh`) and a `.deb` package, both built on
  Ubuntu 22.04, with no vendored libraries and a dedicated app icon.

[0.1.0]: https://github.com/jotagary25/cincel/releases/tag/v0.1.0
