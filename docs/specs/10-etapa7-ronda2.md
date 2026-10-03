# Spec 10: Etapa 7, ronda 2 (correcciones y mejoras tras la prueba del autor)

Estado: v1.0 (2026-10-01), definida con el autor. Fuente de verdad del alcance de la **ronda 2 de la Etapa 7**: lo que el autor pidió después de probar la 0.2.0 sin publicar. Entra en la **misma versión 0.2.0**, antes de publicarla (la etiqueta `v0.2.0` todavía no existe).

Modelo de formato: `09-etapa7-conexiones-imagenes-comentarios.md`. Cada ítem tiene primero un **resumen en lenguaje llano** (para el autor) y después el **diseño técnico** (para los subagentes). Las letras A–J son las de lo acordado con el autor.

Esta spec corrige otras en estos puntos (el orquestador los aplica al cerrar, R2-J; no se tocan antes):
- `09-etapa7-conexiones-imagenes-comentarios.md` §3 (y su D2): el encabezado del chat deja de mostrar el tipo de agente y la insignia de la conexión; las filas del menú pasan a dos líneas (§3 y §4 de esta spec). §5.1/§5.2: en el mensaje enviado las imágenes van **arriba** del texto (§5).
- `02-visual.md` §5 (margen al final del archivo, apariciones de la palabra, espacios en blanco con atajo), §7 (encabezado, menú, imágenes, fila de actividad "Pensando… / Trabajando… / Escribiendo…" en la conversación, flecha "ir al final", bloques de código plegados) y §8 (atajos nuevos).
- `modulos/chat.md`, `modulos/editor.md`, `modulos/workspace.md` y `modulos/settings.md` (claves nuevas de `files`).

---

## 1. Qué trae esta ronda (resumen en lenguaje llano)

1. **Arriba del chat, solo el nombre y el estado de la conexión (A).** El botón de arriba muestra el nombre completo que le pusiste a la conexión y una sola etiqueta de color que dice cómo está la conexión: "conectada", "desconectada", "sesión vencida", "no disponible" o "autenticación requerida". Esa etiqueta no cambia con lo que hace el agente (eso va abajo, punto 6). Desaparecen de ahí el tipo de agente (Claude, Codex…) y la etiqueta "Conectada" de antes. El nombre aprovecha todo el lugar que quedó libre; solo si el panel es tan angosto que no entra, se corta con "…" y el nombre entero aparece al pasar el mouse.
2. **El menú de conexiones, en dos líneas (B).** Cada conexión se ve así: a la izquierda el icono del agente; en el medio, arriba tu nombre para la conexión y abajo, en gris y más chico, qué agente es (Claude, Codex o Antigravity), donde antes decía "Usado hace…"; a la derecha la etiqueta de estado ("Conectada", "Sesión vencida"…). Sin correo ni plan.
3. **Las imágenes, arriba (C).** En el mensaje que mandaste, las imágenes se ven arriba del texto. En la caja donde escribís, las imágenes adjuntas ya estaban arriba del campo de texto: se deja así y se agrega una prueba para que no cambie.
4. **Se puede bajar más allá de la última línea (D).** En el editor podés seguir desplazando hasta que la última línea quede a media pantalla, como en VS Code. No se agregan líneas al archivo: es solo espacio vacío al final. Así la barra flotante de Aceptar/Rechazar ya no tapa las últimas líneas.
5. **Ocho mejoras al escribir código (E).**
   1. Al dejar el cursor sobre una palabra (o al seleccionar un trozo de una sola línea), las demás veces que aparece en lo que estás viendo se marcan con un fondo suave, un instante después de detenerte.
   2. `Ctrl+C` o `Ctrl+X` sin nada seleccionado copian o cortan la línea entera; al pegarla con `Ctrl+V` entra como una línea nueva arriba de la línea donde está el cursor, no en medio del texto.
   3. El margen al final del archivo (el punto 4).
   4. Si escribís `}`, `]` o `)` en una línea que solo tiene la sangría (los espacios del comienzo), Cincel le quita un nivel de sangría, para que el cierre quede alineado con su apertura.
   5. `Enter` en una línea que solo tiene sangría no deja espacios sueltos en la línea que queda atrás.
   6. `Backspace` dentro de la sangría borra un nivel entero (por ejemplo 4 espacios), no un espacio por vez.
   7. `Ctrl+↑` y `Ctrl+↓` desplazan el texto una línea sin mover el cursor. Y `Ctrl+Alt+W` muestra u oculta los espacios en blanco en la pestaña actual.
   8. Al guardar, Cincel quita los espacios que sobran al final de cada línea y se asegura de que el archivo termine con un salto de línea, como Zed. Las dos cosas vienen encendidas y se pueden apagar en Configuración → Archivos. No toca las líneas de un cambio del agente que todavía no decidiste.
6. **La actividad del agente va en la conversación (F).** Mientras el agente trabaja, abajo de todo en la conversación hay una fila con la ruedita girando que dice qué hace: "Pensando…" mientras razona, "Trabajando…" con una herramienta en curso, "Escribiendo…" mientras llega la respuesta, "Esperando permiso…" si pidió uno. Desaparece al terminar el turno. Ya no está arriba.
7. **Flecha para volver al final (G).** Si subiste a leer algo anterior, aparece un botón redondo con una flecha hacia abajo flotando sobre la conversación; al tocarlo vuelve al final.
8. **El chat no te mueve mientras leés (H).** Si estás al final, el chat va bajando solo a medida que llega la respuesta. Si subiste a leer, se queda donde lo dejaste: también cuando la respuesta termina (hoy salta al final en ese momento). Vuelve a seguir la respuesta cuando bajás hasta el final o tocás la flecha. Cuando mandás un mensaje, el chat baja al final (para que veas lo que mandaste).
9. **Bloques de código largos, plegados (I).** Un bloque de código de más de 20 líneas se muestra con sus primeras 12 líneas, un difuminado abajo y un botón "Ver más (N líneas)". Desplegado, al pie dice "Ver menos". Cincel recuerda cuál desplegaste mientras la conversación esté abierta. "Copiar" copia siempre el bloque entero.
10. **Ya arreglado (J).** Al retomar una conversación, a veces aparecía un mensaje tuyo suelto con "@a@b@c" (los archivos que habías mencionado). Ya no aparece.

A vos te queda, al final, la lista de comprobación manual (§16).

---

## 2. Decisiones técnicas de esta spec

