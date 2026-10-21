# Spec 11: Etapa 7, ronda 3 (barra flotante que se achica y botones del segmento en un solo lugar)

Estado: v1.0 (2026-10-21), definida con el autor tras probar la 0.2.1. Fuente de verdad de la **ronda 3 de la Etapa 7**. Dos cambios en el editor de revisión, los dos en `crates/cincel-editor/src/element.rs`. Versión propuesta: **0.2.2** (se confirma al cerrar).

Esta spec corrige `02-visual.md` §6.1 ("Dónde se pone el pill", el pill compacto al 70 %) y §6.2 (la barra flotante: su contenido ahora depende del ancho), `modulos/editor.md` y, si lo describen, `docs/usuario/revision.md`. El orquestador lo aplica al cerrar.

---

## 1. Qué trae esta ronda (lenguaje llano)

1. **La barra de abajo ya no se corta.** Cuando el panel del editor es angosto, la barra flotante de abajo a la derecha ("Aceptar todo · Rechazar todo · ↑ ↓ · cambio 1 de 17 · Revisar todo") se achica por etapas en vez de salirse por la izquierda: primero pierde los atajos de teclado escritos, después deja solo los iconos. Nunca cambia de alto ni se parte en dos filas. Decisión del autor (2026-10-21).
2. **Los botones de cada segmento (Aceptar / Rechazar / Comentar) van siempre en el mismo lugar**: la línea inmediatamente anterior al segmento, a la derecha, sin importar qué texto haya en esa línea. Pueden tapar código de arriba; **nunca tapan el segmento**. Si el segmento empieza en la primera línea del archivo, van en la línea inmediatamente posterior. Si el panel es tan angosto que no entran, se achican a tres iconos, pero por el ancho del panel, nunca por el texto. Desaparece la regla de hoy (buscar una fila con lugar, y si no hay, pintarlos chiquitos y translúcidos encima del segmento). Decisión del autor (2026-10-21): "no me importa si tapa el código de arriba, lo importante es que no tape nada del segmento".

Límite aceptado por el autor: en un segmento más alto que la pantalla, cuando la línea de arriba sale de la vista, los botones se van con ella; quedan `Ctrl+↵` / `Ctrl+⌫` y la barra de abajo.

---

## 2. Decisiones técnicas

| # | Decisión | Motivo |
|---|---|---|
| R1 | La barra flotante tiene tres **niveles** de contenido, elegidos por el ancho disponible `avail = bounds.width − 2·BAR_MARGIN`: se arma el nivel 0; si su ancho supera `avail`, se arma el 1; si también, el 2. El nivel 2 no se reduce más: si aun así no entra, se pega al borde izquierdo (`origin.x = max(bounds.left + BAR_MARGIN, …)`) y se recorta por la derecha, nunca por la izquierda. | Elegido por el autor frente a partirla en dos filas. El nivel 2 mide ~150 px; un panel más angosto que eso no es un caso de uso. |
| R2 | Nivel 0 = el contenido de hoy. Nivel 1 = sin los atajos escritos: se quitan los ítems "Ctrl+Alt+↵" y "Ctrl+Alt+⌫", y "↑ Alt+K" / "↓ Alt+J" pasan a "↑" / "↓" (siguen siendo botones). Nivel 2 = solo iconos: "✓" (aceptar todo), "✗" (rechazar todo), "↑", "↓", "N/M" (posición) y "≡" en `text.accent` (Revisar todo). Separadores "·" solo en los niveles 0 y 1. | Los atajos ya están en los tooltips de los botones del panel, en el modal de atajos y en `docs/usuario/atajos.md`. |
| R3 | Estado "El agente está editando…": nivel 0 como hoy; nivel 1 = "Editando…"; nivel 2 = solo el indicador giratorio. Aviso "Sin cambios pendientes en este archivo · N en otros archivos → siguiente archivo (Alt+L)": nivel 1 = "N en otros archivos → siguiente (Alt+L)"; nivel 2 = "→ siguiente archivo" (botón). | Los tres estados de la barra siguen la misma regla; ninguno puede cortarse. |
| R4 | El pill va en la fila de pantalla inmediatamente anterior a la primera fila del segmento (`range.start − 1`), alineado a la derecha del área de texto como hoy (`PILL_MARGIN` antes de la barra de desplazamiento), sin mirar el texto de esa fila. Si `range.start == 0`, va en la fila inmediatamente posterior al segmento (`range.end`). Si tampoco existe (el archivo entero es el segmento), va en la primera fila del segmento: único caso en que lo tapa, anotado en §13. | Decisión del autor. Fila de pantalla = fila de `DisplayMap` (fantasma incluidas): en una eliminación pura, la fila anterior a las fantasma. |
| R5 | Se pinta solo mientras esa fila está dentro de las filas visibles (`first_row..last_row`); si no, el segmento no tiene pill en ese cuadro. No se pega al borde. | Pegarlo al borde superior lo pondría sobre el segmento. Límite aceptado (§1). |
| R6 | El pill compacto (tres botones de 24 × 24, 4 px entre ellos, 80 px) se usa **solo** cuando `PILL_WIDTH × escala` no entra entre el origen del texto y `pill_right` (`pill_right − text_origin_x < PILL_WIDTH × escala`). Opacidad plena: `COMPACT_PILL_OPACITY` se elimina. | Ya no está sobre el segmento, no hay motivo para translucidez. |
| R7 | Se eliminan `place_pill`, `PillCandidate`, `PILL_TEXT_GAP` y la construcción de `candidate_rows`. `PillLayout.compact` queda (ahora significa "por ancho"). Nada de lo demás del pill cambia: cursor/hover, colores, reloj "Turno anterior", spinner, "Comentar", clics, foco. | Quitar código muerto; D16 sigue valiendo (ni el margen ni la columna del texto cambian). |
| R8 | Accesores `test-support`: `BarLayout.level: u8` expuesto por el accesor de la barra que ya usan los tests, y nada nuevo para el pill (los tests leen `display_row`, `compact`, `bounds`). | Lo mínimo para probar. |

