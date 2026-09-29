# Changelog

All notable changes to Cincel are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
  archivo o de una vez; los archivos binarios (imágenes y demás) también se
  revisan y persisten entre reinicios; un solo atajo (`Alt+Shift+U`) deshace
  el último rechazo, sea de texto o binario.
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
