# Asteroid: síntesis de investigación y propuesta de decisiones

Fecha: 2026-09-17. Estado: **borrador para discutir**. Informes detallados en `docs/research/`.

## 1. Los siete hechos que condicionan el diseño

1. **Claude Code y Codex escriben a disco directo.** Sus adapters ACP no usan `fs/write_text_file`; emiten un `diff` informativo después de escribir. Solo Gemini CLI enruta escrituras por el cliente. Zed tampoco retiene: aplica, guarda y trackea. → El review es **post-hoc**: "aplicar, trackear, revertir", nunca "retener hasta aprobar". (01-acp §1)
2. **El estándar de la industria es escribir a disco de inmediato + snapshot para revertir** (Cursor, Windsurf, VS Code, Zed). Approval-first (Cline, Junie) es lo que los usuarios piden cambiar. (03-review-ux §1)
3. **El accept/reject por hunk o línea no existe en ACP.** Se computa del lado cliente: `base_text` por archivo + diff recomputado (`imara-diff`, Histogram). Accept = avanzar la base; Reject = revertir el buffer y guardar. Zed y VS Code convergen en este algoritmo. (03-review-ux §2)
4. **GPUI es usable fuera de Zed** (Apache-2.0, Wayland/X11 nativo, renderer wgpu) vía `gpui-pre` 0.3.5. **`gpui-kit` 0.6.1** trae un editor de código real (25k LOC, tree-sitter, LSP, folding) y todo el chrome (dock, tabs, tree, markdown streaming). (02-gui-stack, 04-repos)
5. **Pero el editor de gpui-kit no tiene BlockMap**: no puede insertar filas virtuales, que es lo que hace falta para mostrar líneas borradas en rojo entre líneas reales. Zed dedica 236 KB de código a eso. (04-repos adenda b)
6. **Tu escritorio es COSMIC (Pop!_OS 24.04), no GNOME.** COSMIC está hecho en iced. Tu GPU es híbrida AMD+NVIDIA, donde viven la mayoría de los bugs Wayland. (02-gui-stack adenda b)
7. **Zed sí tiene review inline** pero está apagado por defecto desde 0.225.9 porque pisa el diff de git, y está roto en varios casos. Vos lo tenés activado (`single_file_review: true`) y aun así te parece insuficiente. Hay hueco real de producto. (03-review-ux §0)

## 2. Decisiones técnicas propuestas

Cada decisión trae mi recomendación. Las marcadas **[TU DECISIÓN]** cambian materialmente el trabajo y necesitan tu OK; las demás son defaults razonables que podés vetar.

### D1. Framework GUI **[TU DECISIÓN]**
**Recomendación: GPUI (`gpui-pre` o `gpui-unofficial`) + `gpui-kit` para todo el chrome, y un crate de editor propio derivado del editor de gpui-kit (Apache-2.0) al que le agregamos la capa BlockMap.**
- Por qué no Tauri + CodeMirror 6: resuelve el diff inline hoy, pero contradice Rust nativo, liviano y Zed-like; WebKitGTK + NVIDIA es frágil y tiene blur con fractional scaling en Wayland sin fix.
- Por qué no iced/libcosmic desde cero: aunque COSMIC lo usa, `text_editor` no es un editor de código, su texto es grayscale-only, y serían meses antes de igualar lo que gpui-kit ya da.
- Por qué no Freya (el otro candidato serio): tiene un code editor con ropey + tree-sitter + virtual scroll donde el diff inline es composición natural, y accesibilidad real. Pero depende de una sola persona sin financiamiento, usa Skia (binarios gordos), no tiene multi-cursor ni LSP ni chrome, y su IME en el editor está roto. Es la alternativa si el spike del BlockMap fracasa.
- Por qué no usar `editor` de Zed: GPL-3.0 y su closure son 96 crates (71 GPL), incluyendo el cliente RPC y el plumbing de LLM de Zed. Nota: Zed ya no tiene AGPL en ningún crate (relicenciado en mayo 2026).
- Costo real de la recomendación: 2–3k LOC para la capa de diff en el display pipeline, más mantenimiento del fork contra releases semanales de gpui. Hay dos formas de mostrar el texto borrado: como **widget** (CodeMirror, VS Code: barato, pero el texto borrado no es seleccionable ni buscable y su highlighting es limitado) o como **splice** (Zed desde 2025: una capa `DiffTransform` bajo el display map materializa el `base_text` como filas reales read-only, con highlighting, selección, búsqueda y wrap gratis). Para un editor propio recomiendo el splice: `Rope → DiffTransformMap → WrapMap → FoldMap → BlockMap (solo para el pill de botones)`. Referencias permisivas: `phantom_text.rs`/`visual_line.rs` de floem (MIT), `inlays.rs`/`diff.rs` de Makepad, `LineAnnotation` de Helix (MPL). El editor de gpui-kit ya virtualiza el layout de líneas visibles, así que la capa es de dificultad media.
- Regla que aplica elijamos lo que elijamos: **el core del editor (rope, tree-sitter, modelo de review, LSP) vive en crates agnósticos del framework**; la capa UI queda reemplazable.
- Alternativa de menor riesgo para MVP: "review view" read-only propio (~1.5k LOC) que reemplaza al editor en la pestaña mientras hay cambios pendientes, y migrar al BlockMap después. Pierde "editar dentro del review".

