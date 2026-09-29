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
