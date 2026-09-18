# Módulo `asteroid-editor`

El elemento editor de código sobre GPUI. Propio, no derivado de otro editor. Referencia de diseño: el editor de Zed (arquitectura, no código).

## Estructura
- `EditorView` (entidad GPUI): posee `Entity<Buffer>` (de `asteroid-text` envuelto), `SyntaxState`, `DisplayMap`, `Selection { head: Anchor, anchor: Anchor }`, scroll, estado de búsqueda, referencia al `ReviewStore` para su archivo.
- `DisplayMap` = `DiffTransformMap → WrapMap → BlockMap` (`03-arquitectura.md §5`). Tipos: `BufferRow`, `DisplayRow`; `to_display(BufferPoint) -> DisplayPoint`, `to_buffer(DisplayPoint) -> BufferPoint | Phantom { hunk, base_row }`.
- `DiffTransformMap`: se alimenta de `FileReview.hunks`; por cada hunk con `base_rows` no vacío inserta filas fantasma **antes** de `buffer_range.start`. Las filas fantasma tienen su propio `RopeSlice` del `base` y se resaltan con el mismo lenguaje.
- `EditorElement` (impl `Element`): virtualización (solo filas visibles ± 1 pantalla), layout de líneas con `window.text_system().shape_line`, caché de `ShapedLine` por `(buffer_version, base_version, row, width)`, pintado de fondos de línea (actual, diff), selección, cursor, coincidencias, gutter (números, barras de diff, iconos `+`/`−` al hover), pill flotante por hunk (bloque del `BlockMap`), barra flotante de revisión, scrollbar.
- `impl InputHandler for EditorView`: `text_for_range`, `selected_text_range`, `marked_text_range`, `replace_text_in_range`, `replace_and_mark_text_in_range`, `bounds_for_range` → soporte completo de IME y dead keys.
- Acciones (comandos GPUI con nombre): movimiento (carácter, palabra, línea, inicio/fin, página, documento), selección equivalentes con Shift, `editor::insert_newline` con indentación heredada, `editor::tab`/`backtab` (espacios/tab según ajuste y detección del archivo), `editor::delete_word_*`, `editor::undo/redo`, `editor::select_all`, `editor::copy/cut/paste`, `editor::find`, `editor::find_next/prev`, `editor::go_to_line`, `editor::toggle_soft_wrap`, `review::accept_hunk`, `review::reject_hunk`, `review::accept_line`, `review::reject_line`, `review::accept_file`, `review::reject_file`, `review::next_hunk`, `review::prev_hunk`.
- Contextos de teclado: `Editor`, `Editor && review_hunk_under_cursor`, `Editor && searching`. `Ctrl+Enter`/`Ctrl+Backspace` solo se ligan bajo `review_hunk_under_cursor`.
- Mouse: clic posiciona, arrastre selecciona, doble clic palabra, triple clic línea, `Ctrl+clic` no hace nada en v1 (sin multi-cursor), rueda desplaza, clic en fila fantasma posiciona el cursor en ella (solo lectura: se puede seleccionar y copiar).
- Búsqueda: barra sobre el editor (input de `gpui-kit`), coincidencia insensible a mayúsculas por defecto, regex opcional, contador `3/12`, resalta todas.
- Guardar: `editor::save` → `BufferStore::save`; indicador sucio en la pestaña.

## Reglas de la revisión en el editor
- Hunk bajo el cursor = el hunk cuyo `buffer_range` o filas fantasma contienen la fila del cursor. Si el cursor está en una línea concreta de un hunk multi-línea y la acción es `accept_line`/`reject_line`, aplica a ese `LinePair`.
- Al aceptar o rechazar, el cursor salta al siguiente hunk pendiente si `review.jump_to_next_on_decide` (por defecto `true`).
- Mientras el turno está activo (`ReviewStore::turn_active(path)`), pill y barra muestran spinner y las acciones de review están deshabilitadas; la edición manual sigue permitida.
- Word diffs se pintan encima del fondo de línea, solo cuando `Hunk.word_diffs` existe.
- Al hacer clic en el pill o en `+`/`−`, el foco vuelve al editor (no se pierde el cursor).

## Criterios de aceptación
- [ ] Abrir un archivo de 5 000 líneas y hacer scroll de punta a punta mantiene 60 fps (medido con el instrumentador de GPUI) y no aloca layouts fuera del rango visible.
- [ ] Escribir "á", "ñ", "ü" con teclado en español (dead keys) y con IME (`ibus`/`fcitx`) funciona; `Ctrl+Shift+U` de GTK no es requisito.
- [ ] Con un hunk que elimina 2 líneas y añade 3: se ven 2 filas rojas fantasma sin número seguidas de 3 filas verdes numeradas; el cursor puede entrar a las rojas, seleccionar y copiar, pero escribir en ellas escribe en la primera verde.
- [ ] `Ctrl+Enter` con el cursor en un hunk lo acepta; fuera de un hunk inserta salto de línea. `Ctrl+Backspace` idem con rechazar / borrar palabra.
- [ ] Rechazar un hunk restaura el texto, guarda el archivo y muestra el aviso con "Deshacer".
- [ ] El pill sigue al hunk cuando el usuario escribe líneas por encima.
- [ ] Copiar/pegar con el portapapeles de Wayland funciona (incluye pegar desde otra app).
