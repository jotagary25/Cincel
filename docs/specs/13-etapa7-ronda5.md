# Spec 13: Etapa 7, ronda 5 (conversaciones aisladas, barra y asa, párrafos con referencias seleccionables, arriba/abajo en los bordes)

Fecha: 2026-11-07. Origen: cinco observaciones del autor al usar la 0.2.3
(publicada el 2026-10-24, `v0.2.3`, rama `develop` limpia). Cierra como
**0.2.4**.

## 1. Qué trae esta ronda (lenguaje llano)

- **A. Una conversación nueva arranca de cero.** Hoy, los parches de lo que
  el usuario rechazó o editó (y los comentarios que no mandó) quedan en una
  cola por proyecto y viajan con el próximo mensaje, aunque sea de una
  conversación nueva. Desde esta ronda, crear una conversación nueva vacía
  esa cola.
- **B. Los agentes no recuerdan entre conversaciones.** Cincel lanza a Claude
  y a Codex con su memoria persistente apagada, siempre, sin ajuste para
  encenderla. Antigravity no ofrece ninguna palanca documentada: queda
  documentado como "no aislable".
- **C. La barra del chat no tapa el asa del panel.** La barra se corre hacia
  adentro y solo toma el clic cuando el puntero está realmente sobre ella.
- **D. Los párrafos con referencias `@archivo` se pueden seleccionar**, con
  las fichas incrustadas en el texto.
- **E. Arriba en la primera fila va al principio; abajo en la última va al
  final**, en el editor y en el cuadro del chat.

## 2. Decisiones técnicas

- D1. (A) `Review` gana un método que descarta lo pendiente de reportar
  (`reportable`, los registros por turno del store y los comentarios no
  enviados) sin tocar los archivos ni los hunks en revisión; `Agents::begin_conversation`
  lo llama antes de arrancar la sesión nueva. Retomar una conversación
  guardada (`session/load`) no lo llama: la cola ya es de esa conversación.
- D2. (B) Las variables van en `Profile::process_env` (`cincel-connections`),
  por agente: Claude recibe `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`; Codex recibe
  `CODEX_CONFIG` con `{"features":{"memories":false},"memories":{"use_memories":false,"generate_memories":false},"history":{"persistence":"none"}}`.
  Se respeta `CLAUDE.md` / `AGENTS.md` del proyecto (son archivos del usuario,
  no memoria del agente). Antigravity no recibe nada nuevo. Las notas de
  memoria que Claude ya guardó bajo el perfil de la conexión no se borran
  solas: la lista manual trae el comando. Las variables se prueban en los
  tests de `process_env`; contra los adaptadores reales se verifica a mano
  (lista manual), porque exige la cuenta del autor.
- D3. (C) La pista de la barra se corre 6 px hacia adentro (el asa de
  gpui-base ocupa de −6 a −1 px del borde derecho del dock); la pista queda
  en `[right − 14, right − 6]`. `MouseDown` deja la fase de captura: pasa a
  la fase normal y decide por `hitbox.is_hovered(window)` de la pista, no
  por `bounds.contains`, así un elemento que ocluye (el asa) gana.
- D4. (D) Un párrafo con referencias se dibuja con un solo `TextView` de
  gpui-base, y cada ficha va como `InlineElement` (gpui-base
  `text/inline.rs`, como las imágenes en línea). El `TextViewState` es
  seleccionable como los demás y entra en la misma caché `bubble_views`
  (clave entrada/pieza/segmento). Si `InlineElement` no permite reproducir la
  ficha tal cual (icono + nombre, clic abre el archivo), la ficha se dibuja
  como texto `@nombre` dentro del `TextView` con el estilo de función y el
  clic sigue abriendo el archivo; se anota como desviación.
- D5. (E) `EditorView::move_vertically`: si la fila de destino recortada es
  la misma fila actual, el cursor va a la columna 0 (hacia arriba) o al
  final de la fila (hacia abajo) y `goal_column` se reinicia. Con `extend`
  vale igual (Shift+flecha extiende hasta ahí). La lista de archivos del `@`
  abierta sigue capturando arriba/abajo como hoy.

## 3. Comportamiento y criterios

### 3.1 Conversación nueva (A)

- R1. Al crear una conversación nueva, la cola de parches y los comentarios
  no enviados se descartan: el primer prompt de la nueva no lleva ningún
  bloque `<user_review_feedback>` salvo lo que se genere en esa conversación.
- R2. Los hunks pendientes de revisión en el editor no cambian: siguen ahí
  para aceptar o rechazar, y lo que se rechace después de crear la
  conversación sí va en el próximo prompt (es de esta conversación).
- R3. Retomar una conversación guardada no descarta nada.
- A1. Test: rechazar un hunk, crear conversación nueva, mandar un prompt: el
  prompt no lleva feedback. A2: rechazar, crear nueva, rechazar otro, mandar:
  solo el segundo. A3: comentarios sin enviar + conversación nueva: se
  descartan y sus tarjetas no aparecen. A4: retomar conserva la cola.

### 3.2 Memoria de los agentes (B)

- R4. El proceso de Claude recibe `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`; el de
  Codex recibe `CODEX_CONFIG` con el JSON de D2; Antigravity no recibe
  variables nuevas. Las variables se aplican en todos los arranques del
  adaptador (sesión nueva, retomar, login si corre el mismo binario).