| # | Decisión | Motivo |
|---|---|---|
| R1 | El encabezado pinta una sola insignia, `HeaderBadge`, con el estado de la **conexión** solamente, por prioridad: sesión vencida > no disponible > autenticación requerida > desconectada > conectada. La actividad del agente (pensando / trabajando / escribiendo / esperando permiso) va en la fila al pie de la conversación (§8). | Decisión del autor (2026-10-01): "que solo diga el estado, si está conectado/desconectado; pensando/trabajando/respondiendo queda para el estado de abajo". |
| R2 | Durante un turno la insignia del encabezado no cambia: dice "conectada" (o el estado de conexión que corresponda) todo el tiempo. La actividad se muestra solo abajo (§8): "Pensando…" con razonamiento o antes de la primera señal, "Trabajando…" con una herramienta en curso, "Escribiendo…" mientras llega la respuesta, "Esperando permiso…" con un pedido pendiente. | Decisión del autor (2026-10-01). |
| R3 | En el menú, la segunda línea muestra el tipo **siempre**, aunque el nombre de la conexión sea igual al tipo (la regla D2 de la spec 09 se deja solo para la lista de "Eliminar conexión…", que sigue en una línea). | Con dos líneas, todas las filas miden lo mismo y "Claude" arriba / "Claude" abajo no se lee como una palabra repetida. |
| R4 | El nombre del encabezado no se corta mientras entre; con el panel en su ancho mínimo (280 px) un nombre largo se corta con "…" y lleva tooltip con el nombre entero. | "Sin truncar" es imposible en 280 px con cualquier largo de nombre; esto es lo más cerca posible y no se pierde información. |
| R5 | La fila de actividad no es una `Entry` (no se guarda en la conversación): se pinta al pie del último elemento de la lista. Se muestra mientras el turno está en curso, con el texto de §8.1 según la última señal del agente ("Pensando…" / "Trabajando…" / "Escribiendo…" / "Esperando permiso…"). La única excepción es cuando el último elemento es la fila de pensamiento en vivo ("Pensando… (N s)" con su ruedita), que ya cumple el papel y no se duplica. | Es un estado, no un mensaje; pegado al último elemento no cambia la cantidad de filas de la lista ni dispara medidas nuevas. |
| R6 | El seguimiento del final usa el `FollowMode::Tail` que la lista de GPUI (`gpui-pre` 0.3.5, `ListState`) ya tiene: deja de seguir cuando el usuario sube (rueda, barra) y vuelve a seguir sola cuando la posición llega al final. Se **quitan** las llamadas a `ListState::scroll_to_end()` de `ChatPanel::push` (la causa del salto al terminar: `TurnEnded` agrega un separador con `push`). Quedan solo en "abrir conversación", "conversación nueva", "enviar mensaje" y la flecha de G, que además llaman `set_follow_mode(FollowMode::Tail)`. | El mecanismo ya existe y está probado en GPUI; el defecto es nuestro. Enviar un mensaje es un gesto del usuario que pide ver lo que mandó. |
| R7 | Bloques largos: la respuesta (y cada trozo Markdown de una burbuja) se parte en **segmentos**: texto corriente y bloques de código largos de nivel superior. Cada segmento es su propio `TextViewState` de gpui-kit; un bloque largo plegado lleva en **su** `TextView` un estilo de bloque con `max_h` y `overflow_hidden`. Sin tocar gpui-kit. | gpui-kit aplica un solo estilo a todos los bloques de un `TextView` y no deja intervenir el pintado de un bloque; con un `TextView` por bloque largo, el recorte es del propio bloque (su borde y sus esquinas quedan enteros) y la selección y el "Copiar" de gpui-kit siguen funcionando. |
| R8 | Solo se pliegan los bloques cercados que empiezan en la columna 0 (` ``` ` o `~~~`); un bloque dentro de una lista o de una cita (con sangría) se muestra entero, como hoy. | Partir el Markdown dentro de una lista cambiaría su numeración y su sangría; los agentes ponen los bloques largos en el nivel superior. Límite anotado en §13. |
| R9 | Un bloque se pliega también mientras llega (en cuanto pasa las 20 líneas). | Es la regla acordada; si no, un bloque de 300 líneas en curso llenaría la pantalla hasta terminar. |
| R10 | Margen al final del editor: media altura visible (`floor(filas_visibles / 2)` filas), solo en `EditorChrome::Full`. El compositor del chat y la caja de comentario (`Minimal`) no lo tienen. Sin ajuste en `settings.json`. | Es lo acordado (D) y en un campo de texto que crece con su contenido no tiene sentido. |
| R11 | Apariciones (E1): se pintan con el mismo camino que las coincidencias de búsqueda (`prepaint.matches` → `paint_quad`), en color `selection` al **50 %** (`OCCURRENCE_ALPHA = 0.5`). No se pintan mientras la barra de búsqueda tiene una consulta. Distinguen mayúsculas, palabra completa, solo filas reales visibles (no las fantasma), retardo de **150 ms**. | Reutiliza lo existente (pedido). El color de la selección, más tenue, es lo que hacen VS Code y Zed; el amarillo de la búsqueda confundiría las dos cosas. |
| R12 | Copiar sin selección (E2): el texto va al portapapeles del sistema y Cincel recuerda en un `Global` (`LineClipboard`) que ese texto fue "línea entera"; al pegar, si el portapapeles trae exactamente ese texto, se pega como línea. Además se escribe la marca como metadato JSON (`ClipboardItem::new_string_with_json_metadata`). | En Linux el metadato de GPUI no sobrevive siempre al portapapeles del sistema; el `Global` hace el comportamiento determinista y probable en tests. |
| R13 | E2, E4, E5 y E6 solo en `EditorChrome::Full`. | En el compositor del chat (Markdown) y en la caja de comentario, `Ctrl+X` sin selección borrando la línea o `}` moviendo la sangría sorprenderían. |
| R14 | E4 y E6 usan el nivel de sangría del archivo (`EditorView::indent_unit`: lo detectado en el archivo, si no `editor.tab_size`/`insert_spaces`). | Es la misma regla que ya usan `Tab` y `Shift+Tab`. |
| R15 | Atajo de espacios en blanco: **`Ctrl+Alt+W`** → `editor::toggle_whitespace` (acción existente, hasta hoy sin atajo), en la pestaña actual, como `Alt+Z`. Verificado libre en `DEFAULT_KEYMAP_JSONC`, `cincel_editor::default_key_bindings`, `cincel_chat::actions::default_key_bindings`, los `modal_bindings` de `cincel-workspace/src/keymap.rs` y el resto de `KeyBinding::new` del árbol; no es `Super`, ni `Alt+F*`, ni `Ctrl+Alt+flecha` (`02-visual.md` §8). `Ctrl+↑`/`Ctrl+↓` también están libres en todos ellos. | "W" de *whitespace*; `Ctrl+Alt+letra` ya se usa (`Ctrl+Alt+S`). |
| R16 | Limpieza al guardar (E8): `files.trim_trailing_whitespace_on_save` y `files.ensure_final_newline_on_save`, `true` por defecto. Se aplican en `Center::save_path` (guardar, guardar todo, autoguardado, "Guardar y salir") como **una** transacción del buffer (`EditSource::User`, un `Ctrl+Z` la deshace) antes de escribir. No se tocan: las filas de segmentos pendientes del agente, los archivos Markdown (`.md`, `.markdown`) para el recorte de espacios (dos espacios al final son un salto de línea en Markdown), y nada mientras el agente tiene un turno en curso sobre ese archivo. Un archivo vacío queda vacío; solo se agrega el salto final si falta (no se quitan líneas en blanco del final). | Lo acordado ("como Zed"), sin alterar lo que el usuario todavía revisa ni mezclar ediciones en un turno en curso (misma regla que el autoguardado, `center.rs`). |
| R17 | Versión: sigue **0.2.0**; la entrada `[0.2.0]` de `CHANGELOG.md` suma lo de esta ronda y cambia su fecha a la de cierre de la ronda. | La 0.2.0 no se publicó todavía. |

---

## 3. Encabezado del chat (A)

### 3.1 Comportamiento
- Con conexión activa, el encabezado tiene, de izquierda a derecha: el botón de la conexión (icono del proveedor, **nombre completo de la conexión**, flecha), **una** insignia (`HeaderBadge`, R1), el espacio libre y los botones "Nueva conversación" e "Historial" (sin cambios).
- Se quitan del encabezado el tipo de agente (`agent_type_label`) y la insignia de la conexión ("Conectada"); esta última queda solo en las filas del menú (§4).
- La insignia muestra SOLO el estado de la conexión (decisión del autor, 2026-10-01: la actividad del agente, pensando / trabajando / escribiendo, va en la fila al pie de la conversación, §8), por prioridad (R1, R2):

  | Situación | Texto | Color |
  |---|---|---|
  | `ConnectionBadge::SessionExpired` | "sesión vencida" | `status.warning` |
  | `ConnectionBadge::Unavailable { reason }` | "no disponible" (tooltip: el motivo) | `status.error` |
  | `AgentStatus::AuthRequired` | "autenticación requerida" | `status.warning` |
  | `AgentStatus::Disconnected` (proceso caído o detenido) | "desconectada" | `status.error` |
  | cualquier otro estado con el agente en marcha (`Ready`, `Thinking`, `WaitingPermission`) | "conectada" | `status.ok` |
- Sin conexión activa: el botón "Conectar" de siempre y ninguna insignia.
- El nombre no se corta mientras entre (R4); si no entra, "…" y tooltip con el nombre entero.
- La píldora de la barra de estado (que pinta el workspace) **no cambia** en esta ronda.

### 3.2 Aspecto (`02-visual.md` §2, §7; todo con `ChatSettings::px`, que aplica `ui_scale`)
- Encabezado de 32 px (`HEADER_HEIGHT`), sin cambios de alto.
- Botón: icono 16 px, nombre en `TEXT_BODY` (13 px) `text`, `ChevronDown` en `text.muted`, `px_1p5`, `py_0p5`, radio `CARD_RADIUS`, fondo `bg.surface` al pasar el mouse.
- Insignia: la forma de la píldora de hoy (`px_1p5`, `py_0p5`, radio `CARD_RADIUS`, fondo del color al 15 %, texto `TEXT_LABEL` 11 px en el color), `flex_shrink_0`.
- El nombre lleva `min_w(0)`, `flex_shrink` y `truncate`; la insignia y los botones no se encogen.

### 3.3 Diseño técnico (`crates/cincel-chat`)
- `model.rs`: `pub enum HeaderBadge { Connected, Disconnected, AuthRequired, SessionExpired, Unavailable { reason: String } }` con `label(&self) -> &'static str` (textos de §3.1) y `ChatPanel::header_badge(&self) -> Option<HeaderBadge>` (`None` sin conexión activa). `AgentStatus` y su `label()` no cambian (los usa la barra de estado y sus tests).
- `render.rs`: `render_header` deja de pintar `agent_type_label` y `connection_badge`; `status_pill` se reemplaza por `header_badge_pill(&HeaderBadge, theme, s)` con `debug_selector("chat-header-badge")`. El nombre lleva `debug_selector("chat-connection-name")` y `.tooltip(...)` con el nombre (solo se ve si está cortado; el tooltip se pone siempre, es inocuo).
- `agent_type_label` queda (lo usa la lista de "Eliminar conexión…" del workspace y la regla D2 allí). Construcción (2026-10-01): se eliminó de `render.rs`; el workspace usa `visible_agent_type` (`model.rs`).

### 3.4 Criterios de aceptación
- [ ] Con una conexión "Claude personal" de tipo Claude, el encabezado contiene el texto "Claude personal" y exactamente una insignia; no contiene "Claude" como texto aparte ni "Conectada".
- [ ] La insignia dice "conectada" durante todo un turno (pensando, herramientas, escribiendo, esperando permiso) y nunca cambia por la actividad del agente; muestra "sesión vencida", "no disponible" (con su motivo en tooltip), "autenticación requerida" y "desconectada" en cada caso de la tabla, respetando la prioridad.
- [ ] Con el panel a 380 px, un nombre de 20 caracteres se ve entero (sin "…"); a 280 px un nombre de 40 se corta y el tooltip lo muestra entero.
- [ ] El alto del encabezado no cambia (32 px × escala).

### 3.5 Tests
- `crates/cincel-chat/src/header_badge_tests.rs` (nuevo; `VisualTestContext`, feature `test-support`): la tabla de §3.1 completa con `header_badge()` y con el texto pintado bajo `debug_selector("chat-header-badge")`; prioridad (conexión vencida + agente pensando → "sesión vencida"); que la actividad del agente no toque la insignia (`Thinking` → sigue "conectada"); que no haya elementos `chat-connection-type` ni `chat-connection-badge`; ancho del nombre con `debug_bounds("chat-connection-name")` a 380 px (20 caracteres sin recorte: ancho del elemento ≥ ancho del texto medido con `window.text_system()`).
- Los tests de `connections_view_tests.rs` que afirmaban el tipo y la insignia **en el botón** se ajustan con `Edit` (es comportamiento que cambió por decisión del autor); los de las filas siguen en §4.

---

## 4. Menú de conexiones (B)

### 4.1 Comportamiento
- Cada fila: icono del agente a la izquierda (centrado en el alto de la fila); en el medio, dos líneas: arriba el nombre de la conexión, abajo el tipo de agente (`agent_name`: "Claude", "Codex", "Antigravity", o el `agent_id` si no es uno de los tres), **siempre** (R3); a la derecha la insignia de estado de la conexión ("Conectada" / "Sesión vencida" / "No disponible" con tooltip del motivo) y, si corresponde, "Volver a conectar" o "Reparar".
- Sin correo, sin plan, sin "Usado hace…". Todo lo demás (clic derecho → "Renombrar", pie, resaltado de la activa, cierre del menú) no cambia.
- La lista de "Eliminar conexión…" (`cincel-workspace/src/connection_modal.rs`) **no cambia** (una línea, regla D2).