---

## 3. Barra flotante por niveles (A)

### 3.1 Comportamiento
- Con el panel ancho (≥ 1 000 px) la barra es igual a hoy.
- Al angostar el panel, la barra pasa al nivel 1 y después al 2 (R1–R3). Siempre entera dentro del panel, siempre 30 px de alto, siempre pegada abajo a la derecha con 24 px de margen.
- Cada botón hace lo mismo en los tres niveles (aceptar/rechazar el turno, segmento anterior/siguiente, abrir el panel de revisión, siguiente archivo). El hover y el cursor de mano también.

### 3.2 Diseño técnico
- `layout_bar`: separar la construcción de `specs` en `fn bar_specs(level: u8, …) -> Vec<BarSpec>`; medir con `shape_ui_runs`; elegir el nivel más alto que entre (R1); guardar `level` en `BarLayout`.
- El ancho disponible se calcula sobre `bounds` del elemento (el área del editor, sin el margen izquierdo).

### 3.3 Criterios de aceptación
- [ ] A 1 200 px de ancho, la barra contiene "Ctrl+Alt+↵" (nivel 0).
- [ ] A 560 px no contiene "Ctrl+Alt+↵" ni "Alt+K", sí "Aceptar todo" y "↑" (nivel 1).
- [ ] A 380 px contiene "✓", "✗", "↑", "↓", "1/3" y "≡" y ninguna palabra (nivel 2).
- [ ] En los tres anchos, `bar.bounds.left ≥ bounds.left + BAR_MARGIN` y `bar.bounds.right = bounds.right − BAR_MARGIN`; alto 30 px.
- [ ] Un clic en "✓" del nivel 2 emite la misma acción que "✓ Aceptar todo" del nivel 0; ídem "≡" / "Revisar todo" y "↓" / "↓ Alt+J".
- [ ] Con el agente escribiendo, a 380 px la barra muestra solo el indicador; el aviso de "sin cambios" a 380 px muestra "→ siguiente archivo" y sigue siendo botón.

### 3.4 Tests
`crates/cincel-editor/src/bar_levels_tests.rs` (nuevo; `VisualTestContext`, redimensionando la ventana con `simulate_resize`): los seis criterios. Los tests existentes de la barra en `review_tests.rs` (`the_bar_lists_the_actions_and_the_position`, etc.) corren a ancho grande y no cambian.

---

## 4. El pill en un solo lugar (B)

### 4.1 Comportamiento
R4–R6. Además:
- El pill de un segmento se muestra cuando el cursor está en el segmento o el mouse pasa por él o por el pill, como hoy.
- Un clic en el pill sigue haciendo lo de hoy; el foco vuelve al editor.
- Ninguna fila del segmento queda debajo del pill (salvo el caso "archivo entero").

