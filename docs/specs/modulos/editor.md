# Módulo `cincel-editor`

El elemento editor de código sobre GPUI. Propio, no derivado de otro editor. Referencia de diseño: el editor de Zed (arquitectura, no código).

## Estructura
- `EditorView` (entidad GPUI): posee `Entity<Buffer>` (de `cincel-text` envuelto), `SyntaxState`, `DisplayMap`, `Selection { head: Anchor, anchor: Anchor }`, scroll, estado de búsqueda, referencia al `ReviewStore` para su archivo.
- `DisplayMap` = `DiffTransformMap → WrapMap → BlockMap` (`03-arquitectura.md §5`). Tipos: `BufferRow`, `DisplayRow`; `to_display(BufferPoint) -> DisplayPoint`, `to_buffer(DisplayPoint) -> BufferPoint | Phantom { hunk, base_row }`.
- `DiffTransformMap`: se alimenta de `FileReview.hunks`; por cada hunk con `base_rows` no vacío inserta filas fantasma **antes** de `buffer_range.start`. Las filas fantasma tienen su propio `RopeSlice` del `base` y se resaltan con el mismo lenguaje.
- `EditorElement` (impl `Element`): virtualización (solo filas visibles ± 1 pantalla), layout de líneas con `window.text_system().shape_line`, caché de `ShapedLine` por `(buffer_version, base_version, row, width)`, pintado de fondos de línea (actual, diff), selección, cursor, coincidencias, gutter (números, barras de diff, iconos `+`/`−` al hover), pill flotante por hunk (bloque del `BlockMap`), barra flotante de revisión, scrollbar.
- `impl InputHandler for EditorView`: `text_for_range`, `selected_text_range`, `marked_text_range`, `replace_text_in_range`, `replace_and_mark_text_in_range`, `bounds_for_range` → soporte completo de IME y dead keys.
- Acciones (comandos GPUI con nombre): movimiento (carácter, palabra, línea, inicio/fin, página, documento), selección equivalentes con Shift, `editor::insert_newline` con indentación heredada, `editor::tab`/`backtab` (espacios/tab según ajuste y detección del archivo), `editor::delete_word_*`, `editor::undo/redo`, `editor::select_all`, `editor::copy/cut/paste`, `editor::find`, `editor::find_next/prev`, `editor::go_to_line`, `editor::toggle_soft_wrap`, `review::accept_hunk`, `review::reject_hunk`, `review::accept_line`, `review::reject_line`, `review::accept_file`, `review::reject_file`, `review::next_hunk`, `review::prev_hunk`.
- Contextos de teclado: `Editor`, `Editor && review_hunk_under_cursor`, `Editor && searching`. `Ctrl+Enter`/`Ctrl+Backspace` solo se ligan bajo `review_hunk_under_cursor`.
- Mouse: clic posiciona, arrastre selecciona, doble clic palabra, triple clic línea, `Ctrl+clic` no hace nada en v1 (sin multi-cursor), rueda desplaza, clic en fila fantasma posiciona el cursor en ella (solo lectura: se puede seleccionar y copiar).
- Búsqueda: barra sobre el editor (input de `gpui-kit`), coincidencia insensible a mayúsculas por defecto, regex opcional, contador `3/12`, resalta todas. Sus teclas (`Tab`/`Shift+Tab` para cambiar de campo, `Enter`/`Ctrl+Enter` para reemplazar uno o todos) viven, desde la Etapa 6, en el `keymap.json` por defecto (`Editor && searching`/`Editor && searching && replacing`, `modulos/settings.md`); `search_bar_bindings` (que antes las reinstalaba en código por encima del keymap del usuario) se retiró.
- Parpadeo del cursor: la tarea que lo hace parpadear (`view.rs`) termina a los 5 s sin escribir (o si `cursor_blink` está apagado) en vez de seguir despertando el hilo principal cada 500 ms para siempre; la próxima tecla la vuelve a lanzar. Es una de las dos causas que encontró la Etapa 6 para el consumo de CPU en reposo (la otra, los tres vigilantes de archivos: `modulos/watch.md`); con el cursor fijo, Cincel mide 0 % de CPU y 0 cuadros pintados en 60 s de reposo (M8, `docs/rendimiento.md`).
- Guardar: `editor::save` → `BufferStore::save`; indicador sucio en la pestaña.