### 4.2 Aspecto
- Fila: `px_1p5`, `py_1`, `gap_2`, radio `CARD_RADIUS`; icono 18 px; nombre `TEXT_BODY` 13 px `text` con `truncate`; tipo `TEXT_LABEL` 11 px `text.muted` con `truncate`; las dos líneas en un `v_flex` con `gap_0p5`, `flex_1`, `min_w(0)`. Insignia igual que hoy. Todo con `ChatSettings::px`.
- `CONNECTION_POPOVER_MAX_HEIGHT` (380 px) no cambia; con dos líneas entran menos filas antes del desplazamiento (aceptado).

### 4.3 Diseño técnico
- `render.rs`, `render_connection_rows`: el `h_flex` del nombre y el tipo pasa a `v_flex`; el tipo sale de `connection.agent_name` directamente (no de `agent_type_label`), con `debug_selector("connection-type-{index}")`; el nombre con `debug_selector("connection-name-{index}")`.

### 4.4 Criterios de aceptación
- [ ] Cada fila tiene el nombre arriba y el tipo abajo (la `y` del tipo es mayor que la del nombre y sus `x` de inicio coinciden), el icono a la izquierda de ambos y la insignia a la derecha.
- [ ] Una conexión llamada "Claude" de tipo Claude muestra "Claude" arriba y "Claude" abajo.
- [ ] Ninguna fila muestra correo, plan ni "Usado".

### 4.5 Tests
- `crates/cincel-chat/src/connection_rows_tests.rs` (nuevo): posiciones con `debug_bounds` de `connection-name-0`, `connection-type-0`, `connection-badge-0`; R3; sin identidad (fixture con correo y plan en `identity`); clic real en la fila sigue emitiendo `ConnectionSelected`.

---

## 5. Imágenes arriba del texto (C)

### 5.1 Comportamiento y aspecto
- **Mensaje enviado**: dentro de la burbuja, de arriba abajo: fila de miniaturas (igual que hoy: hasta 96 px de alto y 160 de ancho, radio 6, `gap_1p5`), texto, tarjetas de comentario. Un mensaje sin texto muestra solo las miniaturas (y las tarjetas). El `gap` de 6 px de la burbuja separa las tres partes.
- **Caja de texto**: ya está como se pidió (etiquetas de comentario, después miniaturas, después la fila con el clip, el campo y el botón de enviar). No se cambia; se fija con un test.

### 5.2 Diseño técnico
- `render_media.rs`: `render_message_media` se parte en `render_message_images(...) -> Option<AnyElement>` y `render_message_cards(...) -> Vec<AnyElement>`; `render_user_message` (`render.rs`) pone las imágenes antes del `v_flex` del texto y las tarjetas después. Selectores: `user-images-{index}` (ya existe), `user-text-{index}` (ya existe).

### 5.3 Criterios de aceptación
- [ ] En un mensaje con texto e imágenes, el borde de abajo de `user-images-{i}` está por encima del borde de arriba de `user-text-{i}`.
- [ ] Las tarjetas de comentario siguen debajo del texto.
- [ ] En la caja, la fila de miniaturas está por encima del campo de texto (`chat-input`).

### 5.4 Tests
- `crates/cincel-chat/src/message_media_order_tests.rs` (nuevo): los tres criterios con `debug_bounds`; imágenes generadas en el test con `image` (nada binario en el repo), como `attachments_tests.rs`.

---

## 6. Margen al final del archivo (D, = E3)

### 6.1 Comportamiento
- En un editor de código (`EditorChrome::Full`) se puede desplazar hasta que la última fila visual quede a media pantalla: el desplazamiento máximo suma `floor(filas_visibles / 2) × alto_de_línea`. Es solo espacio: el archivo no cambia, la cantidad de líneas no cambia, los números de línea no cambian.
- Vale para la rueda, la barra de desplazamiento, `set_scroll_row` (restaurar la posición de una pestaña), `scroll_rows` y el centrado de "ir al cambio" (un salto al último segmento ahora puede centrarlo).
- `Ctrl+End` y el movimiento del cursor solo traen el cursor a la vista (como hoy); no van al fondo del margen.
- Un clic en el margen vacío pone el cursor al final de la última línea (como un clic debajo del texto hoy).
- Un archivo corto que entra entero en pantalla también puede desplazarse hasta que su última línea quede a media pantalla (como VS Code).
- La barra flotante de revisión (`BAR_MARGIN` 24 px, `BAR_HEIGHT` 30 px) queda dentro del margen al desplazar hasta el final: deja de tapar las últimas líneas.
- D16: el margen solo cambia hasta dónde se desplaza; el ancho del margen izquierdo, la columna del texto y el ajuste de línea no cambian.

### 6.2 Diseño técnico (`crates/cincel-editor`)
- `view.rs`: `pub(crate) fn max_scroll_top(&self, viewport_height: f32) -> f32` = `(visual_row_count × lh + margen − viewport).max(0)` con `margen = if chrome == Full { (viewport / lh / 2).floor() × lh } else { 0 }`. La usan `set_scroll_row`, `scroll_rows` y el `prepaint` de `element.rs` (líneas del cálculo de `max_scroll`), que es quien la pasa a `scroll_by` y a la barra.
- `element.rs`, barra de desplazamiento: el contenido usado para el alto del pulgar es `visual_rows × lh + margen` (para que el pulgar llegue abajo de todo al final).
- El clic con la posición por debajo de la última fila ya se acota a la última fila; se verifica que siga así con el margen.

### 6.3 Criterios de aceptación
- [ ] Con un archivo de 200 líneas y 40 filas visibles, el desplazamiento máximo es `(200 + 20 − 40) × lh`; con el compositor del chat (`Minimal`), `(filas − visibles) × lh` como hoy.
- [ ] Con el desplazamiento al máximo, la última fila termina en `bounds.top + (filas_visibles − 20) × lh` (media pantalla), y la barra flotante de revisión no se superpone con ninguna fila de texto.
- [ ] El origen `x` del texto y el ancho del margen izquierdo son iguales antes y después de desplazar al máximo.
- [ ] El buffer no cambia (mismo texto y misma cantidad de líneas) y no se marca como modificado.

### 6.4 Tests
- `crates/cincel-editor/src/scroll_margin_tests.rs` (nuevo; `VisualTestContext`): los cuatro criterios; rueda (`ScrollWheelEvent`) hasta el tope; `set_scroll_row(10_000.)` se acota al nuevo máximo; `Minimal` sin margen; clic en el margen vacío pone el cursor al final de la última línea; con soft wrap y con una caja de comentario abierta (filas de bloque) el margen se suma a `visual_row_count`.

---

## 7. Editor: las ocho mejoras (E)

Todo lo de esta sección vale solo en `EditorChrome::Full` (R10, R13), salvo E7 (las dos acciones sirven también en `Minimal`, donde no hacen daño). D16: ninguna de estas mejoras cambia el ancho del margen, la columna del texto ni el alto de las filas.

### 7.1 E1: apariciones de la palabra bajo el cursor

**Comportamiento**
- Sin selección, si el cursor está **dentro o justo al final** de una palabra (caracteres alfanuméricos o `_`, la misma clase que `class_of == 1` de `view.rs`), se marcan las demás apariciones de esa palabra **como palabra completa** (el carácter anterior y el siguiente no son de palabra), distinguiendo mayúsculas.
- Con una selección dentro de **una sola fila** del buffer, no vacía, de hasta 256 bytes y que no sea solo espacios: se marcan las demás apariciones del texto seleccionado; si el texto empieza o termina con un carácter de palabra, se exige el límite de palabra en ese lado.
- La aparición donde está el cursor (o la selección misma) no se marca.
- Solo en las filas reales visibles (más el `OVERSCAN` de siempre); las filas fantasma no.
- Aparecen 150 ms después de la última vez que se movió el cursor o cambió la selección o el texto; cualquier movimiento las borra en el momento y vuelve a esperar.
- No se pintan con la barra de búsqueda abierta y una consulta no vacía (la búsqueda gana), ni en un buffer de solo lectura de un binario, ni en `Minimal`.

**Aspecto**: rectángulo con el color `selection` al 50 % (`OCCURRENCE_ALPHA`), esquinas rectas, debajo de la selección (mismo orden de pintado que las coincidencias de búsqueda, que se pintan antes que la selección).

**Diseño técnico**
- `theme.rs`: `pub const OCCURRENCE_ALPHA: f32 = 0.5;`.
- `view.rs`: estado `occurrences: Option<OccurrenceQuery { text: String, whole_word_start: bool, whole_word_end: bool, own: Range<usize> }>` y `occurrence_timer: Option<Task<()>>`. En `after_input` (y tras ediciones) se borra `occurrences` y se reprograma el temporizador (`cx.spawn` con `background_executor().timer(OCCURRENCE_DELAY)`, `OCCURRENCE_DELAY = Duration::from_millis(150)`); el temporizador calcula la consulta desde el cursor/selección y hace `cx.notify()`. Es una tarea de un solo disparo: en reposo no queda nada corriendo (M8).
- `element.rs`, `prepaint`: si hay consulta, busca sus apariciones en el texto de `visible_byte_range` (las mismas filas que ya se recorren para las coincidencias de búsqueda) con una búsqueda de subcadena simple, filtra por límite de palabra y descarta `own`; las agrega a una lista `occurrence_quads` construida igual que `matches` (mismo cálculo de `row_bounds` por fila de ajuste). `paint` las pinta justo antes que `prepaint.matches`.
- Función pura testeable en `search.rs`: `pub fn find_occurrences(text: &str, base: usize, query: &OccurrenceQuery) -> Vec<Range<usize>>`.

**Criterios**
- [ ] Con el cursor en `total` en `let total = a + total_b; total`, se marcan las otras dos apariciones de `total` como palabra completa y no `total_b`.
- [ ] Una selección de `a + ` en una fila marca sus otras apariciones exactas; una selección de dos filas no marca nada.
- [ ] Antes de 150 ms no hay marcas; después sí; mover el cursor las borra en el mismo cuadro.
- [ ] Con `Ctrl+F` abierto y una consulta, no hay marcas de apariciones.
- [ ] En reposo, 1 s después de la última tecla, no queda ninguna tarea del editor programada (mismo control que `idle_timer_tests.rs`).
- [ ] `--bench typing` (M4) no empeora (R2-F y R2-I).

**Tests**: unitarios de `find_occurrences` en `crates/cincel-editor/src/occurrences_tests.rs` (nuevo; límites de palabra, Unicode, mayúsculas, propia aparición excluida) y, en el mismo archivo, con `VisualTestContext` y `cx.executor().advance_clock(...)`: los criterios de arriba leyendo las marcas preparadas por el elemento (accesor `test-support` `EditorView::occurrence_ranges_for_test()`).

