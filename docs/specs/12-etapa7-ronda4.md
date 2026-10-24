# Spec 12: Etapa 7, ronda 4 (el adjunto de revisión no se muestra al retomar, y barra de desplazamiento en el chat)

Fecha: 2026-10-22. Origen: dos observaciones del autor al usar la 0.2.1 y la
0.2.2 (publicada el 2026-10-22, commit `0671999`, etiqueta `v0.2.2`). Cierra
como **0.2.3**.

## 1. Qué trae esta ronda (lenguaje llano)

- **A. El adjunto de revisión no aparece nunca en el chat.** Cuando el usuario
  rechaza o edita a mano lo que escribió el agente, Cincel le adjunta en
  silencio al siguiente mensaje un parche con lo que quedó distinto (el bloque
  `<user_review_feedback>`, spec 09 §6.8). Hoy, al retomar una conversación
  (`session/load`), el agente repite el historial y ese adjunto se pinta como
  si fuera un mensaje del usuario, enorme (puede traer archivos enteros), y
  queda guardado en la conversación. Desde esta ronda el chat lo reconoce, no
  lo pinta, no lo guarda, y limpia el que ya quedó guardado en conversaciones
  anteriores. Lo que el agente recibe no cambia en nada.
- **B. Barra de desplazamiento vertical en el chat**, como la del editor:
  fina, aparece al mover el ratón o al desplazar, se desvanece sola, y se
  puede arrastrar para subir rápido en una conversación larga.

## 2. Decisiones técnicas

- D1. La etiqueta del adjunto vive en un solo lugar: `cincel-acp::protocol`
  exporta las constantes de apertura y cierre que usa `wrap_review_feedback`,
  y `cincel-chat` (que ya depende de `cincel-acp`) las usa para reconocerlo.
  Ninguna copia de la cadena en el chat.
- D2. La limpieza es una función pura de `cincel-chat`
  (`strip_review_feedback(&str) -> Option<String>` o equivalente, en un módulo
  propio): quita cada bloque completo `<user_review_feedback>…</user_review_feedback>`
  con los saltos de línea que lo separan del resto; devuelve el texto que queda
  (`None` si no queda nada que no sea espacio). Una etiqueta de apertura sin
  cierre se deja como está (no es el adjunto).
- D3. La barra del chat se dibuja encima del transcript sin cambiar su ancho
  ni el de las burbujas (una superposición absoluta en el borde derecho),
  alimentada por los datos que `ListState` ya expone
  (`max_offset_for_scrollbar`, `scroll_px_offset_for_scrollbar`,
  `viewport_bounds`, `set_offset_from_scrollbar`, `scrollbar_drag_started` /
  `scrollbar_drag_ended`). Si el valor disponible al pintar es el del cuadro
  anterior, el retraso de un cuadro se acepta y se anota.
- D4. Tiempos y aspecto idénticos a los del editor (`02-visual.md` §5
  "Scroll"): 8 px de ancho, pulgar de al menos 24 px y radio 3, `text.muted`
  al 80 % sobre pista al 40 % del fondo, 1000 ms visible y 200 ms de
  desvanecimiento. Las dos constantes de tiempo se repiten en `cincel-chat`
  (el editor no las exporta; corregido el 2026-10-22: el chat sí depende de
  `cincel-editor`, el motivo original estaba mal); se anota.

## 3. El adjunto de revisión repetido (A)

### 3.1 Comportamiento

- R1. Durante una reproducción (`ChatPanel::begin_replay` hasta
  `SessionCreated`), un `user_message_chunk` de texto pasa por D2 antes del
  filtro "ya está en pantalla": si no queda nada, se descarta; si queda texto,
  ese texto es el que se compara y, de ser nuevo, el que se muestra.
- R2. Fuera de una reproducción vale lo mismo (R1 no depende de `replaying`):
  un agente que repitiera el adjunto en otro momento tampoco lo pinta.
- R3. Al cargar una conversación guardada (`import_transcript` y
  `load_conversation`), cada bloque de texto de cada mensaje de usuario pasa
  por D2; un mensaje que queda sin bloques se elimina del transcript. La
  conversación limpia se guarda en el siguiente guardado normal; no hace
  falta reescribirla al abrir.
- R4. No cambia lo que se manda al agente (`build_prompt_content`,
  `wrap_review_feedback`) ni las tarjetas de comentarios enviados.

### 3.2 Criterios de aceptación

- A1. Reproducir un mensaje cuyo único bloque de texto es el adjunto no agrega
  ninguna entrada.
- A2. Reproducir "adjunto + texto del usuario" (en cualquiera de los dos
  órdenes, en un chunk o en dos) muestra solo el texto del usuario, y no lo
  muestra si ya estaba en pantalla.
- A3. Importar un transcript con un mensaje de usuario que es solo el adjunto
  lo elimina; con "adjunto + texto" conserva solo el texto; el resto de las
  entradas no se tocan.
- A4. El texto de un usuario que escribió a mano solo `<user_review_feedback>`
  (sin cierre) se muestra tal cual.
- A5. Los tests existentes de `cincel-acp` sobre el prompt siguen en verde sin
  cambios.

### 3.3 Tests

Archivo nuevo `crates/cincel-chat/src/replay_feedback_tests.rs`: A1 a A4 con
el panel real (`test-support`), más tests unitarios de D2 (bloque solo,
bloque al principio, al final, dos bloques, apertura sin cierre, bloque con
líneas que empiezan con `-` y `+`). **Datos sintéticos solamente**: nada de
leer conversaciones guardadas del autor.

