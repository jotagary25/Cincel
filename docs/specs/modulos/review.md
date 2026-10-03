# Módulo `cincel-review`

El modelo de la revisión de cambios del agente. Sin GPUI. Es el crate más importante y el más testeado.

## Tipos
```rust
pub struct ReviewStore { files: HashMap<PathBuf, FileReview>, undo_stack: Vec<RejectUndo>, .. }

pub struct FileReview {
    pub path: PathBuf,
    pub base: Rope,                    // texto "aceptado": lo que había antes del agente, avanzado por cada accept
    pub status: FileStatus,            // Modified | Created { previous: Option<Rope> } | Deleted { previous: Rope }
    pub turn_id: TurnId,               // último turno que lo tocó
    pub hunks: Vec<Hunk>,              // derivado; recomputado
    pub base_version: u64, pub buffer_version: u64,
}

pub struct Hunk {
    pub id: HunkId,
    pub base_rows: Range<u32>,         // filas en `base` (vacío si solo añade)
    pub buffer_range: Range<Anchor>,   // en el buffer vivo (vacío si solo elimina)
    pub kind: HunkKind,                // Added | Deleted | Modified
    pub word_diffs: Option<WordDiffs>, // solo si ambos lados ≤ 5 líneas
    pub lines: Vec<LinePair>,          // descomposición por línea para accept/reject por línea
}
pub struct LinePair { pub base_row: Option<u32>, pub buffer_row: Option<Anchor> }
```

## De dónde sale la base (v2, 2026-09-28)

**v2 (2026-09-28): la base viene de la foto del proyecto; las herramientas son solo una pista.** El store no cambia de forma: sigue recibiendo `capture_base_*`, `file_written`, `file_created` y `file_deleted`. Lo que cambió es quién lo alimenta (`cincel-workspace`, `review.rs` + `review_snapshot.rs`; regla completa en `03-arquitectura.md` §4):

- Antes de cada prompt el host saca una foto de todos los archivos del proyecto (`cincel_project::ProjectSnapshot`, mismo recorrido y exclusiones que el árbol).
- Todo lo que cambia en disco durante el turno es del agente, lo haya hecho con una herramienta o con un script: para una ruta que no está en revisión, la base es la copia de la foto (`capture_base_text`) o, si no existía, `file_created`; después `file_written`/`file_deleted` con lo que hay en disco. Renombrar = borrar + crear.
- Las pistas del agente (`fs/read_text_file`, `fs/write_text_file`, `tool_call` edit/delete/move) siguen capturando la base antes, como en v1, pero dejaron de ser requisito.
- Lo que el propio Cincel guarda durante el turno actualiza la foto y no se atribuye al agente.
- `capture_base` es un no-op para un archivo ya en revisión: entre turnos encadenados la base original se conserva hasta que el usuario decide.
- **Los archivos binarios quedan fuera de la revisión** (decisión del autor tras probar la 1.0): la revisión es de todos los archivos de texto. Binario = no UTF-8, o un NUL en los primeros 8 KB (`cincel_project::looks_binary`, el mismo criterio que el store). Si el agente crea, cambia o borra un binario (un `.pyc` de `__pycache__`, una imagen), o convierte un archivo de texto en binario, el cambio se aplica sin preguntar y no aparece como pendiente en ningún lado: árbol, "Revisar todo", barra de estado, diálogo de cierre. La foto registra de los binarios solo tamaño y hash (`SnapshotContent::Binary`, sin copia y sin gastar el tope de memoria), para distinguirlos de los de texto.
- Un archivo de **texto sin copia en la foto** (por encima de `review.max_file_size_kb` o del tope total) sigue entrando en revisión, fuera del store, entero y **solo aceptar** ("cambiado por el agente, sin copia previa"); vive solo en memoria.

## Deshacer y repaso en segundo plano (v1.0, `08-etapa6-cierre-1-0.md` §5.2 y §5.3)

