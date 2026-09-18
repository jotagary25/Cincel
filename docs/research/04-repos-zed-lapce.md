# Informe 04: Análisis de código de repos (Zed, gpui-component, Lapce, Helix, ACP SDK, adapters)

Fecha: 2026-09-17. Subagente Opus con clones shallow en clones locales shallow.

## 1. Zed / gpui

**gpui ya no usa Blade.** Renderer: **wgpu 29.0.4** (`crates/gpui_wgpu`, 5.3k LOC). gpui está partido en crates Apache-2.0: `gpui` (92k LOC), `gpui_platform` (selector), `gpui_linux` (15.8k LOC), `gpui_wgpu`, `gpui_macos`, `gpui_windows`, `gpui_web`.

**Linux/Wayland de primera clase**: `crates/gpui_linux/Cargo.toml` con features `wayland` (calloop-wayland-source, wayland-client/cursor/protocols, xkbcommon, ashpd) y `x11`, ambas por defecto. Hay `examples/layer_shell.rs`.

**Standalone: sí, oficialmente** (`crates/gpui/README.md`):
```toml
gpui = { version = "*" }
gpui_platform = { version = "*", features = ["font-kit", "wayland", "x11"] }
```
```rust
gpui_platform::application().run(|cx: &mut App| { ... });
```

**Publicación en crates.io (crítico):** `gpui` 0.2.2 está **congelado desde 2025-10-22**. El canal vivo es **`gpui-pre` 0.3.5 (2026-09-14)** publicado desde el repo de Zed, junto con `gpui-pre-platform`, `gpui-pre-web`, `gpui-pre-macros`.

### Licencias y LOC

| Crate | LOC `.rs` | Licencia |
|---|---:|---|
| `gpui` | 92 416 | Apache-2.0 |
| `gpui_linux` | 15 843 | Apache-2.0 |
| `gpui_wgpu` | 5 358 | Apache-2.0 |
| `sum_tree` | 3 327 | Apache-2.0 |
| `util`, `collections`, `refineable`, `scheduler`, `http_client`, `zlog` | — | Apache-2.0 |
| `editor` | 181 225 (49k tests) | GPL-3.0 |
| `project` | 116 994 | GPL-3.0 |
| `agent` | 88 654 | GPL-3.0 |
| `agent_ui` | 86 619 | GPL-3.0 |
| `language` | 27 631 | GPL-3.0 |
| `multi_buffer` | 17 655 | GPL-3.0 |
| `acp_thread` | 16 515 | GPL-3.0 |
| `languages` | 15 371 | GPL-3.0 |
| `terminal` (alacritty_terminal) | 10 352 | GPL-3.0 |
| `text` (CRDT/anchors) | 6 592 | GPL-3.0 |
| `agent_servers` | 6 194 | GPL-3.0 |
| `rope` | 4 405 | GPL-3.0 |
| `buffer_diff` | 4 395 | GPL-3.0 |
| `action_log` | 3 552 | GPL-3.0 |

Frontera limpia: framework Apache-2.0, editor GPL-3.0. `rope`, `text`, `buffer_diff`, `action_log` son GPL.

### El flujo de review en Zed

`AcpThread::write_text_file` (`crates/acp_thread/src/acp_thread.rs:4654`): calcula diff contra el snapshot compartido y lo aplica como edits al Buffer:
```rust
let old_text = snapshot.text();
text_diff(old_text.as_str(), &content).into_iter()
    .map(|(range, replacement)| (snapshot.anchor_range_inside(range), replacement))
```
```rust
buffer.start_transaction();
buffer.edit(edits, None, cx);
buffer.end_transaction_with_source(BufferEditSource::Agent, cx);
action_log.update(cx, |log, cx| log.buffer_edited(buffer.clone(), cx));
project.update(cx, |project, cx| project.save_buffer(buffer, cx)).await
```
**Toca disco** (`save_buffer`). La revisión es post-hoc contra una base en memoria. `read_text_file` registra `action_log.buffer_read()` y cachea el snapshot en `shared_buffers`.