### 7.2 E2: copiar y cortar la línea sin selección

**Comportamiento**
- `Ctrl+C` sin selección: copia la fila del buffer del cursor entera **con su salto de línea** (en la última línea sin salto, se agrega `\n` al texto copiado). En una fila fantasma copia su texto (el de la base) con salto. El cursor no se mueve.
- `Ctrl+X` sin selección: hace lo mismo y además borra la fila (como `editor::delete_line`, un solo paso de deshacer). En una fila fantasma (solo lectura) copia y no borra.
- `Ctrl+V` cuando el portapapeles trae una línea copiada así (R12) y **no hay selección**: inserta el texto tal cual (sin reacomodar la sangría) al **comienzo** de la fila del cursor; el cursor queda en la misma columna de la misma línea de texto (que ahora está una fila más abajo). Con selección, pega reemplazando la selección como siempre.
- Copiar o cortar **con** selección borra el recuerdo de "línea entera".
- El menú contextual del editor ("Cortar", "Copiar", "Pegar") hace lo mismo que las teclas.

**Diseño técnico**
- `view.rs`: `on_copy`, `on_cut`, `on_paste`. `struct LineClipboard(Option<String>)` como `gpui::Global` (en `view/editing.rs`); escritura con `ClipboardItem::new_string_with_json_metadata(text, LineCopyMetadata { whole_line: true })`. En `on_paste`: es línea si `metadata_json::<LineCopyMetadata>()` dice `whole_line` **o** si el texto coincide con `LineClipboard`.
- Solo con `chrome == Full` (R13); en `Minimal`, copiar/cortar sin selección no hace nada (como hoy).

**Criterios**
- [ ] `Ctrl+C` sin selección en la fila 3 de "a\nb\nc\n" deja "b\n" en el portapapeles; `Ctrl+V` con el cursor en la fila 1, columna 1 deja "b\na\nb\nc\n" y el cursor en la fila 2, columna 1.
- [ ] `Ctrl+X` sin selección borra la fila y un `Ctrl+Z` la devuelve.
- [ ] Copiar con selección y pegar sigue igual que hoy (incluido el reacomodo de sangría multilínea).
- [ ] La última línea sin salto final se copia con `\n` y se pega como línea.
- [ ] En el compositor del chat, `Ctrl+C`/`Ctrl+X` sin selección no cambian nada.

**Tests**: `crates/cincel-editor/src/line_clipboard_tests.rs` (nuevo; `TestAppContext` con `cx.write_to_clipboard`/`read_from_clipboard`): los cinco criterios, fila fantasma y el metadato.

### 7.3 E3: margen al final

Es D: §6.

### 7.4 E4: quitar un nivel al escribir un cierre

**Comportamiento**
- Al escribir `}`, `]` o `)` sin selección, si en la fila del buffer el texto antes del cursor es solo espacios o tabulaciones (no vacío) y el texto después del cursor está vacío o es solo espacios: se quita un nivel de sangría (la misma regla que `Shift+Tab`, `indent_rows(.., false, ..)`: una unidad de `indent_unit`, o un tab, o los espacios que haya si son menos) y se escribe el carácter; todo en **un** paso de deshacer.
- Si justo después del cursor está el mismo cierre insertado automáticamente (`auto_closers`), gana el "saltar por encima" de hoy y no se toca la sangría.
- Con escritura compuesta (IME, `marked_range`) no aplica.

**Diseño técnico**: `view/editing.rs`, `fn outdent_for_closer(&mut self, text: &str, cx) -> bool`, llamada en `replace_text_in_range` (`view.rs`) antes de `type_with_pairs` y después de comprobar el salto del auto-cierre; aplica con `apply_edits` una edición que reemplaza la sangría por la sangría reducida más el carácter. Helper puro en `ops.rs`: `pub fn outdent_once(indent: &str, unit: &str) -> &str`.

**Criterios**
- [ ] "fn a() {\n        |" (8 espacios, unidad 4) + `}` → "fn a() {\n    }" y el cursor después de `}`.
- [ ] Con tabulaciones: "\t\t|" + `)` → "\t)".
- [ ] "    foo|" + `}` no cambia la sangría.
- [ ] `Ctrl+Z` deja la línea como antes de escribir el cierre (sangría incluida).
- [ ] En el compositor del chat no aplica.

**Tests**: `crates/cincel-editor/src/indent_editing_tests.rs` (nuevo), con `cx.simulate_input`.

### 7.5 E5: `Enter` sobre una línea que solo tiene sangría

**Comportamiento**: sin selección, si la fila del buffer del cursor es entera solo espacios o tabulaciones (o vacía), `Enter` deja esa fila **vacía** y la fila nueva con la sangría que había antes del cursor; el cursor queda al final de esa sangría. Un solo paso de deshacer. Las demás reglas de `Enter` (entre un par `{|}`, después de `{ ( [ :`) no cambian.

**Diseño técnico**: `view.rs`, `on_insert_newline`: si se cumple la condición, reemplaza el rango `inicio_de_fila..fin_de_fila` por `"\n" + sangría_antes_del_cursor` con `replace_buffer_range`.

**Criterios**
- [ ] "x\n        |" + `Enter` → "x\n\n        " con el cursor en la fila 2, columna 8; la fila 1 queda vacía.
- [ ] "    |    " (cursor en medio de 8 espacios) + `Enter` → fila vacía y fila nueva con 4 espacios.
- [ ] "    foo|" + `Enter` sigue como hoy ("    foo\n    ").

**Tests**: en `indent_editing_tests.rs`.

### 7.6 E6: `Backspace` dentro de la sangría

**Comportamiento**: sin selección y sin par automático que borrar (`backspace_pair` gana), si el cursor no está en la columna 0 y todo el texto antes del cursor en la fila del buffer son **espacios**: borra hasta el tope de sangría anterior, es decir `((columna − 1) % tamaño) + 1` espacios, con `tamaño` = el de `indent_unit` (R14). Con un tab antes del cursor, o con espacios mezclados con tabs, borra un carácter como hoy. Fuera de la sangría, como hoy.

**Diseño técnico**: `view.rs`, `on_backspace`, rama nueva antes del borrado de un carácter; helper puro `ops::spaces_to_previous_stop(column: usize, size: usize) -> usize`.

**Criterios**
- [ ] Con unidad de 4: 8 espacios + `Backspace` → 4; 6 espacios → 4; 1 espacio → 0.
- [ ] "    a|" + `Backspace` borra solo la `a`.
- [ ] Con selección, borra la selección como hoy.

**Tests**: en `indent_editing_tests.rs` y unitarios de `spaces_to_previous_stop` en `ops.rs`.

### 7.7 E7: `Ctrl+↑`/`Ctrl+↓` y mostrar espacios en blanco

**Comportamiento**
- `Ctrl+↑` / `Ctrl+↓`: desplazan una fila hacia arriba / abajo (`scroll_rows(±1.)`, con el margen de §6), **sin mover el cursor** ni la selección; si el cursor queda fuera de la vista, queda ahí hasta que se escriba o se mueva (entonces la vista vuelve a él, como hoy).
- `Ctrl+Alt+W`: muestra u oculta los espacios en blanco en la pestaña actual (`editor::toggle_whitespace`, que ya existe), como `Alt+Z` con el ajuste de línea; el valor permanente sigue en Configuración → Editor → "Mostrar espacios en blanco".

**Acciones y bindings** (verificados libres, R15)

| Acción | Atajo | Contexto | Dónde se agrega |
|---|---|---|---|
| `editor::scroll_line_up` (`ScrollLineUp`, nueva) | `ctrl-up` | `Editor` | `cincel_editor::actions` (`named_actions!` y `default_key_bindings`) y `DEFAULT_KEYMAP_JSONC` sección `Editor` |
| `editor::scroll_line_down` (`ScrollLineDown`, nueva) | `ctrl-down` | `Editor` | ídem |
| `editor::toggle_whitespace` (existente) | `ctrl-alt-w` | `Editor` | ídem |

- Comentario en `DEFAULT_KEYMAP_JSONC` (español): `// Desplazar una línea sin mover el cursor, y mostrar u ocultar los espacios en blanco en esta pestaña.`
- `shortcuts_modal::description`: "Desplazar una línea hacia arriba", "Desplazar una línea hacia abajo", "Mostrar u ocultar los espacios en blanco".
- `docs/usuario/atajos.md`: las tres filas (`Ctrl+↑`, `Ctrl+↓`, `Ctrl+Alt+W`); `user_docs_tests.rs` lo exige.
- `defaults.rs`, `default_keymap_has_every_shortcut_of_the_spec`: tres filas nuevas.

**Criterios**
- [ ] `Ctrl+↓` tres veces suma 3 al `scroll_row()` y el cursor sigue en la misma posición.
- [ ] `Ctrl+↑` en el principio no hace nada; `Ctrl+↓` se detiene en el máximo con margen.
- [ ] `Ctrl+Alt+W` alterna `settings().show_whitespace` de esa vista y no el de otra pestaña.
- [ ] El keymap por defecto resuelve las tres teclas en `Editor` a esas acciones.

**Tests**: en `scroll_margin_tests.rs` (las de desplazamiento) y `crates/cincel-editor/src/whitespace_toggle_tests.rs` (nuevo); filas nuevas en `cincel-settings/src/defaults.rs`.

### 7.8 E8: limpieza al guardar

**Comportamiento** (R16)
- Al guardar un archivo (cualquier vía de `Center::save_path`), si `files.trim_trailing_whitespace_on_save` está encendido, se quitan los espacios y tabulaciones del final de cada línea; si `files.ensure_final_newline_on_save` está encendido y el archivo no está vacío y no termina en salto de línea, se agrega uno (el del archivo: `\n` o `\r\n`, lo pone `cincel-text` al escribir).
- No se tocan: filas dentro de segmentos **pendientes** del agente (sus filas del buffer); el recorte de espacios en archivos Markdown (`.md`, `.markdown`; el salto final sí); nada si hay un turno del agente en curso sobre ese archivo (`review.turn_active && tracked.contains(path)`), ni en pestañas de solo lectura (que ya no se guardan).
- Es una sola transacción `EditSource::User`: un `Ctrl+Z` la deshace; el cursor y la selección siguen al texto; los comentarios y anclas siguen sus líneas (por los `BufferEvent` de siempre).
- Si no hay nada que limpiar, no hay transacción (el historial no cambia).

