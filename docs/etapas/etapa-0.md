# Etapa 0: cimientos y experimentos de riesgo

Estado: **lista para comprobación manual** (2026-09-18). Las tres apuestas técnicas funcionan en la máquina de referencia (Pop!_OS 24.04, COSMIC, Wayland, GPU AMD RADV + NVIDIA).

## Qué se construyó

| Crate | Contenido | Tests |
|---|---|---|
| `asteroid` (bin) | CLI (`[RUTA] --smoke-test --log-level --help --version`), logging `tracing`, arranque de la ventana, último escalón de GPU (aviso y salida si solo hay software, salvo `ASTEROID_ALLOW_SOFTWARE_GPU=1`) | 5 |
| `asteroid-workspace` | `Theme` con los tokens de `02-visual.md §2` proyectados sobre gpui-kit; barra de título integrada; dock de tres paneles (Chat 380 px / centro / Archivos 240 px) redimensionables y colapsables (`Ctrl+Shift+A`, `Ctrl+Shift+E`); barra de estado de 26 px; geometría de ventana persistida en `~/.local/state/asteroid/window.json` | 4 |
| `asteroid-acp` | `AgentRegistry` (descarga y caché 24 h del registro oficial), `AgentConnection` (proceso hijo en grupo propio, transporte NDJSON propio tolerante a líneas basura, hilo tokio dedicado, canales `AgentCommand`/`AgentEvent`), `Autonomy`, ejemplo `chat`, agente falso para tests | 15 |
| `asteroid-text` | `Buffer` mínimo sobre ropey: ediciones, versión, filas, `Point`⇄offset, UTF-8⇄UTF-16, CRLF→LF | 8 |
| `asteroid-editor` | `DisplayMap` con `DiffTransformMap` (filas fantasma, O(log n)), `EditorView` (cursor, selección, teclado, mouse, rueda, IME vía `EntityInputHandler`, copiar), `EditorElement` virtualizado (gutter, fondos de diff, pill Aceptar/Rechazar simulado), ejemplo `phantom` | 19 |

Total: ~7 100 líneas de Rust, 51 tests. `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace --features asteroid-editor/test-support` y `cargo deny check licenses` en verde (sin ninguna dependencia GPL/AGPL en el grafo; se permitieron además `CDLA-Permissive-2.0` de los certificados raíz y `bzip2-1.0.6`, ambas permisivas).

## Verificado por el orquestador
- `./target/debug/asteroid --smoke-test` → exit 0, GPU `AMD Radeon Graphics (RADV RENOIR)` por Vulkan, 4 frames.
- `cargo run -p asteroid-editor --example phantom -- --smoke-test` → exit 0.
- Ejemplo `chat` contra `claude-acp` y `codex-acp` (ejecutado por el subagente con las sesiones locales): ambos respondieron "hola" con `stop reason: end_turn`; se recibieron modos, modelos y esfuerzo como `configOptions`.
- Tiempos de frame del editor (build dev): p50 0,67 ms con 217 líneas y 1,18 ms con 50 000 líneas; la virtualización pinta las mismas ~40–68 filas en ambos casos.