- B1. Tests en `process_env` / `profile.rs`: cada agente recibe exactamente
  esas variables y ninguna otra nueva; el JSON de Codex es válido.
- B2. `docs/usuario/conexiones.md`: sección breve "Las conversaciones no
  comparten memoria", con la excepción de Antigravity explicada.

### 3.3 Barra y asa (C)

- R5. Pista en `[right − 14, right − 6]`, 8 px; pulgar dentro de ella.
- R6. Con la barra visible, un `MouseDown` en la franja del asa (los 6 px del
  borde) no la toca: el asa redimensiona. Un `MouseDown` sobre la pista
  arrastra o salta como hoy.
- C1. Test: barra visible, press en `right − 3`: la barra no entra en
  arrastre y el evento no se detiene. C2: press en `right − 10`: la barra
  arrastra. C3: los tests existentes de la barra siguen en verde con la
  geometría nueva.

### 3.4 Párrafos con referencias (D)

- R7. Arrastrar sobre el texto de un párrafo con referencias selecciona ese
  texto (antes y después de las fichas); copiar devuelve el texto con cada
  ficha como `@nombre`.
- R8. Las fichas siguen viéndose como hoy (icono, nombre, color) y el clic
  sigue abriendo el archivo.
- D1t. Test con el panel envuelto en `TextSelectionLayer` (como la sonda del
  2026-11-07: sin esa capa nada se selecciona en los tests): arrastre sobre
  "mirá @a.rs y @b.rs y decime" selecciona y `TextSelection::selected_text`
  devuelve el texto con los nombres. D2t: el primer mensaje con referencias
  se selecciona. D3t: clic en la ficha emite `ChatEvent::OpenFile` como hoy.

### 3.5 Arriba/abajo en los bordes (E)

- R9. Primera fila, cursor en el medio, Arriba: columna 0. Última fila,
  Abajo: final de la fila. Shift+Arriba / Shift+Abajo extienden la selección
  hasta ahí. Una sola fila cubre los dos casos.
- R10. Con varias filas, nada cambia: Arriba/Abajo siguen conservando la
  columna objetivo.
- E1. Tests en archivo nuevo de `cincel-editor`: los cuatro casos de R9 en
  el editor y en el cuadro del chat (un `EditorView` de chat), más R10 y el
  caso con filas envueltas (wrap): la "fila" es la visual.

## 4. Reglas fijas de construcción

Las de la spec 10 §14.1: mismo árbol, `Edit` en archivos compartidos, tests
nuevos en archivos nuevos, features de test fijas
(`cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`,
solo las de los crates seleccionados en corridas por crate), sin `cargo sweep`
salvo el orquestador, cargo en primer plano con tope, nada de commits, nada
de capturas, sin datos personales (no leer `~/.local/state/cincel` ni
`~/.local/share/cincel`). Cuatro subagentes en paralelo con superficies de
edición disjuntas; `docs/etapas/etapa-7.md`, versión, CHANGELOG, metainfo y
README los toca el orquestador.

## 5. Verificación (orquestador)

Los comandos de la spec 10 §17 completos, con `--no-fail-fast`, más
`--bench typing` (dos corridas) y `--bench idle` una vez. Paquetes con
`packaging/build.sh` + `verify.sh`.

## 6. Lista de comprobación manual (autor)

1. Rechazar un cambio del agente, crear una conversación nueva, mandar un
   mensaje: el agente no recibe ningún parche ni menciona "cambios
   anteriores".
2. Lo mismo, pero rechazando un segundo cambio después de crear la nueva:
   el agente recibe solo ese.
3. Borrar las notas de memoria que Claude ya guardó, una sola vez (sin ellas
   no hay nada que recordar):
   `rm -rf ~/.local/share/cincel/connections/*/projects/*/memory`
4. Claude: una conversación, cerrar, conversación nueva en el mismo
   proyecto, preguntarle qué recuerda de la anterior: nada; y no reaparece
   ninguna carpeta `memory` bajo `~/.local/share/cincel/connections/`.
5. Codex: lo mismo; no aparece `memories_v2/` ni `history.jsonl` bajo su
   perfil en `~/.local/share/cincel/connections/`.
6. Conversación larga: tomar el borde derecho del chat con el ratón y
   ensanchar: funciona aunque la barra esté visible; arrastrar la barra
   sigue funcionando.
7. Mensaje con referencias `@archivo`: seleccionar su texto y copiarlo; el
   portapapeles trae el texto con `@nombre`; clic en la ficha abre el
   archivo.
8. Editor y cuadro del chat, una sola línea, cursor en el medio: Arriba va
   al principio, Abajo al final; Shift+Abajo selecciona hasta el final. Con
   varias líneas, igual que antes.

## 7. Cierre

Versión **0.2.4** donde la ronda 4 subió a 0.2.3 (`Cargo.toml`, `README.md`,
metainfo, plantillas y `docs/publicacion.md`), `CHANGELOG.md` (entrada breve),
`<release>` nuevo; docs: `modulos/workspace.md` (A), `modulos/connections.md`
y `docs/usuario/conexiones.md` (B), `02-visual.md` y `modulos/chat.md` (C, D),
`modulos/editor.md` y `docs/usuario/atajos.md` si describe las flechas (E);
sección "Ronda 5" en `docs/etapas/etapa-7.md`; paquetes en `dist/`.
