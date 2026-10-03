# Spec 09: Etapa 7, conexiones más limpias, imágenes en el chat y comentarios para el agente

Estado: v1.0 (2026-09-30), definida con el autor. Fuente de verdad del alcance de la Etapa 7 (`05-plan-etapas.md`). Cierre de etapa = versión **0.2.0**.

Modelo de formato: `08-etapa6-cierre-1-0.md`. Cada ítem tiene primero un **resumen en lenguaje llano** (para el autor) y después el **diseño técnico** (para los subagentes). Las palabras técnicas de los resúmenes se explican la primera vez.

Esta spec corrige otras en estos puntos (el orquestador los aplica al cerrar, E7-J; no se tocan antes):
- `02-visual.md §6.1`: los botones del segmento son tres (Aceptar, Rechazar, Comentar) y miden 280 px (compactos: 80 px); `§5`: marca de comentario en el margen; `§7`: miniaturas de imágenes, etiquetas de comentarios en la caja de texto y tarjetas de comentario en el mensaje enviado; encabezado del chat sin correo, plan ni "Usado hace…".
- `03-arquitectura.md §5`: el `BlockMap` deja de ser futuro (aloja las cajas de comentario, §6.5); `§4`: lo que viaja al agente incluye los comentarios (§6.8); `§6`: `state.json` versión 2 con comentarios.
- `modulos/chat.md`: "pegar imagen no soportado en v1" pasa a soportado (§5); popover de conexiones y encabezado (§3); cierre de menús (§4).
- `modulos/acp.md` "Prompt": bloque `image` (§5.3.3).
- `modulos/review.md`: comentarios en el store, formato del bloque al agente, persistencia v2 (§6.4, §6.8).
- `modulos/editor.md`, `modulos/workspace.md`, `modulos/connections.md` (la identidad solo se muestra en Configuración → Conexiones).

---

## 1. Qué trae esta etapa (resumen en lenguaje llano)

1. **La lista de conexiones, más limpia.** En el menú que se abre con "Conectar" y en el botón de arriba del chat se ve solo el nombre que le diste a cada conexión, qué agente es (Claude, Codex o Antigravity, con su icono) y si está conectada, con la sesión vencida o no disponible. El correo, el plan y "Usado hace…" desaparecen de ahí: siguen en Configuración → Conexiones, donde ya estaban.
2. **Los menús del chat se cierran solos.** Hoy el menú de conexiones y el del historial de conversaciones se quedan abiertos si hacés clic en otro lado o pasás al editor. Desde esta etapa se cierran al hacer clic fuera, al pasar a cualquier otra parte (editor, árbol, otra ventana), con `Esc` o cuando la ventana deja de estar en primer plano. Lo mismo para los demás menús que se despliegan en Cincel (el de modelo y modo del agente, el de `@` y `/`, el menú de la barra de título).
3. **Imágenes en el chat.** Podés mandarle al agente una captura o una foto de tres formas: pegarla con `Ctrl+V`, arrastrar el archivo desde el explorador de archivos hasta la caja de texto, o con el botón del clip ("Adjuntar"), que abre la ventana de elegir archivos. Sirven PNG, JPEG, GIF y WebP. Cada imagen adjunta aparece como una miniatura dentro de la caja, con una `×` para quitarla. Si la imagen es muy grande (más de 2 000 píxeles de lado) Cincel la achica antes de mandarla, como hace Zed; si aun así pesa más de 10 MB, te avisa y no la adjunta. En el chat, tu mensaje muestra las imágenes en chico; un clic la abre en grande dentro de Cincel y `Esc` la cierra. Si el agente conectado no acepta imágenes, al intentar adjuntar te avisa "«Nombre» no acepta imágenes". Las imágenes se guardan con la conversación, así que se siguen viendo cuando la volvés a abrir. Hoy los tres agentes las aceptan.
4. **Comentarios para el agente sobre el código.** Junto a "Aceptar" y "Rechazar" de cada cambio del agente aparece un tercer botón, **"Comentar"**: abre una cajita debajo de esas líneas donde escribís qué querés (por ejemplo "esto hacelo con un diccionario"). También podés comentar cualquier grupo de líneas, aunque el agente no las haya tocado: las seleccionás y elegís "Comentar selección" en el menú del botón derecho, o apretás `Ctrl+Shift+M`. La cajita no corre el código hacia los costados ni cambia el margen: se intercala entre las líneas como las líneas rojas de lo que el agente quitó.
   - Comentar es independiente de decidir: podés comentar y además aceptar, rechazar, decidir línea por línea, o dejar el cambio sin decidir.
   - Cada comentario pendiente aparece en la caja del chat como una etiqueta (por ejemplo "calc.py:24-28") con su `×`. **Se mandan solos con el próximo mensaje que escribas**; no hay un botón aparte.
   - Al agente le llega, junto con lo que ya le llega hoy (lo que rechazaste o editaste a mano), cada comentario con el archivo, las líneas, cómo quedó el cambio (sin decidir, aceptado, rechazado, mixto o "sin cambios del agente"), el código como está ahora y tu texto. Nunca le llega lo que aceptaste o dejaste pendiente por sí solo, y nada se repite en el mensaje siguiente.
   - En el chat, tu mensaje muestra una tarjeta por comentario enviado (archivo, líneas, código y texto), para que quede a la vista qué le dijiste.
   - Si cerrás Cincel con comentarios sin mandar, se guardan y vuelven al abrir la carpeta.
5. **El terminal integrado queda para la Etapa 8**, como decidiste. Lo demás de "después de la 1.0" sigue esperando.

A vos te queda, al final, la lista de comprobación manual (§11), que esta vez incluye probar con Claude, Codex y Antigravity reales que entiendan y atiendan un comentario y una imagen.

---

## 2. Decisiones técnicas de esta spec

| # | Decisión | Motivo |
|---|---|---|
| D1 | `cincel_chat::ChatConnection` suma `agent_name: String` ("Claude"/"Codex"/"Antigravity", de `AgentKind::display_name`) y **conserva** `identity` y `last_used`: los sigue usando Configuración → Conexiones (`settings_view.rs`, que pinta esos mismos `ChatConnection`). El chat deja de pintarlos; la lista de "Eliminar conexión…" (`connection_modal.rs`) sigue la misma regla que el menú (icono, nombre, tipo). La confirmación "Listo: conectado como …" del flujo de conexión **no cambia**: es el aviso del momento de conectar, no una lista. | La identidad queda "únicamente en Configuración → Conexiones" (decisión A) sin romper la pantalla que la necesita. |
| D2 | En el botón del encabezado, la insignia de estado de la conexión va dentro del botón; la píldora de actividad del agente ("listo", "pensando…") sigue afuera, a su derecha, como hoy. El tipo de agente se omite si el nombre de la conexión ya es exactamente el tipo (sin distinguir mayúsculas), para no leer "Claude Claude" (desde la ronda 2, solo en la lista de 'Eliminar conexión…'; spec 10 R3). | Son dos cosas distintas (estado de la cuenta y actividad del turno); la decisión A habla del botón. |
| D3 | Cierre de menús del chat: foco propio del menú (`popover_focus: FocusHandle`), `on_mouse_down_out` en la caja del menú, `cx.on_focus_out` del foco del menú (y del compositor para `@` y `/`), `cx.observe_window_activation` y `Esc`. Un clic en el mismo botón que abrió el menú lo cierra una sola vez (guarda `just_dismissed`, §4.2). | Es el mecanismo de GPUI que ya usa gpui-kit en `PopupMenu` (`on_mouse_down_out`); el foco cubre `Ctrl+L`, el clic en el editor o el árbol y otra ventana. |
| D4 | Las imágenes van al agente como `ContentBlock::Image(ImageContent::new(data_base64, mime_type))` **sin `uri`**, después de los bloques de texto y menciones, en el orden en que se adjuntaron. `PromptBlock::Image { mime_type: String, data: Arc<[u8]> }` lleva los bytes; la conversión a base64 se hace en `into_wire` (`base64` 0.22, ya está en `Cargo.lock`). | `codex-acp` 1.13.1 usa el `uri` en vez de los datos si es `http(s):` o `data:`; `claude-agent-acp` 0.84.0 usa `data` si viene. Sin `uri` los tres toman los datos. |
| D5 | Procesado con `image` 0.25 (ya en el árbol por GPUI; sin crates nuevos en `Cargo.lock`): se detecta el formato por el contenido, no por la extensión. Si el lado mayor es ≤ 2 000 px y el archivo ≤ 10 MB, **se mandan los bytes originales sin recodificar**. Si hay que reducir: `resize` a 2 000 px de lado mayor con `FilterType::Lanczos3`, orientación EXIF aplicada; PNG → PNG; JPEG → JPEG calidad 85; WebP → PNG si tiene transparencia, si no JPEG 85 (`image` solo codifica WebP sin pérdida, que pesaría más); GIF → primer cuadro en PNG con aviso. Límite de 10 imágenes por mensaje. | Lo mínimo para que la imagen llegue igual que se ve; recodificar solo cuando hace falta. El tope de 10 está por debajo del que aceptan los modelos y evita mensajes enormes. |
| D6 | Capacidad: `cincel_acp::agent_supports_images(&AgentCapabilities) -> bool` (= `prompt_capabilities.image`), leída en `AgentEvent::Connected`. Sin conexión activa o antes de `Connected`, no se adjunta ("Conectá un agente para adjuntar imágenes"). | ACP obliga al cliente a no mandar `image` si el agente no lo anunció. |
| D7 | Las imágenes enviadas se guardan en `~/.local/state/cincel/workspaces/<hash>/conversations/<id>/images/<sha256>.<ext>`; el JSON de la conversación guarda solo la referencia (`MessageBlock::Image(ImageRef)`). `CONVERSATION_VERSION` pasa a 2 (lee la 1 igual). Borrar una conversación (o las de una conexión) borra su carpeta. | La decisión B: no base64 dentro del JSON; se ven al reabrir. |
| D8 | El visor de imagen a tamaño completo es una capa del workspace (`image_viewer.rs`) sobre toda la ventana, no del panel del chat. | El panel del chat es angosto; flotante no mueve nada (D16 de la spec 07). |
| D9 | Los comentarios viven en `cincel-review` (`comments.rs`, dentro de `ReviewStore`), con anclas `cincel_text::Anchor` en un `AnchorMap` por archivo, alimentado con los `BufferEvent` que el workspace ya recibe; el estado (pendiente, aceptado, rechazado, mixto, sin cambios) se calcula **al enviar**, con marcas de decisión que cada aceptar/rechazar deja en los comentarios que toca. | El store ya es el dueño de los segmentos, sus decisiones y su persistencia; sin GPUI, se prueba con tests unitarios. |
| D10 | La caja de comentario es una **fila de bloque** del `BlockMap` nuevo (`crates/cincel-editor/src/block_map.rs`), capa entre `WrapMap` y el elemento, como preveía `03-arquitectura.md §5`: filas enteras de interfaz (alto = `ceil(alto_px / alto_de_línea)`) insertadas después de una fila de texto. El cursor nunca se para en ellas; el ancho del margen y la columna del texto no cambian; si un bloque aparece o desaparece **arriba** de la primera fila visible, el desplazamiento se compensa para que la fila que estás mirando no se mueva. | Es el mismo modelo que las filas fantasma (filas que se intercalan) pero sin texto, que es lo que pidió el autor; filas enteras mantienen la cuenta `fila × alto de línea` que usa todo el elemento. |
| D11 | El texto del comentario se escribe en un `EditorView` anidado con `EditorChrome::Minimal` (como el compositor del chat), con contexto de teclas `comment_box`. | Acentos, IME, deshacer y pegar ya funcionan ahí; nada nuevo que probar. |
| D12 | El editor no tenía menú contextual: se agrega uno (clic derecho) con "Cortar", "Copiar", "Pegar", separador y "Comentar selección" (o "Comentar línea" sin selección), con el `ContextMenu` de gpui-kit que ya usa el árbol. Atajo nuevo **`Ctrl+Shift+M`** → `editor::comment_selection`, libre (verificado contra `DEFAULT_KEYMAP_JSONC`, `cincel_editor::actions`, `cincel_chat::actions` y `cincel-workspace/src/keymap.rs`). | El acuerdo pide la entrada en el menú contextual. "M" de mensaje; `Ctrl+/` ya comenta código y confundiría. |
| D13 | En la caja del chat, los comentarios pendientes van en una **fila de etiquetas** dentro de la caja, encima del texto, con el mismo aspecto que la etiqueta de archivo del mensaje enviado más una `×`. No son texto dentro del borrador. | Desde las correcciones de la Etapa 2 las menciones `@archivo` son texto que se escribe y edita; un comentario no se escribe en el chat, así que su etiqueta es un objeto aparte que solo se quita con la `×` (o desde el margen). |
| D14 | Formato al agente en **texto plano en inglés**, dentro del mismo `<user_review_feedback>` que hoy, **después** de los parches (§6.8). Números de línea desde 1, rangos inclusivos, como está el archivo ahora. Fragmento entre vallas de código con la extensión como lenguaje; tope 120 líneas / 12 KB por comentario. | Mismo idioma que `REPORT_HEADER`; no depende de ninguna capacidad del agente. |
| D15 | Persistencia: `state.json` versión 2 con `"comments": [...]`; el fragmento de cada comentario se guarda como objeto en `objects/<sha256>` (el mismo almacén por contenido). Una versión 1 carga sin comentarios. Al abrir, si el archivo cambió fuera de Cincel, el comentario se reubica buscando su fragmento; si no aparece, se ancla a la línea más cercana. | Decisión C.9; reubicar sigue la regla C.3. |
| D16 | Los comentarios se toman del store al despachar el mensaje (`prepare_prompt`). Si el mensaje no llega a salir (se cancela mientras espera la foto, o el agente se va en esa espera), vuelven al margen y a la caja del chat (`ReviewStore::restore_comments`) y su tarjeta dice "No se envió: el comentario volvió al margen". Si salió, no vuelven aunque el turno falle (igual que hoy los parches). | Que un comentario nunca se pierda sin haber salido. |
| D17 | El diálogo de cierre ("Hay N cambios de agente sin decidir…") aparece en los mismos casos que hoy; si además hay comentarios, suma una línea que los cuenta. Sus botones no cambian y ninguno borra comentarios. Con solo comentarios (sin cambios pendientes) no aparece: se guardan igual (C.8 l). | "Los cuenta como pendientes, sin cambiar su comportamiento". |
| D18 | Versión **0.2.0** en `[workspace.package]`; entrada en `CHANGELOG.md`; release por etiqueta `v0.2.0` con el procedimiento validado (`docs/etapas/etapa-6.md`, "Publicación"). | La 0.1.0 es la publicada. |