**Ajustes nuevos** (`cincel-settings`, `FilesSettings`, `#[serde(default)]` como el resto)

| Clave | Tipo | Por defecto | Comentario en `DEFAULT_SETTINGS_JSONC` |
|---|---|---|---|
| `files.trim_trailing_whitespace_on_save` | bool | `true` | `// Al guardar, quitar los espacios del final de cada línea (no en Markdown ni en los cambios del agente sin decidir).` |
| `files.ensure_final_newline_on_save` | bool | `true` | `// Al guardar, terminar el archivo con un salto de línea si no lo tiene.` |

**Filas en Configuración → Archivos** (`settings_view.rs`, `SettingsSection::Files`, `Control::Switch`, después de "Pausa del autoguardado"):
- `files.trim_trailing_whitespace_on_save` — título "Quitar espacios al final de las líneas al guardar", descripción "No toca los archivos Markdown ni los cambios del agente que todavía no decidiste."
- `files.ensure_final_newline_on_save` — título "Terminar el archivo con un salto de línea al guardar", descripción "Si al archivo le falta el salto de línea final, se agrega al guardar."
- `docs/usuario/ajustes.md`, sección "Archivos": las dos claves en lenguaje llano (`user_docs_tests.rs` lo exige).

**Diseño técnico**
- `cincel-editor/src/ops.rs`: `pub fn save_cleanup_edits(lines: &[&str], protected_rows: &[Range<u32>], trim: bool, final_newline: bool, ends_with_newline: bool) -> Vec<(Range<usize>, String)>` (pura, offsets del buffer).
- `cincel-editor/src/view.rs`: `pub fn clean_up_for_save(&mut self, options: SaveCleanup, cx) -> bool` con `pub struct SaveCleanup { pub trim_trailing_whitespace: bool, pub ensure_final_newline: bool }`; las filas protegidas salen de `self.review.hunks` con decisión pendiente; aplica con `apply_edits` (mapeo de selección incluido).
- `cincel-workspace/src/center.rs`, `save_path`: antes de `project.save`, si el editor existe, no es solo lectura y no hay turno activo sobre el archivo, arma `SaveCleanup` desde `settings(cx).files` (con `trim` en `false` si la extensión es Markdown) y llama `clean_up_for_save`.

**Criterios**
- [ ] "a  \nb\t\nc" guardado → en disco "a\nb\nc\n"; un `Ctrl+Z` devuelve "a  \nb\t\nc" (sucio).
- [ ] Con los dos ajustes apagados, el disco recibe exactamente el texto del buffer.
- [ ] Un segmento pendiente con espacios finales en sus filas los conserva; las demás filas se limpian.
- [ ] En `notas.md`, "hola  \n" conserva los dos espacios; "hola" sin salto recibe el salto.
- [ ] Un archivo CRLF sigue CRLF tras guardar, con el salto final en CRLF.
- [ ] Con un turno en curso sobre el archivo, el guardado manual escribe el buffer sin limpiar.
- [ ] Un archivo vacío queda vacío.
- [ ] El autoguardado (las dos modalidades) aplica la misma limpieza.

**Tests**: unitarios de `save_cleanup_edits` en `ops.rs`; `cincel-settings` (los defaults parsean, `deny_unknown_fields` acepta las claves); `crates/cincel-workspace/src/save_cleanup_tests.rs` (nuevo; `TestAppContext`, carpeta temporal): los ocho criterios con el archivo leído del disco.

---

## 8. Fila de actividad del agente al pie de la conversación (F)

### 8.1 Comportamiento (R5; decisión del autor, 2026-10-01)
- Mientras el turno está en curso (`AgentStatus::Thinking`), al pie de la conversación hay UNA fila de actividad con la ruedita (`Spinner` de gpui-kit) cuyo texto dice qué está haciendo el agente, según el último `SessionUpdate` recibido en el turno:

  | Última señal del agente | Texto |
  |---|---|
  | nada todavía, o `AgentThoughtChunk` (razonamiento en curso) | "Pensando…" |
  | `ToolCall` / `ToolCallUpdate` con una herramienta en curso (editar, leer, ejecutar, buscar) | "Trabajando…" |
  | `AgentMessageChunk` (la respuesta se está escribiendo) | "Escribiendo…" |
  | `PermissionRequest` pendiente | "Esperando permiso…" |
- Cuando ya existe la fila de pensamiento en vivo ("Pensando… (N s)", la de los `AgentThoughtChunk` plegables) y es el último elemento, no se duplica: esa fila cumple el papel.
- Desaparece al terminar o cancelarse el turno. Cambia de texto en el momento en que cambia la señal (por ejemplo, tras una herramienta vuelve a "Pensando…" si el agente razona de nuevo, y a "Escribiendo…" cuando empieza la respuesta).
- No se guarda con la conversación.
- Ya no hay actividad en el encabezado (§3): allí solo el estado de la conexión.

### 8.2 Aspecto
- Fila de `TOOL_ROW_HEIGHT` (28 px), `px_1`, `gap_1`, `items_center`; `Spinner::new().xsmall().color(text.accent)` + el texto en `TEXT_SMALL` (12 px) `text.muted`; separada del elemento anterior por `ROW_GAP` (4 px). Todo con `ChatSettings::px`.
- Sin animación de altura (la lista crece una fila; con el seguimiento activo se ve, §9).

### 8.3 Diseño técnico
- `model.rs`: `pub enum AgentActivity { Thinking, Working, Writing, WaitingPermission }` con `label()`; `panel.rs`: `pub(crate) fn activity(&self) -> Option<AgentActivity>` (`None` fuera de un turno o cuando la fila de pensamiento en vivo es el último elemento) actualizado en `apply_update` a partir del tipo de `SessionUpdate` y de `pending_permission`.
- `render.rs`, `render_list_item`: para el último índice, si `activity()` es `Some`, agrega la fila debajo del elemento (antes del `pb_2` del último), con `debug_selector("chat-activity-row")`. `render_transcript` ya llama `remeasure_items(0..count)` en cada render, así que el alto nuevo se mide.

### 8.4 Criterios de aceptación
- [ ] Tras enviar, antes de cualquier fragmento, existe `chat-activity-row` y dice "Pensando…".
- [ ] Con un `ToolCall` en curso dice "Trabajando…"; al primer `AgentMessageChunk` dice "Escribiendo…"; con un permiso pendiente dice "Esperando permiso…"; con `AgentThoughtChunk` en vivo como último elemento no hay fila duplicada.
- [ ] Al terminar o cancelar el turno la fila no está.
- [ ] La conversación guardada no contiene la fila.

### 8.5 Tests
- `crates/cincel-chat/src/activity_row_tests.rs` (nuevo): los criterios con eventos `AgentEvent` simulados (patrón de `tests.rs`), incluida la secuencia pensar → herramienta → pensar → escribir → fin.

---

## 9. Seguir el final y botón "ir al final" (G, H)

### 9.1 Comportamiento
- **Siguiendo** (estado inicial, y al abrir una conversación o empezar una nueva): todo lo que llega (texto, herramientas, separadores, la fila "Pensando…") mantiene la vista al final.
- **El usuario sube** (rueda hacia arriba, arrastrar la barra, `PageUp` si existiera): deja de seguir. A partir de ahí la vista **no se mueve** con nada de lo que llega: fragmentos, herramientas, el fin del turno (separador), un pedido de permiso, el re-renderizado del Markdown (cambio de zoom o de tema, `rebuild_views`) ni el re-medido de alturas.
- **Vuelve a seguir** cuando el usuario baja hasta el final (con la rueda o la barra) o pulsa la flecha (G). Enviar un mensaje también baja al final y vuelve a seguir (R6).
- **Flecha (G)**: visible solo cuando no se está siguiendo y la lista no está al final (`is_scrolled_to_end() == Some(false)`); oculta si no hay desplazamiento posible. Al pulsarla: va al final y vuelve a seguir. Tooltip "Ir al final".

### 9.2 Aspecto (`02-visual.md` §4: único movimiento permitido, el desvanecido de 120 ms de un botón flotante)
- Botón circular de 28 px (`s.px(28.)`), centrado horizontalmente sobre la conversación, a 12 px del borde inferior de la conversación (por encima de la caja de texto, sin taparla), `bg.elevated`, borde 1 px `border`, sombra de popover (`0 2px 8px #0006`), `IconName::ArrowDown` 14 px en `text`; fondo `bg.surface` al pasar el mouse. Aparece y desaparece con desvanecido de 120 ms. Es una capa: no ocupa lugar ni mueve nada.

### 9.3 Diseño técnico (`crates/cincel-chat`, R6)
- `panel.rs`: `push` **ya no** llama `self.list.scroll_to_end()`. `load_conversation`, `clear`/`start_new_conversation`, `send` (al agregar el mensaje del usuario) y la flecha llaman `self.list.set_follow_mode(FollowMode::Tail)` (que ya lleva al final y reactiva el seguimiento). Ningún otro camino mueve la vista.
- `pub(crate) fn is_following_tail(&self) -> bool` (= `list.is_following_tail()`), `pub(crate) fn shows_jump_to_end(&self) -> bool` (= `!following && list.is_scrolled_to_end() == Some(false)`), `pub fn jump_to_end(&mut self, cx)`.
- `render.rs`, `render_transcript`: el contenedor de la lista lleva `.relative()` y, si `shows_jump_to_end()`, un hijo `absolute().bottom(s.px(12.))` centrado con el botón (`id("chat-jump-to-end")`, `debug_selector("chat-jump-to-end")`), con `with_animation` de opacidad 0→1 en 120 ms (al ocultarse se quita; el desvanecido de salida se omite si complica: ver §15 riesgos). La lista vuelve a pintarse con cada desplazamiento (GPUI notifica a la vista dueña), así que el botón se actualiza sin suscripciones nuevas.
- Accesores `test-support`: `transcript_scroll_offset() -> gpui::ListOffset` (`list.logical_scroll_top()`).