**`ActionLog`** (`crates/action_log/src/action_log.rs`, 3 552 LOC), por buffer:
```rust
pub struct TrackedBuffer {
    buffer: Entity<Buffer>,
    diff_base: Rope,                 // texto "antes del agente"
    unreviewed_edits: Patch<u32>,    // edits pendientes, en filas
    status: TrackedBufferStatus,     // Created{existing_file_content} | Modified | Deleted
    diff: Entity<BufferDiff>,
    snapshot: text::BufferSnapshot,
}
```
- `keep_edits_in_range` (:641): no toca el buffer; avanza `diff_base` copiando el texto nuevo y elimina el edit de `unreviewed_edits`. **Aceptar = mover la base.**
- `reject_edits_in_ranges` (:712): `buffer.edit(edits_to_revert)` restaurando `old_text` desde `diff_base`, guarda `PerBufferUndo` (`undo_last_reject`) y `save_buffer`. Para `Created` borra el archivo solo si el contenido es 100% del agente; si el usuario editó encima, conserva ambos.
- `keep_all_edits`, `reject_all_edits`, `changed_buffers()`, `diff_stats()`, `stale_buffers()`.

**UI de review** (`crates/agent_ui/src/agent_diff.rs`, 2 352 LOC), dos modos:
1. `AgentDiffPane`: `MultiBuffer` + `SplittableEditor`; `update_excerpts` itera `action_log.changed_buffers(cx)`.
2. **Inline en el editor normal** (`AgentDiff::register_editor`, :1516): `multibuffer.add_diff(diff_handle)`, `editor.set_diff_hunk_renderer(agent_diff_renderer(...))`, `editor.set_expand_all_diff_hunks(cx)`. Keep/reject operan sobre selección → `editor.diff_hunks_in_ranges` → `action_log.keep_edits_in_range`.

**`buffer_diff`** usa `imara-diff` + `sum_tree`. `DiffHunk { range, buffer_range: Range<Anchor>, diff_base_byte_range, secondary_status, buffer_word_diffs, base_word_diffs }` (incluye word-level diff).

**Líneas borradas**: `MultiBuffer` tiene `diff_transforms: SumTree<DiffTransform>` con `DiffTransform::DeletedHunk` que materializa el texto base como filas reales del multibuffer. Ingeniería pesada y muy acoplada.

### Spawning de agentes ACP en Zed
Ya no hay `claude.rs`/`gemini.rs`. Hay registro remoto:
```rust
// crates/project/src/agent_registry_store.rs:20
const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
```
41 agentes, `distribution` de tipo `npx` o `binary` (por plataforma, con sha256).

| id | comando | versión | licencia |
|---|---|---|---|
| `claude-acp` | `npx @agentclientprotocol/claude-agent-acp@0.79.0` | 0.79.0 | proprietary |
| `codex-acp` | `npx @agentclientprotocol/codex-acp@1.12.0` | 1.12.0 | Apache-2.0 |
| `gemini` | `npx @google/gemini-cli@0.60.0 --acp` | 0.60.0 | Apache-2.0 |
| `github-copilot-cli` | `npx @github/copilot@1.0.85 --acp` | — | proprietary |

`AcpConnection::stdio` (`crates/agent_servers/src/acp.rs:807`) lanza el proceso con pipes, `BufReader::lines()` sobre stdout, y declara (`:764`):
```rust
acp::ClientCapabilities::new()
    .fs(acp::FileSystemCapabilities::new().read_text_file(true).write_text_file(true))
    .terminal(true)
    .auth(acp::AuthCapabilities::new().terminal(true))
    .elicitation(...)
```
Handlers en `connect_client_future` (:666-750). Como el SDK entrega closures `Send` y gpui es `!Send`, Zed reenvía por un canal `ForegroundWork` a un dispatch loop en el hilo foreground (`ClientContext`, :286-391). **Patrón a copiar.**