---

## 3. Conexiones: lista y encabezado del chat (A)

**Corregido por la spec 10 (ronda 2, 2026-10-01):** el encabezado muestra solo el nombre completo de la conexión y una insignia con el estado de la conexión (conectada / desconectada / sesión vencida / no disponible / autenticación requerida); el tipo de agente y la insignia 'Conectada' ya no van en el encabezado. Las filas del menú son de dos líneas (nombre arriba, tipo abajo, siempre) con el icono a la izquierda y la insignia a la derecha; la regla D2 (no repetir el tipo si el nombre es el tipo) queda solo para la lista de 'Eliminar conexión…'. Ver spec 10 §3 y §4.

### 3.1 Comportamiento
- **Menú "Conectar"** (`Popover::Connections`): una fila por conexión con, de izquierda a derecha: icono del proveedor (`provider_icon`, 18 px), nombre de la conexión, tipo de agente en gris (D2: se omite si es igual al nombre), insignia de estado (Conectada / Sesión vencida / No disponible, con el motivo en el globo de ayuda como hoy) y, si corresponde, "Volver a conectar" o "Reparar". Una sola línea por fila. Sin correo, sin plan, sin "Usado hace…".
- **Botón del encabezado**: icono (16 px), nombre, tipo en gris (D2), insignia de estado y la flecha. Sin identidad. Sin conexión activa sigue siendo el botón "Conectar".
- **"Eliminar conexión…"**: cada fila con icono, nombre y tipo; sin identidad.
- **Configuración → Conexiones**: sin cambios (nombre, identidad, "Usado hace…", insignia, acciones).
- Todo lo demás del menú (clic derecho → "Renombrar", pie con "Conectar nuevo agente…" y "Eliminar conexión…", resaltado de la activa o preseleccionada) no cambia.

### 3.2 Aspecto
- Fila: `px_1p5`, `py_1`, alto de una línea de 13 px; el tipo en `TEXT_LABEL` (11 px) `text.muted`; insignia igual que hoy. Todo con `ChatSettings::px` (escala `ui_scale`).
- `CONNECTION_POPOVER_MAX_HEIGHT` (380 px) no cambia; con filas de una línea entran más conexiones antes del desplazamiento.

### 3.3 Diseño técnico
- `cincel-chat/src/model.rs`: `ChatConnection.agent_name: String`; la documentación de `identity` y `last_used` dice que el chat ya no los pinta y que los usa Configuración.
- `cincel-chat/src/render.rs`: `render_header` y `render_connection_rows` dejan de leer `identity` y `last_used`; nueva función `agent_type_label(connection) -> Option<SharedString>` (D2); la insignia se extrae a `connection_badge(badge, theme, s)` y la usan las filas y el botón.
- `cincel-workspace/src/agents.rs` (`refresh_connections`): llena `agent_name` con `AgentKind::from_agent_id(...)` (devuelve `Result`) `.map(AgentKind::display_name)`, o el `agent_id` si no es uno de los tres.
- `cincel-workspace/src/connection_modal.rs` (lista de borrado): quita `identity.summary()` y suma el tipo.
- `Identity::summary` queda (la usa Configuración por medio de `ChatConnection.identity`).

### 3.4 Criterios de aceptación
- [ ] Con una conexión que tiene correo y plan, ni el menú ni el botón del encabezado ni la lista de borrado muestran el correo, el plan ni "Usado".
- [ ] Configuración → Conexiones sigue mostrando correo, plan y "Usado hace…".
- [ ] El menú y el botón muestran icono, nombre, tipo e insignia; una conexión llamada "Claude" de tipo Claude no repite la palabra.
- [ ] "Sesión vencida" y "No disponible" se ven en el botón del encabezado cuando la conexión activa está así.

### 3.5 Tests
- `cincel-chat/src/connections_view_tests.rs`: el texto pintado de las filas y del botón (con `debug_selector` de cada fila y del botón) no contiene `identity` ni `last_used`; contiene nombre, tipo e insignia; regla D2.
- `cincel-workspace/src/connections_list_tests.rs`: `refresh_connections` llena `agent_name` para los tres tipos; la lista de borrado no muestra la identidad; la pestaña de configuración sí (con `ConnectionsFixture`).

---

## 4. Menús desplegables que se cierran solos (A, agregado por el autor)

### 4.1 Regla
Todo menú desplegable de Cincel se cierra solo en cualquiera de estos casos: **clic fuera de él**, **el foco pasa a otro elemento** (editor, árbol, chat, otra pestaña, `Ctrl+L`, un modal), **`Esc`**, o **la ventana pierde el foco**. Elegir una fila sigue cerrándolo como hoy. Un clic en el mismo botón que lo abrió lo cierra (y no lo vuelve a abrir).

Menús alcanzados:
- Chat, encabezado: conexiones (`Popover::Connections`), su menú contextual (`Popover::ConnectionMenu`) e historial (`Popover::Conversations`). **Hoy tienen el defecto.**
- Chat, compositor: selectores de modelo/modo/esfuerzo (`Popover::Config`, `Popover::Modes`), `@` (`Popover::Files`) y `/` (`Popover::Commands`). Mismo mecanismo propio del chat, **mismo defecto**.
- Barra de título (menú), menú contextual del árbol, desplegables de la pestaña de configuración y el menú contextual nuevo del editor (§6.5.6): son `PopupMenu`/`Select` de gpui-kit, que ya se cierran con clic fuera (`on_mouse_down_out`). Se prueban con la misma lista de casos; si alguno falla (por ejemplo, al perder la ventana el foco), se corrige con el mismo mecanismo desde Cincel (escuchando `DismissEvent` y la activación de la ventana), sin tocar gpui-kit.

### 4.2 Diseño técnico (`cincel-chat/src/panel.rs`, `render.rs`)
- `ChatPanel.popover_focus: FocusHandle`. `toggle_popover(popover, window, cx)` (suma `window`): al abrir un menú del encabezado o un selector del pie, `window.focus(&popover_focus)` y guarda el foco anterior (`restore_focus: Option<FocusHandle>`), que se devuelve al cerrar con `Esc` o al elegir una fila (no al cerrar porque el foco ya se fue a otro lado). `@` y `/` no toman el foco (el usuario sigue escribiendo).
- La caja del menú (`render_overlay`, `id("chat-popover")`) lleva `.track_focus(&popover_focus)` (salvo `@` y `/`) y `.on_mouse_down_out(...)` → `dismiss_popover(cx)`.
- En `ChatPanel::new`: `cx.on_focus_out(&popover_focus, window, …)` → `dismiss_popover`; `cx.on_focus_out(&composer_focus, window, …)` → cierra `Files`/`Commands`; `cx.observe_window_activation(window, …)` → si `!window.is_window_active()`, `dismiss_popover`.
- `dismiss_popover` pone `Popover::Closed` y `just_dismissed = Some(popover)`; `cx.on_next_frame` lo limpia. `toggle_popover(p)` con `just_dismissed == Some(p)` no abre (así el clic en el botón que lo abrió, que primero dispara el "clic fuera" o el cambio de foco, lo cierra una sola vez).
- `Esc` ya cierra (`cancel_turn` cierra el menú antes de cancelar nada; `chat::close_popover`); con el foco en el menú, la tecla llega por el contexto `Chat`, que lo contiene.
- Barra de título y demás menús de gpui-kit: test primero; arreglo solo si falla (§4.1).

### 4.3 Criterios de aceptación
- [ ] Para el menú de conexiones y para el historial, por separado: abrir → clic en el editor → cerrado; abrir → `Ctrl+L` → cerrado; abrir → `Esc` → cerrado; abrir → la ventana pierde el foco → cerrado; abrir → clic en el texto de la conversación → cerrado; abrir → clic en el mismo botón → cerrado y sigue cerrado.
- [ ] Lo mismo para el menú contextual de una conexión, los selectores del pie, `@` y `/` (para estos dos, el foco que sale es el del compositor).
- [ ] Lo mismo para el menú de la barra de título, el menú contextual del árbol y el del editor.
- [ ] Elegir una fila sigue funcionando (el clic dentro del menú no lo cierra antes de tiempo) y `Esc` devuelve el foco a donde estaba.

