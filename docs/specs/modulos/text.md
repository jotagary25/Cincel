# Módulo `asteroid-text`

Buffer de texto sin dependencias gráficas.

## Responsabilidades
- `Buffer`: rope (`ropey`) + versión monótona (`u64`) + historial de transacciones.
- **Anclas**: `Anchor { offset, bias: Before | After }` que se transforman con cada edición. API: `buffer.anchor_before(offset)`, `anchor_after`, `buffer.resolve(&anchor) -> usize`. Un conjunto de anclas se actualiza en O(k log n) por edición.
- **Transacciones**: `buffer.start_transaction(source)`, `edit(ranges, text)`, `end_transaction()`. `EditSource { User, Agent { turn_id }, Review, Load }`. Cada transacción cerrada es una unidad de undo. Transacciones `User` consecutivas dentro de 300 ms se agrupan (como Zed).
- **Undo/redo** por transacción; `undo()` devuelve el `EditSource` de lo deshecho.
- **Eventos**: `BufferEvent::Edited { source, old_ranges, new_ranges, version }` para que review, syntax y editor reaccionen.
- **Fin de línea y BOM**: al cargar se detecta `LF`/`CRLF` y BOM; en memoria siempre `LF` sin BOM; al serializar para guardar se reaplican. `line_ending()` consultable.
- **Codificación**: UTF-8; si el archivo no es UTF-8 válido, se abre en solo lectura con aviso (v1 no convierte).
- Utilidades: `Point { row, column }` ⇄ offset, `line(row) -> RopeSlice`, `text_in(range)`, `snapshot() -> BufferSnapshot` (clon barato del rope + versión, `Send`).

## Criterios de aceptación
- [ ] Anclas sobreviven a inserciones y borrados antes, dentro y después de ellas con el sesgo correcto (tests con 1 000 ediciones aleatorias vs. modelo ingenuo).
- [ ] `undo` de una transacción `Agent` restaura exactamente el texto previo y devuelve `EditSource::Agent`.
- [ ] Un archivo CRLF cargado y guardado sin cambios produce bytes idénticos (también con BOM).
- [ ] `snapshot()` de un buffer de 10 MB tarda < 1 ms.