### D2. Alcance del review para v1 **[TU DECISIÓN]**
Granularidad: hunk + **línea** (estilo Antigravity) + archivo + turno. ¿La edición manual dentro de un hunk pendiente es requisito v1 o puede esperar? Cursor y Antigravity lo permiten; implica `apply_non_conflicting_edits` desde el día uno (recomendado igual).

### D3. Semántica de disco
Escribir a disco inmediatamente (el agente lo hace igual), `fs/read_text_file` sirve el buffer en memoria, reject = revert quirúrgico + save. Persistir `base_text` content-addressed en `.asteroid/review/` y **nunca reescribir el archivo al restaurar**; si el hash cambió por fuera, descartar ese review. Formatters solo al aceptar.

### D4. Protocolo
ACP v1 con `agent-client-protocol` 2.1.0 (requiere Rust ≥ 1.88; hoy tenés 1.85). Anunciar `fs.readTextFile/writeTextFile`, `elicitation`, `session.configOptions`. No anunciar `terminal` en v1. Descubrimiento de agentes vía el registry oficial (`registry.json`, 41 agentes) igual que Zed, sin hardcodear comandos. Soportar `agentFileChangeReport` (extensión de JetBrains que Claude y Codex implementan) para saber qué archivos tocó el turno.

### D5. Captura de la base y detección de cambios
`base_text` de un archivo se fija en el primer contacto del turno: `fs/read_text_file`, o `oldText` del primer `diff`, o lectura de disco al ver el primer `tool_call kind=edit` sobre ese path. Los cambios reales se detectan releyendo disco al completarse cada tool call de edición y con un file watcher (`notify`) como red; el `diff` del tool call es señal de UI, no fuente de verdad. Los hunks salen de `imara-diff` contra `base_text`, recomputados con debounce ~50 ms en background.

### D6. Feedback al agente
Al enviar el siguiente prompt, por archivo revisado: patch unificado del delta entre lo que el agente escribió y lo que quedó ("The user made the following updates to your content:"), con cambios de formateo separados. Es el mecanismo probado de Cline/Roo.

### D7. Permission gate
Usar `session/request_permission` como preview pre-escritura opcional por patrón de archivo (como `chat.tools.edits.autoApprove` de VS Code: `.env`, `Cargo.lock`, etc. piden confirmación; el resto pasa y se revisa post-hoc). Tres niveles de autonomía como Antigravity: "Siempre aplicar" / "Revisar después" (default) / "Pedir antes".

### D8. Login de agentes
Claude y Codex se autentican con la sesión ya existente del CLI del sistema. Si no hay sesión, el adapter pide auth de tipo `terminal`: en v1 abrimos el terminal del sistema con el comando de login (o mostramos el comando), sin terminal embebida. `alacritty_terminal` queda para v2.