### 4.4 Tests
- `cincel-chat/src/popover_dismiss_tests.rs` (`VisualTestContext`): los seis casos de §4.3 para `Connections` y `Conversations`, y clic fuera / foco fuera / `Esc` / ventana inactiva (`cx.deactivate_window()`) para `ConnectionMenu`, `Config`, `Modes`, `Files`, `Commands`; elegir fila con clic real (`simulate_click` sobre `debug_bounds("connection-row-0")`) sigue emitiendo `ConnectionSelected`.
- `cincel-workspace/src/menu_dismiss_tests.rs`: con el workspace completo, los dos menús del chat → clic en el editor (`debug_bounds` del editor), `Ctrl+L`, `Esc`, ventana inactiva; menú de la barra de título y menú contextual del árbol con los mismos casos.

---

## 5. Imágenes en el chat (B)

### 5.1 Comportamiento
- **Tres formas de adjuntar** (todas terminan en la misma función, `ChatPanel::attach_images`):
  1. `Ctrl+V` en la caja de texto: si el portapapeles trae una imagen (`ClipboardEntry::Image`, formatos PNG/JPEG/WebP/GIF), se adjunta y no se pega texto; si trae archivos (`ClipboardEntry::ExternalPaths`, por ejemplo copiados en el explorador de archivos), se adjuntan los que sean imágenes; si trae solo texto, se pega el texto como hoy.
  2. Arrastrar uno o más archivos desde el explorador de archivos del sistema hasta la caja de texto. Mientras se arrastra encima, el borde de la caja se pinta en `border.focus`.
  3. Botón **"Adjuntar"** (clip) dentro de la caja, a la izquierda: abre el diálogo de archivos del sistema ("Adjuntar imagen", varios a la vez, filtro "Imágenes": png, jpg, jpeg, gif, webp).
- **Antes de adjuntar**: sin conexión activa → aviso "Conectá un agente para adjuntar imágenes" y no se adjunta. Si el agente conectado no anuncia imágenes → aviso "«Nombre» no acepta imágenes" (Nombre = nombre de la conexión) y no se adjunta.
- **Cada archivo**: formato por contenido; si no es PNG, JPEG, GIF ni WebP → "«nombre» no es una imagen PNG, JPEG, GIF o WebP". Si es más grande que 2 000 px de lado, se reduce (D5); si un GIF animado se reduce: "La animación de «nombre» se pierde al reducirla". Si tras reducir pesa más de 10 MB → "«nombre» pesa 14,2 MB aun reducida; el máximo es 10 MB" y no se adjunta. Más de 10 imágenes en el mensaje → "Se pueden adjuntar hasta 10 imágenes por mensaje" (se adjuntan las que entran). Un archivo que no se puede leer → "«nombre» no se pudo leer: motivo".
- **En la caja**: cada imagen es una miniatura cuadrada con una `×` arriba a la derecha para quitarla. Mientras se procesa, la miniatura muestra el indicador de carga y la línea de ayuda dice "Preparando 1 imagen…" / "Preparando N imágenes…"; `Enter` no envía hasta que terminen.
- **Enviar**: un mensaje puede tener solo imágenes (sin texto). Al agente van como bloques `image` después del texto (D4). Si con imágenes en el borrador se cambia a una conexión que no acepta imágenes, las miniaturas se quedan, arriba de la caja aparece "«Nombre» no acepta imágenes: quitá las imágenes para enviar" y `Enter` no envía hasta que se quiten.
- **Corregido por la spec 10 §5:** en el mensaje enviado las imágenes van arriba del texto; las tarjetas de comentario, debajo.
- **El mensaje enviado** muestra las imágenes en miniatura debajo del texto; clic en una → **visor**: la imagen centrada sobre un fondo oscuro que cubre la ventana, a su tamaño real si entra o ajustada a la ventana si no, con una línea abajo "nombre · 1920 × 1080 · 1,2 MB"; `Esc`, clic fuera de la imagen o la `×` lo cierran y el foco vuelve a donde estaba.
- **Al reabrir** una conversación del historial, las imágenes se ven desde su carpeta. Si un archivo falta, en su lugar dice "Imagen no disponible".
- Si un agente devuelve imágenes al reproducir una conversación (`session/load`, `user_message_chunk` con `image`), se muestran en miniatura desde sus datos (no se guardan aparte).
- Arrastrar una imagen **desde el árbol de Cincel** sigue insertando la mención `@archivo`, como hoy (ver dudas, §10).

### 5.2 Aspecto (`02-visual.md` §7; todo escalado con `ChatSettings::px`)
- Botón "Adjuntar": `IconName::Paperclip`, `Button::small().ghost()`, abajo a la izquierda dentro de la caja (alineado con el botón de enviar a la derecha), tooltip "Adjuntar imagen".
- Fila de miniaturas: dentro de la caja, encima del texto (debajo de la fila de etiquetas de comentarios, §6.6), con `gap_1p5`, que se parte en varias líneas si no entran. Miniatura 56 × 56 px, recorte centrado (`ObjectFit::Cover`), radio 6, borde 1 px `border`; `×` de 16 px en un círculo `bg.elevated` en la esquina superior derecha, tooltip "Quitar imagen". Tooltip de la miniatura: "nombre · ancho × alto · tamaño".
- **Corregido por la spec 10 §5:** en el mensaje enviado las imágenes van arriba del texto; las tarjetas de comentario, debajo.
- Mensaje enviado: miniaturas de 96 px de alto como máximo y ancho proporcional (máx. 160 px), radio 6, `gap_1p5`, debajo del texto dentro de la burbuja; cursor de mano.
- Visor: fondo `bg.app` al 85 %; imagen centrada con margen de 32 px; línea de leyenda 12 px `text.muted`; `×` arriba a la derecha (`IconName::X`).
- La caja del chat crece hacia arriba con las filas nuevas; nada de esto toca el editor.

### 5.3 Diseño técnico

#### 5.3.1 Procesado (`cincel-chat/src/attachments.rs`, sin GPUI)
```rust
pub const MAX_IMAGE_SIDE: u32 = 2_000;
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_IMAGES_PER_MESSAGE: usize = 10;
pub const JPEG_QUALITY: u8 = 85;

pub struct PreparedImage {
    pub name: String,          // file name, or "Imagen pegada"
    pub mime_type: &'static str, // image/png | image/jpeg | image/gif | image/webp
    pub bytes: Arc<[u8]>,
    pub sha256: String,
    pub width: u32, pub height: u32,
    pub resized: bool,
    pub animation_dropped: bool,
}
pub enum ImageError { NotAnImage, TooLarge { bytes: usize }, Unreadable(String) }
pub fn prepare_image(bytes: &[u8], name: &str) -> Result<PreparedImage, ImageError>;
```
- Corre en `cx.background_executor()`; el panel guarda `PendingAttachment { id, state: Processing | Ready(PreparedImage) }` en `ChatPanel.attachments`.
- `Cargo.toml` raíz, `[workspace.dependencies]`: `image = { version = "0.25", default-features = false, features = ["png", "jpeg", "gif", "webp"] }`, `base64 = "0.22"`. `cincel-chat` usa `image`; `cincel-acp`, `base64`. Si `Cargo.lock` sumara un crate nuevo, se informa y debe pasar `cargo deny`.

#### 5.3.2 Entrada en el panel (`panel.rs`, `render.rs`)
- `Ctrl+V`: la caja (`chat-input-box`) toma `cincel_editor::Paste` en fase de captura (como ya hace con `InsertNewline`): lee `cx.read_from_clipboard()`; con `Image`/`ExternalPaths` → `attach_images` y `stop_propagation`; si no, deja pasar.
- Arrastre: la caja lleva `.drag_over::<ExternalPaths>(|style, _, _| style.border_color(theme.border_focus))` y `.on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.attach_paths(paths.paths(), window, cx)))`.
- Botón: emite `ChatEvent::PickImages`; el workspace abre `rfd::AsyncFileDialog::new().set_title("Adjuntar imagen").add_filter("Imágenes", &["png","jpg","jpeg","gif","webp"]).pick_files()` (como "Abrir carpeta", `workspace.rs`) y llama `chat.attach_paths(paths, …)`. Acción `chat::attach_image` (sin atajo por defecto) hace lo mismo que el botón.
- Capacidad: `ChatPanel::set_image_support(Option<bool>)` (lo llama el workspace en `Connected` y al cambiar de conexión; `None` = sin conexión). `attach_*` comprueba antes de leer nada.
- Avisos: el panel emite `ChatEvent::Notify { level, text }` y el workspace los muestra como aviso emergente (`toast`).
- Envío (`send`): bloques = texto/menciones (como hoy) + `PromptBlock::Image` por cada adjunto listo. `UserMessage.blocks` suma `MessageBlock::Image(ImageRef)`:
  ```rust
  pub struct ImageRef {
      pub file: String,            // "<sha256>.<ext>"
      pub name: String,
      pub mime_type: String,
      pub width: u32, pub height: u32, pub bytes_len: u64,
      #[serde(skip)] pub data: Option<Arc<[u8]>>, // in memory until saved
  }
  ```
- Pintado: miniaturas con `img(Arc<gpui::Image>)` desde `data` o, si no está, desde la ruta del archivo (la arma el workspace: `ChatPanel::set_conversation_dir(Option<PathBuf>)`). Clic → `ChatEvent::OpenImage { source: ImageSource }` (ruta o bytes, más nombre y tamaño).
- `block_text` (`ContentBlock::Image`) deja de devolver "«imagen»" para las repeticiones de `session/load`: se agrega como `MessageBlock::Image` con `data` (sin archivo).

#### 5.3.3 ACP (`cincel-acp/src/protocol.rs`)
- `PromptBlock::Image { mime_type: String, data: Arc<[u8]> }`; `into_wire` → `ContentBlock::Image(ImageContent::new(BASE64_STANDARD.encode(&data), mime_type))`, sin `uri` (D4).
- `pub fn agent_supports_images(capabilities: &AgentCapabilities) -> bool`.
- **Capacidades encontradas en las versiones instaladas** (leídas en la máquina de referencia el 2026-09-30, sin copiar nada al repo):

  | Adaptador | Versión | `promptCapabilities.image` | Cómo usa el bloque |
  |---|---|---|---|
  | `claude-agent-acp` | 0.84.0 | `true` (también `embeddedContext`) | `data` base64 → imagen de la API de Anthropic; sin `data` usa `uri` solo si empieza con `http` |
  | `codex-acp` | 1.13.1 (caché de `npx`; Cincel todavía no lo tiene descargado en su carpeta de agentes) | `true` | `uri` si es `http(s):`/`data:`, si no arma `data:<mime>;base64,<data>`; si el modelo elegido no acepta imágenes responde el error "The current model does not support image input" |
  | `antigravity-acp` (`agy_acp_server`) | 1.2.1 | `true` (también `audio` y `embedded_context`) | — (binario empaquetado; no se inspeccionó más) |

  E7-D vuelve a comprobarlo contra las versiones que haya instaladas al construir y lo anota en `docs/etapas/etapa-7.md`. Un error del agente por una imagen se muestra como cualquier error del turno (aviso en el chat), no se traga.