- ~~**Binarios persistidos (D13)** y **deshacer unificado** (`review_binaries.rs`, `binaries.json`, `binaries/<sha256>`, `RejectLog`).~~ Retirados en las correcciones tras la prueba de la 1.0 (`docs/etapas/etapa-6.md`): los binarios ya no se revisan. Al abrir un proyecto, el host borra el `binaries.json` y la carpeta `binaries/` que haya dejado una versión anterior.
- **Deshacer.** `Alt+Shift+U` deshace el último rechazo con la pila del propio store (`undo_last_reject`, tope `UNDO_LIMIT` = 32). `ReviewStore::undo_depth()` sigue disponible (cuántos rechazos puede deshacer; lo usan los tests).
- **Repaso final en segundo plano (D12).** Al terminar el turno (`end_turn`, `agent_gone`, o `begin_prompt` si se encadenan), `ProjectSnapshot::changes()` corre en el ejecutor de fondo sobre un `Arc<ProjectSnapshot>` y el turno **sigue activo** (`PhotoState::Sweeping`: botones de decidir deshabilitados) hasta que vuelve el resultado. Mientras tanto los lotes del watcher se siguen adoptando contra la foto y lo que Cincel escribe (un `Ctrl+S`) queda aparte (`host_writes`), nunca como cambio del agente. Los archivos que difieren se adoptan en el hilo principal en tramos de a lo sumo 4 ms (meta: ningún bloqueo de más de 8 ms); después se cierra el turno como antes (`store.end_turn`, informe, persistencia). Un prompt enviado durante el repaso espera (`Review::turn_waiter`, que cubre foto y repaso) y su turno empieza, con foto nueva, recién al terminar; cancelarlo mientras espera lo descarta. Si el repaso dura más de 1 s, la barra de estado dice "Revisando los cambios del agente…". En los tests con proyecto inerte el repaso es en línea salvo `set_photo_in_background(true)`. `Review::last_sweep()` da los tiempos (comparación, tramo más largo del hilo principal, total).

## Operaciones
- `begin_turn(turn_id)`, `end_turn(turn_id)`.
- `capture_base(path, text)`: fija la base si el archivo no está en revisión; si ya lo está, no hace nada (la base de un archivo persiste entre turnos hasta que se acepta o rechaza todo).
- `file_written(path, new_text, source)`: actualiza `buffer_version` y programa recomputar.
- `file_created(path, text, previous: Option<Rope>)`, `file_deleted(path, previous)`.
- `user_edited(path, edit)`: **rebase**: si la edición no intersecta ningún hunk pendiente, se aplica también a `base` (así no aparece como cambio del agente); si intersecta, se deja y pasa a formar parte del hunk. Estructurar como `rebase(user_edit, hunks) -> Result<Rebased, Conflict>` para poder mejorar a OT después.
- `accept_hunk(id)`: `base.replace(base_rows, buffer.text(buffer_range))`. Sin I/O. Recompute.
- `reject_hunk(id) -> Vec<BufferEdit>`: devuelve las ediciones a aplicar al buffer (texto de `base_rows` sobre `buffer_range`); quien llama las aplica con `EditSource::Review` y guarda. Apila `RejectUndo`.
- `accept_line(pair)` / `reject_line(pair)`: igual pero por par de líneas; inserta/borra cuando un lado es `None`.
- `accept_file(path)`, `reject_file(path) -> Vec<BufferEdit>` (para `Created` sin ediciones del usuario devuelve `DeleteFile`; si el usuario editó encima, conserva el archivo y solo revierte los hunks), `accept_turn(turn_id)`, `reject_turn(turn_id)`, `accept_all()`, `reject_all()`.
- `undo_last_reject() -> Vec<BufferEdit>`, `can_undo_reject()`, `undo_depth() -> usize` (cuántos rechazos puede deshacer).
- `recompute(path, buffer_snapshot)`: en fondo, con debounce 50 ms, descartando por versión. Diff de líneas con Histogram + postproceso de hunks (heurística de deslizamiento/indentación de git). `WordDiffs` con `similar` a nivel de palabra Unicode cuando ambos lados ≤ 5 líneas. Comparación **sin** ignorar espacios en blanco. Matching CRLF-agnóstico (los buffers ya están normalizados a LF).
- `stats(path) -> (added, removed)`, `pending_count()`, `next_hunk(after: (path, row))`, `prev_hunk(...)` con wrap entre archivos.
- `report_for_agent(turn_id) -> Option<String>`: para cada archivo tocado en el turno, un parche unificado entre lo que el agente dejó y lo que quedó tras la revisión, con encabezado "The user made the following updates to your changes:"; separado, un parche de cambios de formateo si los hubo. `None` si no hubo rechazos ni ediciones.
- Persistencia: `save(dir)` / `load(dir, read_current: impl Fn(&Path) -> Option<Vec<u8>>)` según `03-arquitectura.md §6`.
- Límites: archivos > 2 MB o > 50 000 líneas → `FileReview` con `hunks` vacío y flag `too_large`; solo operaciones por archivo. Binarios (bytes nulos en los primeros 8 KB) → `binary`, solo por archivo.

## Comentarios para el agente (v0.2.0, `09-etapa7-conexiones-imagenes-comentarios.md` §6.4 y §6.8)

`src/comments.rs` (modelo, dentro de `ReviewStore`) y `src/feedback.rs` (texto al agente). Sin GPUI.