## 2. agent-client-protocol
`zed-industries/agent-client-protocol` → `agentclientprotocol/agent-client-protocol`, solo schema (Apache-2.0, JSON Schema v1 1.22.0 / v2 experimental). Runtime Rust en `agentclientprotocol/rust-sdk` → crate `agent-client-protocol` 2.1.0 (2026-09-04, Apache-2.0, 24k LOC). Zed pinea `= "2.1.0"` con `features = ["unstable"]`.

API: builder, no traits. Ejemplo `examples/v2_one_shot_client.rs`:
```rust
Client.v2().name("v2-one-shot-client")
  .on_receive_notification(async move |n: v2::UpdateSessionNotification, _c| { ... },
                           agent_client_protocol::on_receive_notification!())
  .on_receive_request(async move |r: v2::RequestPermissionRequest,
                                  responder: Responder<v2::RequestPermissionResponse>, _c| {
      responder.respond(v2::RequestPermissionResponse::new(v2::RequestPermissionOutcome::Cancelled))
  }, agent_client_protocol::on_receive_request!())
  .connect_with(agent, async move |connection| { ... }).await?;
```
tokio multi-thread; stdio en `src/stdio.rs` vía `blocking::Unblock`.

Schema clave (`schema/src/v1/`):
- `client.rs:1223` `WriteTextFileRequest { session_id, path: PathBuf, content: String, meta }` (contenido completo, no patch).
- `tool_call.rs:546` `enum ToolCallContent { Content(Content), Diff(Diff), Terminal(Terminal) }`
- `tool_call.rs:674` `Diff { path: PathBuf, old_text: Option<String>, new_text: String, meta }`

## 3. claude-agent-acp
npm `@agentclientprotocol/claude-agent-acp@0.79.0`, TypeScript ~58k LOC (`src/acp-agent.ts` 10 329 líneas). Deps: `@anthropic-ai/claude-agent-sdk@0.3.274`, `@agentclientprotocol/sdk@1.4.0`.

**NO enruta escrituras por `fs/write_text_file`** (método existe, sin llamadores). El SDK escribe a disco con `Write`/`Edit`; el adapter:
- intercepta con `canUseTool` (:7024) → `session/request_permission`;
- hooks `PostToolUse` + `src/diff.ts` convierten `structuredPatch` en `ToolCallContent::Diff` + `ToolCallLocation`;
- opcional `enableFileCheckpointing` + `src/file-change-audit.ts` (extensión `agentFileChangeReport`).

Opciones del query (:8037): `settingSources: ["user","project","local"]`, `cwd`, `permissionMode`, `canUseTool`, `mcpServers`, `pathToClaudeCodeExecutable: process.env.CLAUDE_CODE_EXECUTABLE ?? claudeCliPath()`.

**Auth**: `AuthMethod`s tipo `terminal` (:2039): `claude-ai-login` → `--cli auth login --claudeai`, `console-login` → `--cli auth login --console`; remoto `claude-login` → `claude /login`. Requiere que el cliente declare `auth.terminal`. **Asteroid necesita una terminal embebida para el login** (o delegar al terminal del sistema). Extensión `_auth/status_update` (`src/auth-status.ts`) empuja la identidad activa.

## 4. codex-acp
Registro: `@agentclientprotocol/codex-acp@1.12.0`. El clon de `zed-industries/codex-acp` (v0.16.0, Rust, 6.7k LOC, Apache-2.0) está desactualizado; depende de `codex-core`, `codex-login`, `codex-apply-patch` de `openai/codex`.
**Tampoco enruta escrituras por `fs/write_text_file`.** Parsea `apply_patch`/`FileChange` y los emite como `ToolCallContent::Diff` (`thread.rs:3427`, `:3797`).
**Auth** (`codex_agent.rs:464`): `ChatGpt` (OAuth navegador, suscripción), `CodexApiKey`, `OpenAiApiKey`, vía `AuthManager::shared`.