### 4.2 Diseño técnico
En el `prepaint` de `element.rs`, reemplazar el bloque de `candidate_rows` / `place_pill` por: `let row = if range.start > 0 { range.start − 1 } else if range.end < total_display_rows { range.end } else { range.start }`; `row_y` con la misma función que hoy (`text_right` sin usar el ancho) o `wrap_y`; si la fila no está entre `first_row` y `last_row`, `continue`; `compact = pill_right − text_origin_x < PILL_WIDTH × escala`. Borrar lo listado en R7 y el párrafo "Where the pill goes" del doc del módulo (reescribirlo).

### 4.3 Criterios de aceptación
- [ ] Un segmento que empieza en la fila 5 tiene el pill en la fila 4, aunque la fila 4 tenga un texto que ocupe todo el ancho (el pill queda encima del texto).
- [ ] Un segmento que empieza en la fila 0 tiene el pill en la fila `range.end`.
- [ ] Una eliminación pura (filas fantasma) tiene el pill en la fila anterior a la primera fantasma.
- [ ] Ningún `pill.bounds` se superpone con el rango vertical de las filas de su segmento (test que recorre varios segmentos y anchos).
- [ ] Con el área de texto más angosta que 280 px, el pill es compacto (`compact == true`), en la fila de arriba, con opacidad 1.
- [ ] Con un segmento de 200 filas y la vista desplazada 50 filas hacia abajo, no hay pill; al volver arriba, está.
- [ ] El margen y la columna del texto no cambian con o sin pill (D16, como el test actual).
- [ ] Los clics en las tres partes y el reloj "Turno anterior" siguen funcionando (tests existentes, ajustados a la fila nueva).

### 4.4 Tests
En `review_tests.rs`, con `Edit`: reemplazar `the_pill_sits_on_a_short_first_row`, `a_long_first_row_sends_the_pill_to_the_next_short_one`, `the_row_above_is_the_last_candidate`, `with_no_room_anywhere_the_pill_turns_compact_and_translucent` y `a_pure_deletion_places_the_pill_on_its_phantom_rows` por los criterios de §4.3 (nombres nuevos, uno por criterio); ajustar los demás tests del pill que asuman la fila vieja. Nuevos que no quepan ahí van en `crates/cincel-editor/src/pill_placement_tests.rs`.

---

## 5. Reglas fijas de construcción
Las de la spec 10 §14.1, sin cambios: mismo árbol, `Edit` en archivos compartidos, tests nuevos en archivos nuevos, features de test fijas (`cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`), sin `cargo sweep` salvo el orquestador, cargo en primer plano con tope, nada de commits, nada de capturas, sin datos personales. Un solo subagente (los dos cambios están en `element.rs`).

## 6. Verificación (orquestador)
Los comandos de la spec 10 §17 menos los bancos de rendimiento, más `--bench typing` y `--bench idle` una vez (el `prepaint` del pill se simplifica, no puede empeorar). Paquetes con `packaging/build.sh` + `verify.sh`.

## 7. Lista de comprobación manual (autor)
1. Panel del editor ancho: la barra de abajo se ve como siempre, con los atajos.
2. Angostá el panel de a poco: la barra pierde primero los atajos, después las palabras; nunca se corta ni se parte. Tocá "✓" y "≡" en la versión de iconos: hacen lo mismo que antes.
3. Con cambios del agente pendientes: los botones Aceptar / Rechazar / Comentar están en la línea de arriba del segmento, aunque esa línea tenga texto largo; nunca sobre el segmento.
4. Un cambio que empiece en la línea 1 del archivo: los botones están en la línea de abajo del segmento.
5. Un cambio muy largo: bajá hasta que la línea de arriba salga de la vista; los botones no están; `Ctrl+↵` acepta igual; subí y vuelven.
6. Panel muy angosto: los botones son tres iconos, nítidos, en la línea de arriba.
7. `cincel --version` dice la versión de cierre; instalar el `.deb` y repetir 2 y 3.

## 8. Cierre
Versión y `CHANGELOG.md` (entrada breve), `<release>` en `packaging/linux/dev.cincel.Cincel.metainfo.xml`, nombres de archivo del `README.md`, correcciones de `02-visual.md` §6.1 y §6.2, `modulos/editor.md`, `docs/usuario/revision.md` si describe la posición; sección "Ronda 3" en `docs/etapas/etapa-7.md`; paquetes en `dist/`.