## Hallazgos que corrigen la investigación o las specs
1. **La cascada de GPU ya está dentro de gpui-pre 0.3.5** (`Backends::VULKAN | GL` fijo, ordena adaptadores y prueba cada uno con la superficie real; única variable `ZED_DEVICE_ID`). `03-arquitectura.md §7` queda satisfecho sin código propio salvo el último escalón.
2. **Wayland/X11**: gpui elige por `ZED_HEADLESS`, `WAYLAND_DISPLAY`, `DISPLAY`. Forzar X11: `WAYLAND_DISPLAY= DISPLAY=:1`. En X11 gpui redibuja cada 16 ms (zed#57002); en Wayland no.
3. **Compilar exige `libxkbcommon-x11-dev`** por la feature `x11` de `gpui_platform`. Añadido a requisitos del README.
4. **El registro ACP real no es "uno de" `npx`/`binary`**: hay agentes con ambos canales, un tercer canal `uvx`, `sha256` ausente en 9 agentes y `npx.package` ya versionado. El parser tolera todo eso y salta entradas inválidas.
5. **Método de login de Claude**: `claude-login` ("Log in with Claude") con `args: ["--cli"]`, no `claude-ai-login`. Codex expone `api-key` como método pero funciona con la sesión ChatGPT existente sin invocarlo.
6. **SDK Rust 2.1**: `AcpAgentConfig` no permite fijar `cwd`, así que lanzamos el proceso con `tokio::process` y armamos el transporte a mano (ventaja: filtramos líneas no-JSON). Los handlers de permisos y `fs/*` deben responder desde una tarea (`spawn`), nunca inline, o se traba el dispatch. Encadenar `on_receive_request` por tipo concreto.
7. **gpui-pre 0.3.5**: el trait de entrada es `EntityInputHandler` conectado con `window.handle_input`; `shape_line` entra en pánico con `\n`; `Hitbox::is_hovered()` es falso tras entrada de teclado (usar `is_hovered_at`); sombras con `paint_drop_shadows`. Activar `gpui/test-support` como dev-dependency enciende `leak-detection` y hace abortar la app al salir ("Exited with leaked handles"): por eso `asteroid-editor` tiene su propia feature `test-support`. **Ese leak al cerrar es real y hay que investigarlo en E1.**
8. **gpui-kit 0.6.1 publicado ≠ repo main**: no tiene `Panel::title_bar`; la tira de título del grupo central no se puede suprimir y hoy hace de barra de pestañas vacía.
9. **Fuentes**: ni Inter ni JetBrains Mono están instaladas en la máquina de referencia (hay `JetBrainsMono Nerd Font Mono`); ambos crates resuelven una cadena de fallback contra `all_font_names()`.

## Limitaciones conocidas al cierre de E0 (esperadas)
- Editor sin resaltado de sintaxis, sin scroll horizontal, sin undo, sin anclas, sin word diff; el ajuste de hunks tras editar es por delta de líneas.
- Posición de ventana en Wayland no se restaura (el compositor manda); el tamaño sí.
- Layout del dock no persistido aún (solo la ventana).
- `asteroid-acp`: sin agentes custom, sin descargas `binary`/`uvx`, sin `session/load`, sin `agentFileChangeReport`, sin cancelación completa, sin `elicitation`. Lista completa en el reporte de E0-b, trasladada a `modulos/acp.md` como pendientes de E2.
- Dead keys e IME reales no probados con teclado físico (no hay `wtype`/`xdotool`); la ruta de código está cubierta por tests.

## Lista de comprobación manual (para el autor)
Desde la raíz del repo, con `libxkbcommon-x11-dev` instalado:

1. **Ventana**
   ```bash
   cargo run -p asteroid
   ```
   Debe abrir una ventana oscura con título "Asteroid", panel "Chat" a la izquierda, tira vacía y el texto "Abrí un archivo o hablale al agente" al centro, "Archivos" a la derecha y la barra de estado abajo. Probá: arrastrar los bordes entre paneles; `Ctrl+Shift+A` y `Ctrl+Shift+E` colapsan y expanden; redimensionar la ventana; cerrarla y volver a abrir (debe recordar el tamaño).
2. **Agente** (usa tus sesiones; manda un prompt corto por agente)
   ```bash
   cargo run -p asteroid-acp --example chat -- --agent claude-acp "Respondé solo con la palabra: hola"
   ```
   ```bash
   cargo run -p asteroid-acp --example chat -- --agent codex-acp "Respondé solo con la palabra: hola"
   ```
   Debe imprimir el agente, la sesión, las opciones de configuración disponibles, la respuesta en streaming y `stop reason: end_turn`. Sin pedir claves.
3. **Fila fantasma**
   ```bash
   cargo run -p asteroid-editor --example phantom
   ```
   Debe verse un archivo Rust con dos hunks: cerca de la línea 20, dos filas rojas sin número seguidas de tres verdes; cerca de la 60, una fila roja sola. Probá: escribir con el cursor en una fila roja (el texto va a la primera fila real de abajo); seleccionar con `Shift+↓` cruzando rojo y verde y `Ctrl+C`, pegar en otra app (incluye el texto rojo); escribir una letra con acento; clic en "✓ Aceptar" (desaparece el hunk, el texto queda) y en "✗ Rechazar" del otro (vuelve el texto viejo). `Ctrl+Q` cierra.

Si las tres pasan, la Etapa 0 está cerrada y podés hacer el commit. Sugerencia de mensaje: `feat: stage 0 foundations — GPUI shell, ACP client spike, phantom-row editor spike`.

## Siguiente: Etapa 1 (editor usable)
Ver `docs/specs/05-plan-etapas.md`. Arranca con resaltado de sintaxis, undo/anclas/transacciones en `asteroid-text`, proyecto y árbol real, pestañas, guardar, búsqueda, settings/keymap/tema, y la investigación del leak al cerrar.