```rust
pub struct CommentId(pub u64);
pub enum CommentState { Pending, Accepted, Rejected, Mixed, NoAgentChange } // agent_text()
pub enum SentRange { Lines, RemovedBefore, DeletedFile }
pub struct CommentView { id, path, display_path, rows: Range<u32>, text, from_hunk: Option<HunkId> }
pub struct SentComment { id, path, display_path, first_line, last_line, kind, state,
                         code, removed, truncated_lines, unsaved, lang, text }   // Serialize
pub enum CommentDropReason { Missing, NotText }
pub const COMMENT_MAX_CHARS: usize = 8000;
pub const COMMENT_SNIPPET_MAX_LINES: usize = 120;
pub const COMMENT_SNIPPET_MAX_BYTES: usize = 12 * 1024;

impl ReviewStore {
    fn add_comment(&mut self, path, &BufferSnapshot, rows: Range<u32>, from_hunk: Option<HunkId>, text: String) -> Option<CommentId>;
    fn edit_comment(&mut self, CommentId, String) -> bool;        // texto en blanco = borrar
    fn remove_comment(&mut self, CommentId) -> bool;
    fn drop_comments_in(&mut self, path) -> usize;                // binario o archivo que ya no existe
    fn comment_buffer_event(&mut self, path, &BufferEvent, &BufferSnapshot);
    fn comments(&self, snapshot_of: Fn(&Path) -> Option<BufferSnapshot>) -> Vec<CommentView>; // ruta, fila
    fn comment_count(&self) -> usize;  fn comment_count_in(&self, path) -> usize;
    fn commented_paths(&self) -> Vec<PathBuf>;
    fn comment_state(&self, CommentId) -> Option<CommentState>;
    fn take_comments_for_prompt(&mut self, snapshot_of: Fn(&Path) -> Option<(BufferSnapshot, bool /*sin guardar*/)>) -> Vec<SentComment>;
    fn restore_comments(&mut self, &[SentComment], snapshot_of: Fn(&Path) -> Option<BufferSnapshot>);
}
pub fn format_feedback(report: Option<&str>, comments: &[SentComment]) -> Option<String>;
pub fn fence_for(text: &str) -> String;
```