### D9. Editor core
Rope: `ropey` (lo que usa gpui-kit y Helix; ojo bug `Ord` #120 sin release). Diff: `similar` 3.2 (Histogram para líneas y word-level para hunks ≤ 5 líneas; `imara-diff` está estancado, su fork vivo es `gix-imara-diff`), **siempre con postprocess de hunks** (heurística slider/indent, requisito de corrección según Zed). Tree-sitter 0.27 **siempre desde crates.io** (evita la doble copia de `tree-sitter-language`), gramáticas estáticas por cargo feature (10–15 lenguajes iniciales; evaluar `arborium` para ampliar). Búsqueda sobre rope: `regex-cursor`. LSP: el cliente de gpui-kit. Fuzzy: `frizbee` (MIT, activo; `nucleo` está estancado y es MPL). Git: `gix` para lectura y el binario `git` para push/credenciales (como Zed). Watcher: `notify` 9 rc + debouncer, limitado por `.gitignore` y observando directorios. Diálogos: `rfd` vía portal. Texto: el renderizado subpixel de GPUI (cosmic-text/iced es grayscale-only, otra razón para GPUI).

### D10. Undo
La edición del agente es una transacción en el undo stack (Ctrl+Z la deshace). Accept no toca el buffer (no es deshacible por Ctrl+Z). "Undo last reject" con toast.

### D11. Git
No pisar el diff de git. Accept/Reject del agente es una variante del mismo widget de hunk con otro renderer (patrón `DiffHunkRenderer` de Zed). Si el usuario commitea un hunk pendiente, se marca aceptado.

### D12. Distribución y GPU
Tarball + `install.sh` con `dist`, compilado en Ubuntu 22.04; `.deb` con `cargo-deb`. Sin Snap ni Flatpak. wgpu con Vulkan **y** GL, cascada manual, software GPU detrás de env var. Redibujar por evento, no a 60 Hz. `cargo deny check licenses` desde el commit 1 (hay una cadena GPL cuestionada en gpui, issue #55470). Objetivo de binario: 20–60 MB, arranque < 400 ms.

### D13. Licencia del proyecto **[TU DECISIÓN]**
Con la recomendación de D1 el proyecto puede ser Apache-2.0/MIT o propietario. Si eligieras reutilizar código GPL de Zed, Asteroid sería GPL-3.0. ¿Open source? ¿Qué licencia?

## 3. Especificación visual propuesta

### Layout (inspirado en tus capturas y en Zed)
- Ventana con CSD propias estilo Zed (título integrado en la barra de tabs), override por env var.
- **Izquierda**: panel de chat del agente (dock izquierdo, como tenés configurado en Zed), redimensionable y colapsable. Arriba: selector de agente (Claude / Codex / Gemini / OpenCode…) y de sesión; abajo: input con selector de modo, modelo y esfuerzo poblados desde `configOptions` del agente (igual que la barra de tu captura: "Approve for me · Default · 5.6 Terra · Low · Fast mode").
- **Centro**: tabs + editor. Breadcrumb con path y símbolo actual.
- **Derecha**: árbol de archivos (colapsable), con archivos tocados por el agente en color "modificado" y sufijo `+N −M`, contador agregado en la raíz.
- **Abajo**: barra de estado minimalista (posición, lenguaje, LSP, agente activo, hunks pendientes).
- Sin terminal integrado en v1.

### Review inline (la feature)
- Líneas eliminadas: bloques fantasma sobre el hunk, fondo rojo oscuro (~15–20% sobre el fondo), texto atenuado con syntax highlighting, seleccionables.
- Líneas añadidas: fondo verde oscuro a ancho completo.
- Palabras cambiadas: sobre-resaltado más brillante (word diff), solo hunks ≤ 5 líneas.
- Pill flotante arriba-derecha del hunk: `✓ Accept | ✗ Reject | ✎ Edit`; al hover sobre cada línea, `+` / `−` para accept/reject por línea.
- Barra flotante abajo-derecha del editor (24 px de margen): `Accept Changes Ctrl+Enter · Reject Ctrl+Backspace · ↑ Alt+K ↓ Alt+J · 3/7 changes`, con spinner mientras el agente escribe.
- Gutter: barras verde/rojo propias (también para archivos nuevos y cambios ya commiteados, cosa que Antigravity no hace).
- Mientras el agente escribe: botones deshabilitados, diagnostics del LSP silenciados, autosave del usuario pausado en ese archivo.
- Panel "Review" (Ctrl+Shift+R): lista de archivos del turno con `+N −M`, Accept All / Reject All, navegación entre archivos con wrap.

### Chat
- Streaming markdown incremental (gpui-kit lo trae), bloques de código con copy, tool calls colapsables con estado (pending / running / done / failed), diffs de tool call como tarjetas clicables que llevan al hunk en el editor, plan del agente como checklist, pedidos de permiso como tarjeta con botones (Allow once / Always / Reject).
- Slash commands del agente (`available_commands_update`) con autocompletado.

### Tema y tipografía
- Dark por defecto, tema claro siguiendo el portal `Settings`. Paleta neutra tipo Zed One Dark / Ayu; los colores de diff son tokens del tema.
- Fuente UI: Inter o la del sistema; código: JetBrains Mono / Zed Mono con fallback del sistema. Ligaduras opcionales.

### Keybindings (Linux, sin choques con COSMIC/GNOME)

| Acción | Tecla | Solo si… |
|---|---|---|
| Accept hunk/línea bajo cursor | `Ctrl+Enter` | cursor en hunk pendiente (fallthrough a newline) |
| Reject hunk/línea bajo cursor | `Ctrl+Backspace` | idem (fallthrough a borrar palabra) |
| Accept / Reject archivo | `Ctrl+Shift+Enter` / `Ctrl+Shift+Backspace` | archivo con pendientes |
| Accept / Reject todo el turno | `Ctrl+Alt+Enter` / `Ctrl+Alt+Backspace` | — |
| Siguiente / anterior cambio | `Alt+J` / `Alt+K` (secundario `F7` / `Shift+F7`) | — |
| Siguiente archivo con cambios | `Alt+L` | — |
| Panel Review | `Ctrl+Shift+R` | — |
| Undo last reject | `Alt+Shift+U` | — |
| Expandir/colapsar hunk | `Ctrl+'` | — |

Nunca `Ctrl+Y` / `Ctrl+N` (choque con Redo / New File, documentado en Cursor y VS Code).

## 4. Arquitectura propuesta (workspace Cargo)

```
asteroid/
  crates/
    asteroid-app        binario, ventana, dock, settings, keymap
    asteroid-editor     editor propio (derivado de gpui-kit input/editor, Apache-2.0) + BlockMap + DiffHunkRenderer
    asteroid-review     ReviewState/ActionLog: base_text, hunks, accept/reject/rebase, persistencia
    asteroid-acp        cliente ACP: spawn de agentes, registry, sesiones, puente Send→!Send hacia gpui
    asteroid-chat       panel de chat: transcript, tool calls, permisos, config options
    asteroid-project    worktree, file tree, watcher, git status, buffers abiertos
    asteroid-syntax     tree-sitter grammars + temas
```

Dependencias clave: `gpui-pre` 0.3.5, `gpui-kit` 0.6.1, `agent-client-protocol` 2.1.0, `ropey`, `imara-diff`, `similar`, `tree-sitter` 0.27, `notify`, `gix`, `nucleo`, `rfd`, `tokio`.

## 5. Roadmap propuesto

- **M0 (1 semana): spikes.** (a) Ventana GPUI + gpui-kit en tu COSMIC/Wayland con GPU híbrida: arranque, IME, escala, copy/paste. (b) Cliente ACP mínimo en Rust que lanza `claude-agent-acp` y `codex-acp` con tu sesión, hace un prompt y loguea `tool_call` diffs. (c) Prototipo del BlockMap sobre el editor de gpui-kit vendorizado: una fila fantasma roja entre dos líneas reales. Si (c) fracasa, decidimos entre review view read-only o cambio de stack.
- **M1 (3–4 semanas): editor usable.** Tree, tabs, editor con highlighting, búsqueda, guardar, settings, tema, keymap.
- **M2 (3–4 semanas): chat + ACP completo.** Registry, sesiones, streaming, tool calls, permisos, modos/modelos, slash commands, cancel.
- **M3 (4–6 semanas): review inline.** ReviewState, hunks, accept/reject por hunk y línea, barra flotante, panel Review, persistencia, feedback al agente, rebase de ediciones del usuario.
- **M4: pulido y distribución.** LSP, git gutter, `install.sh`, `.deb`, CI con `cargo deny`.

## 6. Riesgos principales

| Riesgo | Mitigación |
|---|---|
| `gpui-pre` rompe API semanalmente | Pinear versión exacta; upgrade planificado mensual |
| BlockMap más caro de lo estimado | Spike M0(c) antes de comprometer; fallback review view read-only |
| Wayland en GPU híbrida AMD+NVIDIA | Spike M0(a); cascada Vulkan→GL; `ZED_DEVICE_ID`-like override |
| Adapters cambian comportamiento (Claude/Codex) | Diseño no depende de `fs/write`; releer disco + watcher |
| Contaminación GPL en gpui (#55470) | `cargo deny` desde el commit 1 |
| Rust 1.85 < MSRV 1.88 | `rustup update` |

## 7. Decisiones tomadas (2026-09-17, conversación con el autor)

- **D1 Framework**: GPUI + gpui-kit (la misma tecnología de Zed) con editor propio. Confirmado por defecto: el usuario quiere aspecto y sensación Zed.
- **D2 Alcance v1**: hunk + línea + archivo + turno. Edición manual dentro de un cambio pendiente: permitida (default), simplificable si complica.
- **D3–D12**: aceptados como defaults.
- **D13 Licencia**: **MIT**. Uso personal/local, sin fin comercial. No copiar código GPL de Zed para mantener el repo MIT limpio; sí copiar el diseño.
- **Layout visual**: aceptado (chat izquierda, editor centro, árbol derecha, sin terminal en v1, dark por defecto).
- **Principio de producto**: los cambios NUNCA se revisan en el chat. Siempre en el editor, segmento por segmento, aplicados primero y reversibles uno a uno. El chat solo enlaza al segmento.