## 5. gpui-component (longbridge)
Apache-2.0, v0.6.1 en crates.io (2026-09-09), commit 2026-09-18, muy activo. Depende de **`gpui-pre` 0.3.5** con `gpui_platform` features `["font-kit","x11","wayland","runtime_shaders"]`.
- `crates/base/src/input/` = **24 857 LOC de editor real** (`base/state.rs` 9 370, `base/element.rs` 3 375, `display_map/text_wrapper.rs` 1 545).
- Rope: **`ropey` 2.0.0-beta.1**.
- Tree-sitter con grammars **estáticos por feature** (`tree-sitter-languages`: bash, c, cpp, css, go, html, java, javascript, json, rust, …). Seam desacoplado:
```rust
pub trait InputHighlighter {
    fn language(&self) -> SharedString;
    fn update(&mut self, edit: Option<InputEdit>, text: &Rope, folding: bool, window, cx);
    fn styles(&self, range: &Range<usize>, resolver: &dyn HighlightStyleResolver) -> Vec<(Range<usize>, HighlightStyle)>;
    fn fold_ranges(&self, text: &Rope) -> Vec<FoldRange>;
}
```
- **LSP incluido**: `editor/lsp/{completions,hover,code_actions,definitions,semantic_tokens,document_colors}.rs`, más `diagnostics.rs`, `indent.rs`, `auto_close.rs`, `search.rs`, `decorations.rs`.
- **No trae vista de diff ni multibuffer.** Eso lo construimos.

## 6. Alternativas descartadas

| Proyecto | LOC | Licencia | Último commit | Veredicto |
|---|---:|---|---|---|
| floem | editor-core 9 102 + views/editor 10 536 | MIT | 2026-06-22 | winit forkeado; estancado ~3 meses |
| lapce | 67 958 | Apache-2.0 | 2026-09-06 | Activo pero `tree-sitter 0.22.6`, rope propio, acoplado a floem |
| helix-core | 20 086 | MPL-2.0 | 2026-07-23 | Headless real pero no publicado en crates.io; grammars `.so` |
| cosmic-edit | 6 701 | GPL-3.0 | 2026-09-16 | Lógica en `cosmic-text`; syntect, no tree-sitter |

## Qué reutilizar y qué reescribir

### Reutilizar (crates.io, Apache-2.0)
1. **`gpui-pre` 0.3.5 + `gpui-pre-platform`** con `wayland`/`x11`. Nunca `gpui` 0.2.2. Pinear versión exacta.
2. **`gpui-component` 0.6.1**: ahorra ~25k LOC de editor (rope, wrap, multi-cursor, folding, tree-sitter, LSP, diagnostics). Mayor acelerador.
3. **`agent-client-protocol` 2.1.0**. Copiar el puente `Send`→`!Send` de `agent_servers/src/acp.rs:286-391`.
4. **Registro ACP** remoto para descubrir/lanzar agentes.
5. **`imara-diff`** para hunks; **`alacritty_terminal`** para terminal embebida (camino de login de Claude).

### Reescribir (portar la idea, no el código GPL)
6. **`ActionLog`**: `diff_base: Rope` por buffer + `unreviewed_edits: Patch`; accept = avanzar `diff_base`; reject = aplicar inverso + guardar; `TrackedBufferStatus::{Created, Modified, Deleted}`; undo del último reject; no borrar archivo creado si el usuario editó encima. ~800–1200 LOC propias.
7. **Capa de diff/hunks** sobre `imara-diff`: `DiffHunk { buffer_range, base_byte_range, kind }`. Evitar replicar `DiffTransform`; renderizar líneas eliminadas como bloques decorativos sobre el hunk (gpui-component soporta decorations/overlays).
8. **Review multi-archivo**: panel simple de un archivo a la vez + navegación, no multibuffer CRDT.

### Decisión de arquitectura sobre el review
Ni Claude ni Codex usan `fs/write_text_file`; Zed también persiste a disco. Por tanto:
- Declarar `fs.write_text_file(true)` igual (Gemini lo usa), pero no diseñar el review asumiéndolo.
- Capturar `diff_base` en `fs/read_text_file` y en el primer `Diff.old_text` de cada ruta.
- Para rutas tocadas sin pasar por nosotros: releer disco y diffear contra `diff_base` al cierre del tool call.
- Aceptar que los edits llegan a disco antes del review: el valor es el accept/reject per-hunk/línea post-hoc con reject = revert quirúrgico + save.

