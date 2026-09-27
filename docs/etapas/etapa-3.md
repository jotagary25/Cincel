# Etapa 3: revisión de cambios

Estado: **integración completa en el workstream C** (2026-09-25).
`cincel-review` (A) y la UI de revisión de `cincel-editor` (B) llegaron a
esta sesión terminados y sin commitear; este documento registra la
integración en `cincel-workspace` y `cincel` (C) y deja constancia de A/B
tal como se los encontró. No se tocó ningún archivo de `cincel-review`,
`cincel-editor`, `cincel-chat` ni `cincel-acp`.

## Qué se construyó

### A — `cincel-review` (recibido terminado)

`ReviewStore` sin GPUI ni async: base por archivo, hunks exactos entre
recálculos (invariantes de `file.rs`), rebase de ediciones del usuario
(`rebase::rebase`), aceptar/rechazar por segmento, línea, archivo, turno y
todo, pila de deshacer rechazos (32), navegación con vuelta entre archivos,
informe para el agente (`report_for_agent` con parche unificado y sección de
formateo), recálculo en un `RecomputeJob` `Send` con cancelación por
generación, persistencia content-addressed (`state.json` + `objects/`) que
nunca reescribe archivos al restaurar, límites de 2 MB / 50 000 líneas y
binarios. 72 tests (unitarios, de protocolo, de bordes, de propiedades y de
informe).

### B — revisión en `cincel-editor` (recibido terminado)

`ReviewView` / `ReviewHunkView` / `ReviewLineView` / `ReviewWordDiffs` como
datos planos, `EditorView::set_review` (barato cuando nada cambió de filas,
conserva cursor y resaltado de las filas fantasma), filas fantasma con la
gramática al 70 %, `diff.*.bg`, palabras, barra de 3 px y borde de 2 px en el
gutter, pill (con reloj "Turno anterior"), iconos `+`/`−` por línea, barra
flotante con spinner "El agente está editando…" y el aviso "Sin cambios
pendientes en este archivo…", `EditorEvent::Review(ReviewAction)` para toda
decisión o navegación (el editor nunca decide), `turn_active` que deshabilita
las decisiones sin impedir escribir, y
`EditorSettings::jump_to_next_on_decide`. 34 tests de revisión.

### C — integración en `cincel-workspace` (esta sesión)

0. **Roturas de compilación del cambio de contrato**: `theme::editor_settings`
   mapea `jump_to_next_on_decide` desde `settings.review.jump_to_next_on_decide`
   (el campo ya existía en `cincel-settings`, no hizo falta el TODO);
   `CenterPanel` maneja `EditorEvent::Review`.
1. **`crates/cincel-workspace/src/review.rs`, entidad `Review`**: dueña del
   `ReviewStore` del proyecto (`set_workspace_root` con la raíz).
   - Base en el primer contacto: `FsRead` (desde el buffer, antes de
     contestar), `FsWrite` (antes de aplicar), primer `tool_call` de tipo
     `edit`/`delete`/`move` que nombra un path (en `locations` o en
     `diff.path`, también si llega en un `tool_call_update` posterior), y
     `file_created` para un path que no existía (lo anuncia un tool call o lo
     crea un `fs/write_text_file`).
   - `begin_turn` al mandar un prompt (`Agents::prepare_prompt`), `end_turn`
     en `TurnEnded` (y si el proceso del agente muere).
   - La verdad es el disco: tras cada `tool_call_update` `completed` (o
     `failed`) de `edit`/`delete`/`move`, en cada `FileChangeReport` y en cada
     recarga del watcher de un path trackeado se relee el archivo
     (`BufferStore::reload_from_disk`, ediciones mínimas) y se informa
     `file_written` / `file_deleted` / `file_created` como `EditSource::Agent`.
     El `diff` del agente nunca se aplica.
   - Suscripción (`Buffer::subscribe`) a cada buffer trackeado: los eventos se
     encolan y se drenan en el hilo principal inmediatamente después de cada
     operación propia (snapshot justo después del evento → camino rápido) y
     desde una tarea de despertar para lo que edita otro (el usuario).
   - Recálculo: debounce de 50 ms, `recompute_job` en el hilo principal,
     `job.run()` en `cx.background_executor()`, `apply_recompute` de vuelta, y
     el `ReviewView` fresco a cada editor abierto: filas resueltas desde los
     anchors, `deleted_lines` desde las filas de la base, palabras, pares de
     líneas en filas del buffer, `from_previous_turn`, `current_index`,
     `turn_active`, `pending_in_file` / `pending_in_other_files`.