### 9.4 Criterios de aceptación
- [ ] Siguiendo: 200 fragmentos largos dejan `is_scrolled_to_end() == Some(true)`.
- [ ] Tras subir con la rueda: `is_following_tail() == false`; fragmentos, un `ToolCall`, un pedido de permiso, `TurnEnded` y un cambio de zoom (`set_settings` con otra escala) dejan `transcript_scroll_offset()` igual al de antes.
- [ ] Bajar con la rueda hasta el final reactiva el seguimiento: el fragmento siguiente deja la vista al final.
- [ ] La flecha aparece solo tras subir; un clic real (`simulate_click` sobre `debug_bounds("chat-jump-to-end")`) deja la vista al final, siguiendo, y la flecha se va.
- [ ] Enviar un mensaje estando arriba baja al final y sigue.
- [ ] Con una conversación corta (sin desplazamiento posible) la flecha no aparece nunca.

### 9.5 Tests
- `crates/cincel-chat/src/transcript_follow_tests.rs` (nuevo; `VisualTestContext` con la ventana a 380 × 500, `ScrollWheelEvent` sobre `debug_bounds("chat-transcript")`, `run_until_parked` y, para lo que dependa de un cuadro, `cx.update(|window, _| window.refresh())` antes de leer): los seis criterios.

---

## 10. Bloques de código largos plegados (I)

### 10.1 Comportamiento
- Alcance: solo bloques de código cercados (` ``` ` o `~~~`) de las respuestas del agente y de los trozos Markdown de las burbujas del usuario, que empiecen en la columna 0 (R8). No se pliegan el código en línea, las salidas de herramientas (tienen su propio tope, `MAX_OUTPUT_LINES`), los pensamientos ni las tarjetas de comentario.
- Un bloque con **más de 20 líneas** de código (sin contar las vallas) se muestra **plegado**: se ven sus primeras 12 líneas, con un difuminado sobre la parte de abajo, y al pie un botón "Ver más (N líneas)", con N = líneas ocultas (total − 12).
- Clic en "Ver más…": se despliega entero y el pie pasa a "Ver menos"; clic en "Ver menos": vuelve a plegarse. El estado es por bloque y se recuerda mientras la conversación esté abierta (abrir otra conversación o reabrirla desde el historial lo vuelve a plegado).
- Mientras llega, un bloque se pliega en cuanto pasa las 20 líneas (R9); el número del botón se actualiza.
- "Copiar" (en la cabecera del bloque, como hoy) copia **siempre el bloque entero**, plegado o no. La selección de texto dentro del bloque sigue funcionando; la selección de un tramo que cruza el borde entre el texto y un bloque largo queda limitada a cada parte (ver §15).
- Plegar y desplegar con la vista "siguiendo" la deja al final; sin seguir, la posición se conserva (el contenido de abajo se corre, como cualquier cambio de alto).

### 10.2 Aspecto (`02-visual.md` §7; escalado con `ChatSettings::scale`)
- El bloque conserva su caja (`bg.editor`, borde 1 px `border`, radio `CODE_BLOCK_RADIUS` 6, cabecera de `CODE_BLOCK_HEADER` 22 px con lenguaje y "Copiar").
- Plegado: alto máximo = `(CODE_BLOCK_HEADER + 6) × escala` (relleno de arriba) + `12 × alto_de_línea_del_código` + `8 × escala` (relleno de abajo) + 2 px de borde; `alto_de_línea_del_código` se calcula como lo hace gpui-kit para `max_lines` (`TextStyle::line_height_in_pixels` con `TEXT_CODE × escala` sobre el estilo de texto de la ventana).
- Difuminado: franja de 40 px × escala pegada al borde de abajo por dentro (1 px de margen a cada lado y abajo, radio inferior 5), `linear_gradient(180°, bg.editor al 0 % → bg.editor al 100 %)`.
- Pie: debajo de la caja, 4 px × escala de separación, botón `Button::new(..).xsmall().ghost()` centrado con `IconName::ChevronDown` (plegado) o `IconName::ChevronUp` (desplegado) y el texto "Ver más (N líneas)" / "Ver menos".

### 10.3 Diseño técnico (`crates/cincel-chat`, R7)
- `code_folds.rs` (nuevo, sin GPUI): `pub const CODE_FOLD_MIN_LINES: usize = 21; pub const CODE_FOLD_VISIBLE_LINES: usize = 12;` `pub enum SegmentKind { Prose, LongCode { lines: usize, closed: bool } }`, `pub struct MdSegment { pub range: Range<usize>, pub kind: SegmentKind }`, `pub fn split_markdown(md: &str) -> Vec<MdSegment>`: recorre por líneas; una valla de apertura es una línea que empieza en la columna 0 con ≥ 3 `` ` `` o ≥ 3 `~`; la de cierre, la primera línea posterior con el mismo carácter repetido al menos la misma cantidad y nada más que espacios; un bloque con más de 20 líneas (cerrado o abierto al final del texto) es `LongCode` con su valla incluida en el rango; lo demás se junta en segmentos `Prose`. Segmentos `Prose` vacíos o solo de espacios no se emiten.
- `model.rs`: `AgentText` suma `#[serde(skip)] pub segments: Vec<TextSegment>` con `TextSegment { range: Range<usize>, kind: SegmentKind, view: Entity<TextViewState> }`; `view` (el de hoy) se reemplaza por los segmentos (se ajustan los usos). El JSON de la conversación no cambia (`markdown` y `streaming`).
- `panel.rs`: al llegar un fragmento, `markdown.push_str(chunk)`; se vuelve a partir solo la cola (`split_markdown(&markdown[ultimo.range.start..])`); si da un único segmento del mismo tipo que el último, `append_chunk` a su vista (el camino rápido de hoy); si no, `set_text` del último con su nuevo rango y vistas nuevas para los segmentos que aparecieron. `rebuild_views` arma los segmentos desde `markdown`. Estado: `ChatPanel.expanded_code_blocks: HashSet<CodeBlockKey>` con `CodeBlockKey { entry: usize, piece: usize, segment: usize }` (`piece` = 0 en las respuestas); se vacía en `clear` y `load_conversation`; cuando `push` descarta entradas viejas (`max_entries`), las claves se corren como `tool_index`.
- `render.rs`: `render_agent_text` y la rama `UserPiece::Markdown` de `render_user_message` pintan un `v_flex` (con `gap` = el espacio entre párrafos de gpui-kit, `paragraph_gap`) de segmentos; un `LongCode` plegado usa `markdown_element_scaled` con un `TextViewStyle` cuyo `code_block` suma `.max_h(alto)` y `.overflow_hidden()` (`markdown.rs`: `text_view_style_folded(theme, scale, max_height)`), envuelto en un `div().relative()` con el difuminado `absolute()` y, debajo, el pie. Las burbujas usan `bubble_views` con la clave extendida a `(entry, piece, segment)`.
- Selectores: `code-block-{entry}-{piece}-{segment}` (la caja), `code-fold-toggle-{entry}-{piece}-{segment}` (el botón).

### 10.4 Criterios de aceptación
- [ ] Un bloque de 30 líneas sale plegado: el alto de `code-block-…` es el de §10.2 (± 1 px) y el botón dice "Ver más (18 líneas)".
- [ ] Un bloque de 20 líneas no se pliega y no tiene botón; uno de 21 sí ("Ver más (9 líneas)").
- [ ] Clic real en el botón: el alto crece hasta mostrar las 30 líneas y el botón dice "Ver menos"; otro clic vuelve a plegar.
- [ ] El estado de cada bloque es independiente (dos bloques largos en la misma respuesta) y se conserva al llegar más fragmentos y al cambiar el zoom; abrir otra conversación y volver lo deja plegado.
- [ ] "Copiar" de un bloque plegado emite `CopyToClipboard` con las 30 líneas.
- [ ] En streaming, un bloque que llega línea a línea queda plegado al llegar a la línea 21 y el número se actualiza; el texto anterior al bloque no se vuelve a crear (misma entidad de vista).
- [ ] Un bloque largo dentro de una lista (con sangría) no se pliega.
- [ ] Una burbuja del usuario con un bloque de 25 líneas lo muestra plegado.
- [ ] El JSON guardado de la conversación no cambia de formato.

### 10.5 Tests
- Unitarios en `crates/cincel-chat/src/code_folds.rs` (`split_markdown`: vallas `` ` `` y `~`, cierre más largo, bloque abierto al final, 20/21 líneas, varios bloques, bloque con sangría, texto sin bloques).
- `crates/cincel-chat/src/code_fold_tests.rs` (nuevo; `VisualTestContext`): los criterios de pintado, clic, copia, streaming y burbuja.

---

## 11. Ya corregido por el orquestador (J)

- **Qué pasaba**: al retomar una sesión (`session/load`) el agente reproduce el historial; las menciones `@archivo`, que viajan al agente como `resource_link`, volvían como fragmentos de mensaje del usuario y se mostraban como un mensaje suelto "@a@b@c".
- **Qué se hizo**: `ChatPanel::is_already_shown` (`crates/cincel-chat/src/panel.rs`) trata todo `UserMessageChunk` con `ContentBlock::ResourceLink` o `ContentBlock::Resource` durante la reproducción como ya mostrado (el mensaje guardado ya tiene esas menciones como texto). Test: `a_replayed_file_mention_is_not_shown_as_a_new_message` (`crates/cincel-chat/src/tests.rs`).
- R2-J lo anota en `docs/etapas/etapa-7.md` ("Ronda 2") y en `CHANGELOG.md`.

---

## 12. Resumen de acciones, ajustes y textos

### 12.1 Acciones nuevas o con atajo nuevo
| Acción | Atajo por defecto | Contexto |
|---|---|---|
| `editor::scroll_line_up` (nueva) | `ctrl-up` | `Editor` |
| `editor::scroll_line_down` (nueva) | `ctrl-down` | `Editor` |
| `editor::toggle_whitespace` (existente) | `ctrl-alt-w` | `Editor` |

Las tres van en `cincel_editor::default_key_bindings` **y** en `DEFAULT_KEYMAP_JSONC` (sección `Editor`), en `shortcuts_modal::description`, en `docs/usuario/atajos.md` y en `default_keymap_has_every_shortcut_of_the_spec`. No hay contextos nuevos. La flecha "ir al final" y "Ver más/Ver menos" son botones, sin acción ni atajo.

### 12.2 Ajustes nuevos
`files.trim_trailing_whitespace_on_save` (`true`) y `files.ensure_final_newline_on_save` (`true`), con sus filas en Configuración → Archivos (§7.8) y en `docs/usuario/ajustes.md`.

### 12.3 Textos de interfaz nuevos (español)
Encabezado: "conectada", "desconectada", "sesión vencida", "no disponible", "autenticación requerida" (en minúscula, en la insignia del encabezado; las filas del menú siguen con "Conectada", "Sesión vencida", "No disponible"). Conversación: "Pensando…", "Trabajando…", "Escribiendo…", "Esperando permiso…", "Ir al final" (tooltip), "Ver más (N líneas)", "Ver menos". Configuración: "Quitar espacios al final de las líneas al guardar", "No toca los archivos Markdown ni los cambios del agente que todavía no decidiste.", "Terminar el archivo con un salto de línea al guardar", "Si al archivo le falta el salto de línea final, se agrega al guardar." Atajos: "Desplazar una línea hacia arriba", "Desplazar una línea hacia abajo", "Mostrar u ocultar los espacios en blanco".

---

## 13. Qué cubre respecto de lo acordado

| Decisión | Cómo la cubre esta spec | Recortes o límites, con motivo |
|---|---|---|
| **A** Encabezado: nombre completo y una sola insignia con el estado de la conexión; sin "Conectada" ni tipo; la actividad del agente va abajo (decisión del autor, 2026-10-01) | §3, R1, R2, R4. | Con el panel en su mínimo, un nombre que no entra se corta con "…" y tooltip (R4: no hay otra forma en 280 px). |
| **B** Menú en dos líneas: nombre arriba, tipo en gris abajo, icono a la izquierda, insignia a la derecha, sin correo ni plan | §4, R3. | El tipo se muestra siempre, también si es igual al nombre (R3). |
| **C** Imágenes arriba del texto en el mensaje; miniaturas arriba del campo en la caja | §5. | En la caja ya era así: se fija con test, sin cambio de código. |
| **D** Margen de media pantalla al final, solo visual; la barra flotante deja de tapar | §6, R10. | Solo en el editor de código (no en el compositor ni en la caja de comentario). |
| **E1** Apariciones de la palabra o de la selección de una línea, palabra completa, filas visibles, retardo corto, pintura de la búsqueda | §7.1, R11. | No en filas fantasma; no con una búsqueda abierta. |
| **E2** Copiar/cortar sin selección = línea; pegar como línea | §7.2, R12, R13. | Solo en el editor de código. |
| **E3** = D | §6. | — |
| **E4** `}` `]` `)` en una línea de solo sangría quitan un nivel | §7.4, R14. | Gana el salto sobre un cierre auto-insertado. |
| **E5** `Enter` en línea de solo sangría sin espacios sueltos | §7.5. | — |
| **E6** `Backspace` en la sangría borra un nivel | §7.6, R14. | Con tabs mezclados borra un carácter (como hoy). |
| **E7** `Ctrl+↑/↓` sin mover el cursor y atajo libre para espacios en blanco | §7.7, R15: `Ctrl+Alt+W`, verificado. | El atajo alterna en la pestaña actual, como `Alt+Z`. |
| **E8** Dos ajustes encendidos por defecto, fila en Configuración → Archivos | §7.8, R16. | No se recortan espacios en Markdown ni en segmentos pendientes del agente, ni durante un turno sobre ese archivo (R16). |
| **F** "Pensando…" como último elemento de la conversación con su animación; desaparece al empezar la respuesta | §8, R5. | Se oculta también mientras se ve la fila de pensamiento en vivo, que ya dice "Pensando… (N s)" con la misma ruedita (para no repetirla). |
| **G** Flecha flotante "ir al final" solo cuando el usuario subió | §9, R6. | — |
| **H** Seguir solo si estás al final; quedarse si subiste (también al terminar y al re-renderizar); volver a seguir al bajar o con la flecha | §9, R6. | Enviar un mensaje también vuelve a seguir (R6: lo pide el gesto del usuario). |
| **I** Bloques ``` de más de 20 líneas plegados a 12 con degradado y "Ver más (N líneas)"/"Ver menos"; estado por bloque en la sesión; "Copiar" entero | §10, R7, R8, R9. | Solo bloques que empiezan en la columna 0 (R8). La selección de un tramo que cruza el borde de un bloque largo queda partida (R7). |
| **J** Menciones reproducidas como "@a@b@c" | §11 (hecho). | — |