### Recomendación
**gpui-pre + gpui-component como base del editor; `agent-client-protocol` 2.1.0 como capa ACP; `ActionLog` + hunks reimplementados en casa.** Evita ~600k LOC GPL de Zed. Riesgo principal: `gpui-pre` es pre-1.0 con breaking changes frecuentes. Riesgo secundario: gpui-component no trae diff view, que es justamente el diferencial de Asteroid.

---

# Adenda 04-b: verificación directa del display map de gpui-kit (BlockMap ausente)

Verificado sobre el clon (commit d2e6bba, 2026-09-18), `crates/base/src/input/`:

```
Buffer (Rope) → WrapMap → FoldMap → DisplayMap      (README del módulo)
```
Archivos: `display_map.rs` (375 líneas), `fold_map.rs` (343), `wrap_map.rs` (162), `text_wrapper.rs` (1545), `folding.rs` (19). **No hay `block_map.rs`**; cero ocurrencias de `block|phantom|inlay|widget` en el display map. `TextDecoration { range, HighlightStyle }` solo colorea texto existente.

Comparación con `zed/crates/editor/src/display_map/`: `inlay_map.rs` 100 KB, `fold_map.rs` 92 KB, `tab_map.rs` 70 KB, `wrap_map.rs` 76 KB, **`block_map.rs` 236 KB**, `crease_map.rs` 17 KB. El BlockMap (filas virtuales para líneas borradas, headers, etc.) es el módulo más grande del display map de Zed.

Estructura del elemento de pintado de gpui-kit (`base/element.rs`, 3375 líneas incl. ~600 de tests): `calculate_visible_range` (:954), `layout_line_numbers` (:1021), `layout_lines` (:1378), `highlight_lines` (:1498), `layout_cursors` (:488), `layout_selections` (:889), `layout_fold_icons` (:1208), `prepaint` (:1788, ~400 líneas), `paint` (:2201, ~350 líneas). `display_row` aparece en 11 sitios; `line_height` en 100. `state.rs` tiene 9370 líneas.

**Estimación de agregar un BlockMap propio en un fork de gpui-kit:** nueva capa `FoldMap → BlockMap → DisplayMap` (~800–1500 LOC) + tocar `calculate_visible_range`, `layout_lines`, `layout_line_numbers` (saltar números en filas de bloque), scroll (row count), cursor/selección (impedir caret en filas fantasma), pintado de fondos de línea a ancho completo, y un slot de render para el contenido del bloque (~1000–1500 LOC de modificaciones). Total estimado 2–3k LOC y 3–5 semanas de trabajo con agentes, más mantenimiento del fork contra `gpui-pre` semanal. Viable; no trivial.

**Alternativa sin BlockMap para el MVP:** un elemento propio de "review view" read-only (renderizado con gpui directamente: filas con fondo rojo/verde, syntax highlighting reutilizando el `InputHighlighter`, botones por hunk) que reemplaza al editor en la pestaña mientras hay cambios pendientes, con toggle a edición normal. Mucho más barato (~1.5k LOC) pero no permite editar dentro del review (Antigravity sí lo permite).

**Conclusión revisada del stack:** GPUI + gpui-kit sigue siendo la mejor base *si* Asteroid es dueño de su editor: vendorizar `crates/base/src/input/editor` + `base` (Apache-2.0, ~25k LOC) en un crate propio y añadir BlockMap ahí, o bien empezar con el review view read-only y migrar. Tauri + CodeMirror 6 resuelve el problema hoy pero contradice los requisitos de Rust nativo, liviano y Zed-like.

---

# Adenda 04-c: cómo renderiza Zed realmente las líneas borradas, licencias y diff