2. **Acciones**: `EditorEvent::Review` → `CenterEvent::Review` → `Review`:
   aceptar/rechazar segmento, línea y archivo; los rechazos se aplican en una
   sola transacción `EditSource::Review` con `edit_many` y se guardan;
   `FileOp::DeleteFile` borra el archivo y cierra la pestaña;
   `FileOp::WriteFile` lo escribe. Aviso "Segmento rechazado · Deshacer
   (Alt+Shift+U)" con botón "Deshacer" (también "Línea rechazada", "Archivo
   rechazado", "Turno rechazado"). `NextHunk`/`PrevHunk`/`NextFile` usan
   `next_hunk`/`prev_hunk` con vuelta entre archivos, abren la pestaña (fijada)
   si hace falta y fijan `current_index`. `OpenReviewPanel` abre el panel. Los
   comandos globales `workspace::next_change`, `prev_change`,
   `next_file_with_changes`, `accept_turn`, `reject_turn`, `undo_last_reject`
   y `open_review_panel` hacen lo real (ya no hay "Disponible en Etapa 3").
   "Aplicar sin revisar" llama `accept_turn` en `TurnEnded`. "Ver en el
   editor" del chat salta al primer segmento pendiente del archivo.
3. **Panel de revisión** (`Workspace::render_review_panel`): popover anclado a
   la barra de estado (clic en "N pendientes" o `Ctrl+Shift+R`), lista de
   archivos con icono, ruta relativa, `+N −M` y etiqueta
   `pendiente`/`decidido`/`nuevo`/`eliminado`, botones `Button` de gpui-kit
   neutrales (`outline`) "Aceptar todo (Ctrl+Alt+↵)" / "Rechazar todo
   (Ctrl+Alt+⌫)", ✓/✗ por archivo, aviso "Demasiado grande para revisar por
   segmentos" en los archivos solo por archivo, y clic en una fila para abrir
   el archivo en su primer segmento pendiente. Clic fuera lo cierra.
4. **Árbol, pestañas y barra**: archivos con pendientes en `#e5c07b` con
   sufijo `+N −M` (11 px, `text.muted`), archivos nuevos del agente en
   `#98c379`, carpetas con hijos pendientes con el punto de 6 px, y una línea
   de raíz arriba del árbol con el total ("7 pendientes · +12 −3"). Pestañas
   con `+N −M` chico junto al título; el placeholder "◆"
   (`Tab::agent_touched`) desapareció. Barra de estado: "N pendientes" (en
   `#e5c07b` si hay), clic abre el panel.
5. **Diálogo de buffer sucio** (`CenterPanel::render_dirty_dialog`): antes de
   un `FsWrite` sobre un buffer con cambios sin guardar, o cuando arranca un
   tool call de edición sobre uno: "«x» tiene cambios sin guardar" con
   Guardar / Descartar / Mantener. Las peticiones del agente sobre ese path
   (lecturas, escrituras, relecturas) quedan retenidas hasta la respuesta.
   Guardar → base = tu versión, el segmento es solo del agente; Descartar →
   tus cambios se tiran como edición tuya; Mantener → el agente escribe
   encima y la base es lo último guardado, así que el segmento incluye tus
   cambios. Si el permiso del tool call ya se concedió automáticamente
   (la política fija concede solas las ediciones fuera de rutas sensibles),
   se guarda sin preguntar. Mientras hay un turno
   activo el autoguardado por cambio de foco salta los archivos trackeados.
6. **Devolución al agente**: al mandar el próximo prompt,
   `report_for_agent(turno)` de cada turno terminado va como `feedback` de
   `AgentCommand::Prompt` (cincel-acp lo envuelve en
   `<user_review_feedback>`), seguido de `forget_turn`.
7. **Persistencia**: `~/.local/share/cincel/review/<hash de ruta>/`, con el
   mismo hash que `layout.json`. Se guarda 1 s después de cada decisión, del
   fin de turno y de cada guardado, al cambiar de proyecto y al salir; se
   carga al abrir el proyecto con `read_current` desde disco. Una entrada
   descartada muestra "La revisión de «x» se descartó: el archivo cambió fuera
   de Cincel"; las restauradas reabren su buffer en segundo plano (sin
   pestaña) para que la vista esté lista.
8. **Settings**: `review.jump_to_next_on_decide` (al editor),
   `review.max_file_size_kb` y `review.max_lines` (un archivo que los supera
   se revisa solo por archivo, con aviso en el panel y navegación que cae en
   el archivo), `review.sensitive_paths` (política de permisos; ahora también
   se recarga en caliente). Todo ya existía en `cincel-settings`.
9. **Tests**: 26 nuevos en `crates/cincel-workspace/src/review_tests.rs`
   (agente falso real para el turno con `fs/write_text_file`; eventos ACP
   hechos a mano para el resto) + 2 adaptados (`agents.rs`, `tests.rs`).
   Además, los tres módulos de tests comparten ahora un único
   `test_support::isolate_state`: antes había dos `OnceLock` que podían
   apuntar `XDG_*` a lugares distintos en paralelo (la causa del fallo
   intermitente de `the_old_chat_json_becomes_the_first_conversation`).
10. Este documento.

## Verificación