- Agente falso (`tests/fake_agent/main.rs`): anuncia `prompt_capabilities.image = true` salvo con `FAKE_NO_IMAGES`; con `FAKE_PROMPT_LOG=<ruta>` agrega una línea JSON por cada `session/prompt` con el arreglo `prompt` completo tal como llegó (tipo, texto, `mimeType`, largo de `data` y SHA-256 de los datos decodificados, en lugar de los datos); nuevo guion `"echo-images"` → responde "N imágenes: mime1 WxH, …" decodificando los datos.

#### 5.3.4 Almacenamiento (`cincel-workspace/src/conversations.rs`)
- `Conversations::image_dir(id) -> PathBuf` = `<dir>/<id>/images/`. Al guardar una conversación (`save`), por cada `MessageBlock::Image` con `data` y sin archivo en disco: escritura atómica (`.tmp` + `rename`) de `<sha256>.<ext>`; luego `data` puede soltarse (se conserva en memoria mientras la conversación esté abierta).
- `delete`, `delete_for_connection` y `delete_for_connection_everywhere` borran también `<dir>/<id>/`.
- `CONVERSATION_VERSION = 2`; una conversación versión 1 se lee igual (los bloques nuevos son variantes adicionales).

#### 5.3.5 Visor (`cincel-workspace/src/image_viewer.rs`)
- `ImageViewer { source, name, size, previous_focus }` como capa flotante del `Workspace` (misma familia que el buscador `Ctrl+P` y los diálogos), contexto de teclas `ImageViewer`, acción `workspace::close_image_viewer` con `escape` en ese contexto (sección nueva en `DEFAULT_KEYMAP_JSONC`). Tamaño: `min(1, (ventana − 64 px) / imagen)` por eje.

### 5.4 Criterios de aceptación
- [ ] `Ctrl+V` con una captura PNG en el portapapeles agrega una miniatura y no pega texto; con texto pega texto como hoy.
- [ ] Soltar dos archivos PNG y uno `.txt` sobre la caja: dos miniaturas y el aviso del `.txt`.
- [ ] El botón "Adjuntar" abre el diálogo (lista manual) y los archivos elegidos se adjuntan (test por la función que llama).
- [ ] Una imagen de 4 000 × 3 000 llega al agente de 2 000 × 1 500; una de 800 × 600 llega con los mismos bytes del archivo.
- [ ] Una imagen que tras reducir pesa más de 10 MB no se adjunta y avisa con su peso; la undécima imagen avisa y no se adjunta.
- [ ] La `×` de una miniatura la quita y no viaja.
- [ ] Con el agente falso con `FAKE_NO_IMAGES`, las tres vías avisan "«Nombre» no acepta imágenes" y no adjuntan; sin conexión avisan "Conectá un agente para adjuntar imágenes".
- [ ] El agente recibe un bloque `image` por imagen, con `mimeType` correcto, `data` en base64 que decodifica a los bytes adjuntos y sin `uri`, después del texto.
- [ ] El mensaje enviado muestra las miniaturas; un clic abre el visor; `Esc` lo cierra y el foco vuelve a la caja.
- [ ] Cerrar y reabrir la conversación desde el historial muestra las mismas imágenes, leídas de `conversations/<id>/images/`; el JSON no contiene base64; borrar la conversación borra la carpeta.
- [ ] Cambiar a una conexión sin imágenes con imágenes en el borrador bloquea el envío con el aviso de §5.1.

### 5.5 Tests
- Unitarios `cincel-chat/src/attachments.rs` (con imágenes generadas en el test con `image`, nada binario en el repo): los cuatro formatos; detección por contenido con extensión equivocada; reducción con la relación de aspecto; "sin recodificar" cuando entra; JPEG calidad; WebP con y sin transparencia; GIF animado → primer cuadro PNG con `animation_dropped`; tope de 10 MB (imagen de ruido grande); archivo truncado → `Unreadable`.
- `cincel-chat/src/image_attach_tests.rs` (`VisualTestContext`): `cx.write_to_clipboard(ClipboardItem::new_image(...))` + `ctrl-v` → miniatura; portapapeles con texto → texto; `cx.simulate_event(FileDropEvent::Entered{..})` + `Submit` sobre la caja → adjuntos y aviso; `×` con clic real; tope de 10; capacidad `None`/`false` con sus avisos; `Enter` bloqueado mientras procesa; `send` arma `PromptBlock::Image` en orden después del texto; reabrir con `set_conversation_dir` pinta desde archivo y "Imagen no disponible" si falta.
- `cincel-acp`: unitario de `into_wire` (base64, sin `uri`) y de `agent_supports_images`; `tests/integration.rs` suma "echo-images" (dos imágenes → la respuesta las describe) y `FAKE_NO_IMAGES` (capacidad falsa en `Connected`).
- `cincel-workspace/src/image_e2e_tests.rs` (agente falso, `ConnectionsFixture`, `FAKE_PROMPT_LOG`): adjuntar por la función del botón, enviar, el registro tiene los bloques `image` con el SHA-256 correcto; la conversación guardada referencia los archivos y estos existen; borrarla los borra; el visor abre y cierra con `Esc`.
- `cincel-workspace/src/conversations.rs`: lectura de una conversación versión 1.

---

## 6. Comentarios sobre segmentos y selecciones (C)

### 6.1 Regla general (fijada por el autor)
Al agente viaja **solo** lo que hoy ya viaja (el parche de los rechazos y de las ediciones a mano, una sola vez) **más** los comentarios que el usuario escriba, una sola vez. Nunca lo aceptado ni lo pendiente por sí mismos. Nada se acumula entre turnos.

### 6.2 Comportamiento
1. **Crear desde un segmento**: el tercer botón **"Comentar"** de los botones del segmento abre la caja debajo de las líneas del segmento (debajo de su última fila; en un segmento que solo quita líneas, debajo de sus filas rojas). Si el segmento ya tiene un comentario creado desde su botón, lo abre para editar. "Comentar" funciona también mientras el agente escribe (no es una decisión).
2. **Crear desde una selección**: clic derecho → "Comentar selección" (con texto seleccionado) o "Comentar línea" (sin selección), o `Ctrl+Shift+M`. Rango = filas de la selección (si termina en la columna 0 de una fila, esa fila no cuenta); sin selección, la fila del cursor; si el cursor está dentro de un segmento pendiente y no hay selección, `Ctrl+Shift+M` equivale a "Comentar" de ese segmento. Sirve sobre cualquier rango, lo haya tocado el agente o no. Una selección sobre filas rojas se ajusta a la fila real más cercana.
3. **La caja**: título "Comentario para el agente · calc.py:24-28", campo de texto (varias líneas; `Enter` es salto de línea), ayuda "Ctrl+Enter guarda · Esc cancela", botones "Cancelar" y "Guardar". `Ctrl+Enter` (o "Guardar") guarda; `Esc` (o "Cancelar") descarta lo escrito (si era una edición, el comentario queda como estaba). Guardar vacío = cancelar (si era una edición, borra el comentario). Máximo 8 000 caracteres (pasados los 7 000, la ayuda muestra cuántos quedan).
4. **Guardado**: queda una **marca en el margen** en la primera fila del rango y la caja se muestra **plegada**: una fila con el icono y el texto en una línea (cortado con "…"). Clic en el texto o en la marca del margen la despliega (texto completo, solo lectura) y la vuelve a plegar. En la caja plegada o desplegada, "Editar" y "Borrar" (visibles al pasar el mouse y siempre con el cursor de teclado en el rango).
5. **Independiente de la decisión** (C.2): comentar no acepta ni rechaza nada; el segmento se puede aceptar, rechazar, decidir por línea o dejar. Comentar un segmento no cambia los demás.
6. **Anclaje** (C.3): el comentario sigue sus líneas cuando se escribe arriba, abajo o adentro; si sus líneas desaparecen (por ejemplo, se borran o un rechazo las reemplaza), queda sobre la línea más cercana.
7. **En la caja del chat** (C.4): una etiqueta por comentario sin enviar ("calc.py:24-28", ordenadas por archivo y línea) con su `×`; la `×` lo borra (también del margen); "Borrar" en el margen lo quita de la caja. Clic en la etiqueta abre el archivo en esa línea. **Se envían con el próximo mensaje** (texto o imagen) que el usuario mande; no hay botón aparte. Un mensaje vacío no se envía aunque haya comentarios.
8. **Al enviar** (C.5–C.7): el agente recibe los comentarios en el bloque de §6.8; el mensaje del chat muestra una tarjeta por comentario; los comentarios desaparecen del margen, de la caja y del store, y no se repiten.
9. **Contadores** (C.9): barra de estado "3 cambios pendientes · 2 comentarios" (o "2 comentarios" solos; "1 comentario"), mismo botón que hoy; árbol: en la fila de cada archivo con comentarios, el icono y la cantidad después de `+N −M`; en la raíz, "· 2 comentarios" después de los totales.
10. **Persistencia** (C.9, C.8 l): los comentarios sin enviar se guardan con la revisión y vuelven al abrir la carpeta.
11. **Binarios** (C.8 m): no tienen editor de texto (la pestaña dice "Archivo binario: no se muestra"), así que no tienen botón, menú ni atajo de comentario. Si un archivo comentado se vuelve binario durante un turno, el comentario se descarta con el aviso "El comentario sobre «x» se descartó: el archivo ya no es de texto".
12. **Archivos que desaparecen**: borrado por el agente → el comentario queda en la pestaña de solo lectura del archivo borrado, sobre su segmento; borrado por otro programa fuera de un turno, o ausente al abrir → se descarta con "El comentario sobre «x» se descartó: el archivo ya no existe".