## Zed usa "text splice", no block widgets, para el texto borrado
[PR #22994](https://github.com/zed-industries/zed/pull/22994) (2025-01-24, 240 commits) sacó las líneas borradas de las block decorations y las metió en el espacio de coordenadas del `MultiBuffer`:
```rust
pub struct MultiBufferSnapshot {
    excerpts: SumTree<Excerpt>,
    diffs: SumTree<DiffStateSnapshot>,
    diff_transforms: SumTree<DiffTransform>,   // capa de splice
}
enum DiffTransform {
    BufferContent { summary, inserted_hunk_info: Option<DiffTransformHunkInfo> },
    DeletedHunk   { summary, buffer_id, hunk_info, base_text_byte_range: Range<usize>, has_trailing_newline: bool },
}
```
Vive **debajo** del display map: el texto borrado es texto real (highlighting tree-sitter, selección, búsqueda, cursor, soft wrap gratis). El `BlockMap` queda para botones Keep/Reject, headers y spacers.

| | (i) Widget (CodeMirror, VS Code) | (ii) Text splice (Zed) |
|---|---|---|
| Highlighting del borrado | manual; CodeMirror lo capa a 3000 chars | gratis |
| Selección/búsqueda/copiar cruzando el borde | difícil | gratis |
| Cursor en texto borrado | imposible | funciona |
| UI arbitraria en la fila fantasma | trivial | necesita block layer igual |
| Costo | una capa de bloques | capa de transform + traducción de edits |

**Para Asteroid:** la capa de splice encaja naturalmente en un editor propio: `Rope → DiffTransformMap (inserta filas del base_text) → WrapMap → FoldMap → BlockMap (pill de botones)`. Las filas de texto borrado son read-only y se mapean a `base_text`; los edits del usuario se traducen saltándolas.

## Referencias adicionales
- CodeMirror `@codemirror/merge` 6.12.2: `unifiedMergeView` con `syntaxHighlightDeletions` (cap 3000), `acceptChunk`/`rejectChunk`, `goToNextChunk`. Las block decorations deben venir de un `StateField`, nunca de un `ViewPlugin`.
- Helix `helix-core::text_annotations::LineAnnotation` (`insert_virtual_lines`) reserva filas vacías; solo `InlineDiagnostics` lo implementa; Helix no muestra líneas borradas inline ([#9127](https://github.com/helix-editor/helix/issues/9127)).
- Lapce: diff con tabla LCS DP cuadrática en tiempo y memoria.

## Licencias de Zed, actualizado
- [PR #57948](https://github.com/zed-industries/zed/pull/57948) (2026-05-28): `collab` y `ztracing` relicenciados de AGPL a GPL. **Ya no hay AGPL en Zed**; CI `script/check-licenses` lo impide.
- Tally 245 crates: 204 GPL-3.0-or-later, 33 Apache-2.0, 8 sin campo.
- Publicados en crates.io (Apache-2.0, 0.2.2): `gpui`, `gpui-macros`, **`gpui_sum_tree`**, `gpui_collections`, `gpui_util`, `gpui_refineable`, `gpui_http_client`; `zed_extension_api` 0.7.0. **No publicados**: `gpui_platform`, `gpui_wgpu`, `gpui_apple`, `gpui_web`, `gpui_shared_string` → por eso el README de gpui no funciona y existe `gpui-pre`/`gpui-unofficial`.
- Closure de `crates/editor`: 96 crates first-party, 71 GPL (incluye `client`, `rpc`, `proto`, `db`, `language_model`, `remote`). 181k LOC, todo `publish = false`. Descartado con números.

## `action_log`, nombres exactos a copiar
`TrackedBuffer` = `BufferDiff` con base pre-agente. Hooks: `buffer_read`, `buffer_created`, `buffer_edited`, `will_delete_buffer`. Ops: `keep_edits_in_range`, `reject_edits_in_ranges`, `keep_all_edits`, `reject_all_edits`, `undo_last_reject` con `PerBufferUndo { edits_to_restore: Vec<(Range<Anchor>, String)>, status }`. Hunks en `SumTree<InternalDiffHunk>` anclado → `hunks_intersecting_range` O(log n).