| Comando | Resultado |
|---|---|
| `cargo check -p cincel-workspace -p cincel` (paso 0) | limpio |
| `cargo fmt --all --check` | limpio |
| `cargo clippy -p cincel -p cincel-workspace --all-targets --features cincel-workspace/test-support -- -D warnings` | limpio |
| `cargo test -p cincel-workspace --features cincel-workspace/test-support` | 94/0 antes de los dos últimos tests; el binario final (ver abajo) 96/0, 5 corridas seguidas sin fallos intermitentes |
| tests de `cincel` (`main.rs`) | 7/0 |
| `cargo run -p cincel -- --smoke-test .` | exit 0, 8 cuadros dibujados |
| `cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .` (fugas) | **no corrido**: ver abajo |
| `pgrep -af sleep` | vacío |

### Disco lleno

El disco de la máquina se llenó durante la sesión (`/` al 100 %: `target/debug`
ocupa 95 GB, casi todo binarios de ≈1,1 GB de etapas anteriores y de otros
workstreams). Con menos espacio libre que un binario enlazado, los binarios de
tests y `cincel` se enlazaron con `cargo rustc … -- -C strip=debuginfo` (el
mismo código y las mismas dependencias ya compiladas, solo sin símbolos de
depuración en el enlace final) y se ejecutaron directamente:

```bash
cargo rustc -p cincel-workspace --lib --profile test --features test-support -- -C strip=debuginfo
target/debug/deps/cincel_workspace-<hash>        # 96 passed
cargo rustc -p cincel --bin cincel --profile test -- -C strip=debuginfo
target/debug/deps/cincel-<hash>                  # 7 passed
cargo rustc -p cincel --bin cincel -- -C strip=debuginfo
target/debug/cincel --smoke-test .               # exit 0
```

La corrida con el detector de fugas (`--features
cincel-workspace/test-support`) necesita recompilar `gpui` con otro conjunto
de features y se quedó sin espacio (`No space left on device`). Queda para
después de liberar disco (por ejemplo `cargo clean`):

```bash
cargo build -p cincel-acp --bin cincel-acp-fake-agent
cargo test -p cincel -p cincel-workspace --features cincel-workspace/test-support
cargo run -p cincel -- --smoke-test .
timeout 5 cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .
```

## Desviaciones

- **La relectura del disco entra al buffer como `EditSource::Load`**:
  `BufferStore::reload_from_disk` no recibe una fuente y usa `Load` en la
  transacción del buffer. La revisión la reclasifica al informarla al store
  (`buffer_edited`/`file_written` con `EditSource::Agent { turn_id }`), que es
  lo que decide los hunks; lo único que queda con `Load` es la entrada del
  historial de deshacer del buffer (deshacerla sigue siendo "un solo paso" y
  el segmento desaparece). Reescribir el archivo con `set_text_minimal` +
  `save` para tener `Agent` en el historial habría vuelto a escribir lo que el
  agente acaba de escribir, con el fin de línea original.
- **Una recarga del watcher durante un turno es del agente**: los eventos
  `Load` de un archivo trackeado con un turno activo se informan como
  `Agent`; sin turno, como `Load` (el store los trata igual: crecen los
  hunks, `03-arquitectura.md` §7).
- **Límites del settings por encima de los del store no tienen efecto**:
  `max_file_size_kb`/`max_lines` se aplican en el host (un archivo que los
  supera se muestra y se decide solo por archivo), pero el store ya pasa a
  "solo por archivo" a los 2 MB / 50 000 líneas y eso no es configurable.
- **Archivos no UTF-8 o binarios no se revisan**: `BufferStore` no los abre,
  así que no hay buffer al que suscribirse; el agente puede escribirlos, pero
  no aparecen en la revisión.
- **Un archivo existente que el agente toca sin anunciarlo** (ni `FsRead`, ni
  tool call con `locations`, ni `FsWrite`) y que no estaba abierto no tiene
  base conocida: se registra un aviso en el log y no se revisa. Si estaba
  abierto, su buffer todavía tiene el texto de antes y se usa como base.
- **`turn_active` es por archivo**: la vista de un archivo lo recibe mientras
  el turno en curso lo tocó (`ReviewStore::turn_active(path)`, como pide el
  protocolo del store); los pendientes de turnos anteriores en archivos que el
  turno en curso no toca siguen decidibles. `accept_turn`/`reject_turn` y
  "Aceptar/Rechazar todo" sí se niegan mientras hay un turno activo.