## Reglas de la revisión en el editor
- Hunk bajo el cursor = el hunk cuyo `buffer_range` o filas fantasma contienen la fila del cursor. Si el cursor está en una línea concreta de un hunk multi-línea y la acción es `accept_line`/`reject_line`, aplica a ese `LinePair`.
- Al aceptar o rechazar, el cursor salta al siguiente hunk pendiente si `review.jump_to_next_on_decide` (por defecto `false` desde la Etapa 5).
- Mientras el turno está activo (`ReviewStore::turn_active(path)`), pill y barra muestran spinner y las acciones de review están deshabilitadas; la edición manual sigue permitida.
- Word diffs se pintan encima del fondo de línea, solo cuando `Hunk.word_diffs` existe.
- Al hacer clic en el pill o en `+`/`−`, el foco vuelve al editor (no se pierde el cursor).

## Etapa 5: git en el margen, buscar y reemplazar

Detalle, verificación y desviaciones en `docs/etapas/etapa-5.md`. Spec:
`docs/specs/07-etapa5-productividad.md` §6 y §10.2.

- **Git en el margen** (`git_gutter.rs`): `EditorView::set_git_diff(Option<Vec<GitGutterHunk>>,
  cx)` recibe filas del archivo **guardado**, en su propia columna de 3 px
  (`GitGutterKind::{Added, Modified, Deleted}`), sin cambiar el ancho del
  gutter (`GUTTER_PADDING_LEFT` y el `GUTTER_BAR_GAP` de la barra del agente
  ceden 5 px repartidos, no se agranda el gutter). El editor no conoce git:
  ancla las filas recibidas al buffer (`cincel_text::Buffer::track_anchor`) y
  las traslada a la versión actual del texto cuando hay ediciones sin
  guardar, así que siguen los cambios entre un cálculo y el siguiente. Las
  filas fantasma nunca llevan barra de git. Colores por
  `GitGutterColors`/`EditorView::set_git_colors` (tokens `git.added`,
  `git.modified`, `git.deleted`).
- **Buscar y reemplazar con filas fantasma** (`search.rs`, `Ctrl+H`):
  `MatchLocation::{Buffer, Phantom}` en orden de pantalla; la búsqueda
  recorre también el texto de las filas fantasma (las líneas que el agente
  quitó), que se resaltan y cuentan pero **no se reemplazan** — reemplazar
  ahí avisa en la barra y salta a la siguiente coincidencia real. "Reemplazar
  todo" es una única transacción del buffer (`EditSource::User`, un
  `Ctrl+Z` la deshace entera). Reemplaza la limitación de etapas anteriores
  de que la búsqueda no recorría las filas fantasma y no tenía reemplazar.

## Etapa 6: fin del parpadeo en reposo y teclas de búsqueda en el keymap

Detalle, verificación y desviaciones en `docs/etapas/etapa-6.md`. Spec:
`docs/specs/08-etapa6-cierre-1-0.md` §5.4 y §9.3 (E6-G).

- **`idle_timer_tests.rs`**: la tarea de parpadeo termina en reposo, una
  tecla la relanza, y con `cursor_blink` apagado no queda ninguna corriendo.
- **`keymap_search_tests_e6.rs`** (`cincel-workspace`): con el keymap por
  defecto, `Tab` en la barra cambia de campo y no inserta un tabulado;
  `Enter` en el campo de reemplazo reemplaza; un `keymap.json` de usuario
  que reasigna `tab` en su propia sección `Editor` (sin `searching`) gana
  también con la barra de búsqueda abierta (consecuencia aceptada de D14).

## Etapa 7: comentarios para el agente (E7-C)