Regla del proyecto: nada de lo acordado se achicó en silencio; cada límite tiene su motivo y las dudas de §15 se le preguntan al autor antes de la Ola 1.

---

## 14. Plan de subetapas

### 14.1 Reglas fijas de construcción (iguales a §9.1 de la spec 09)
1. **Mismo árbol, sin worktrees** (cada uno duplicaría el `target`). Cada subagente trabaja sobre crates o archivos disjuntos de los de su ola.
2. **Solo `Edit`** (nunca reescritura completa) en archivos que otra subetapa de la misma ola también toca: `render.rs`, `panel.rs`, `model.rs` (chat); `view.rs`, `element.rs`, `ops.rs`, `actions.rs`, `view/editing.rs` (editor); `defaults.rs`, `shortcuts_modal.rs`, `docs/usuario/atajos.md`, `CHANGELOG.md`.
3. **Tests nuevos en archivos nuevos** (los nombres de §3–§10). Los tests existentes que afirman un comportamiento que esta spec cambia (el tipo y la insignia en el botón del encabezado, `scroll_to_end` en `push`) se ajustan con `Edit` y se anota cuáles.
4. **Features de test fijas**: toda corrida de tests o clippy usa exactamente `--features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support` sobre los crates con GPUI; los crates sin GPUI, sin features. Nunca otra combinación.
5. **`cargo sweep --stamp` / `--file` solo el orquestador**, antes y después de cada verificación completa; los builds en contenedor usan `target/ubuntu-22.04` y se borran al terminar.
6. Cargo en primer plano con timeout; **sin procesos en segundo plano** ni bucles de espera; al terminar, `pgrep -af "sleep|cincel"` vacío.
7. **Nada que solo corra en GitHub sin auditarlo localmente** paso por paso: si cambia un workflow, cada bloque `run:` se ejecuta en local y en la imagen Ubuntu 22.04 del empaquetado; los tests no suponen herramientas del entorno, todo tiempo límite usa `CINCEL_PERF_BUDGET_FACTOR`, y un test que depende de un hilo real espera con tope (no un solo `run_until_parked`). Esta ronda no cambia workflows.
8. **Nadie hace commits** ni push ni crea etiquetas; eso lo hace el autor.
9. **Nada de capturas de pantalla completa** ni de la sesión del autor; las imágenes de los tests se generan en el test.
10. **Sin datos personales**: nada de credenciales, tokens, rutas personales, nombre del equipo ni correos en el repo (los fixtures de conexión usan correos de ejemplo `@example.com`).

### 14.2 Subetapas

| Sub | Contenido | Crates / archivos | Depende de | Subagente | Tamaño |
|---|---|---|---|---|---|
| **R2-A** | Encabezado (§3), menú en dos líneas (§4), imágenes arriba (§5); ajuste de los tests viejos del botón. | chat: `model.rs` (`HeaderBadge`), `render.rs` (`render_header`, `render_connection_rows`, pastilla), `render_media.rs`; tests `header_badge_tests.rs`, `connection_rows_tests.rs`, `message_media_order_tests.rs` | — | Sonnet | S |
| **R2-B** | Fila de actividad al pie de la conversación (§8), seguir el final y flecha (§9). | chat: `model.rs` (`AgentActivity`), `panel.rs` (`push`, `send`, `load_conversation`, `clear`, `activity`, `jump_to_end`), `render.rs` (`render_transcript`, `render_list_item`); tests `activity_row_tests.rs`, `transcript_follow_tests.rs` | — | Opus | M |
| **R2-C** | Bloques de código plegados (§10). | chat: `code_folds.rs` (nuevo), `markdown.rs`, `model.rs` (`AgentText.segments`), `panel.rs` (fragmentos, `rebuild_views`, `expanded_code_blocks`), `render.rs` (`render_agent_text`, burbujas); test `code_fold_tests.rs` | B (mismas zonas de `panel.rs`/`render.rs`) | Opus | M |
| **R2-D** | Margen final (§6) y E7 (§7.7): `max_scroll_top`, barra, `ScrollLineUp/Down`, bindings `ctrl-up`/`ctrl-down`/`ctrl-alt-w` en `actions.rs` y `DEFAULT_KEYMAP_JSONC`, filas del test del keymap, `shortcuts_modal::description`, `atajos.md`. | editor (`view.rs` funciones de desplazamiento, `element.rs` cálculo de `max_scroll` y barra, `actions.rs`), settings (`defaults.rs`, `Edit`), workspace (`shortcuts_modal.rs`, `Edit`), `docs/usuario/atajos.md` (`Edit`); tests `scroll_margin_tests.rs`, `whitespace_toggle_tests.rs` | — | Sonnet | S |
| **R2-E** | E2, E4, E5, E6 (§7.2, §7.4–§7.6). | editor: `view.rs` (`on_copy`, `on_cut`, `on_paste`, `on_insert_newline`, `on_backspace`, `replace_text_in_range`), `view/editing.rs` (`LineClipboard`, `outdent_for_closer`), `ops.rs` (helpers); tests `line_clipboard_tests.rs`, `indent_editing_tests.rs` | — | Sonnet | M |
| **R2-F** | E1 apariciones (§7.1) y medición `--bench typing`. | editor: `view.rs` (estado y temporizador), `element.rs` (`prepaint`/`paint` de apariciones), `search.rs` (`find_occurrences`), `theme.rs`; test `occurrences_tests.rs` | — | Opus | M |
| **R2-G** | E8 limpieza al guardar (§7.8). | settings (`settings.rs` `FilesSettings`, `defaults.rs` `DEFAULT_SETTINGS_JSONC`), editor (`ops.rs` `save_cleanup_edits`, `view.rs` `clean_up_for_save`), workspace (`center.rs` `save_path`, `settings_view.rs` filas), `docs/usuario/ajustes.md`; test `save_cleanup_tests.rs` | E (comparten `ops.rs` y `view.rs`) | Sonnet | M |
| **R2-H** | Documentación de usuario: `docs/usuario/chat.md` (encabezado, menú, "Pensando…", flecha, bloques plegados), `docs/usuario/conexiones.md` (menú en dos líneas), archivo nuevo `docs/usuario/editor.md` ("Escribir en el editor": apariciones, copiar la línea, sangría, margen final, limpieza al guardar) con su entrada en `docs/usuario/README.md`; `user_docs_tests.rs` y `tools/check-links.sh` en verde. | `docs/usuario/` | A, B, C, D, E, F, G | Sonnet | S |
| **R2-I** | Verificación del orquestador con §17 completa (incluido `--bench typing` y `--bench idle`) y corrección de lo que falle (devuelto al subagente dueño). | — | todas | orquestador | S |
| **R2-J** | Cierre (§18). | docs, `CHANGELOG.md`, `dist/` | I | orquestador | S |