- **Diálogo de buffer sucio**: en `FsWrite` siempre pregunta (la respuesta del
  agente queda retenida, así que siempre se puede esperar); en el inicio de un
  tool call solo pregunta si la política no concedió el permiso sola; si lo
  concedió, guarda sin preguntar (`03-arquitectura.md` §4.1: "si no se puede
  preguntar, se guarda"). "Mantener" solo puede fijar la base en lo guardado
  si el archivo todavía no estaba en revisión; si el agente ya lo había leído
  antes (base = tu buffer con los cambios), tus cambios quedan en la base. El
  diálogo no tiene atajos de teclado, solo botones.
- **"Aceptar/Rechazar todo" del panel decide todo lo pendiente** (turnos
  anteriores incluidos); `Ctrl+Alt+↵`/`Ctrl+Alt+⌫` deciden el último turno con
  pendientes. Con un solo turno pendiente —el caso normal— son lo mismo.
- **Relectura también al `failed`**: un tool call de edición que falla puede
  haber escrito algo, así que se relee igual que al completarse.
- **Guardar al salir con buffers sucios**: el `current_hash` persistido es el
  del buffer; si al salir había cambios sin guardar en un archivo en
  revisión, al reabrir no coincide con el disco y esa entrada se descarta (con
  su aviso).
- **Detección de "nuevo"**: un path que un tool call anunció antes de existir
  (o que ni el árbol ni el `BufferStore` conocían) y que aparece al releer se
  registra como creado por el agente.

## Deseos de API

- `cincel-project`: `BufferStore::reload_from_disk_as(path, source:
  EditSource)` (o un parámetro de fuente en `reload_from_disk`), para que la
  relectura del agente quede como `Agent` también en el historial del buffer.
- `cincel-review`: límites configurables (`ReviewStore::set_limits(max_bytes,
  max_lines)`) para que `review.max_file_size_kb`/`max_lines` puedan también
  *subir* el umbral del store, y para que el store y el host no tengan que
  llevar dos criterios.
- `cincel-review`: `pending_count` / `locations` que respeten un criterio de
  "solo por archivo" externo (hoy el host lo recalcula).
- `cincel-review`: `capture_base_text` que pueda reemplazar la base de un
  archivo trackeado *sin hunks* (para que "Mantener" funcione aunque el agente
  haya leído el archivo antes).
- `cincel-editor`: un evento de navegación con la posición de origen
  (`ReviewAction::NextHunk { from: Point }`), o no mover el cursor antes de
  emitir `NextHunk`/`PrevHunk`: hoy el editor salta dentro de su archivo antes
  de avisar, y el host reconstruye el punto de partida con el
  `CursorMoved` inmediatamente anterior.
- `cincel-editor`: que `AcceptFile`/`RejectFile` se emitan también sin hunks
  inline cuando la vista tiene `pending_in_file > 0` (archivos solo por
  archivo), para decidirlos con `Ctrl+Shift+Enter`/`Backspace`; hoy solo desde
  el panel.
- `cincel-acp`: un escenario del agente falso que edite el disco
  directamente dentro de un tool call con `locations` (hoy ese caso se prueba
  con eventos ACP hechos a mano).
- `cincel-chat`: que la tarjeta de edición tome `+N −M` de la revisión (un
  setter) en vez de calcularlos del `diff` del agente.

## Lista de comprobación manual (autor)

Cierra la Etapa 3 de `docs/specs/05-plan-etapas.md`. Con Claude autenticado en
la máquina (nada de esto lo corre un agente):

```bash
cargo build -p cincel
./target/debug/cincel ~/algun/proyecto
```

1. **Un cambio en 3 archivos.** Elegí Claude en el chat y pedile un cambio que
   toque tres archivos (por ejemplo "renombrá la función X en a, b y c").
   Mientras escribe: las pestañas y el árbol muestran `+N −M`, la barra dice
   "El agente está editando…" y los botones de decidir no responden. Al
   terminar, en cada archivo se ven las filas rojas (fantasma) y verdes; la
   barra de estado dice "N pendientes".
2. **Aceptar uno entero.** En el primer archivo, `Ctrl+Shift+Enter`: el color
   desaparece, el archivo en disco no cambia (`git diff` igual que antes) y
   el contador baja.
3. **Rechazar otro.** En el segundo, `Ctrl+Shift+Backspace`: el archivo vuelve
   a como estaba (mirá `git diff`), aparece el aviso "Archivo rechazado ·
   Deshacer (Alt+Shift+U)".
4. **Rechazar una sola línea del tercero.** Pasá el mouse por una línea
   verde del tercer archivo y tocá el `−` del gutter (o `Alt+Backspace` con el
   cursor ahí): solo esa línea vuelve al original; el resto del segmento sigue
   pendiente.
5. **Tipear dentro de un segmento pendiente.** Escribí algo en una línea
   verde: el segmento se acomoda y lo tuyo queda como parte del lado nuevo;
   escribí también fuera de todo segmento: ahí no aparece ningún segmento
   nuevo.
6. **Deshacer un rechazo.** `Alt+Shift+U` (o "Deshacer" del aviso): el texto
   rechazado vuelve y el segmento reaparece.
7. **Alt+J entre archivos.** Con pendientes en dos archivos y solo uno
   abierto, `Alt+J` repetido recorre los segmentos, abre la pestaña del otro
   archivo y al final da la vuelta al primero. `Alt+K` hace lo mismo al revés,
   `Alt+L` salta de archivo.
8. **Panel.** Clic en "N pendientes" (o `Ctrl+Shift+R`): la lista muestra cada
   archivo con `+N −M` y su estado (pendiente / decidido / nuevo /
   eliminado). Clic en una fila abre el archivo en su primer segmento. Probá
   "Rechazar todo" y después `Alt+Shift+U`.
9. **Contadores del árbol.** Los archivos con pendientes en amarillo con
   `+N −M`, los creados por el agente en verde, las carpetas que los contienen
   con un punto, y arriba del árbol el total.
10. **Cerrar y reabrir con pendientes.** Dejá segmentos sin decidir, cerrá
    Cincel y abrilo de nuevo en el mismo proyecto: los segmentos siguen ahí
    (y el disco no se tocó). Repetí cambiando uno de los archivos con otro
    editor mientras Cincel está cerrado: al abrir aparece "La revisión de
    «x» se descartó: el archivo cambió fuera de Cincel".
11. **Buffer sucio.** Editá un archivo sin guardar y pedile al agente que toque
    ese archivo: como el permiso de edición se concede solo, Cincel guarda
    tu versión antes de que escriba y el segmento es solo del agente. Para ver
    el diálogo "«x» tiene cambios sin guardar" (Guardar / Descartar /
    Mantener), repetilo con una ruta sensible (por ejemplo un `.env` del
    proyecto): ese pedido llega al chat y el diálogo aparece antes de
    escribir. Probá las tres (Mantener: el segmento incluye lo tuyo).
12. **El agente sabe qué rechazaste.** Después de rechazar algo, mandale
    "¿qué cambios míos ves sobre tu última edición?": su respuesta debe
    mencionar lo rechazado (le llegó en `<user_review_feedback>`).
13. **Fin del turno.** Con un archivo con cambios del agente abierto, esperá
    el último mensaje: la píldora y la barra flotante dejan de estar
    atenuadas enseguida; abrí otro archivo que el agente tocó en ese turno y
    también está decidible. Repetí cortando el turno con el botón de detener.
14. **Chat.** A simple vista se distingue quién escribió qué: tus mensajes en
    burbuja azulada a la derecha, las respuestas en texto plano, el código de
    las respuestas en un bloque oscuro con borde y cabecera "rust · Copiar".
    Mandá un mensaje muy largo (y uno sin espacios): se parte dentro de la
    burbuja. No hay selector de autonomía; los selectores que quedan son los
    del agente (modo, modelo, esfuerzo).
15. **Vista previa de Markdown.** Abrí un `.md` y `Ctrl+Shift+V`: el editor se
    reemplaza por el texto renderizado (encabezados, listas, bloques de
    código resaltados); la barra de estado dice "Vista previa" en vez de
    `Ln, Col`. Escribí algo en el editor, volvé a `Ctrl+Shift+V` y confirmá que
    el cambio aparece. El icono al final del breadcrumb hace lo mismo con el
    mouse. Cerrá Cincel con la vista previa activa y volvé a abrir el mismo
    proyecto: la pestaña reabre ya en vista previa. Probá `Ctrl+Shift+V` en una
    pestaña de Rust: no pasa nada.
16. **Operaciones nuevas del editor.** Sobre un archivo de código: `Alt+↑`/`Alt+↓`
    mueve la línea del cursor; `Shift+Alt+↓` la duplica hacia abajo;
    `Ctrl+Shift+K` la borra entera; `Ctrl+/` comenta/descomenta la selección;
    `Ctrl+J` une la línea siguiente a la del cursor; `Ctrl+D` selecciona la
    palabra bajo el cursor y, repetido, la próxima aparición igual; tipear `(`,
    `[`, `{`, `"` o `'` cierra el par solo (y `Backspace` entre un par vacío
    borra los dos).

## Correcciones tras la prueba manual del autor

1. **Contraste entre mensajes** (`cincel-chat`, `render.rs`,
   `markdown.rs`, `theme.rs`, `settings.rs`). Mensaje del usuario: burbuja a
   la derecha con `text.accent` al 14 % sobre `bg.app`, borde de 1 px en
   `text.accent` al 35 %, radio 8, ancho máximo 85 %. Respuesta del agente:
   texto plano sobre el fondo del panel, sin burbuja y sin regla (con la
   burbuja teñida el contraste alcanza; la regla de 2 px no sumaba). Bloques
   de código de la respuesta: `bg.editor` (nuevo `ChatTheme::bg_editor`),
   borde de 1 px `border`, radio 6 y una cabecera chica arriba a la derecha
   con el lenguaje ("rust", o "código") y "Copiar". Las tarjetas de
   herramienta siguen en `bg.surface`; las salidas y bloques monoespaciados
   de las tarjetas también pasan a `bg.editor` con borde. Se quitaron las
   etiquetas "Vos" y el nombre del agente. Capturas de `examples/chat_demo.rs`
   revisadas: se distingue quién escribió qué a simple vista.
   - Desviación: en Cincel Dark `bg.editor` (`#282c34`) es *más claro* que
     el panel (`bg.app`, `#1e2127`), no más oscuro como decía el pedido. Se
     usó `bg.editor` igual: el bloque es gris neutro con borde y cabecera, la
     burbuja es azulada, y no se confunden.
   - Además: el `+N −M` de las tarjetas usaba los fondos de diff al 18 % como
     color de texto (casi invisible); ahora usa los colores sólidos del
     gutter. Los bloques monoespaciados usaban la familia `"monospace"`, que
     gpui no resuelve (salían en proporcional); ahora usan la familia mono de
     `gpui-kit` (la del editor).
2. **Texto largo que se salía de la burbuja**. Cada trozo de texto de la
   burbuja es `min_w_0` + `max_w_full` + `whitespace_normal` y la burbuja
   `min_w_0` + `overflow_hidden`: en gpui el ancho mínimo de un texto es su
   ancho sin partir, así que sin `min_w_0` el trozo no se achicaba. El
   partidor de líneas de gpui ya corta dentro de una palabra más larga que la
   línea. Tests con 400 caracteres con y sin espacios
   (`a_long_message_with_spaces_wraps_inside_its_bubble`,
   `a_long_token_without_spaces_breaks_inside_its_bubble`, que fallan sin el
   arreglo) y los dos casos en la demo (`--long-messages`).
3. **Escala tipográfica** (`cincel-chat/src/settings.rs`: `TEXT_BODY` 13,
   `TEXT_CODE` 12,5, `TEXT_SMALL` 12, `TEXT_LABEL` 11, `TEXT_HEADING_MAX` 15,
   `heading_size`). Aplicada en todo el panel: cuerpo 13; bloques de código,
   comandos y salidas 12,5 monoespaciado; filas de herramienta, pensamiento,
   plan, permiso respondido, avisos y ayudas 12; selectores, píldora de
   estado, `+N −M`, rutas y fechas 11; encabezados del markdown 15 / 14 / 13.
   - La causa de "código más grande que el cuerpo": `gpui-kit` mide un
     párrafo con código en línea leyendo `window.text_style()` en la fase de
     layout, cuando ya no está el estilo del panel, así que esos párrafos
     salían a 16 px (el `rem` de la ventana). El transcript pasó a ser una
     `gpui::list` (que sigue el final, `FollowMode::Tail`): la lista hace el
     layout de cada fila dentro del estilo de texto del panel. Test
     `a_paragraph_with_inline_code_keeps_the_body_size`.
   - Desviación: el código **en línea** dentro de una respuesta queda en
     0,875 × 13 ≈ 11,4 px y no en 12,5: `gpui-kit` 0.6.1 fija esa escala para
     los spans de código y no la expone.
4. **Selector de autonomía retirado; queda un único comportamiento fijo.**
   Fuera el chip del composer, `ChatEvent::AutonomyChanged`, `autonomy` de
   `TranscriptDump`/`Conversation`/`layout.json` (los archivos viejos cargan
   igual: la clave se ignora), `AgentsSettings.autonomy` (un `settings.json`
   que la tiene avisa "ajuste retirado: …; se ignora, podés borrarlo") y la
   aceptación automática al final del turno (`Review::end_turn` ya no recibe
   `auto_accept`). En `cincel-acp`, `autonomy.rs` pasó a
   `permission_policy.rs`: `PermissionPolicy::default()` +
   `with_sensitive_globs`, sin `AutonomyMode`. Comportamiento: `edit`,
   `read`, `search`, `think` se conceden solos; `execute`, `fetch`, `delete`,
   `move`, `other` llegan al usuario; una ruta de `review.sensitive_paths`
   llega siempre al usuario. El selector de modos del agente ("Manual/Auto…",
   de sus `configOptions`) no se tocó. Docs: `01-producto.md` §F5,
   `02-visual.md` §7, `modulos/chat.md`, `acp.md`, `settings.md`; en esta
   lista de comprobación salieron los pasos de "Preguntar antes" y "Aplicar
   sin revisar" y el del buffer sucio se reescribió.
   - Consecuencia a tener en cuenta: como toda edición se concede sola, el
     modo "Manual" del agente no da un "preguntar antes de escribir" en
     Cincel (su pedido de permiso de edición se responde solo, según la
     regla fija). El diálogo de buffer sucio aparece con rutas sensibles o
     cuando el agente escribe sin anunciar el tool call.
5. **Controles de revisión atenuados después del turno.** El camino
   `TurnEnded` → `Review::end_turn` → vistas con `turn_active = false`
   funciona con todos los `stop_reason` (`end_turn`, `max_tokens`,
   `max_turn_requests`, `refusal`, `cancelled`), con un permiso respondido o
   pendiente, y para una pestaña abierta después del turno (tests
   `every_stop_reason_releases_the_review_controls` y
   `the_fake_agent_turns_release_the_review_controls`, con el agente falso
   real). Los caminos en los que el turno quedaba abierto para siempre eran
   los que **nunca reciben `TurnEnded`**:
   - un `session/prompt` que falla: `cincel-acp` lo informa como
     `AgentEvent::Error` y no manda `TurnEnded` (tampoco el chat salía de
     "pensando…"). Ahora `Agents` recuerda que hay un prompt en vuelo; un
     `Error` mientras no haya salido otro pedido que pueda fallar por su
     cuenta (modo, opción, autenticación, sesión) cierra el turno de revisión
     y libera el chat (`ChatPanel::abort_turn`). Si hubo otro pedido, el
     error es ambiguo y el turno sigue hasta su `TurnEnded` (test
     `an_error_from_another_request_keeps_the_turn_open`, con el
     `session/set_mode` que el agente falso no implementa);
   - reemplazar el proceso del agente a mitad de turno (elegir otro agente,
     relanzar, cambiar de proyecto): la conexión vieja se tiraba con su
     `TurnEnded`. Ahora `spawn_agent` y `disconnect` cierran el turno
     (`replacing_the_agent_mid_turn_ends_its_turn`).
   - Además, `Review::view_for` solo marca `turn_active` si el host tiene un
     turno abierto **y** el store dice que ese archivo es del turno, así que
     ninguna vista construida después de `end_turn` puede salir atenuada.
   - Test `a_failed_prompt_releases_the_review_controls`.
6. Este apartado.

### Segunda ronda (tras la segunda prueba manual)

1. **El zoom escala todo.** `Workspace::zoom` (ahora `pub(crate)`) además del
   `rem` y de la fuente del editor le pasa al chat `ChatSettings::scale`
   (`theme::chat_settings` lo toma de `ui_scale`; también en la recarga de
   ajustes). El chat multiplica por él todos sus tamaños en píxeles
   (`ChatSettings::px`, `text_body()`, `text_code()`, `text_small()`,
   `text_label()`, `heading()`): cuerpo, código, filas de herramienta,
   etiquetas, cabecera, radios, burbujas, bloques de código del markdown
   (`text_view_style_scaled`, `code_block_style_scaled`) y la fuente del
   composer. El árbol, las pestañas, el breadcrumb, la barra de estado, el
   panel de revisión, los diálogos de cierre, la posición de los avisos y la
   vista previa de Markdown multiplican sus constantes por `ui_scale`. El chat
   no pasó a `rem`. Tests: `the_zoom_scales_the_chat_and_the_editor_and_resets`
   (zoom 1,5 → tamaños del chat, composer y editor × 1,5; reset → 1,0),
   `the_zoom_multiplies_every_size`, `the_zoom_reaches_the_composer_font`.
2. **Selección visible en el chat.** `text.accent` al 35 %
   (`ChatTheme::text_selection`, `SELECTION_ALPHA`) en los dos campos que usa
   el `TextView` de `gpui-kit`: el `selection` de su `TextViewStyle` (el que
   pinta párrafos, código en línea y bloques) y, como respaldo, el token
   `colors.selection` del tema de `gpui-kit` (sincronizado a `gpui-base`).
   El composer usa el mismo color. El `selection` del editor de código no
   cambió. Tests: `the_chat_selection_is_the_accent_at_35_percent_in_both_themes`
   (oscuro y claro: kit, base y `TextViewStyle`; editor intacto),
   `the_text_views_select_with_the_accent_at_35_percent`,
   `the_selection_is_the_accent_at_35_percent`.
3. **Composer Markdown y burbujas renderizadas.** El `Textarea` de
   `gpui-kit` se reemplazó por un `cincel_editor::EditorView` con Markdown
   del registro, ajuste de línea, sin gutter (`EditorChrome::Minimal`), alto
   automático de 1 a 8 filas (`EditorSettings::auto_height`, medido con
   `request_measured_layout` al ancho real) y prosa en la fuente de la
   interfaz con el código en la mono (`EditorSettings::prose_font_family`; el
   ajuste de línea mide los avances reales de los glifos). Todo es aditivo en
   el editor: `TextDecorations` / `Decorator` + `set_decorator`,
   `set_row_backgrounds`, `set_placeholder`, `set_read_only`, `set_text`,
   `cursor_offset`, `is_empty`. `Enter`: la caja del composer (contexto
   `Composer`, `Chat > Composer`) toma `editor::insert_newline` en la fase de
   captura, antes que el editor, y responde el permiso pendiente, confirma el
   popover o envía; flechas, `Tab` y `Esc` manejan el popover; `Shift+Enter`
   es `chat::newline`. Los popovers `@` / `/` quedan anclados al borde
   superior del composer aunque crezca. Mapeo de colores (solo los 12 tokens
   de sintaxis):

   | Markdown | Token |
   |---|---|
   | línea de encabezado; marcadores de lista (`-` `*` `+` `1.` `1)`), de cita (`>`) y de énfasis (`**` `__` `*` `_` `~~`) | `keyword` |
   | código en línea (con los backticks) | `string` |
   | líneas de fence (con el info string), regla horizontal, URL de un enlace | `comment` |
   | texto de un enlace (con corchetes), menciones `@archivo` | `function` |
   | código de un fence con lenguaje conocido | capturas de ese lenguaje |

   Cada fila de un fence (fences incluidos) lleva fondo `bg.editor` (el
   composer está sobre `bg.surface`) y va en mono. La burbuja del usuario
   pasa por el mismo renderer que las respuestas; un párrafo con menciones se
   arma con texto y chips en línea. Tests: `composer::tests::*` (7),
   `the_composer_is_a_markdown_editor_without_gutter`,
   `shift_enter_breaks_the_line_and_enter_sends_it`,
   `the_composer_paints_the_markdown_structure`,
   `the_composer_grows_to_eight_rows_and_then_scrolls`,
   `a_pending_permission_makes_the_composer_read_only`,
   `a_bubble_is_markdown_except_the_paragraphs_with_mentions`,
   `a_sent_message_is_rendered_as_markdown`, `text_field_tests::*` (8) en el
   editor; los tests del composer anteriores (`@`, `/`, flechas, `Tab`, `Esc`,
   clic, permiso con `Enter`) se adaptaron y pasan.
   - Capturas de `chat_demo --markdown-composer` (borrador con encabezado,
     lista, negrita, código en línea, enlace y un bloque Python; después
     enviado): marcadores en violeta `#c678dd`, código en línea en verde,
     fences y URL en gris `#5c6370`, texto del enlace en azul, Python con sus
     colores de One Dark sobre la banda `bg.editor`; la burbuja muestra lista,
     código en línea, enlace y el bloque con cabecera "python · Copiar". Nada
     fuera de la paleta.
   - Desviaciones: la negrita y la cursiva del markdown no cambian el peso de
     la fuente en esta máquina, ni en la burbuja ni en las respuestas (ya
     pasaba; es el renderer de `gpui-kit` con la fuente del sistema). En las
     filas de prosa `↑` / `↓` conservan la columna en caracteres, no la x en
     píxeles. Una mención dentro de una lista o de un bloque de código vuelve
     texto plano ese bloque de la burbuja.
4. **Colores del pill.** Habilitado: `✓` en `status.ok`, `✗` en
   `status.error`, palabras en `text`, opacidad plena, `bg.surface` detrás de
   la mitad bajo el mouse; deshabilitado: todo `text.muted` y el spinner. La
   barra flotante: `✓ Aceptar` / `✗ Rechazar` con los mismos colores y
   `bg.surface` en el botón bajo el mouse. Su lógica de habilitado se mantuvo
   tal cual estaba (activos siempre que la barra lo esté, sobre el segmento
   del cursor o el actual); el pedido decía "solo con un segmento bajo el
   cursor", pero el código nunca lo hizo y cambiarlo rompía el comportamiento
   existente. Tests: `an_enabled_pill_reads_as_enabled_and_highlights_the_hovered_half`,
   `a_disabled_pill_is_muted_and_never_highlighted`,
   `the_bar_buttons_read_as_enabled_and_highlight_on_hover`.
5. **`+`/`−` por línea sin correr el código.** Fuera `REVIEW_COLUMN`: la
   columna de números mide al menos 3 dígitos (`MIN_NUMBER_DIGITS`) y los dos
   iconos se pintan sobre ella, pegados a su borde derecho con 2 px entre
   ellos, ocultando el número de esa fila; 14 px si entran, 11 px si no
   (`line_icon_size`: con 14 px de fuente tres dígitos miden ≈ 25 px, así
   que en la práctica son de 11). Clic y `Alt+Enter` / `Alt+Backspace` sin
   cambios. Tests: `the_gutter_is_as_wide_with_and_without_a_review`,
   `the_line_icons_sit_over_the_number_column_and_hide_its_number`,
   `the_line_icons_shrink_to_eleven_pixels_when_fourteen_do_not_fit`,
   `toggling_the_review_reshapes_no_text_row`.
6. **Pill que no tapa código.** Sigue siendo una capa alineada al borde
   derecho, de 24 px. Candidatas: primera fila del segmento, las siguientes,
   la fila de arriba; gana la primera cuyo texto (ancho ya medido de la
   línea) termina 12 px antes del pill. Sin candidata: pill compacto (dos
   botones de 24 × 24, 52 px, más una celda de spinner si el agente escribe)
   al 70 % en la primera fila; en el compacto no va el reloj de "Turno
   anterior". Los hitboxes siguen a lo pintado y nada se corre. Tests:
   `the_pill_sits_on_a_short_first_row`,
   `a_long_first_row_sends_the_pill_to_the_next_short_one`,
   `the_row_above_is_the_last_candidate`,
   `with_no_room_anywhere_the_pill_turns_compact_and_translucent` (incluye
   clic en los dos botones), `a_pure_deletion_places_the_pill_on_its_phantom_rows`.