Spec: `docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.5.

- **`BlockMap`** (`block_map.rs`): `DiffTransformMap → WrapMap → BlockMap`.
  `BlockPlacement { id, after_wrap_row, rows }`; `BlockMap::new(wrap_rows,
  placements)`, `to_visual_row`, `from_visual_row -> VisualCell::{Wrap,
  Block { id, row_in_block }}`, `total_rows`, `block_range`,
  `nearest_wrap_row`, `compensation`. El elemento pinta, desplaza y mide en
  filas visuales; cursor, selección, arrastre y búsqueda siguen en filas de
  ajuste y nunca caen en un bloque. Si un bloque de arriba de la primera
  fila visible aparece, desaparece o cambia de alto, `scroll_top` se
  compensa. Sin comentarios el mapa es la identidad y no reserva memoria.
- **Datos del anfitrión**: `EditorView::set_comments(Vec<ReviewCommentView
  { id, rows, text, from_hunk }>)` (filas del buffer, fin exclusivo; vacío =
  antes de `rows.start`) y `EditorView::set_display_name(Option<SharedString>)`
  para el título de la caja. El editor desplaza las filas al escribir arriba
  hasta el próximo `set_comments`, como con los segmentos.
- **Eventos**: `CommentAction::{Create { rows, from_hunk, text }, Edit { id,
  text }, Delete { id }}`, emitido como segundo tipo de evento de la vista
  (`EventEmitter<CommentAction>`, junto a `EditorEvent`). El editor no guarda
  nada: cierra la caja y el comentario aparece cuando vuelve `set_comments`.
- **Caja** (`view/comments.rs`): un `EditorView` hijo (`EditorChrome::Minimal`,
  fuente de la interfaz 13 px, de 2 a 8 filas), contexto `comment_box`;
  `Ctrl+Enter` / "Guardar" guarda (vacío = cancelar, o borrar si era una
  edición), `Esc` / "Cancelar" descarta; tope de 8 000 caracteres (la ayuda
  cuenta los que quedan pasados los 7 000). Guardada: plegada (una fila,
  "…") o desplegada (clic en el texto o en la marca); "Editar"/"Borrar" con
  el mouse encima o con el cursor en sus filas. Consultas:
  `comment_draft()`, `comment_draft_editor()`, `is_comment_expanded(id)`,
  `toggle_comment(id)`, `comment_hunk(id)`, `edit_comment(id)`,
  `delete_comment(id)`, `block_map()`, `visual_row_count()`.
- **Marca del margen**: `MessageSquareText` de 10 px en `text.accent`,
  centrada en el hueco de 12 px entre números y texto, en la primera fila del
  rango (la primera fila roja si es de un segmento); clic pliega/despliega,
  globo con los primeros 200 caracteres.
- **Botones del segmento**: tres partes iguales (`PILL_WIDTH` 280 px;
  compactos: tres de 24 × 24, 80 px). "Comentar" (`review::comment_hunk`)
  sigue habilitado mientras el agente escribe; si el segmento ya tiene un
  comentario de su botón, lo abre para editar.
- **Acciones y teclas** (`default_key_bindings`): `editor::comment_selection`
  (`ctrl-shift-m`, `Editor`; sin selección dentro de un segmento = su
  "Comentar"; nada en `EditorChrome::Minimal`; en la pestaña de solo lectura
  de un archivo borrado comenta su único segmento), `editor::save_comment`
  (`ctrl-enter`) y `editor::cancel_comment` (`escape`) en `Editor &&
  comment_box`, al final; `review::comment_hunk` sin tecla.
  `DEFAULT_KEYMAP_JSONC` (`cincel-settings/src/defaults.rs`) suma las mismas
  filas (E7-F), incluida la sección `Editor && comment_box` al final.
- **Menú contextual** (clic derecho sobre el texto): `PopupMenu` de gpui-kit
  que la vista abre y suelta al cerrarse, con el foco del editor como
  contexto de acción: "Cortar", "Copiar", "Pegar", separador, "Comentar
  selección" / "Comentar línea"; solo lectura: "Copiar" y "Comentar
  selección". Un clic fuera de la selección mueve antes el cursor.
- **`EditorSettings::ui_scale`** (1 por defecto): escala la caja, la marca y
  los botones del segmento.
- Tests: `block_map_tests.rs`, `comment_box_tests.rs`, `comment_rows_tests.rs`.

## Ronda 2 de la Etapa 7: margen al final y mejoras al escribir (`docs/specs/10-etapa7-ronda2.md` §6 y §7)

Correcciones y mejoras tras la prueba del autor, dentro de la misma 0.2.0. Todo vale solo en `EditorChrome::Full` (el compositor del chat y la caja de comentario, `Minimal`, no cambian), salvo `ScrollLineUp`/`ScrollLineDown`.

- **Margen al final** (§6): `EditorView::end_margin(viewport)` = `floor(filas_visibles / 2)` filas (0 en `Minimal`) y `EditorView::max_scroll_top(viewport)` = todas las filas visuales + margen − alto de la vista. Los usan la rueda, la barra (el alto del pulgar suma el margen), `set_scroll_row`, `scroll_rows` y el `prepaint` de `element.rs`. Es solo espacio: el buffer, el número de líneas y el ancho del margen izquierdo no cambian.
- **Desplazar sin mover el cursor** (E7): acciones `ScrollLineUp` (`editor::scroll_line_up`, `ctrl-up`) y `ScrollLineDown` (`editor::scroll_line_down`, `ctrl-down`), que llaman `scroll_rows(±1.)`. `ToggleWhitespace` (`editor::toggle_whitespace`) tiene ahora el binding `ctrl-alt-w` (en la pestaña actual). Los tres, en `default_key_bindings` y en `DEFAULT_KEYMAP_JSONC`.
- **Portapapeles de línea** (E2, `view/editing.rs`): `Ctrl+C`/`Ctrl+X` sin selección copian o cortan la fila entera con su salto de línea; `LineClipboard(Option<String>)` (`gpui::Global`) recuerda que ese texto fue "línea entera" y `LineCopyMetadata { whole_line }` viaja como metadato JSON del portapapeles. `Ctrl+V` sin selección pega esa línea al comienzo de la fila del cursor.
- **Sangría** (E4, E5, E6, `view/editing.rs` y `ops.rs`): `outdent_for_closer` (al escribir `}`, `]` o `)` en una fila que solo tiene sangría, quita un nivel; helper puro `ops::outdent_once`), `newline_on_indent_row` (`Enter` en una fila de solo sangría la deja vacía y pasa la sangría a la fila nueva) y `backspace_in_indent` (`Backspace` dentro de la sangría borra hasta el tope anterior; helper puro `ops::spaces_to_previous_stop`). Usan el nivel de sangría del archivo (`indent_unit`).
- **Limpieza al guardar** (E8): `ops::save_cleanup_edits` (pura) arma las ediciones; `SaveCleanup { trim_trailing_whitespace, ensure_final_newline }` y `EditorView::clean_up_for_save(options, cx) -> bool` las aplican como una sola transacción del buffer (`EditSource::User`, un `Ctrl+Z` la deshace) sin tocar las filas de segmentos pendientes del agente. La llama `Center::save_path` del workspace (ver `modulos/workspace.md`).
- **Apariciones de la palabra** (E1): `search.rs::OccurrenceQuery`, `find_occurrences` (pura), `view.rs::OCCURRENCE_DELAY` (150 ms; tarea de un solo disparo, en reposo no queda nada corriendo) y `theme::OCCURRENCE_ALPHA` (0,5 sobre el color `selection`). Se pintan como las coincidencias de búsqueda, solo en las filas reales visibles y no con la barra de búsqueda abierta con una consulta. Accesor de pruebas: `occurrence_ranges_for_test`.
- Tests nuevos: `scroll_margin_tests.rs`, `occurrences_tests.rs`, `line_clipboard_tests.rs`, `indent_editing_tests.rs`, `whitespace_toggle_tests.rs` y los unitarios de `ops.rs`.

## Criterios de aceptación
- [ ] Abrir un archivo de 5 000 líneas y hacer scroll de punta a punta mantiene 60 fps (medido con el instrumentador de GPUI) y no aloca layouts fuera del rango visible.
- [ ] Escribir "á", "ñ", "ü" con teclado en español (dead keys) y con IME (`ibus`/`fcitx`) funciona; `Ctrl+Shift+U` de GTK no es requisito.
- [ ] Con un hunk que elimina 2 líneas y añade 3: se ven 2 filas rojas fantasma sin número seguidas de 3 filas verdes numeradas; el cursor puede entrar a las rojas, seleccionar y copiar, pero escribir en ellas escribe en la primera verde.
- [ ] `Ctrl+Enter` con el cursor en un hunk lo acepta; fuera de un hunk inserta salto de línea. `Ctrl+Backspace` idem con rechazar / borrar palabra.
- [ ] Rechazar un hunk restaura el texto, guarda el archivo y muestra el aviso con "Deshacer".
- [ ] El pill sigue al hunk cuando el usuario escribe líneas por encima.
- [ ] Copiar/pegar con el portapapeles de Wayland funciona (incluye pegar desde otra app).