### 6.3 Aspecto (`02-visual.md` §5 y §6; tamaños escalados por `ui_scale`; D16: el texto nunca se corre a los costados ni cambia el margen)
- **Botones del segmento** (`02-visual.md` §6.1): `✓ Aceptar` · `✗ Rechazar` · `Comentar` (`IconName::MessageSquarePlus`, 12 px, en `text.accent`; la palabra en `text`). `PILL_WIDTH` 190 → 280 px (tres partes iguales); compacto: tres botones de 24 × 24 con 4 px entre ellos (80 px), el tercero con el icono. Con el agente escribiendo, Aceptar/Rechazar en `text.muted` y el indicador; "Comentar" sigue habilitado. La regla de dónde se ponen no cambia (con 280 px, más segmentos usarán la versión compacta: aceptado).
- **Caja abierta** (fila de bloque): desde el borde izquierdo del área de texto hasta 12 px antes de la barra de desplazamiento; `bg.surface`, borde 1 px `border.focus`, radio 6, relleno 8 px; título 11 px `text.muted`; campo de 2 a 8 filas (crece; después desplaza), fuente de la interfaz 13 px; pie con la ayuda 11 px `text.muted` y botones `xsmall` ("Cancelar" fantasma, "Guardar" primario).
- **Caja plegada**: una fila de texto de alto más 4 px arriba y abajo (en filas enteras, D10); `bg.surface`, borde 1 px `border`, radio 6; `IconName::MessageSquareText` 12 px `text.accent` + texto 13 px `text` en una línea; "Editar" y "Borrar" a la derecha, `xsmall` fantasma.
- **Marca del margen**: `IconName::MessageSquareText` de 10 px en `text.accent`, centrada en el hueco de 12 px entre los números y el texto (`GUTTER_TEXT_GAP`), en la primera fila del rango; globo de ayuda con el texto (primeros 200 caracteres). No cambia el ancho del margen ni toca las columnas de git y de cambios del agente.
- **Etiqueta en la caja del chat**: mismo aspecto que la etiqueta de archivo del mensaje enviado (`bg.elevated`, `TEXT_SMALL`, `text.accent`, radio `CARD_RADIUS`) con `IconName::MessageSquareText` 11 px delante y una `×` de 12 px detrás (tooltip "Quitar comentario"); tooltip de la etiqueta: "src/calc.py, líneas 24 a 28" y debajo el texto (200 caracteres). Fila propia dentro de la caja, encima de las miniaturas y del texto.
- **Tarjeta en el mensaje enviado**: dentro de la burbuja, debajo del texto y de las imágenes, ancho completo de la burbuja; `bg.surface`, borde 1 px `border`, radio `CARD_RADIUS`, relleno 8 px; encabezado 11 px: icono + "calc.py" (`text`) + " · líneas 24-28 · rechazado" (`text.muted`), con cursor de mano (abre el archivo en la línea); fragmento en monoespaciada 12,5 px sobre `bg.editor`, 6 líneas como máximo y "… N líneas más" (clic despliega); texto del comentario 13 px.

### 6.4 Modelo (`crates/cincel-review/src/comments.rs`, parte de `ReviewStore`)
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommentId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentState { Pending, Accepted, Rejected, Mixed, NoAgentChange }

/// What the UI shows of an unsent comment.
pub struct CommentView {
    pub id: CommentId, pub path: PathBuf, pub display_path: String,
    pub rows: Range<u32>,          // buffer rows, end exclusive; empty = before `rows.start`
    pub text: String,
    pub from_hunk: Option<HunkId>, // created from that hunk's "Comentar"
}

/// One comment as it left with a prompt (also what the chat card shows).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentComment {
    pub path: PathBuf, pub display_path: String,
    pub first_line: u32, pub last_line: u32,   // 1-based, inclusive (equal for one line)
    pub kind: SentRange,                       // Lines | RemovedBefore | DeletedFile
    pub state: CommentState,
    pub code: Option<String>,                  // current text of those lines
    pub removed: Option<String>,               // base lines of a pending pure deletion / deleted file
    pub truncated_lines: u32,
    pub unsaved: bool,                         // the buffer differs from disk
    pub lang: Option<String>,
    pub text: String,
}