## 4. Barra de desplazamiento del chat (B)

### 4.1 Comportamiento

- R5. Vertical, 8 px, pegada al borde derecho del transcript (no del
  compositor ni del encabezado), encima del contenido: no cambia el ancho de
  las burbujas ni de la lista. Solo existe mientras hay algo que desplazar.
- R6. Aparece al mover el ratón sobre el transcript y cada vez que el
  contenido se desplaza (rueda, arrastre, "Ir al final", seguimiento de la
  cola mientras el agente escribe); queda 1000 ms y se desvanece en 200 ms.
  Mientras se arrastra o el ratón está sobre la pista, no se desvanece.
- R7. Pulgar proporcional (alto visible / alto total × alto visible, mínimo
  24 px, nunca más que la pista). Arrastrar el pulgar desplaza la lista en
  proporción; clic en la pista fuera del pulgar lleva el pulgar a centrarse
  ahí (como en el editor).
- R8. Arrastrar pausa el seguimiento de la cola igual que la rueda
  (`pause_following_tail`); la flecha "Ir al final" aparece y desaparece con
  las mismas reglas que con la rueda.
- R9. La barra no toma el foco del teclado, no cambia el cursor de texto, no
  tapa los controles de las burbujas al cliquear fuera de ella, y mientras es
  invisible no pide cuadros (`--bench idle` sigue en 0 cuadros).

### 4.2 Criterios de aceptación

- B1. Transcript corto (todo visible): sin barra, aunque se mueva el ratón.
- B2. Transcript largo: sin barra hasta que el ratón se mueve sobre él; luego
  visible, con el pulgar de alto proporcional y mínimo 24 px, en la posición
  correspondiente al desplazamiento.
- B3. Pasados 1000 ms sin ratón ni desplazamiento, se desvanece y a los 200 ms
  no está.
- B4. Arrastrar el pulgar hasta abajo deja la lista al final; hasta arriba, al
  principio; con el pulgar en el medio, el desplazamiento es el proporcional.
- B5. Clic en la pista centra el pulgar en el punto del clic.
- B6. Con el agente escribiendo y la cola siguiéndose, la barra aparece y el
  pulgar queda abajo; un arrastre hacia arriba pausa el seguimiento y muestra
  "Ir al final".

### 4.3 Tests

Archivo nuevo `crates/cincel-chat/src/scrollbar_tests.rs`: B1 a B6 con el
panel real y el ratón simulado (como `transcript_follow_tests.rs`), midiendo
pista y pulgar por `debug_selector` (`chat-scrollbar-track`,
`chat-scrollbar-thumb`) o por un accesor `test-support` que devuelva sus
`Bounds` y la opacidad.

## 5. Reglas fijas de construcción

Las de la spec 10 §14.1, sin cambios: mismo árbol, `Edit` en archivos
compartidos, tests nuevos en archivos nuevos, features de test fijas
(`cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`),
sin `cargo sweep` salvo el orquestador, cargo en primer plano con tope, nada
de commits, nada de capturas, sin datos personales (en particular, no leer
`~/.local/state/cincel` ni `~/.local/share/cincel`). Un solo subagente (los
dos cambios están en `cincel-chat`).

## 6. Verificación (orquestador)

Los comandos de la spec 10 §17 completos (la ronda 3 ya corrió los bancos;
se repiten `--bench typing` y `--bench idle` una vez porque R9 lo exige).
Paquetes con `packaging/build.sh` + `verify.sh`.

## 7. Lista de comprobación manual (autor)

1. Retomar la conversación del proyecto donde apareció el adjunto gigante: el
   bloque `</user_review_feedback>` ya no está, ni al abrir ni al retomar.
2. Rechazar un cambio del agente, mandarle un mensaje, cerrar Cincel, abrir
   y retomar: el chat muestra solo tu mensaje; el agente respondió respetando
   el rechazo.
3. Conversación larga: mover el ratón sobre el chat muestra una barra fina a
   la derecha; quieto un segundo, desaparece.
4. Arrastrar el pulgar hasta arriba lleva al principio; soltarlo y usar la
   rueda sigue funcionando igual.
5. Clic en la pista, arriba del pulgar: la vista salta ahí.
6. Con el agente escribiendo, arrastrar hacia arriba: la respuesta no te
   arrastra hacia abajo y aparece "Ir al final".
7. Conversación corta: no hay barra.

## 8. Cierre

Versión **0.2.3** en todos los lugares que la ronda 3 tocó al subir a 0.2.2
(commit `0671999`: `Cargo.toml` del workspace, `README.md`, metainfo,
`docs/usuario/instalacion.md` si nombra archivos), `CHANGELOG.md` (entrada
nueva `[0.2.3] - 2026-10-22`, breve: un párrafo y una línea por cambio),
`<release>` nuevo en `packaging/linux/dev.cincel.Cincel.metainfo.xml`, `02-visual.md`
(sección del chat: la barra, con referencia a §5), `modulos/chat.md`
(reproducción: el adjunto; la barra), `modulos/acp.md` §Prompt (una línea: el
chat lo reconoce por las constantes), `docs/usuario/chat.md` si describe el
desplazamiento, el doc de `cincel-chat/src/lib.rs`; sección "Ronda 4" en
`docs/etapas/etapa-7.md`; paquetes en `dist/`.