**Olas** (todas en el mismo árbol): **Ola 1**: R2-A, R2-B, R2-D, R2-E, R2-F en paralelo (chat: A y B comparten `render.rs`, `panel.rs` no lo toca A; editor: D, E y F comparten `view.rs` y `element.rs` en funciones distintas; solo `Edit`). **Ola 2**: R2-C y R2-G en paralelo (crates distintos salvo que G toca `cincel-editor/src/ops.rs` y `view.rs`, y C ninguno del editor). **Ola 3**: R2-H. **Ola 4**: R2-I. **Ola 5**: R2-J.

Cada subagente entrega: qué criterios cumple (con el test que lo prueba), cuáles no y por qué (desviación anotada), qué tests existentes ajustó, y la salida de los comandos de §17 para su parte.

---

## 15. Riesgos y dudas

**Dudas para el autor** (no bloquean la construcción: si no hay respuesta, se construye lo propuesto):
- **"conectada" en el encabezado** (R2): resuelta por el autor el 2026-10-01: arriba solo el estado de la conexión; la actividad (pensando / trabajando / escribiendo / esperando permiso) solo en la fila al pie de la conversación.
- **Markdown al guardar** (R16): propongo no quitar los espacios del final en archivos `.md`, porque dos espacios al final de una línea son un salto de línea en Markdown. ¿O preferís que se quiten también ahí?

**Riesgos técnicos:**
- Partir una respuesta en varios `TextView` (R7) cambia cómo se ven dos cosas en el borde de un bloque largo: la selección que cruza el borde queda partida, y un enlace de estilo referencia (`[x][1]` con `[1]: …` al final) definido en otro segmento no se resuelve. Los dos casos son raros en respuestas de agentes; se prueban en la lista manual.
- La animación de salida de la flecha: GPUI quita el elemento al ocultarlo; si el desvanecido de salida exige estado extra, se deja solo el de entrada (sigue dentro de §4 de `02-visual.md`) y se anota como desviación.
- Apariciones (E1): el `prepaint` suma una búsqueda de subcadena sobre el texto visible por cuadro mientras hay consulta; R2-F mide `--bench typing` antes y después (M4 no puede empeorar) y el reposo (`--bench idle`, M8: 0 cuadros en reposo con el cursor quieto, ya que el temporizador es de un solo disparo).
- Limpieza al guardar en un archivo en revisión: quitar espacios en filas **ya decididas** es una edición del usuario y viaja al agente como parche en el próximo mensaje (así funciona hoy cualquier edición a mano). Es correcto (el archivo cambió), pero puede sorprender; se documenta en `docs/usuario/editor.md`.
- `ListState` de `gpui-pre` 0.3.5 deja de seguir solo con desplazamientos del usuario; si algún re-medido moviera la vista sin seguir, R2-B lo fija en el test de §9.4 (cambio de zoom) y lo corrige reanclando `logical_scroll_top` antes y después de `rebuild_views`.

Construcción (2026-10-01): R5 se corrigió para coincidir con §8; la fila dice "Escribiendo…" mientras llega la respuesta.

---

## 16. Lista de comprobación manual (autor)

Con el paquete que deja R2-J en `dist/` (o con `cargo run -p cincel --release`).

1. **Encabezado**: con una conexión activa, arriba del chat se ve el nombre completo y una sola etiqueta ("conectada"); no se ve "Conectada" ni el tipo. Mandá un mensaje: la etiqueta sigue diciendo "conectada" todo el turno (no cambia con lo que hace el agente). Cerrá el agente por fuera (o dejá vencer la sesión) y la etiqueta pasa a "desconectada" o "sesión vencida".
2. **Menú**: abrí "Conectar": cada fila tiene el icono a la izquierda, tu nombre arriba, el tipo (Claude/Codex/Antigravity) abajo en gris y la etiqueta de estado a la derecha; sin correo ni plan.
3. **Imágenes**: mandá un mensaje con texto y una imagen: en el mensaje enviado la imagen está arriba del texto. En la caja, al adjuntar, la miniatura aparece arriba de donde escribís.
4. **Actividad al pie**: al mandar un mensaje, abajo de todo en la conversación aparece "Pensando…" girando; pasa a "Trabajando…" cuando el agente usa una herramienta, a "Escribiendo…" cuando empieza la respuesta y a "Esperando permiso…" si pide uno; desaparece al terminar. Arriba no cambia nada.
5. **No te mueve mientras leés**: pedile al agente algo largo; mientras escribe, subí con la rueda a leer: la vista se queda quieta, también cuando termina. Aparece la flecha redonda abajo; tocala: baja al final y vuelve a seguir. Repetí bajando con la rueda hasta el final en vez de tocar la flecha.
6. **Bloques largos**: pedile "mostrame un archivo de 60 líneas en un bloque de código": se ve plegado a 12 líneas con difuminado y "Ver más (48 líneas)". Tocalo: se despliega y dice "Ver menos". "Copiar" pega las 60 líneas en el editor.
7. **Margen final**: abrí un archivo largo, bajá hasta el final: la última línea puede quedar a media pantalla; con un cambio del agente pendiente al final del archivo, la barra flotante ya no tapa las últimas líneas.
8. **Apariciones**: poné el cursor sobre una variable: un instante después se marcan suavemente sus otras apariciones visibles. Seleccioná un trozo de una línea: se marcan sus repeticiones.
9. **Copiar la línea**: sin seleccionar nada, `Ctrl+C` en una línea, bajá a otra y `Ctrl+V`: la línea entra entera arriba de la del cursor. `Ctrl+X` sin selección corta la línea; `Ctrl+Z` la devuelve.
10. **Sangría**: en un archivo con sangría de 4, después de `{` y `Enter`, escribí `}` en la línea vacía: queda alineado con la línea de la `{`. En una línea solo con sangría, `Enter`: la línea de arriba queda sin espacios. `Backspace` en la sangría borra 4 espacios de una vez.
11. **`Ctrl+↑`/`Ctrl+↓` y espacios**: `Ctrl+↓` varias veces: el texto se mueve y el cursor no. `Ctrl+Alt+W`: se ven los espacios en blanco en esa pestaña; otra vez, se ocultan.
12. **Guardar limpio**: escribí "hola   " (con espacios al final) en la última línea, sin `Enter`, y guardá: los espacios se van y el archivo termina con un salto de línea (`cat -A archivo` muestra `hola$`). `Ctrl+Z` lo devuelve. En Configuración → Archivos apagá las dos opciones nuevas, repetí: se guarda tal cual. Volvé a encenderlas.
13. **Retomar una conversación**: con una conversación donde mencionaste archivos con `@`, cerrá Cincel, abrilo y retomala desde el historial: no aparece un mensaje suelto "@a@b@c".
14. `cincel --version` dice `cincel 0.2.0`; instalar el `.deb` (o el comprimido) y repetir 1, 5 y 12 rápido.
15. Leer `docs/usuario/editor.md` y las partes nuevas de `docs/usuario/chat.md`; si algo no se entiende, avisar.
16. Commit, push, etiqueta `v0.2.0` y publicar el borrador (procedimiento de `09-etapa7-conexiones-imagenes-comentarios.md` §9.3, sin cambios).

---

## 17. Comandos de verificación

Siempre con el mismo conjunto de features:

```sh
cargo sweep --stamp                                                        # solo el orquestador
cargo fmt --all --check
cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings
cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
cargo test --workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support
cargo deny check licenses advisories bans sources
cargo run -p cincel -- --smoke-test .
cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .   # control de fugas
cargo build --release -p cincel && target/release/cincel --smoke-test --bench .
target/release/cincel --bench typing <corpus>/medio --bench-file <corpus>/medio/cinco-mil.rs   # M4 no empeora (R2-F, R2-I)
target/release/cincel --bench idle <corpus>/medio                          # M7/M8 (R2-I)
packaging/build.sh && packaging/verify.sh                                  # R2-J
tools/privacy-check.sh                                                     # R2-J
cargo sweep --file                                                         # solo el orquestador
pgrep -af "sleep|cincel"; docker ps                                        # vacíos
```

Un subagente verifica solo su parte:
- Si tocó un crate con GPUI (`cincel-editor`, `cincel-chat`, `cincel-workspace`, `cincel`): `cargo test -p cincel-editor -p cincel-chat -p cincel-workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support` (y lo mismo con `clippy`).
- Si tocó solo crates sin GPUI (`cincel-settings`): `cargo test -p cincel-settings` sin features. R2-D y R2-G, que tocan `cincel-settings` y crates con GPUI, corren las dos.

---

## 18. Cierre (R2-J)

1. `docs/etapas/etapa-7.md` gana la sección **"Ronda 2"** (después de "Verificación"): qué se construyó por subetapa (R2-A a R2-H), las decisiones R1–R17, el arreglo de J, desviaciones con motivo, la tabla de verificación de §17 y la lista de §16. El encabezado del documento suma "Ronda 2: cerrada (fecha)".
2. Correcciones de otras specs listadas al comienzo (`09` §3 y §5, `02-visual.md` §5, §7 y §8) y `modulos/chat.md`, `modulos/editor.md`, `modulos/workspace.md`, `modulos/settings.md`.
3. `CHANGELOG.md`: la entrada `[0.2.0]` suma lo de esta ronda y toma la fecha de cierre (R17). `[workspace.package]` sigue en `0.2.0`.
4. Paquetes 0.2.0 reconstruidos con `packaging/build.sh` y verificados con `packaging/verify.sh` (Ubuntu 22.04 y 24.04); `tools/privacy-check.sh` sobre el árbol y sobre los paquetes.
5. `pgrep -af "sleep|cincel"` y `docker ps` vacíos; `cargo sweep --file`.
6. Publicación: la hace el autor (§16, punto 16).