impl ReviewStore {
    pub fn add_comment(&mut self, path: &Path, snapshot: &BufferSnapshot, rows: Range<u32>,
                       from_hunk: Option<HunkId>, text: String) -> CommentId;
    pub fn edit_comment(&mut self, id: CommentId, text: String) -> bool;
    pub fn remove_comment(&mut self, id: CommentId) -> bool;
    pub fn comments(&self, snapshot_of: impl Fn(&Path) -> Option<BufferSnapshot>) -> Vec<CommentView>; // by path, then row
    pub fn comment_count(&self) -> usize;
    pub fn comment_count_in(&self, path: &Path) -> usize;
    pub fn comment_buffer_event(&mut self, path: &Path, event: &BufferEvent);
    pub fn comment_state(&self, id: CommentId) -> CommentState;
    pub fn take_comments_for_prompt(&mut self, snapshot_of: impl Fn(&Path) -> Option<(BufferSnapshot, bool /*dirty*/)>) -> Vec<SentComment>;
    pub fn restore_comments(&mut self, sent: &[SentComment], snapshot_of: …);
    pub fn drop_comments_in(&mut self, path: &Path) -> usize;
}
pub fn format_feedback(report: Option<&str>, comments: &[SentComment]) -> Option<String>; // §6.8
```
- **Anclas**: por comentario, inicio = `anchor_before(inicio de la fila start)` y fin = `anchor_after(fin de la fila end−1)`, en un `AnchorMap<(CommentId, End)>` por ruta, transformado con `AnchorMap::apply_event` para cada `BufferEvent::Edited` (de cualquier origen: usuario, recarga del disco por el agente, rechazo, deshacer). Si el rango queda vacío, el comentario queda en la fila del ancla de inicio (C.3).
- **Marcas de decisión**: cada `accept_hunk`, `reject_hunk`, `accept_line`, `reject_line`, `accept_file`, `reject_file`, `accept_turn`, `reject_turn`, `accept_all`, `reject_all` marca `accepted`/`rejected` en los comentarios del mismo archivo cuyo rango corta las filas decididas (para una eliminación pura: la fila donde se insertan las filas rojas; para un archivo borrado: todo). Las marcas de un rechazo se guardan en su `RejectUndo`; `undo_last_reject` las quita.
- **Estado al enviar**: si alguna fila del rango corta un segmento todavía pendiente → `Pending`; si no: solo `accepted` → `Accepted`; solo `rejected` → `Rejected`; ambas → `Mixed`; ninguna → `NoAgentChange`.
- **Fragmento**: texto actual de las filas del buffer (si el buffer tiene cambios sin guardar, `unsaved = true`); tope `COMMENT_SNIPPET_MAX_LINES = 120` y `COMMENT_SNIPPET_MAX_BYTES = 12 * 1024`, el resto se cuenta en `truncated_lines`. `lang` = extensión en minúsculas si es `[a-z0-9+#-]{1,10}`.
- `take_comments_for_prompt` devuelve todos los comentarios ordenados (ruta relativa por bytes, luego primera fila) y los saca del store. `restore_comments` los vuelve a poner (D16) en sus filas.
- Persistencia (D15): `Entry` de `state.json` sin cambios; `StateFile` suma `#[serde(default)] comments: Vec<CommentEntry>` y `STATE_VERSION = 2`:
  ```json
  { "id": 7, "path": "/…/src/calc.py", "start_row": 23, "end_row": 28,
    "text": "…", "created_at": 1790000000, "from_hunk": true,
    "file_hash": "<sha256 del texto del buffer al guardar>",
    "snippet_hash": "<sha256 del texto de esas filas, en objects/>",
    "accepted": false, "rejected": true }
  ```
  `load`: si el archivo no existe → descarte con aviso; si `file_hash` coincide con el disco → mismas filas; si no → busca el fragmento exacto (la aparición más cercana a `start_row`); si no aparece → una sola fila, `start_row` acotada al largo del archivo (C.3). `LoadReport` suma `comments_restored`, `comments_relocated`, `comments_dropped: Vec<(PathBuf, CommentDropReason)>`. `save` referencia los `snippet_hash` (no se borran en la limpieza de objetos). Un `state.json` versión 1 carga sin comentarios; uno versión 2 leído por la 0.1.0 conserva los archivos y pierde los comentarios al guardar (aceptado: no hay vuelta atrás de versión).

### 6.5 Editor (`crates/cincel-editor`)
1. **`BlockMap`** (`block_map.rs`, D10): `pub struct BlockPlacement { pub id: u64, pub after_wrap_row: u32, pub rows: u32 }`; `BlockMap::new(Vec<BlockPlacement>)`, `to_visual_row(wrap_row) -> u32`, `from_visual_row(visual_row) -> VisualCell { Wrap(u32) | Block { id, row_in_block } }`, `total_rows()`. El elemento usa filas visuales para `y = fila × alto_de_línea`, para el desplazamiento y para el alto total; el movimiento del cursor, la selección, el arrastre del mouse y la búsqueda siguen en filas de ajuste y saltan los bloques. Un bloque con `after_wrap_row` antes de la primera fila visible que aparece, desaparece o cambia de alto suma o resta `rows × alto_de_línea` a `scroll_top`. Los botones del segmento y la barra flotante siguen siendo capas (no son bloques).
2. **Datos del anfitrión**: `ReviewView` suma `comments: Vec<ReviewCommentView { id: u64, rows: Range<u32>, text: String, from_hunk: Option<u64> }>` (filas del buffer como `buffer_rows`). Archivos sin segmentos pero con comentarios también reciben `set_review`.
3. **Caja**: el `EditorView` crea a pedido un `EditorView` hijo (`EditorChrome::Minimal`, ajuste de línea, `AutoHeight { min_rows: 2, max_rows: 8 }`) para la caja abierta (una sola abierta por vista); su `key_context` suma `comment_box`. El elemento hace `prepaint`/`paint` de los bloques visibles como hijos (`AnyElement` posicionado en la franja de filas del bloque). Estado local: `CommentDraft { target: DraftTarget::New { rows, from_hunk } | Edit { id }, editor }`; plegado/desplegado por comentario en `HashMap<u64, bool>`.
4. **Eventos**: `EditorEvent::Comment(CommentAction)` con `Create { rows: Range<u32>, from_hunk: Option<u64>, text: String }`, `Edit { id: u64, text: String }`, `Delete { id: u64 }`. El editor no guarda nada por su cuenta: el anfitrión aplica y devuelve el `ReviewView` nuevo.
5. **Acciones** (`actions.rs`): `editor::comment_selection` (`CommentSelection`), `editor::save_comment` (`SaveComment`), `editor::cancel_comment` (`CancelComment`), `review::comment_hunk` (`CommentHunk`, el botón; sin atajo por defecto). Bindings por defecto en `cincel_editor::default_key_bindings` **y** en `DEFAULT_KEYMAP_JSONC`: `ctrl-shift-m` en `Editor`; sección nueva `Editor && comment_box` con `ctrl-enter` → `editor::save_comment` y `escape` → `editor::cancel_comment`, ubicada **después** de todas las secciones de `Editor` (así gana sobre `Editor` y `Editor && review_hunk_under_cursor` en la caja, que es el nodo más profundo; misma regla que D14 de la spec 08). `comment_selection` no hace nada en `EditorChrome::Minimal` (el compositor del chat) ni en un binario.
6. **Menú contextual** (D12): `.context_menu(...)` de gpui-kit sobre la raíz del `EditorView` con `EditorChrome::Full`: "Cortar" (`editor::cut`), "Copiar" (`editor::copy`), "Pegar" (`editor::paste`), separador, "Comentar selección"/"Comentar línea" (`editor::comment_selection`), con sus atajos mostrados (contexto de acción = foco del editor). Un clic derecho fuera de la selección mueve el cursor ahí antes de abrir el menú; dentro de la selección, la conserva. En la pestaña de solo lectura de un archivo borrado: solo "Copiar" y "Comentar selección" (comenta su único segmento).
7. **Botones del segmento**: tercera parte "Comentar" → abre la caja del segmento (§6.2.1). `ReviewAction::is_decision` no cambia (comentar no es una decisión).
8. **Marca del margen**: pintada en `element.rs` en la capa de los iconos `+`/`−` (§6.3); clic → plegar/desplegar; tooltip.

### 6.6 Chat (`crates/cincel-chat`)
- Tipos (sin depender de `cincel-review`; el workspace los traduce): `PendingComment { id: u64, label: String, tooltip: String, path: PathBuf, line: u32 }` y `SentCommentCard { display_path, path, first_line, last_line, kind, state_label: String, code: Option<String>, removed: Option<String>, truncated_lines: u32, lang: Option<String>, text: String, not_sent: bool }`.
- `ChatPanel::set_pending_comments(Vec<PendingComment>, cx)`; la `×` emite `ChatEvent::RemoveComment { id }`; clic en la etiqueta emite `ChatEvent::OpenLocation { path, line }`.
- `ChatPanel::attach_sent_comments(cards: Vec<SentCommentCard>, cx)`: los agrega al último `UserMessage` como `MessageBlock::Comment(SentCommentCard)` (persistidos con la conversación); `mark_comments_not_sent(cx)` pone `not_sent = true` en los de ese mensaje (D16).
- Etiqueta (`label`): `calc.py:24-28`, `calc.py:24` (una línea o eliminación pura), `old.py (borrado)`; si dos comentarios pendientes tienen el mismo nombre de archivo en carpetas distintas, se usa la ruta relativa (como las menciones).
- Textos del estado en las tarjetas: "pendiente", "aceptado", "rechazado", "mixto", "sin cambios del agente".

### 6.7 Workspace (`crates/cincel-workspace`)
- `review_comments.rs` (hijo de `review.rs`, como `review_snapshot.rs`): `Review::add_comment/edit_comment/remove_comment` (desde `EditorEvent::Comment` y `ChatEvent::RemoveComment`), cada uno seguido de `refresh` (empuja `ReviewView.comments` a los editores abiertos), `push_pending_comments` al chat, contadores y `schedule_persist`. Un archivo con comentarios queda vigilado (`ensure_buffer`) aunque no esté en revisión; `on_buffers_edited` pasa cada evento también a `store.comment_buffer_event`.
- **Envío**: `Review::begin_prompt` devuelve `PromptFeedback { turn: u64, feedback: Option<String>, sent: Vec<SentComment> }`: primero `flush`, después `take_comments_for_prompt`, después los parches de siempre (`report_for_agent` + `forget_turn`), y `cincel_review::format_feedback(report, &sent)`. `Agents::prepare_prompt` usa `feedback` como hasta hoy (lo envuelve `cincel-acp`) y llama `chat.attach_sent_comments`. Si el prompt diferido se descarta sin salir (`Cancel` durante la espera de la foto, o `connection` vacía al llegar la foto) → `review.restore_comments(sent)` + `chat.mark_comments_not_sent` (D16).
- **Cambio de conexión** (C.8 k): los comentarios son de la ventana/proyecto, no de la conexión; no se tocan al cambiar de conexión ni al empezar una conversación nueva.
- **Contadores**: `SharedSummary` suma `comments: usize` y `comments_per_file: BTreeMap<PathBuf, usize>`; barra de estado (`count_label` + " · N comentarios") y árbol (§6.2.9). El botón de la barra de estado sigue abriendo el panel de revisión.
- **Diálogo de cierre** (D17): si `comments > 0`, debajo de "Hay N cambios de agente sin decidir en M archivos" una línea 12 px `text.muted`: "También hay 2 comentarios sin enviar: se guardan y vuelven al abrir la carpeta." / "1 comentario sin enviar…". Botones y comportamiento iguales.
- **Persistencia**: la misma de la revisión (`save_now`, `load`); avisos de `comments_dropped` con `toast::warn`.
- **Binario**: cuando `adopt_change` deja pasar un archivo que pasó a binario y tenía comentarios → `drop_comments_in` + aviso (§6.2.11).

### 6.8 Formato exacto de lo que recibe el agente
El bloque sigue siendo uno solo, `<user_review_feedback>…</user_review_feedback>` (`wrap_review_feedback`), primer bloque de texto del prompt. `format_feedback` arma su interior:
1. Si hay parches: exactamente el texto de hoy de `report_for_agent` (`REPORT_HEADER`, parches, sección de formateo).
2. Si hay comentarios: una línea en blanco (solo si hubo parches) y la sección de comentarios.
3. Si no hay ni parches ni comentarios: `None` (no hay bloque).

Sección de comentarios (texto plano; `\n` como fin de línea; sin espacios al final de línea; una línea en blanco entre comentarios):
```text
The user left {N} comment(s) on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn.

Comment {i} of {N}
File: {ruta relativa}[ (you deleted this file)]
Lines: {a}-{b}          ← "Line: {a}" si a == b; "Lines: removed before line {a}" en una eliminación pura
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
{texto del usuario, tal cual, con sus saltos de línea}
```
- `{N}` y "comment"/"comments" según la cantidad ("The user left 1 comment on the code.").
- `{estado}`: `your change, not reviewed yet` · `your change, accepted by the user` · `your change, rejected by the user` · `your change, partly accepted and partly rejected by the user, line by line` · `no change of yours (the user selected these lines)`.
- "Current code" se omite en una eliminación pura pendiente y en un archivo borrado (no hay código actual); "Lines you removed" aparece solo en esos dos casos y mientras estén pendientes.
- `{valla}` = tantos acentos graves como el tramo más largo de acentos graves del código + 1, mínimo 3. `{lang}` = `lang` o nada.
- Rutas relativas a la raíz del proyecto con `/`. Orden: ruta (por bytes), luego primera línea.

Ejemplo completo (rechazo con nota más una selección sin cambios):
````text
<user_review_feedback>
The user made the following updates to your changes:

--- a/src/calc.py
+++ b/src/calc.py
@@ -24,3 +24,3 @@
-def total(items, discount=0):
-    return sum(items) * (1 - discount)
+def total(items):
+    return sum(items)

The user left 2 comments on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn.

Comment 1 of 2
File: src/calc.py
Lines: 24-25
Review state: your change, rejected by the user
Current code:
```py
def total(items):
    return sum(items)
```
Comment:
No cambies la firma: agregá el descuento como una función aparte.

Comment 2 of 2
File: src/util.py
Line: 7
Review state: no change of yours (the user selected these lines)
Current code:
```py
TIMEOUT = 3
```
Comment:
Esto debería salir de la configuración.
</user_review_feedback>
````

### 6.9 Escenarios acordados (cada uno con criterio y test)
Todos en `crates/cincel-workspace/src/comments_e2e_tests.rs` salvo indicación, con el agente falso (`FAKE_SHELL_EDITS` para que el agente cambie archivos, `FAKE_PROMPT_LOG` para leer exactamente lo que recibió), `ConnectionsFixture` y comentarios creados por `EditorEvent::Comment` desde la vista real (y, en los marcados, por clic real en "Comentar").

| | Escenario | Criterio | Test |
|---|---|---|---|
| a | Comentar un segmento pendiente sin decidir y enviar | El bloque tiene solo la sección de comentarios (sin parche), estado `not reviewed yet`, el código del agente y la nota. El segundo turno del agente reescribe esas líneas: el segmento sigue contra la **base original** (sus filas rojas son las de antes del primer turno) y los otros segmentos pendientes del archivo quedan idénticos (mismas filas rojas y verdes). | `comment_on_pending_hunk_reaches_agent_and_its_fix_updates_the_same_hunk` (clic real en "Comentar") |
| b | Comentar y rechazar | Viaja el parche del rechazo y la nota con estado `rejected` y el código restaurado. | `comment_and_reject_sends_patch_and_note` |
| c | Comentar y aceptar | Viaja solo la nota (sin `REPORT_HEADER`), estado `accepted`. | `comment_and_accept_sends_only_the_note` |
| d | Comentar y editar a mano las líneas | Viaja el parche de la edición y la nota con el código **con la edición**; estado `not reviewed yet`; si no se guardó, la variante "as shown in the editor, not saved yet". | `comment_and_manual_edit_sends_patch_and_current_code` |
| e | Comentar una selección sin cambio del agente | Estado `no change of yours…`; ningún parche. | `comment_on_untouched_selection` (vía `editor::comment_selection` con `Ctrl+Shift+M`) |
| f | Varios comentarios en varios archivos | Orden por ruta y línea; "Comment i of N" correlativos; etiquetas en la caja en el mismo orden. | `several_comments_are_sorted_by_file_and_line` |
| g | Comentar y decidir después, antes de enviar (incluye "Aceptar todo" y "Rechazar todo") | El estado sale de la última decisión: `accepted` tras "Aceptar todo"; `rejected` (y su parche) tras "Rechazar todo" confirmado; el comentario sigue en el margen tras decidir. | `decide_after_commenting_including_accept_all_and_reject_all` |
| h | Borrar un comentario antes de enviar | Con la `×` de la etiqueta o "Borrar" del margen: no viaja nada de él; si no había nada más, el prompt no tiene bloque `<user_review_feedback>`. | `deleted_comment_sends_nothing` (las dos vías, clic real) |
| i | Decidir por línea un segmento comentado | Con líneas pendientes en el rango: el comentario sigue sobre ellas y viaja `not reviewed yet`; con todas decididas: `accepted`, `rejected` o `partly accepted and partly rejected…` según corresponda. | `line_decisions_on_a_commented_hunk` |
| j | Tras enviar | Margen, caja del chat, contadores y `state.json` sin comentarios; el mensaje siguiente no los trae; la tarjeta queda en el mensaje enviado. | `sent_comments_disappear_and_never_repeat` |
| k | Cambiar de conexión con comentarios sin enviar | Siguen pendientes y viajan con el próximo mensaje a la conexión nueva. | `comments_follow_the_next_message_after_switching_connection` |
| l | Cerrar Cincel con comentarios sin enviar | Al reabrir el proyecto vuelven en las mismas filas; con un cambio externo se reubican por el fragmento; el diálogo de cierre (con cambios pendientes) muestra la línea de comentarios y sus botones hacen lo mismo que hoy. | `comments_survive_closing_and_reopening` y `close_dialog_counts_comments_without_changing_behavior` (en `pending_review_close_comments_tests.rs`) |
| m | Binarios | La pestaña de un binario no ofrece "Comentar", ni menú, ni el atajo hace nada; un texto comentado que el agente convierte en binario pierde el comentario con aviso. | `binaries_take_no_comments` |

Además (en el mismo archivo): `cancelled_prompt_gives_comments_back` (D16) y `comment_while_agent_writes`.

### 6.10 Criterios de aceptación generales
- [ ] "Comentar" (y `Ctrl+Shift+M`, y el menú contextual) abre la caja debajo de las líneas; `Ctrl+Enter` guarda, `Esc` cancela; queda la marca en el margen y la caja plegada con el texto.
- [ ] Abrir, plegar o borrar una caja no cambia el ancho del margen ni la posición horizontal del texto, y la fila que está arriba de todo en pantalla no se mueve cuando la caja está más arriba que ella.
- [ ] El cursor con flechas pasa de la fila de arriba a la de abajo de la caja sin pararse en ella.
- [ ] El comentario sigue sus líneas al escribir arriba y adentro; si se borran, queda en la línea más cercana.
- [ ] Etiquetas en la caja del chat con su `×`; tarjetas en el mensaje enviado con archivo, líneas, estado, código y texto.
- [ ] El bloque al agente coincide byte a byte con §6.8 en los escenarios de §6.9.
- [ ] Barra de estado y árbol muestran "N comentarios".
- [ ] Con Claude, Codex y Antigravity reales, cada uno entiende y atiende una nota (lista manual, §11).

### 6.11 Tests
- Unitarios `cincel-review/src/comments_tests.rs`: anclas con ediciones arriba/adentro/abajo y con el rango borrado; marcas de cada decisión (las diez) y el cálculo de estado (incluido mixto por línea); deshacer un rechazo quita sus marcas; `take` ordena y vacía; `restore` devuelve; fragmento con tope y vallas con acentos graves; `format_feedback` (sin nada → `None`; solo parches → idéntico a `report_for_agent`; solo comentarios; ambos; singular/plural; eliminación pura; archivo borrado; sin guardar) con textos esperados literales.
- `cincel-review/tests/comments_persist.rs`: v1 → sin comentarios; ida y vuelta v2; reubicación por fragmento; línea más cercana; archivo ausente → descartado; el objeto del fragmento sobrevive a la limpieza.
- `cincel-editor/src/block_map_tests.rs`: conversiones de filas, saltos del cursor, alto total, compensación del desplazamiento.
- `cincel-editor/src/comment_box_tests.rs` (`VisualTestContext`): abrir desde el botón (clic real en la tercera parte, normal y compacta), desde `Ctrl+Shift+M` y desde el menú contextual; `Ctrl+Enter`/`Esc` en la caja (y que `Ctrl+Enter` **no** acepte el segmento); plegar/desplegar; "Editar"/"Borrar"; la marca del margen; el origen del texto y el ancho del margen no cambian (comparando `debug_bounds` antes y después); nada en `EditorChrome::Minimal`.
- `cincel-chat/src/comment_tags_tests.rs`: etiquetas, `×` (clic real) → `RemoveComment`, clic → `OpenLocation`, tarjetas, `not_sent`, persistencia de `MessageBlock::Comment` en `Conversation`.
- `cincel-workspace/src/comments_e2e_tests.rs` y `pending_review_close_comments_tests.rs` (§6.9); `user_docs_tests.rs` sigue pasando con los atajos nuevos documentados.

---

## 7. Resumen de acciones, contextos y textos

### 7.1 Acciones nuevas
| Acción | Atajo por defecto | Contexto |
|---|---|---|
| `editor::comment_selection` | `ctrl-shift-m` | `Editor` |
| `editor::save_comment` | `ctrl-enter` | `Editor && comment_box` |
| `editor::cancel_comment` | `escape` | `Editor && comment_box` |
| `review::comment_hunk` | — (botón "Comentar") | `Editor` |
| `chat::attach_image` | — (botón "Adjuntar") | `Chat` |
| `workspace::close_image_viewer` | `escape` | `ImageViewer` |

Las cuatro con atajo se agregan a `DEFAULT_KEYMAP_JSONC` (con comentario en español), a `shortcuts_modal::description` y a `docs/usuario/atajos.md` (lo exige `user_docs_tests.rs`). El test `default_keymap_has_every_shortcut_of_the_spec` suma estas filas. No hay claves nuevas en `settings.json`.

### 7.2 Contextos nuevos
`comment_box` (en el `EditorView` de la caja de comentario) e `ImageViewer` (capa del visor).

### 7.3 Textos de interfaz nuevos (español)
Conexiones: (sin textos nuevos). Imágenes: "Adjuntar imagen", "Quitar imagen", "Conectá un agente para adjuntar imágenes", "«Nombre» no acepta imágenes", "«Nombre» no acepta imágenes: quitá las imágenes para enviar", "«nombre» no es una imagen PNG, JPEG, GIF o WebP", "«nombre» pesa X MB aun reducida; el máximo es 10 MB", "«nombre» no se pudo leer: motivo", "Se pueden adjuntar hasta 10 imágenes por mensaje", "La animación de «nombre» se pierde al reducirla", "Preparando 1 imagen…"/"Preparando N imágenes…", "Imagen pegada", "Imagen no disponible", título del diálogo "Adjuntar imagen" y filtro "Imágenes". Comentarios: "Comentar", "Comentar selección", "Comentar línea", "Cortar", "Copiar", "Pegar", "Comentario para el agente", "Escribí qué querés que el agente haga con estas líneas…", "Ctrl+Enter guarda · Esc cancela", "Guardar", "Cancelar", "Editar", "Borrar", "Quitar comentario", "N comentarios"/"1 comentario", "También hay N comentarios sin enviar: se guardan y vuelven al abrir la carpeta.", "El comentario sobre «x» se descartó: el archivo ya no existe", "El comentario sobre «x» se descartó: el archivo ya no es de texto", "No se envió: el comentario volvió al margen", estados "pendiente", "aceptado", "rechazado", "mixto", "sin cambios del agente".

---

## 8. Qué cubre respecto de lo acordado

| Decisión | Cómo la cubre esta spec | Recortes o límites, con motivo |
|---|---|---|
| **A** Lista de conexiones y encabezado: solo nombre, tipo con icono e insignia; correo, plan y "Usado hace…" solo en Configuración | §3 (menú, botón, lista de borrado), D1, D2; Configuración intacta. | La confirmación "Listo: conectado como …" del flujo de conexión sigue mostrando la cuenta (es el aviso al conectar, no una lista; D1). La píldora "listo/pensando…" sigue al lado del botón (D2). |
| **A (agregado)** Menús que se cierran solos (clic fuera, foco a otro lado, `Esc`, ventana sin foco), los dos del encabezado y los demás desplegables | §4: mecanismo (D3), los siete menús del chat, revisión con tests de la barra de título, el árbol, la configuración y el menú nuevo del editor. | Los menús de gpui-kit se arreglan desde Cincel solo si el test muestra el defecto (no se modifica gpui-kit). |
| **B** Imágenes: `Ctrl+V`, arrastrar, botón "Adjuntar" (`rfd`); PNG/JPEG/GIF/WebP; reducción a 2 000 px; 10 MB; miniaturas con `×`; vista a tamaño completo con `Esc`; bloques `image` base64; aviso si el agente no las acepta; guardadas con la conversación | §5 completo; capacidades de los tres adaptadores comprobadas (§5.3.3: los tres anuncian `image: true`); API de GPUI verificada (`ClipboardEntry::Image`, `ExternalPaths`/`FileDropEvent`, `drag_over`/`on_drop`). | Un GIF animado que haya que reducir pierde la animación (se manda el primer cuadro, con aviso: `image` no recodifica GIF animados con calidad razonable). Tope de 10 imágenes por mensaje (D5, no estaba en lo acordado; ver dudas). Arrastrar una imagen desde el árbol de Cincel sigue siendo una mención (ver dudas). El diálogo `rfd` se prueba por la función que llama; abrirlo de verdad va a la lista manual. |
| **C.1** Botón "Comentar" + caja como fila no textual + `Ctrl+Enter`/`Esc` + marca y caja plegada; "Comentar selección" en menú contextual y con atajo | §6.2.1–4, §6.5 (`BlockMap`, D10; caja, D11; menú contextual nuevo, D12; `Ctrl+Shift+M` verificado libre). | El editor no tenía menú contextual: se crea con lo mínimo (cortar, copiar, pegar, comentar). |
| **C.2** Independiente de la decisión | §6.2.5, §6.4 (marcas, no decisiones); escenarios b, c, d, g, i. | — |
| **C.3** Anclaje que sigue las ediciones; si desaparece, línea más cercana | §6.4 (anclas), §6.2.6, D15 (también al reabrir). | — |
| **C.4** Etiquetas en la caja del chat con `×`; borrar desde el margen; se envían con el próximo mensaje; sin botón "Enviar comentarios" | §6.2.7, §6.6, D13. | Las etiquetas van en una fila propia dentro de la caja (las menciones son texto desde la Etapa 2; D13). Un mensaje vacío no se envía aunque haya comentarios. |
| **C.5** Qué recibe el agente, mismo bloque, formato exacto, ordenado, texto plano | §6.8 (formato literal y ejemplo), D14. | Fragmento con tope de 120 líneas / 12 KB (se dice cuántas faltan). |
| **C.6** Tarjetas en el mensaje enviado | §6.3, §6.6. | — |
| **C.7** Tras enviar se borran y no se repiten | §6.2.8, escenario j; D16 (si no llegó a salir, vuelven). | — |
| **C.8** Escenarios a–m | §6.9, un criterio y un test por escenario. | — |
| **C.9** Persistencia con migración; contadores | §6.4 (v2, objetos, reubicación), §6.7 (barra de estado, árbol). | Una 0.1.0 que abra una revisión de la 0.2.0 pierde los comentarios al guardar (sin vuelta atrás de versión). |
| **C.10** Comprobación manual con Claude, Codex y Antigravity reales | §11, puntos 7 y 8. | La hace el autor (necesita sus cuentas). |
| **D** Terminal integrado pospuesto a la Etapa 8 | §13 y `05-plan-etapas.md`. | Sin especificar, como se pidió. |

Regla del proyecto: nada de lo acordado se achicó en silencio; cada límite de la tabla tiene su motivo y las dudas de §10 se le preguntan al autor antes de E7-F.

---

## 9. Plan de subetapas

### 9.1 Reglas fijas de construcción (para todos los subagentes; iguales a §10.1 de la spec 08)
1. **Mismo árbol, sin worktrees** (cada uno duplicaría el `target`). Cada subagente trabaja sobre crates o archivos disjuntos de los de su ola.
2. **Solo `Edit`** (nunca reescritura completa) en archivos que otra subetapa de la misma ola también toca (`Cargo.toml` raíz, `defaults.rs`, `review.rs`, `agents.rs`, `workspace.rs`, `panel.rs`, `render.rs`, `actions.rs`, `conversations.rs`).
3. **Tests nuevos en archivos nuevos** (los nombres de §3–§6).
4. **Features de test fijas**: toda corrida de tests o clippy usa exactamente `--features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support` sobre los crates con GPUI; los crates sin GPUI, sin features. Nunca otra combinación.
5. **`cargo sweep --stamp` / `--file` solo el orquestador**, antes y después de cada verificación completa; los builds en contenedor usan `target/ubuntu-22.04` y se borran al terminar.
6. Cargo en primer plano con timeout; **sin procesos en segundo plano** ni bucles de espera; al terminar, `pgrep -af "sleep|cincel"` vacío.
7. **Nada que solo corra en GitHub sin auditarlo localmente** paso por paso (regla de la publicación de la 0.1.0): si cambia un workflow, cada bloque `run:` se ejecuta en local y en la imagen Ubuntu 22.04 del empaquetado; los tests no suponen herramientas del entorno, todo tiempo límite usa `CINCEL_PERF_BUDGET_FACTOR`, y un test que depende de un hilo real espera con tope (no un solo `run_until_parked`).
8. **Nadie hace commits** ni push ni crea etiquetas; eso lo hace el autor.
9. **Nada de capturas de pantalla completa** ni de la sesión del autor; las imágenes de los tests se generan en el test.
10. **Sin datos personales**: nada de credenciales, tokens, rutas personales, nombre del equipo ni correos en el repo; los adaptadores instalados se leen, nunca se copian.

### 9.2 Subetapas

| Sub | Contenido | Crates / archivos | Depende de | Subagente | Tamaño |
|---|---|---|---|---|---|
| **E7-A** | Conexiones (§3) y cierre de menús del chat (§4.2, tests del chat); test de los menús de gpui-kit y arreglo si hace falta. | chat (`render.rs`, `panel.rs`, `model.rs`), workspace (`agents.rs` solo `refresh_connections`, `connection_modal.rs`, `menu_dismiss_tests.rs`) | — | Sonnet | M |
| **E7-B** | Modelo de comentarios (§6.4): `comments.rs`, marcas en las diez decisiones, deshacer, `take`/`restore`, `format_feedback` (§6.8), persistencia v2 con reubicación; tests de §6.11. | review | — | Opus | L |
| **E7-C** | Editor (§6.5): `BlockMap`, caja anidada, plegado, marca del margen, tercera parte de los botones, menú contextual, acciones y bindings del editor; tests. | editor | — | Opus | L |
| **E7-D** | ACP (§5.3.3): `PromptBlock::Image`, base64, `agent_supports_images`, agente falso (`FAKE_NO_IMAGES`, `FAKE_PROMPT_LOG`, "echo-images"); comprobar las capacidades de los adaptadores instalados. | acp, `Cargo.toml` raíz (`Edit`) | — | Sonnet | S |
| **E7-E** | Chat (§5.3.1, §5.3.2, §6.6): `attachments.rs`, pegar/soltar/botón, miniaturas, `MessageBlock::Image` y `MessageBlock::Comment`, etiquetas y tarjetas, `CONVERSATION_VERSION` 2; tests. | chat, `Cargo.toml` raíz (`Edit`) | A, D | Opus | L |
| **E7-F** | Integración de comentarios en el workspace (§6.7): `review_comments.rs`, `begin_prompt`/`prepare_prompt`, D16, contadores, diálogo de cierre, persistencia, binarios, `DEFAULT_KEYMAP_JSONC` (comentarios), `shortcuts_modal`; escenarios a–m (§6.9). | workspace, settings (`defaults.rs`, `Edit`) | B, C, E | Opus | L |
| **E7-G** | Integración de imágenes en el workspace: capacidad por conexión, `rfd`, avisos, almacenamiento en `conversations/<id>/images/`, visor (`image_viewer.rs`), binding `ImageViewer`; `image_e2e_tests.rs`. | workspace (`agents.rs`, `conversations.rs`, `image_viewer.rs`, `workspace.rs` con `Edit`), settings (`defaults.rs`, `Edit`) | D, E | Sonnet | M |
| **E7-H** | Documentación de usuario: `docs/usuario/revision.md` (comentarios), `conexiones.md` (lista), sección nueva de imágenes en el chat, `atajos.md`; `user_docs_tests.rs` en verde; lectura de comprobación por un segundo subagente (Sonnet) contra la app con el agente falso. | `docs/usuario/` | F, G | Sonnet | S |
| **E7-I** | Verificación del orquestador con la lista de §12 completa y corrección de lo que falle (devuelto al subagente dueño). | — | todas | orquestador | S |
| **E7-J** | Cierre: correcciones de otras specs (encabezado), `modulos/*.md`, `docs/etapas/etapa-7.md` (construcción, decisiones, desviaciones, lista manual), versión **0.2.0**, `CHANGELOG.md`, `packaging/build.sh` + `packaging/verify.sh` (22.04 y 24.04), `tools/privacy-check.sh` sobre árbol y paquetes. | docs, `Cargo.toml` raíz, `CHANGELOG.md` | I | orquestador | S |

**Olas** (todas en el mismo árbol): **Ola 1**: E7-A, E7-B, E7-C, E7-D en paralelo (crates disjuntos; A toca de `agents.rs` solo `refresh_connections`). **Ola 2**: E7-E. **Ola 3**: E7-F y E7-G en paralelo (comparten `agents.rs`, `workspace.rs` y `defaults.rs`: solo `Edit`, zonas distintas; F toca `review*.rs`, G `conversations.rs` e `image_viewer.rs`). **Ola 4**: E7-H. **Ola 5**: E7-I. **Ola 6**: E7-J.

Cada subagente entrega: qué criterios cumple (con el test que lo prueba), cuáles no y por qué (desviación anotada), y la salida de los comandos de §12 para su parte.

### 9.3 Publicación de la 0.2.0 (después de la lista manual del autor)
Procedimiento validado en `docs/etapas/etapa-6.md` ("Publicación"): el autor hace el commit y el push a `main` → CI en verde → etiqueta `v0.2.0` (`git tag -a v0.2.0 -m "Cincel 0.2.0"` + `git push origin v0.2.0`) → el workflow Release arma, verifica y deja un **borrador** → el autor instala el `.deb` del borrador y lo publica. Si hay que mover la etiqueta tras un arreglo: `git push origin --delete v0.2.0 && git tag -d v0.2.0 && git tag -a v0.2.0 -m … && git push origin v0.2.0`. Esta etapa no cambia los workflows; si hiciera falta cambiarlos, regla 9.1.7.

---

## 10. Riesgos y dudas

**Dudas para el autor** (no bloquean la Ola 1; se preguntan antes de E7-F/E7-G):
- **Tope de 10 imágenes por mensaje** (D5): no lo acordamos; lo propongo para no mandar mensajes enormes. ¿Te parece bien, otro número, o sin tope?
- **Arrastrar una imagen desde el árbol de Cincel** hasta el chat hoy inserta la mención `@archivo` (el agente recibe la ruta). ¿Querés que, si es una imagen, se adjunte como imagen en vez de mencionarla?
- **Diálogo de cierre con solo comentarios** (sin cambios del agente pendientes): propongo que **no** aparezca, porque los comentarios se guardan solos (D17). ¿O preferís que avise también en ese caso?

**Riesgos técnicos:**
- Hijos `AnyElement` (la caja) dentro del elemento propio del editor: el `prepaint` de bloques fuera de pantalla no debe correr (virtualización); E7-C lo mide con `--bench typing` (M4 no puede empeorar).
- Un modelo de Codex sin imágenes devuelve error al recibirlas aunque el adaptador las anuncie: se muestra el error del turno; queda como límite conocido en la documentación.
- El arrastre externo en Wayland depende del compositor (COSMIC): se prueba en la lista manual; los tests usan `FileDropEvent` simulado.
- Imágenes grandes en memoria (2 000 × 2 000 RGBA ≈ 16 MB decodificadas): GPUI escala la miniatura; con 10 por mensaje el pico es acotado; M7 (memoria en reposo) se vuelve a medir con `--bench idle` en E7-I.
- `PILL_WIDTH` de 280 px hace que más segmentos usen los botones compactos en ventanas angostas: aceptado (§6.3).

---

## 11. Lista de comprobación manual (autor)

Con el paquete que deja E7-J en `dist/` (o con `cargo run -p cincel --release`).

1. **Conexiones**: abrir "Conectar": cada fila muestra solo icono, nombre, tipo y estado; el botón de arriba igual. Configuración → Conexiones sigue mostrando correo, plan y "Usado hace…".
2. **Menús**: abrir el menú de conexiones y hacer clic en el editor → se cierra. Abrirlo y apretar `Ctrl+L` → se cierra. Abrirlo y `Esc` → se cierra. Abrirlo y pasar a otra ventana → se cierra. Repetir con el historial (el reloj), con el selector de modelo y con el menú de la barra de título.
3. **Imágenes, tres vías**: hacer una captura (`Impr Pant` copia al portapapeles) y `Ctrl+V` en el chat; arrastrar un PNG desde el explorador de archivos a la caja; botón del clip y elegir un JPEG. Quitar una con la `×`.
4. **Imagen grande**: adjuntar una foto de más de 2 000 px; enviarla a Claude preguntando "¿qué tamaño tiene esta imagen?" (debería decir 2 000 de lado o menos).
5. **Ver y reabrir**: clic en la miniatura del mensaje enviado → se abre grande; `Esc`. Abrir otra conversación y volver a esta desde el historial: las imágenes siguen.
6. **Repetir 3 y 5 con Codex y con Antigravity** (si alguno avisa "no acepta imágenes", anotarlo).
7. **Comentarios con Claude**: pedirle un cambio en dos archivos. En un segmento: "Comentar", escribir "esto hacelo con un diccionario", `Ctrl+Enter`. Rechazar otro segmento y comentarlo. Seleccionar tres líneas que el agente no tocó, clic derecho → "Comentar selección", escribir algo. Ver las tres etiquetas en la caja del chat y "3 comentarios" en la barra de estado y el árbol. Quitar una con la `×`. Escribir "atendé mis comentarios" y enviar: el mensaje muestra dos tarjetas; las marcas del margen desaparecen. Comprobar que **Claude hace lo que pedían las notas** y que el segmento comentado se actualiza.
8. **Lo mismo que 7 con Codex y con Antigravity** (comprobación obligatoria: que entiendan y atiendan la nota).
9. **Cerrar con comentarios**: dejar un comentario sin enviar, cerrar Cincel (si hay cambios pendientes, el diálogo menciona el comentario), reabrir: el comentario vuelve en su lugar.
10. **Nada se mueve**: abrir y cerrar una caja de comentario mirando el margen y el código: no se corren a los costados.
11. `cincel --version` dice `cincel 0.2.0`; instalar el `.deb` (o el comprimido) y repetir 3 y 7 rápido.
12. Leer `docs/usuario/revision.md` (comentarios) y la sección de imágenes; si algo no se entiende, avisar.
13. Commit, push, etiqueta `v0.2.0` y publicar el borrador (§9.3).

---

## 12. Comandos de verificación

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
target/release/cincel --bench typing <corpus>/medio --bench-file <corpus>/medio/cinco-mil.rs   # M4 no empeora (E7-C, E7-I)
target/release/cincel --bench idle <corpus>/medio                          # M7/M8 (E7-I)
packaging/build.sh && packaging/verify.sh                                  # E7-J
tools/privacy-check.sh                                                     # E7-J
cargo sweep --file                                                         # solo el orquestador
pgrep -af "sleep|cincel"; docker ps                                        # vacíos
cargo test -p cincel-editor -p cincel-chat -p cincel-workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support ranks_fifty_thousand -- --ignored   # medición del buscador, con la máquina tranquila
cargo test -p cincel-editor -p cincel-chat -p cincel-workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support the_sweep_of_five_thousand_files_never_blocks -- --ignored   # bloqueo del repaso (≤ 8 ms), con la máquina tranquila
```

Un subagente verifica solo su parte:
- Si tocó un crate con GPUI (`cincel-editor`, `cincel-chat`, `cincel-workspace`, `cincel`): `cargo test -p cincel-editor -p cincel-chat -p cincel-workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support` (y lo mismo con `clippy`).
- Si tocó solo crates sin GPUI (`cincel-review`, `cincel-acp`, `cincel-settings`): `cargo test -p <crate>` sin features (para `cincel-acp`, antes `cargo build -p cincel-acp --bin cincel-acp-fake-agent`).

---

## 13. Después de esta etapa

- **Etapa 8: terminal integrado** (decisión del autor). Pospuesto; se especificará en su propia spec.
- Siguen pospuestas, sin fecha, las ideas de `05-plan-etapas.md` "Después de v1" que no entraron acá: LSP, autenticación de agentes dentro de la app, multi-cursor y plegado, búsqueda en proyecto, paleta de comandos, OpenCode y agentes con API key o gateway, panel de git, ACP v2, macOS; y las deudas técnicas anotadas en `docs/etapas/etapa-6.md` "Después de la 1.0".
- Ideas que surgieron al escribir esta spec y no se acordaron (no entran): listar los comentarios en el panel "Revisar todo"; recodificar GIF animados reducidos conservando la animación.