- **Filas**: filas del buffer, fin exclusivo; vacío = antes de `rows.start` (comentario sobre una eliminación pura). Anclas: inicio `Before` al principio de la primera fila, fin `After` al final del texto de la última, en un `AnchorMap` por ruta que el anfitrión alimenta con **cada** `BufferEvent` del buffer (`comment_buffer_event`, para toda ruta vigilada, esté o no en revisión; mismo lote que `buffer_edited`, el snapshot puede ser el de después del lote). Si las filas desaparecen, el comentario queda en una fila, la del ancla de inicio. Un comentario sin filas que recibe las líneas de vuelta (rechazo de la eliminación) pasa a cubrirlas.
- **Marcas**: las diez decisiones (`accept/reject_hunk`, `accept/reject_line`, `accept/reject_file`, `accept/reject_turn`, `accept/reject_all`) marcan `accepted`/`rejected` en los comentarios del archivo cuyo rango corta las filas decididas (una eliminación pura: la fila donde se dibujan sus filas rojas; una línea solo de la base: la fila real más cercana de su segmento; un archivo de solo-archivo o borrado: todos). Las marcas de un rechazo viajan en su entrada de deshacer; `undo_last_reject` quita solo las que ese rechazo puso. Ninguna decisión borra comentarios; las firmas de las decisiones no cambiaron.
- **Estado al enviar**: un segmento pendiente bajo el rango → `Pending`; si no, `Accepted`/`Rejected`/`Mixed`/`NoAgentChange` según las marcas.
- **Envío**: `take_comments_for_prompt` los saca de conteos, vistas y persistencia, ordenados por ruta relativa (bytes) y línea, y los deja aparte (siguen anclados, invisibles) hasta el próximo `take`; `restore_comments` (D16) los devuelve tal cual estaban (mismo id, marcas y anclas) o, si ya no estaban aparte, los reconstruye en sus líneas con las marcas que implica su estado.
- **Bloque al agente**: `format_feedback(report_for_agent(turn), &sent)`: sin comentarios devuelve el informe **idéntico**; con comentarios, informe + línea en blanco + sección §6.8; `None` si no hay nada. Se envuelve con `wrap_review_feedback` como siempre. Al agente viaja solo lo de siempre (rechazos y ediciones a mano) más los comentarios, una sola vez.
- **Formato de la sección de comentarios** (`feedback.rs`; texto plano en inglés, `\n`, sin espacios al final de línea, una línea en blanco entre comentarios; rutas relativas a la raíz del proyecto con `/`, líneas desde 1 e inclusivas, como está el archivo ahora):
  ```text
  The user left {N} comment(s) on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn.

  Comment {i} of {N}
  File: {ruta}[ (you deleted this file)]
  Line: {a}  |  Lines: {a}-{b}  |  Lines: removed before line {a}
  Review state: {estado}
  Current code[ (as shown in the editor, not saved yet)]:
  {valla}{lang}
  {código}
  {valla}
  [({k} more lines not shown)]
  [Lines you removed (not reviewed yet):
  {valla}{lang}
  {líneas quitadas}
  {valla}]
  Comment:
  {texto del usuario, tal cual}
  ```
  `{estado}` (`CommentState::agent_text`): `your change, not reviewed yet` · `your change, accepted by the user` · `your change, rejected by the user` · `your change, partly accepted and partly rejected by the user, line by line` · `no change of yours (the user selected these lines)`. "Current code" se omite en una eliminación pura pendiente y en un archivo borrado; "Lines you removed" aparece solo en esos dos casos y mientras estén pendientes; sin código actual, "({k} more lines not shown)" va después de las líneas quitadas. Ejemplo completo en `docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.8.
- **Kinds**: `Lines` (con `code`, `unsaved` si el buffer difiere del disco); `RemovedBefore` (sin código; `removed` = líneas quitadas mientras la eliminación esté pendiente); `DeletedFile` (archivo borrado por el agente; `removed` mientras esté pendiente, nada tras aceptar el borrado). Fragmento con tope de 120 líneas / 12 KB (`truncated_lines`); valla = racha más larga de acentos graves + 1, mínimo 3; `lang` = extensión en minúsculas `[a-z0-9+#-]{1,10}`.
- **Binarios**: `add_comment` devuelve `None` para un snapshot binario; un archivo comentado que se vuelve binario → `drop_comments_in` (el anfitrión avisa).
- **Persistencia**: `state.json` versión 2 (`STATE_VERSION = 2`) suma `comments` (`id`, `path`, `start_row`, `end_row`, `text`, `created_at`, `from_hunk`, `file_hash`, `snippet_hash`, `accepted`, `rejected`); el texto de las filas va como objeto en `objects/` y no se borra en la limpieza mientras esté referenciado. `load`: versión 1 → sin comentarios; `file_hash` igual → mismas filas; distinto → la aparición exacta del fragmento más cercana a `start_row`, si no una fila en `start_row` acotada (`comments_relocated`); archivo ausente → `CommentDropReason::Missing`, no UTF-8 o binario → `NotText` (`comments_dropped`); un archivo borrado que sigue en revisión conserva sus comentarios sobre su texto borrado. Los comentarios ya enviados no se guardan. `LoadReport` suma `comments_restored`, `comments_relocated`, `comments_dropped`.
- Tests: `tests/comments.rs` (anclas, las diez marcas, deshacer, escenarios a–m del motor, take/restore, fragmentos, vallas, formato literal incluido el ejemplo de §6.8) y `tests/comments_persist.rs` (v1, ida y vuelta v2, reubicación, línea más cercana, descartes, objetos).

## Casos de borde obligatorios (tests)
- [ ] Accept de un hunk deja los offsets de los demás hunks correctos (procesar de abajo hacia arriba o con delta).
- [ ] Reject de un hunk no toca texto fuera de su rango; el archivo resultante coincide con `base` en ese rango y con el buffer en el resto.
- [ ] Dos escrituras del agente en la misma región en un turno producen un solo hunk contra la base original.
- [ ] Edición del usuario fuera de todo hunk no genera hunk nuevo (se aplica a la base).
- [ ] Edición del usuario dentro de un hunk lo mantiene pendiente y el texto del usuario queda en el lado "nuevo".
- [ ] Archivo creado por el agente: `reject_file` → `DeleteFile`; si el usuario editó encima → no borra.
- [ ] Archivo con newline final agregado o quitado produce un hunk de una línea, no de archivo completo.
- [ ] Por línea: en un hunk de 3 líneas modificadas, aceptar la del medio deja dos hunks de una línea.
- [ ] `undo_last_reject` restaura exactamente el texto rechazado y el hunk reaparece.
- [ ] Persistir y cargar con archivo intacto reproduce los mismos hunks; con archivo alterado descarta la entrada.
- [ ] `report_for_agent` produce un parche aplicable con `git apply --check` sobre el texto que dejó el agente.
- [ ] Propiedad: para textos aleatorios, `accept_all` deja `base == buffer` y `hunks` vacío; `reject_all` deja `buffer == base_original`.
