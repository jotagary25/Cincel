# Informe 03: UX e implementación del review inline de cambios de agentes

Fecha: 2026-09-17. Subagente Opus.

## 0. Veredicto sobre la premisa de Zed

**Es falso que Zed solo muestre cambios en el chat, pero la percepción del usuario es razonable.** Zed tiene review inline en el editor normal (hunks rojo/verde + Keep/Reject), vía key context `Editor && editor_agent_diff`. Pero está **desactivado por defecto**: `agent.single_file_review` pasó a `false` en Zed 0.225.9 (feb 2026) porque el diff del agente *sobrescribe* el diff de git (un editor solo muestra una base a la vez). Out of the box solo se ve el multibuffer "Review" (`agent::OpenAgentDiff`, `ctrl-shift-r`). Y está roto/parcial: [#61626](https://github.com/zed-industries/zed/issues/61626) (sin efecto con vista split), [#62147](https://github.com/zed-industries/zed/issues/62147) (Keep/Reject no responden), [#49856](https://github.com/zed-industries/zed/issues/49856) (cerrado not planned), [#58939](https://github.com/zed-industries/zed/issues/58939) (el review no sobrevive un reinicio). Demanda explícita de UX tipo Cursor para ACP: [discussion #53169](https://github.com/zed-industries/zed/discussions/53169).

Nota: el usuario tiene `single_file_review: true` en su settings.json de Zed, y aun así lo percibe insuficiente.

**Hueco real de producto**: review inline por defecto, robusto, por línea, coexistiendo con el diff de git.

## 1. Tabla comparativa

| Producto | Granularidad | Dónde vive la UI | Disco durante review | Edición del usuario durante review | Multi-archivo / multi-turno |
|---|---|---|---|---|---|
| Cursor (2026) | hunk + archivo + turno | Inline + barra por archivo + barra inferior | **Escribe a disco de inmediato**; Keep confirma, Undo revierte | Permitido; diff se recalcula | Keep All/Reject All + Next/Prev File; checkpoints por prompt |
| VS Code + Copilot | hunk (innerChanges a nivel palabra) + archivo + sesión | Inline (overlay + hunk toolbar) + multi-diff editor | Autosave al terminar el stream | `_allEditsAreFromUs=false` marca edición humana | Working set, acceptAllFiles, persistencia en disco |
| Windsurf / Cascade | hunk (diff zones) y línea | Inline, on por defecto | Escribe; revert por step/checkpoint | Permitido | Revert a paso anterior |
| **Antigravity** | **línea** (hover ±) + hunk (✓/✗/✎) + archivo | Inline + panel Review Changes + lista `[+12/−3]` | Reject "revierte el archivo a su estado previo" | Permite Edit Chunk antes de aceptar | Accept All / Reject All global; inline y side-by-side |
| Zed | hunk + archivo + thread | Multibuffer Review (default) + inline opt-in | **Escribe a disco inmediatamente** | `apply_non_conflicting_edits` rebasa ediciones del usuario a la base | KeepAll/RejectAll, UndoLastReject |
| Cline / Roo Code | archivo (Save/Revert) | Pestaña de diff separada | No escribe hasta aprobar | Interfiere | Checkpoints git |
| Continue.dev | bloque (vertical diff) | Inline | En buffer | Sí | estado por bloque |
| Aider | commit por edición | Terminal / git | Escribe + commitea | N/A | `/undo` |
| JetBrains Junie | archivo | Panel Done + diff viewer | Escribe directo | N/A | Revert por archivo |
| Claude Code VS Code ext | archivo | Diff side-by-side | Pide permiso antes | Editás la propuesta y se avisa al modelo | Auto-accept mode |
| Codex IDE ext | ninguna | Chat | Aplica directo | — | — |

**Estándar de facto**: escribir a disco inmediatamente + snapshot para revertir, review inline por hunk. Solo Cline/Roo/Junie usan approval-first, y es lo que la gente pide cambiar.

## 2. Modelo de implementación

### 2.1 Algoritmo canónico (Zed y VS Code convergen)
- Estado = `base_text` (snapshot, "lo que el usuario ya aceptó") + `buffer` real.
- El diff se **recalcula** (async, background) en cada cambio de cualquiera de los dos, descartando resultados obsoletos por versión.
- **Accept hunk** → escribir el texto nuevo del hunk dentro de `base_text` (avanza la base). Sin I/O.
- **Reject hunk** → escribir el texto viejo del hunk dentro del buffer (revierte). Guardar a disco.

VS Code (`chatEditingTextModelChangeService.ts`):
```ts
// keep: originalRange := modifiedText
edits.push(EditOperation.replace(edit.originalRange, this.modifiedModel.getValueInRange(edit.modifiedRange)));
// undo: modifiedRange := originalText
this.modifiedModel.pushEditOperations(null, [EditOperation.replace(edit.modifiedRange, orig)], ...);
```
Zed `keep_edits_in_range`: `tracked_buffer.diff_base.replace(old_range, &new_text)` con `delta` acumulado; `reject_edits_in_ranges`: `buffer.edit(edits_to_revert)` + `save_buffer`.

### 2.2 Modelo de datos recomendado (Rust)
```rust
pub struct ReviewState {
    pub base: Rope,                 // texto "aceptado"; = contenido al primer write del turno
    pub hunks: Vec<Hunk>,           // derivado, recomputado
    pub status: FileStatus,         // Created{prev: Option<Rope>} | Modified | Deleted{prev: Rope}
    pub turn_id: TurnId,
    pub buffer_version: u64,
    pub base_version: u64,
}
pub struct Hunk {
    pub base_rows: Range<u32>,      // filas en `base`
    pub buf_anchors: Range<Anchor>, // anclas en el buffer vivo
    pub kind: HunkKind,             // Added | Modified | Deleted
    pub word_diffs: (Vec<Range<usize>>, Vec<Range<Anchor>>),
}
```
**Diff:** `imara-diff` con `Algorithm::Histogram` + `postprocess_lines()` (igual que Zed). El postproceso (heurística slider/indent de git) estabiliza hunks ambiguos. Word-diff intra-línea solo para hunks cortos (Zed: `MAX_WORD_DIFF_LINE_COUNT = 5`).

**Anclas vs recomputar:** recomputar el line-diff completo en cada tecleo es O(N); para <5k líneas es imperceptible si es **debounced (~50 ms) en background**, descartando por versión. No implementar tracking incremental al principio. Sí hacen falta **anclas** (posiciones que se transforman con cada edición) para que los botones no salten y `reject` aplique en el rango correcto. Con `ropey`: anclas `(offset, Assoc::Before|After)` transformadas en cada edición; no hace falta CRDT.

### 2.3 Operaciones
```
accept_hunk(h):   base.replace(rows_to_bytes(base, h.base_rows), buffer.text(h.buf_anchors)); recompute();
reject_hunk(h):   buffer.edit(h.buf_anchors, base.slice(h.base_rows)); recompute(); save_to_disk();
accept_all():     base = buffer.rope().clone(); hunks.clear();
reject_all():     buffer.set_text(base); save_to_disk();  // + delete_file si status==Created
```
En batch, procesar de abajo hacia arriba o llevar `delta`.

### 2.4 Accept/reject por línea (estilo Antigravity)
Un hunk `Modified` con base `[b0..bn)` y buffer `[c0..cm)` se descompone en pares emparejando líneas por word-diff interno (o por índice si `n == m`):
```
split_hunk(h) -> Vec<LinePair { base_row: Option<u32>, buf_row: Option<u32> }>
accept_line(p): base.replace_line(p.base_row, buffer.line(p.buf_row))   // inserta si base_row=None
reject_line(p): buffer.replace_line(p.buf_row, base.line(p.base_row))   // borra si base_row=None
```
Tras cada operación se recomputa el diff y el hunk se re-parte solo.

### 2.5 Ediciones concurrentes del usuario
Zed: `apply_non_conflicting_edits` (`action_log.rs:1141`). Cuando el cambio viene **del usuario**, se aplica también a `base_text` si no intersecta ningún hunk pendiente. Si intersecta, se marca conflicto y no se aplica. Sin esto, todo lo que el usuario escriba aparecería como cambio del agente. **Implementar desde el día uno.** Requiere `EditSource { User, Agent }` en cada transacción del buffer.

## 3. Integración con el agente (ACP)
- ACP v1 solo tiene `fs/read_text_file` y `fs/write_text_file`. Sin delete ni rename: llegan por shell y no pasan por el review (mismo bug de Cursor: [forum #162348](https://forum.cursor.com/t/agent-edits-made-via-the-terminal-script-shell-writes-dont-appear-in-the-inline-edit-review-ui/162348)). `write_text_file` es fire-and-forget.
- Patrón de Zed en `write_text_file`: abrir buffer; diff contra **el snapshot que el agente leyó** (`shared_buffers`), no contra el buffer actual; `buffer.edit` en una transacción `Agent`; notificar action log; format opcional; `save_buffer`.
- **Recomendación: escribir a disco inmediatamente.** (a) terminal/tests/LSP deben ver el código nuevo; (b) es lo que hacen Cursor, Windsurf, VS Code y Zed; (c) approval-first bloquea al agente. Corolario: `fs/read_text_file` sirve el buffer, invalidando buffers tras comandos de terminal ([zed#49162](https://github.com/zed-industries/zed/issues/49162)).
- **Feedback de rechazos** (el protocolo no lo cubre): 1) `read_text_file` devuelve el contenido revertido; 2) inyectar en el siguiente prompt una nota `<user_rejected_edits>` con path + rangos; 3) si el usuario rechaza tras tests, marcar el turno como "parcialmente rechazado" en la nota.

## 4. Diseño visual (replicar Antigravity)
- **Líneas eliminadas**: bloques phantom sobre el hunk, fondo rojo oscuro (~`#3a1d1d`, 15–20% alpha sobre el fondo), texto atenuado con syntax highlighting. Zed las materializa como filas reales (seleccionables/buscables).
- **Líneas añadidas**: fondo verde oscuro (~`#16301c`) a todo el ancho.
- **Palabras cambiadas**: sobre-resaltado más brillante (~35%) por word-diff; solo hunks ≤5 líneas.
- **Pill flotante** arriba-derecha del hunk: `✓ Accept | ✗ Reject | ✎ Edit`; hover por línea: `+` / `−`.
- **Barra inferior flotante**: `Accept Changes ⏎ · Reject ⌫ · ↑ Alt+K ↓ Alt+J · 3/7 changes`.
- **Gutter**: barras verde/rojo clicables para expandir/colapsar.
- **File tree**: nombre en color "modificado" + `+12 −3`; contador agregado.

## 5. Keybindings recomendados (Linux, sin chocar con GNOME)
GNOME reserva `Super*`, `Ctrl+Alt+←/→/↑/↓`, `Alt+F2/F4/F7/F8/F10`, `Ctrl+Alt+T`, `Alt+Tab`.

| Acción | Recomendado | Alternativa | Referencia |
|---|---|---|---|
| Accept hunk | `Ctrl+Enter` | `Alt+Y` | Antigravity/Windsurf; Zed `alt-y` |
| Reject hunk | `Ctrl+Backspace` | `Alt+Z` | Antigravity; Zed `ctrl-alt-z` |
| Accept file | `Ctrl+Shift+Enter` | `Alt+Shift+Y` | VS Code `Ctrl+Shift+Y` |
| Reject file | `Ctrl+Shift+Backspace` | `Alt+Shift+Z` | Cursor |
| Accept todo el turno | `Ctrl+Alt+Enter` | — | VS Code `Ctrl+Alt+Y` |
| Reject todo el turno | `Ctrl+Alt+Backspace` | — | — |
| Next change | `Alt+J` | `F8` | Antigravity; Zed `alt-.` |
| Prev change | `Alt+K` | `Shift+F8` | Antigravity; Zed `alt-,` |
| Abrir panel Review | `Ctrl+Shift+R` | — | Zed |
| Undo last reject | `Alt+Shift+U` | — | Zed |
| Expandir/colapsar hunk | `Ctrl+'` | — | Zed |

**Crítico:** `Ctrl+Backspace`/`Ctrl+Enter` ligados solo bajo key context (`editor_agent_review` + cursor dentro de hunk pendiente), con fallthrough a borrar palabra / nueva línea.

## 6. Edge cases

| Caso | Recomendación |
|---|---|
| Archivo nuevo | `Created{prev: None}`; todo verde. Reject → borrar, **solo si el usuario no lo editó**. |
| Borrado / rename | No llegan por ACP; detectarlos por file-watcher como `Deleted{prev}` / delete+create. |
| Archivo no abierto | Abrir buffer en background; el review vive en el modelo, no en la vista; mostrar `+N −M` en el árbol. |
| Cambios sin guardar | Preguntar antes de que el agente escriba (Zed: `DirtyBufferDecision{Save, Discard, Keep}`); comparar mtime. |
| Misma región dos veces en un turno | Nada especial: el diff contra `base` fusiona. El turno define la base. |
| Usuario edita dentro del hunk | `apply_non_conflicting_edits`. Nunca bloquear la escritura del usuario. |
| Archivos grandes | Cortar review por tamaño (>2 MB o >50k líneas): resumen + diff en pestaña separada. |
| Binarios | Sin review inline; Accept/Reject a nivel archivo. |
| CRLF / BOM / newline final | Normalizar a LF en memoria y reaplicar EOL al guardar. Nunca dejar que el EOL genere un hunk fantasma. |
| Fuera del workspace | Rechazar `fs/write_text_file` con error de protocolo. |
| Git | **No pisar el diff de git** (error que obligó a Zed a apagar `single_file_review`). Estilo propio para el agente, git en gutter, o toggle. Bonus: si el usuario commitea un hunk pendiente, marcarlo aceptado. |
| Persistencia tras reinicio | **Sí.** Esquema VS Code: `.asteroid/review/<workspace>/state.json` + `contents/<sha256>` con `base_text`. |
| Undo/redo | La edición del agente es **una transacción** en el undo stack (Ctrl+Z la deshace). Ctrl+Z **no** deshace un accept. `Alt+Shift+U` = undo last reject con toast. |

## Fuentes
- Zed: [buffer_diff.rs](https://github.com/zed-industries/zed/blob/main/crates/buffer_diff/src/buffer_diff.rs), [action_log.rs](https://github.com/zed-industries/zed/blob/main/crates/action_log/src/action_log.rs), [acp_thread.rs](https://github.com/zed-industries/zed/blob/main/crates/acp_thread/src/acp_thread.rs), [agent_diff.rs](https://github.com/zed-industries/zed/blob/main/crates/agent_ui/src/agent_diff.rs), [default-linux.json](https://github.com/zed-industries/zed/blob/main/assets/keymaps/default-linux.json), [docs Agent Panel](https://zed.dev/docs/ai/agent-panel)
- VS Code: [chatEditingTextModelChangeService.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/chat/browser/chatEditing/chatEditingTextModelChangeService.ts), [chatEditingSessionStorage.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/chat/browser/chatEditing/chatEditingSessionStorage.ts)
- Cursor: [docs/agent/review](https://cursor.com/docs/agent/review), [foro](https://forum.cursor.com/t/why-does-cursor-apply-diffs-immediately-before-the-user-accepts-them/145070)
- Windsurf: [Agent diff zones](https://docs.windsurf.com/windsurf/advanced) · Antigravity: [Review Changes](https://antigravity.google/docs/ide/review-changes-editor/)
- ACP: [File System](https://agentclientprotocol.com/protocol/v1/file-system) · [imara-diff](https://github.com/pascalkuthe/imara-diff) · [Kiro #8968](https://github.com/kirodotdev/Kiro/issues/8968)

---

# Adenda 03-b: correcciones y hallazgos nuevos (Cursor / VS Code)

## 1. Keybindings corregidos
Bindings reales de Cursor (confirmados por staff, [thread 171421](https://forum.cursor.com/t/can-you-review-agent-edits-in-page-order/171421)):

| Acción | Cursor Linux | Cursor mac | VS Code |
|---|---|---|---|
| Accept hunk | `Ctrl+Y` | `Cmd+Y` | `Ctrl/Cmd+Y` |
| Reject hunk | `Ctrl+N` | `Cmd+N` | `Ctrl/Cmd+N` |
| Next change | **`Alt+J`** | `Opt+J` | `Alt+F5` |
| Prev change | **`Alt+K`** | `Opt+K` | `Shift+Alt+F5` |
| Keep file | `Ctrl+Enter` | `Cmd+Enter` | `Ctrl/Cmd+Shift+Y` |
| Reject all | `Ctrl+Shift+Backspace` | `Cmd+Shift+Backspace` | `Ctrl/Cmd+Backspace` (chat input) |
| Keep all | sin binding | — | `Ctrl/Cmd+Alt+Y` |

`Alt+J`/`Alt+K` para next/prev lo usan Cursor y Antigravity: mantener.
**No copiar `Ctrl+Y`/`Ctrl+N`**: en Cursor `Ctrl+N` (New File) rechaza el hunk y borra archivos enteros sin undo ([#160957](https://forum.cursor.com/t/cmd-n-shortcut-conflict/160957)); en VS Code lo reportó su propio ingeniero ([vscode#284845](https://github.com/microsoft/vscode/issues/284845)); `Ctrl+Y` = Redo en Linux. La tabla original (`Ctrl+Enter`/`Ctrl+Backspace` bajo key context, `Alt+Y`/`Alt+Z` alternativa) es la correcta.

## 2. Ediciones concurrentes: VS Code hace OT real
`ChatEditingTextModelChangeService._mirrorEdits`:
```ts
e_user.tryRebase(e_ai.inverse(original))
//   éxito → aplicar también al original (shadow) + rebaseSkipConflicting del set del agente
//   fallo  → e_ai.compose(e_user): el texto del usuario pasa a formar parte del hunk pendiente
```
Más robusto que `apply_non_conflicting_edits` de Zed (por solapamiento de filas). **Recomendación:** implementar el modelo de Zed en v1, pero estructurar como `rebase(user_edit, pending_agent_edits) -> Result<Rebased, Conflict>` para poder subir a OT después.
Mientras el agente escribe (`isCurrentlyBeingModifiedBy`), VS Code: **desactiva autosave**, **filtra diagnostics del LSP**, **deshabilita botones accept/reject**. Copiar las tres.

## 3. Dirección de la industria
Las sesiones Agent Host de VS Code (default 2026) aplican y guardan sin estado pendiente; Keep/Undo per-hunk quedó legacy. Cursor la mantiene y sus usuarios la llaman "su mejor ventaja de UX" ([thread 160856](https://forum.cursor.com/t/bring-back-per-change-apply-inline-diff-review-you-re-throwing-away-your-best-ux-advantage/160856)). **Para Asteroid:** el review debe coexistir con un camino git/diff, y conviene un modo "auto-keep" configurable.

## 4. Persistencia: advertencia de pérdida de datos
Cursor re-aplica a disco los pending edits al arrancar y ha producido truncación a 0 bytes y a 524288 bytes ([#171408](https://forum.cursor.com/t/feature-setting-to-disable-undo-create-diff-prevents-0-byte-524288-truncation/171408)), reescritura de EOL ([#169140](https://forum.cursor.com/t/regression-3-17-8-windows-dirty-agent-review-cache-rewrites-file-eol-on-every-workspace-open/169140)), diffs perdidos ([#163021](https://forum.cursor.com/t/review-of-changes-done-in-agent-chat-window-disappears-after-cursor-restart/163021)) y diffs fantasma ([#161718](https://forum.cursor.com/t/edited-files-queue-polluted-by-prior-resolved-changes/161718)).
**Regla:** al restaurar, **nunca reescribir el archivo**. Persistir solo `base_text` (content-addressed) + metadatos. Al abrir, leer disco tal cual, recomputar diff contra `base_text`; si el hash actual ≠ `current_hash` guardado, **descartar el review de ese archivo**. Modelo VS Code (`chatEditingSessions/{state.json, contents/<hash>}`), no Cursor.

## 5. Otros detalles adoptables
- Overlay de VS Code: `position: absolute; bottom: 24px; right: 24px` con spinner + "N of M" + Keep/Undo/Review.
- **Formatters solo al aceptar** (VS Code guarda con `skipSaveParticipants: true` durante el stream).
- **Auto-accept con countdown cancelable** (`chat.editing.autoAcceptDelay`), hover cancela.
- Cursor/VS Code no hacen word-diff intra-línea en el editor; Zed sí (`MAX_WORD_DIFF_LINE_COUNT = 5`). **Diferenciador barato.**
- Exponer todas las acciones como comandos desde el día uno (Cursor no expone Keep All).
- Cursor pierde el review al cerrar el chat. **Atarlo al workspace + `turn_id`, no a la conversación.**
- Límite configurable de archivos por sesión (VS Code: 10 por defecto).

---

# Adenda 03-c: Windsurf/Antigravity/Cline/Continue/JetBrains y detalles de Cursor/VS Code

## 1. Antigravity: los gutter markers son VCS, no capa de AI-diff
Los agentes escriben a disco y los marcadores de gutter/overview-ruler son decoraciones VCS normales. Los archivos nuevos (untracked) no muestran indicador ([forum](https://discuss.ai.google.dev/t/overview-ruler-diff-highlighting-for-agent-changes-in-antigravity/123383)). **Asteroid tiene `base_text` propio: puede pintar markers también para untracked y para cambios ya commiteados.** Diferenciador gratis.

## 2. Keybindings de la barra flotante de Antigravity no están documentados
Command IDs reales: `antigravity.prioritized.agentAcceptFocusedHunk`, `antigravity.agent.acceptAgentStep`, `antigravity.agent.acceptAllAgentSteps`. Linaje: Windsurf v2.10.7 (dic-2025) "Revamped the Cascade bar UI and added keyboard shortcut support". Trampa: `acceptAgentStep` está gated en `!editorTextFocus`. **Nuestras acciones deben funcionar con foco en el editor.**

## 3. Cline/Roo sí tocan disco
`DiffViewProvider.open()` hace `fs.writeFile(path, "")` para archivos nuevos antes de aprobar. Cline v4.x sustituyó esto por preview read-only y regresionó ([#11934](https://github.com/cline/cline/issues/11934), [PR #13417](https://github.com/cline/cline/pull/13417)): con CRLF el preview no abría y el executor aplicaba igual. **Lección: el matching de texto viejo debe ser CRLF-agnóstico o se pierde la review en silencio.**

## 4. Feedback loop al agente: mecanismo de Cline/Roo
Al guardar computan `createPrettyPatch(newContent, preSaveContent)` y dicen al modelo: **"The user made the following updates to your content:"**, con `autoFormattingEdits` separados. Cubre rechazos y ediciones manuales. **Adoptar tal cual**: en el siguiente prompt, por archivo, patch unificado del delta entre lo que escribió el agente y lo que quedó tras el review, más línea separada para formateo. Continue no reporta ediciones del usuario (carencia).

## 5. JetBrains AI Assistant como referencia
Review-then-write hasta Apply; luego estado inline con Next/Previous Change, Accept All/Discard All, Revert por chunk en gutter. Navegación `F7`/`Shift+F7`. **La red de seguridad real es el gutter VCS normal** (Rollback por chunk, `Ctrl+Alt+Z`). Validación de "no pisar el diff de git": modelar Accept/Reject como una *variante* del mismo widget de hunk de git con distinto renderer (Zed: `trait DiffHunkRenderer`, `DefaultDiffHunkRenderer` vs `AgentDiffHunkRenderer`). Añadir `F7`/`Shift+F7` como binding secundario.

## 6. Continue: detalles
- Todo el stream es UNA unidad de undo (`undoStopBefore:false, undoStopAfter:false`).
- Nunca guarda durante streaming; `saveFile()` en accept y reject. Efecto feo: a mitad de review el disco tiene contenido viejo y `Ctrl+S` persiste una mezcla. Nuestra decisión de escribir a disco inmediatamente elimina esa clase de bug.
- Keybindings: `Shift+Ctrl+Enter` accept / `Shift+Ctrl+Backspace` reject (con when-clause), `Alt+Ctrl+Y`/`Alt+Ctrl+N` por bloque (globales, mala práctica).
- Accept por bloque sin args = bloque superior restante (se mastica de arriba abajo). Modelo simple para v1 si no hay "hunk bajo el cursor".

## 7. Misceláneos
- Windsurf se rebrandeó a "Devin Desktop"; Cascade es legacy. Reverts de checkpoint irreversibles. Nosotros: `Undo last reject`.
- Windsurf v2.2.10: bug de decoraciones desincronizadas tras revert → confirma que son capa separada sobre archivos ya escritos.
- Antigravity tiene comentarios estilo Google Docs sobre diffs (Zed va igual: [#59157](https://github.com/zed-industries/zed/issues/59157), `editor::SubmitDiffReviewComment`). Feature v2.
- Antigravity autonomy: "Always Proceed" / "Agent Decides" / "Request Review" (tres valores, no booleano).
- Claude Code checkpoints: uno por prompt, 100 por sesión, ~30 días; no trackean Bash, subagentes, externos; mensaje "Restored the code, but skipped N files". Copiar esa honestidad en la UI.

## 8. Detalles adicionales VS Code / Cursor (delta)
- VS Code: "Pending means that you can keep or undo the saved edit. It doesn't mean that the edit is waiting to be written to disk."
- VS Code: `chatEditing.acceptAllFiles` = `Ctrl+Enter` y `discardAllFiles` = `Ctrl+Backspace` **solo con foco en el chat input**; pesos de keybinding hunk = `WorkbenchContrib + 1`, archivo = `+10` "win over new-window-action".
- VS Code: diff con `ignoreTrimWhitespace: false` ("NEVER ignore whitespace so that undo/accept edits are correct").
- VS Code: si las ediciones del usuario devuelven el archivo a `initialContent`, el estado pasa a Rejected automáticamente.
- VS Code: undo stack con elemento etiquetado `Chat Edit: '<prompt>'` por request.
- VS Code: navegación wrap entre archivos; `chat.editing.revealNextChangeOnResolve` abre el siguiente archivo al llegar a 0 cambios.
- VS Code: `chat.tools.edits.autoApprove` (glob→bool) = aprobación **pre-escritura** por archivo sensible (`.env`, `.vscode/*.json`). Buena idea para nuestro permission gate.
- VS Code: staging en SCM acepta pending edits automáticamente; discard los descarta.
- VS Code: `startExternalEdits/stopExternalEdits` = hook para que ediciones vía terminal/CLI entren al modelo de review.
- Cursor: asimetría de plataforma (Windows accept = `Ctrl+Shift+Y`). "Review Next File" = `Alt+L`. Setting "Jump to Next Diff on Accept".
- Cursor: switch maestro "Inline Diffs" OFF = auto-keep sin UI; "Review Control Location" = Breadcrumb vs Island.
- Cursor: **no hace rebase de ediciones del usuario** (el agente pisa ediciones manuales entre turnos; mitigación por prompt). Review muestra diffs cacheados obsoletos.
- Cursor: Run Modes no gatean ediciones de archivos (solo shell/MCP/fetch). Único gate pre-escritura: External-file edit protection.
- Cursor: diffs son text models nunca evictados → OOM 4 GB ([thread](https://forum.cursor.com/t/glass-renderer-oom-hidden-diffs-review-changes-and-agent-transcripts-never-leave-memory/172039)). Liberar `base_text` al resolver.
- Cursor: hilo de 236 posts abierto desde 2026-02: solo el primer cambio de un run multi-edit muestra diff ([thread](https://forum.cursor.com/t/only-some-diffs-are-shown-whhen-agent-makes-changes/152099)). Nuestro diseño (diff contra `base` recomputado, no por tool call) evita esa clase.
- Cursor: "next" calculado desde el cursor de texto; click con mouse en Keep no mueve el cursor → salta cambios. Mantener índice de hunk enfocado propio.
- Cursor: sin `+N/−M` en el explorer.

---

# Adenda 03-d: los tres modelos arquitectónicos de review (marco de referencia)

| Modelo | Herramientas | Consecuencia |
|---|---|---|
| **A. Write-then-review** (bytes a disco, review = revert) | Windsurf/Cascade, Antigravity, Aider, Junie, Claude Code en `acceptEdits`/`auto`, Codex IDE, Cursor, VS Code chat edits, Zed | Reject = revertir contenido ya escrito. Git ve el cambio antes de aprobarlo. |
| **B. Buffer-then-review** (buffer sucio, disco intacto) | Cline/Roo 3.x (solo contenido), Continue.dev | Disco tiene contenido viejo a mitad de review; `Ctrl+S` persiste un estado mixto. |
| **C. Propose-then-write** (nada ocurre hasta aprobar) | Claude Code en Manual, JetBrains AI Assistant chat | El diff es propuesta; el rechazo no genera evento de filesystem. |

Asteroid = **modelo A** por obligación (Claude/Codex escriben a disco) y por convicción (es el estándar), con `base_text` propio para que el review funcione también en archivos untracked y sin depender de git. Detalles adicionales: Codex IDE aplica directo sin preview (confirmado por OpenAI Support, 2026-09-08); Claude Code VS Code en Manual muestra diff side-by-side por tool call sin per-hunk ni Accept All ([#33932](https://github.com/anthropics/claude-code/issues/33932)); Claude Code permite editar la propuesta en el diff y se lo informa al modelo; los modos de permiso de Claude Code en 2026 son `default` ("Manual") / `acceptEdits` / `plan` / `auto` / `dontAsk` / `bypassPermissions`, con rutas protegidas (`.git`, `.claude`) nunca auto-aprobadas.

---

# Adenda 03-e: la limitación técnica de Zed con agentes ACP (verificada en código)

El sistema de review de Zed (`ActionLog`) solo se entera de un cambio cuando el agente escribe a través de `fs/write_text_file` (`acp_thread.rs:4721` y `:4739`, únicas llamadas a `buffer_edited` en el flujo ACP). El contenido `Diff` de un tool call se convierte en un `Diff::finalized` que se **renderiza en el chat** (`acp_thread.rs:2034`) y **nunca toca `action_log`** (`diff.rs` no lo referencia). Como los adapters de Claude y Codex escriben a disco por sus propias tools y solo reportan `Diff` informativos, para esos agentes el review inline (`single_file_review`) y la pestaña Review de Zed quedan vacíos aunque estén activados: los cambios solo se ven en el chat. Esa es la "limitación técnica con ACP" que reportó el usuario.

**Asteroid la evita por diseño**: no depende de que el agente avise por `fs/write_text_file`; captura la base al primer contacto con el archivo y detecta los cambios reales releyendo el disco al cerrar cada tool call de edición, con file watcher y `agentFileChangeReport` como respaldo (ver 01-acp §1 y síntesis D5).
