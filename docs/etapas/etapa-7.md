# Etapa 7: conexiones más limpias, imágenes en el chat y comentarios para el agente (cierre = versión 0.2.0)

Estado: **cerrada** (2026-09-30). Spec: `docs/specs/09-etapa7-conexiones-imagenes-comentarios.md`.
Ronda 2: **cerrada** (2026-10-01). Spec: `docs/specs/10-etapa7-ronda2.md` (ver "Ronda 2" más abajo).
Esta etapa cierra la versión **0.2.0** de Cincel (la 0.1.0 es la publicada).

Este documento lo escribió el orquestador (E7-J) integrando el trabajo de las
subetapas E7-A a E7-I, cada una construida por un subagente en el mismo árbol
de trabajo (sin worktrees por subagente, por la misma razón que en las
Etapas 5 y 6: cada worktree duplicaría `target`, varios GB por copia). Cada
subagente tocó crates o archivos disjuntos de los de su ola, con `Edit` (nunca
reescritura completa) en los pocos archivos que dos subetapas de la misma ola
tocaban en paralelo (`Cargo.toml` raíz, `agents.rs`, `workspace.rs`,
`defaults.rs`, `conversations.rs`). Olas: 1 = E7-A, E7-B, E7-C, E7-D;
2 = E7-E; 3 = E7-F y E7-G; 4 = E7-H; 5 = E7-I; 6 = E7-J.

## Qué se construyó

### E7-A: conexiones más limpias y menús que se cierran solos
- **Lista de conexiones** (`crates/cincel-chat/src/model.rs`, `render.rs`):
  `ChatConnection` suma `agent_name` ("Claude", "Codex", "Antigravity", de
  `AgentKind::display_name`; el `agent_id` si no es uno de los tres) y
  conserva `identity` y `last_used`, que ahora solo pinta Configuración →
  Conexiones. El menú "Conectar" y el botón del encabezado muestran icono,
  nombre, tipo en gris e insignia de estado; el tipo se omite si el nombre ya
  es exactamente el tipo (`agent_type_label`, sin distinguir mayúsculas). La
  insignia se extrajo a `connection_badge`. La píldora "listo/pensando…" sigue
  afuera del botón. La lista de "Eliminar conexión…" (`connection_modal.rs`)
  sigue la misma regla. Lo llena `Agents::refresh_connections`.
- **Menús del chat que se cierran solos** (`panel.rs`, `render.rs`): el
  panel tiene un foco propio para el menú (`popover_focus`), la caja del menú
  cierra con `on_mouse_down_out`, `cx.on_focus_out` del foco del menú (y del
  compositor para `@` y `/`) y `cx.observe_window_activation`; `Esc` ya
  cerraba. Un clic en el botón que abrió el menú lo cierra una sola vez
  (`just_dismissed`, atado a un contador de pulsaciones del mouse,
  `press_serial`, para que el clic siguiente sí lo vuelva a abrir);
  `restore_focus` devuelve el foco a donde estaba al cerrar con `Esc` o al
  elegir una fila (`sync_popover_focus`, que mueve el foco al pintar).
- **Menús de gpui-kit** (`crates/cincel-workspace/src/menu_dismiss.rs`): el
  menú de la barra de título y el menú contextual del árbol ya se cerraban con
  clic fuera y `Esc`, pero no cuando el foco se iba a otro elemento (`Ctrl+L`)
  ni cuando la ventana perdía el foco. `dismiss_on_focus_loss` (suscripciones
  de foco y de activación de la ventana) y `send_cancel` les mandan el mismo
  `Cancel` que su `Esc`, de modo que cierran como siempre; sin tocar gpui-kit.
- **Tests**: `cincel-chat/src/connections_view_tests.rs`,
  `cincel-chat/src/menu_dismiss_tests.rs`,
  `cincel-workspace/src/connections_list_tests.rs`,
  `cincel-workspace/src/menu_dismiss_tests.rs`.

### E7-B: modelo de comentarios en `cincel-review`
`crates/cincel-review/src/comments.rs` (dentro de `ReviewStore`) y
`feedback.rs`. `CommentId`, `CommentState` (`Pending`, `Accepted`, `Rejected`,
`Mixed`, `NoAgentChange`), `CommentView`, `SentComment`, anclas por comentario
en un `AnchorMap` por archivo alimentado con cada `BufferEvent`, las diez
decisiones (aceptar y rechazar por segmento, línea, archivo, turno y todo)
que dejan marcas `accepted`/`rejected` en los comentarios que tocan (las de
un rechazo viajan en su entrada de deshacer), `take_comments_for_prompt` /
`restore_comments`, `format_feedback` con el texto exacto de §6.8, fragmento
con tope de 120 líneas / 12 KB y vallas con acentos graves. Persistencia:
`STATE_VERSION` 2 con `comments`, fragmento en `objects/`, reubicación al
abrir (por el fragmento o por la fila más cercana), `LoadReport` con
`comments_restored`, `comments_relocated` y `comments_dropped`. Detalle del
módulo en `docs/specs/modulos/review.md`. Tests: `tests/comments.rs`,
`tests/comments_persist.rs`.

### E7-C: editor, `BlockMap` y caja de comentario
`crates/cincel-editor`: `block_map.rs` (la capa entre `WrapMap` y el
elemento que `03-arquitectura.md` §5 dejaba como futura), `view/comments.rs`
(caja anidada), `comments.rs` (tipos del anfitrión), marca del margen,
tercera parte "Comentar" de los botones del segmento (`PILL_WIDTH` 280 px;
compactos de 80 px), menú contextual del editor (cortar, copiar, pegar,
comentar selección o línea), acciones `editor::comment_selection`
(`Ctrl+Shift+M`), `editor::save_comment` y `editor::cancel_comment`
(`Ctrl+Enter` y `Esc` en `Editor && comment_box`) y `review::comment_hunk`.
El texto nunca se corre a los costados y el margen no cambia de ancho; un
bloque que aparece arriba de la primera fila visible se compensa en
`scroll_top`. Detalle en `docs/specs/modulos/editor.md`. Tests:
`block_map_tests.rs`, `comment_box_tests.rs`, `comment_rows_tests.rs`.

### E7-D: imágenes en ACP
`crates/cincel-acp/src/protocol.rs`: `PromptBlock::Image { mime_type, data }`
(constructor `PromptBlock::image`, que valida el tipo contra
`IMAGE_MIME_TYPES`; el worker valida otra vez antes de enviar) y su
conversión a `ContentBlock::Image` en base64 **sin `uri`**;
`agent_supports_images(&AgentCapabilities)`. Agente falso
(`tests/fake_agent/main.rs`): anuncia `promptCapabilities.image` salvo con
`FAKE_NO_IMAGES`, `FAKE_PROMPT_LOG=<ruta>` agrega una línea JSON por
`session/prompt` (con largo y SHA-256 de los datos, no los datos) y el guion
`"echo-images"` responde "N imágenes: mime WxH, …". `Cargo.toml` raíz suma
`base64` 0.22 e `image` 0.25 (ya estaban en `Cargo.lock`). Tests en
`crates/cincel-acp/tests/integration.rs` y unitarios en `protocol.rs`.

**Capacidades `promptCapabilities.image` de los adaptadores** (leídas en la
máquina de referencia el 2026-09-30, sin copiar nada al repo): `claude-agent-acp`
0.84.0 `true`; `codex-acp` 1.13.1 y 1.12.0 `true` (leídos de la caché de `npx`,
porque Cincel todavía no tenía descargado `codex-acp` en su carpeta de
agentes); Antigravity 1.2.1 `true`. Los tres aceptan imágenes hoy.

### E7-E: chat (imágenes y comentarios)
`crates/cincel-chat`: `attachments.rs` (procesado sin GPUI:
`prepare_image`, detección por contenido, reducción a 2 000 px de lado, tope
de 10 MB y de 10 imágenes), `panel_media.rs` (estado: `Ctrl+V`, soltar,
botón del clip, capacidad por conexión con `set_image_support`,
`PendingAttachment`), `render_media.rs` (miniaturas con `×`, mensaje enviado,
etiquetas y tarjetas), `comments.rs` (`PendingComment`, `SentCommentCard`,
etiquetas), `MessageBlock::Image(ImageRef)` y `MessageBlock::Comment`,
`CONVERSATION_VERSION` 2 con `Conversation::migrate`. Detalle en
`docs/specs/modulos/chat.md`. Tests: `attachments_tests.rs` (procesado y
panel, con imágenes generadas en el test) y `comment_cards_tests.rs`.

### E7-F: integración de comentarios en el workspace
`crates/cincel-workspace/src/review_comments.rs` (hijo de `review.rs`):
suscripción a las acciones de comentario de cada editor, `add_comment` /
`edit_comment` / `remove_comment` seguidos de refresco y guardado, etiquetas
al chat, `begin_prompt` que devuelve `PromptFeedback { turn, feedback, sent }`
(primero los comentarios con `take_comments_for_prompt`, después los parches
de siempre, y `format_feedback` los une en el mismo `<user_review_feedback>`),
`prompt_not_sent` (D16), contadores en la barra de estado y el árbol, línea
extra del diálogo de cierre (D17), descarte con aviso de comentarios sobre
archivos que se vuelven binarios o desaparecen, y carga con avisos de
`comments_dropped`. `DEFAULT_KEYMAP_JSONC`, `shortcuts_modal` y
`docs/usuario/atajos.md` quedan al día con los atajos nuevos (`Ctrl+Shift+M`,
`Ctrl+Enter` y `Esc` de la caja). Tests:
`review_comments_tests.rs` (escenarios a a m de §6.9 más
`cancelled_prompt_gives_comments_back`).

### E7-G: integración de imágenes en el workspace
`crates/cincel-workspace/src/agents/chat_images.rs`: capacidad de imágenes por
conexión, botón del clip (`ChatEvent::PickImages` → `rfd` con título
"Adjuntar imagen" y filtro "Imágenes" → `attach_picked`), avisos del chat
como `toast`, carpeta de la conversación en pantalla, guardado inmediato de la
conversación al enviar con imágenes. `conversations.rs`: las imágenes se
guardan en `conversations/<id>/images/<sha256>.<ext>` con escritura atómica;
el JSON solo lleva la referencia; borrar una conversación (o las de una
conexión) borra su carpeta y deshacer el borrado la restaura.
`image_viewer.rs`: capa del workspace sobre toda la ventana, contexto de
teclas `ImageViewer`, acción `workspace::close_image_viewer` con `Esc`
(sección nueva en `DEFAULT_KEYMAP_JSONC`), cierra también con clic fuera de
la imagen o la `×`. Tests: `image_e2e_tests.rs`.

### E7-H: documentación de usuario
`docs/usuario/`: archivo nuevo `chat.md` (mensajes, imágenes, comentarios en
la caja), sección "Comentarios para el agente" en `revision.md`, la lista de
conexiones en `conexiones.md`, los atajos nuevos en `atajos.md` y el aviso de
imágenes en `problemas.md`; el índice del `README.md` del manual suma `chat.md`.
`user_docs_tests.rs` ata cada atajo del keymap por defecto a su descripción en
`atajos.md`.

### E7-I: verificación del orquestador
Todos los comandos de §12 de la spec (ver "Verificación"), con las
correcciones que mostró la corrida (ver "Desviaciones · Orquestador").

### E7-J: cierre (orquestador)
Esta documentación (`docs/etapas/etapa-7.md`, `modulos/workspace.md`,
`modulos/chat.md`, `modulos/acp.md`, `05-plan-etapas.md`, `README.md`,
`docs/guia-integracion-acp.md`), la versión 0.2.0 en `[workspace.package]`, la
entrada de `CHANGELOG.md` y los paquetes (`packaging/build.sh` y `verify.sh`).

## Decisiones tomadas en la construcción

- **Lo que la spec proponía en "Riesgos y dudas" (§10) se construyó tal como
  estaba propuesto**: tope de 10 imágenes por mensaje; la mención `@archivo`
  de una imagen del árbol se convierte en adjunto (`ChatPanel::mention_or_attach`,
  ver E7-H en las desviaciones); el diálogo de cierre no aparece si solo hay
  comentarios (D17: se guardan solos).
- **Los comentarios son de la ventana, no de la conexión**: cambiar de
  conexión o empezar otra conversación no los toca; viajan con el próximo
  mensaje a la conexión nueva.
- **Los comentarios nunca se pierden sin haber salido** (D16): si el mensaje
  no llega a salir (se cancela mientras espera la foto del proyecto, o el
  agente se va en esa espera), vuelven al margen y a la caja del chat y su
  tarjeta dice "No se envió: el comentario volvió al margen".
- **Al agente viaja texto plano en inglés**, dentro del mismo
  `<user_review_feedback>` que ya existía, después de los parches; no depende
  de ninguna capacidad del agente.
- **Las imágenes viajan sin `uri`** (D4): `codex-acp` usa el `uri` en vez de
  los datos si es `http(s):` o `data:`; sin `uri` los tres adaptadores toman
  los datos.
- **Los menús de gpui-kit se corrigen desde Cincel, sin tocar gpui-kit**
  (`menu_dismiss.rs`): la misma regla de la spec (§4.1) para menús propios y
  ajenos.
- **La caja de comentario es una fila de bloque del `BlockMap`**, no una capa
  flotante: filas enteras de interfaz intercaladas entre las de texto, de modo
  que `fila × alto de línea` sigue valiendo para todo el elemento y el texto
  no se corre.

## Desviaciones

Todas las anota el subagente responsable, con su motivo; ninguna cambia el
comportamiento visible descrito en la spec 09 más allá de lo aquí explicado.

- **Nombres de archivos de test distintos a los de la spec** (mismo contenido, otro archivo): chat `menu_dismiss_tests.rs` (spec: `popover_dismiss_tests.rs`), `attachments_tests.rs` (spec: `image_attach_tests.rs`), `comment_cards_tests.rs` (spec: `comment_tags_tests.rs`); workspace `review_comments_tests.rs` (spec: `comments_e2e_tests.rs` y `pending_review_close_comments_tests.rs`); review `tests/comments.rs` (spec: `src/comments_tests.rs`).
- **E7-A**:
  - El test de que "el chat no pinta correo, plan ni usado" es indirecto
    (altura de fila, selectores de depuración y una guarda sobre el código de
    `render.rs`), no una lectura directa del texto pintado.
  - `toggle_popover` conserva su firma (la spec le sumaba `window`): el foco se
    mueve al pintar (`sync_popover_focus`).
  - Los desplegables `Select` de Configuración no se probaron: gpui-kit ya
    cierra solo según su código.
  - El defecto de cierre existía en **todos** los desplegables del chat
    (no solo los dos del encabezado) y, además, `Ctrl+L` y la ventana inactiva
    no cerraban el menú de la barra de título ni el del árbol. Se corrigió
    desde Cincel, sin tocar gpui-kit (`menu_dismiss.rs`).
  - Los archivos de test del chat se llaman `menu_dismiss_tests.rs` y
    `connections_view_tests.rs` (la spec decía `popover_dismiss_tests.rs`).
- **E7-B**:
  - `add_comment` devuelve `Option<CommentId>` (texto vacío o archivo binario
    → `None`) en vez de `CommentId`.
  - `comment_buffer_event` recibe además el `BufferSnapshot`.
  - `SentComment` lleva `id`.
  - En un fragmento sin código actual, "({k} more lines not shown)" va después
    de las líneas quitadas, no después del código.
  - `STATE_VERSION` pasa a 2 con migración desde la 1.
- **E7-C**:
  - No existen el campo `ReviewView.comments` ni `EditorEvent::Comment`: el
    workspace construye `ReviewView` con literales y hace `match` exhaustivo
    sobre `EditorEvent`. El editor emite `CommentAction` como segundo tipo de
    evento (`EventEmitter<CommentAction>`), que el workspace recibe por
    `cx.subscribe`; los comentarios entran por `EditorView::set_comments`.
  - El menú contextual usa `PopupMenu` de gpui-kit directo, no el envoltorio
    `ContextMenu` que usa el árbol: este tiene un ciclo de referencias que
    dispara la detección de fugas.
  - `BlockMap::new` recibe las filas de ajuste (`BlockMap::new(wrap_rows,
    placements)`).
  - No se corrió `--bench typing` en la subetapa (la spec lo pedía para E7-C y
    E7-I): queda a cargo de la verificación del orquestador.
- **E7-D**:
  - `PromptBlock::Image { mime_type, data: Arc<[u8]> }` (la spec decía
    `Vec<u8>`).
  - Los tests de imágenes van en `crates/cincel-acp/tests/integration.rs`
    junto al resto de los de integración.
  - `codex-acp` no estaba descargado en la carpeta de agentes de Cincel: se
    leyó la caché de `npx` (1.13.1 y 1.12.0 anuncian imágenes). Claude 0.84.0 y
    Antigravity 1.2.1 también.
- **E7-E**:
  - GPUI descarta un `drop` si lo último que pasó fue una tecla: el test mueve
    el mouse antes de soltar. Probar el arrastre real en la lista manual.
  - Portapapeles en Wayland: la imagen llega si el origen no ofrece también
    texto (una captura de pantalla funciona). Los archivos copiados en el
    explorador llegan como texto `file://…` y se adjuntan igual.
  - `MentionFile` del árbol pasa por `mention_or_attach`: si es una imagen se
    adjunta en vez de mencionarse.
  - Los tests están en `attachments_tests.rs` y `comment_cards_tests.rs` (la
    spec decía `image_attach_tests.rs` y `comment_tags_tests.rs`).
- **E7-F**:
  - Los tests van en un solo archivo, `review_comments_tests.rs` (la spec
    pedía `comments_e2e_tests.rs` y `pending_review_close_comments_tests.rs`).
  - Los comentarios vuelven solo si el mensaje nunca salió: si el agente da
    error después de haberlo recibido, no vuelven (igual que los parches).
  - El editor no sabe qué es un binario: `center.rs` bloquea el atajo y el
    clic derecho en las pestañas binarias.
  - El agente falso mueve un archivo preparado para cambiar el texto por
    turno; los reemplazos por script se hicieron en archivos no compartidos
    con otras subetapas.
- **E7-G**:
  - Sin ajustes de imágenes en `defaults.rs`: la spec no define ninguno. Sí se
    agregó la sección `ImageViewer` (`escape` → `workspace::close_image_viewer`)
    al keymap y su descripción en el modal de atajos.
  - `MentionFile` se maneja en `workspace.rs`.
  - `OpenImage` llega al visor por una suscripción del `Workspace` al chat.
  - Al enviar con imágenes la conversación se guarda en el momento (para que
    sus archivos existan), no en el próximo autoguardado.
  - Deshacer el borrado de una conversación restaura también sus imágenes.
  - `image` es dev-dependency de `cincel-workspace`, solo para generar
    imágenes de test.
- **E7-H**:
  - El árbol no tiene arrastre real (gpui-kit no ofrece fuente de arrastre,
    ver la desviación de la Etapa 2): "Mencionar en el chat" del menú
    contextual adjunta la imagen si el archivo lo es.
  - Arrastrar una imagen entre ventanas en Wayland depende del escritorio
    (COSMIC); no se probó, va a la lista manual.
- **Orquestador (E7-I)**:
  - Dos tests de tiempo pasaron a `#[ignore]` y a `slow-tests.yml`: el
    buscador con 50 000 rutas (`ranks_fifty_thousand`) y el bloqueo del repaso
    en fondo (`the_sweep_of_five_thousand_files_never_blocks`, ≤ 8 ms). Fallan
    solo bajo carga de compilaciones concurrentes; se corren aparte con la
    máquina tranquila y pasan.
  - `updating_node_from_the_settings_tab_completes` esperaba el resultado del
    hilo de instalación antes de tiempo; se corrigió el orden de las esperas
    (misma clase de falla que la de la publicación de la 0.1.0: un hilo real
    exige esperar con tope).

## Verificación

Máquina de referencia (Pop!_OS 24.04, COSMIC Wayland), 2026-09-30, siempre
con `--features cincel-editor/test-support,cincel-workspace/test-support,
cincel-chat/test-support` en los crates con GPUI:

| Comando | Resultado |
|---|---|
| `cargo fmt --all --check` | ok, limpio |
| `cargo clippy --workspace --all-targets --features … -- -D warnings` | ok, 0 avisos |
| `cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins` + `cargo test --workspace --features …` | ok: tests del workspace completos en verde |
| `cargo deny check licenses advisories bans sources` | ok |
| `cargo run -p cincel -- --smoke-test .` | ok |
| `cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .` (control de fugas) | ok |
| `cargo test … ranks_fifty_thousand -- --ignored` (máquina tranquila) | ok |
| `cargo test … the_sweep_of_five_thousand_files_never_blocks -- --ignored` (máquina tranquila) | ok |

Los dos tests `#[ignore]` quedaron sumados a `.github/workflows/slow-tests.yml`
junto al de la descarga lenta.

Pasos de §12 de la spec que corresponden a E7-J y no figuran en la tabla (los
anota el orquestador al correrlos, junto con la versión 0.2.0 y el
`CHANGELOG.md`): compilación de release con `--smoke-test --bench`,
`--bench typing` (M4 no puede empeorar con los bloques del `BlockMap`),
`--bench idle` (M7/M8 con imágenes en memoria), `packaging/build.sh` +
`packaging/verify.sh` en `ubuntu:22.04` y `24.04`, `tools/privacy-check.sh`
sobre el árbol y sobre los paquetes, y `pgrep -af "sleep|cincel"; docker ps`
vacíos.

## Lista de comprobación manual (autor)

Tal como la deja `docs/specs/09-etapa7-conexiones-imagenes-comentarios.md`
§11, para recorrerla con el paquete que deja E7-J en `dist/` (o con
`cargo run -p cincel --release`):

1. **Conexiones**: abrir "Conectar": cada fila muestra solo icono, nombre,
   tipo y estado; el botón de arriba igual. Configuración → Conexiones sigue
   mostrando correo, plan y "Usado hace…".
2. **Menús**: abrir el menú de conexiones y hacer clic en el editor → se
   cierra. Abrirlo y apretar `Ctrl+L` → se cierra. Abrirlo y `Esc` → se
   cierra. Abrirlo y pasar a otra ventana → se cierra. Repetir con el
   historial (el reloj), con el selector de modelo y con el menú de la barra
   de título.
3. **Imágenes, tres vías**: hacer una captura (`Impr Pant` copia al
   portapapeles) y `Ctrl+V` en el chat; arrastrar un PNG desde el explorador
   de archivos a la caja; botón del clip y elegir un JPEG. Quitar una con la
   `×`.
4. **Imagen grande**: adjuntar una foto de más de 2 000 px; enviarla a Claude
   preguntando "¿qué tamaño tiene esta imagen?" (debería decir 2 000 de lado
   o menos).
5. **Ver y reabrir**: clic en la miniatura del mensaje enviado → se abre
   grande; `Esc`. Abrir otra conversación y volver a esta desde el historial:
   las imágenes siguen.
6. **Repetir 3 y 5 con Codex y con Antigravity** (si alguno avisa "no acepta
   imágenes", anotarlo).
7. **Comentarios con Claude**: pedirle un cambio en dos archivos. En un
   segmento: "Comentar", escribir "esto hacelo con un diccionario",
   `Ctrl+Enter`. Rechazar otro segmento y comentarlo. Seleccionar tres líneas
   que el agente no tocó, clic derecho → "Comentar selección", escribir algo.
   Ver las tres etiquetas en la caja del chat y "3 comentarios" en la barra de
   estado y el árbol. Quitar una con la `×`. Escribir "atendé mis
   comentarios" y enviar: el mensaje muestra dos tarjetas; las marcas del
   margen desaparecen. Comprobar que **Claude hace lo que pedían las notas** y
   que el segmento comentado se actualiza.
8. **Lo mismo que 7 con Codex y con Antigravity** (comprobación obligatoria:
   que entiendan y atiendan la nota).
9. **Cerrar con comentarios**: dejar un comentario sin enviar, cerrar Cincel
   (si hay cambios pendientes, el diálogo menciona el comentario), reabrir: el
   comentario vuelve en su lugar.
10. **Nada se mueve**: abrir y cerrar una caja de comentario mirando el margen
    y el código: no se corren a los costados.
11. `cincel --version` dice `cincel 0.2.0`; instalar el `.deb` (o el
    comprimido) y repetir 3 y 7 rápido.
12. Leer `docs/usuario/revision.md` (comentarios) y la sección de imágenes
    (`docs/usuario/chat.md`); si algo no se entiende, avisar.
13. Commit, push, etiqueta `v0.2.0` y publicar el borrador. Procedimiento
    validado en `docs/etapas/etapa-6.md` ("Publicación"): push a `main` → CI en
    verde → `git tag -a v0.2.0 -m "Cincel 0.2.0"` + `git push origin v0.2.0` →
    el workflow Release arma y verifica los paquetes y deja un **borrador** →
    instalar el `.deb` del borrador y publicarlo. Esta etapa no cambió los
    workflows.

Cosas que los tests no pueden probar y que esta lista cubre: el arrastre real
de archivos desde el explorador en COSMIC/Wayland (los tests simulan
`FileDropEvent`, y GPUI descarta un `drop` si lo último fue teclado), el
portapapeles real de una captura de pantalla, el diálogo de archivos del
sistema (los tests llaman a la función que recibe sus archivos) y que los tres
agentes reales entiendan y atiendan un comentario y una imagen.

## Ronda 2 (correcciones y mejoras tras la prueba del autor)

Estado: **cerrada** (2026-10-01). Spec: `docs/specs/10-etapa7-ronda2.md`
(v1.0). Entra en la **misma versión 0.2.0**, que todavía no se había
publicado. El autor probó la 0.2.0 de la ronda 1 y pidió diez cosas (A–J de
la spec); se construyeron en cinco olas por ocho subagentes en el mismo árbol
(sin worktrees, `Edit` en los archivos compartidos, tests nuevos en archivos
nuevos) y el orquestador verificó el conjunto (R2-I) y cerró (R2-J).

### Qué se construyó

- **R2-A: encabezado, menú e imágenes** (`cincel-chat`). El botón del
  encabezado muestra el nombre completo de la conexión (se corta con "…" y
  tooltip solo si no entra) y **una** insignia con el estado de la
  **conexión**: "conectada", "desconectada", "sesión vencida", "no
  disponible" (motivo en tooltip) o "autenticación requerida"
  (`HeaderBadge`, `ChatPanel::header_badge()`, prioridad vencida > no
  disponible > autenticación > desconectada > conectada). La insignia no
  cambia con la actividad del agente (decisión del autor). El menú de
  conexiones pasó a filas de dos líneas: icono, nombre arriba, tipo abajo
  (siempre), insignia a la derecha; sin correo, plan ni "Usado hace…". En el
  mensaje enviado las imágenes van arriba del texto y las tarjetas de
  comentario debajo (`render_message_images` / `render_message_cards`).
  Tests: `header_badge_tests.rs`, `connection_rows_tests.rs`,
  `message_media_order_tests.rs`.
- **R2-B: fila de actividad y seguimiento del final** (`cincel-chat`). Al
  pie de la conversación, mientras el turno está en curso, una fila con la
  ruedita dice "Pensando…", "Trabajando…" (herramienta en curso),
  "Escribiendo…" (llega la respuesta) o "Esperando permiso…"
  (`AgentActivity`, `ChatPanel::activity()`, selector `chat-activity-row`);
  no se duplica con la fila de pensamiento en vivo y no se guarda. La vista
  sigue el final con el `FollowMode::Tail` de la lista de GPUI: `push` ya no
  llama `scroll_to_end()` (era la causa del salto al terminar el turno); solo
  abrir una conversación, empezar una nueva, enviar un mensaje y la flecha
  vuelven al final y reactivan el seguimiento. Botón flotante circular "Ir al
  final" (`chat-jump-to-end`) visible solo cuando el usuario subió y hay más
  abajo. Tests: `activity_row_tests.rs`, `transcript_follow_tests.rs` (con el
  salto viejo repuesto a propósito, el test falla).
- **R2-C: bloques de código largos plegados** (`cincel-chat`). `code_folds.rs`
  parte el Markdown de una respuesta (o de un trozo de burbuja) en segmentos:
  prosa y bloques cercados de nivel superior con más de 20 líneas
  (`split_markdown`, `SegmentKind`, `CodeBlockKey`). Cada segmento es su
  propio `TextViewState` de gpui-kit (`AgentText.segments` reemplaza a
  `view`); un bloque largo plegado muestra 12 líneas con un difuminado y un
  pie "Ver más (N líneas)" / "Ver menos"; el estado se recuerda mientras la
  conversación está abierta (`expanded_code_blocks`); se pliega también
  mientras llega; "Copiar" copia el bloque entero. El JSON de la conversación
  no cambió. Tests: unitarios en `code_folds.rs` y `code_fold_tests.rs`.
- **R2-D: margen al final y E7** (`cincel-editor`, `cincel-settings`,
  `cincel-workspace`). `EditorView::max_scroll_top` suma media pantalla
  (`floor(filas_visibles / 2)` filas) solo en `EditorChrome::Full`; lo usan la
  rueda, la barra (pulgar incluido), `set_scroll_row`, `scroll_rows` y el
  centrado de "ir al cambio". Acciones nuevas `editor::scroll_line_up` /
  `editor::scroll_line_down` (`Ctrl+↑` / `Ctrl+↓`, sin mover el cursor) y
  atajo `Ctrl+Alt+W` para `editor::toggle_whitespace`, en
  `default_key_bindings`, `DEFAULT_KEYMAP_JSONC`, el modal de atajos y
  `docs/usuario/atajos.md`. Tests: `scroll_margin_tests.rs`,
  `whitespace_toggle_tests.rs`.
- **R2-E: portapapeles de línea y sangría** (`cincel-editor`, solo `Full`).
  `Ctrl+C` / `Ctrl+X` sin selección copian o cortan la línea entera con su
  salto (marca JSON `whole_line` + `LineClipboard` global) y `Ctrl+V` la pega
  como línea arriba de la del cursor; `}` `]` `)` en una línea de solo
  sangría quitan un nivel (`outdent_for_closer`, `ops::outdent_once`);
  `Enter` en una línea de solo sangría la deja vacía
  (`newline_on_indent_row`); `Backspace` dentro de la sangría borra hasta el
  tope anterior (`backspace_in_indent`, `ops::spaces_to_previous_stop`).
  Tests: `line_clipboard_tests.rs`, `indent_editing_tests.rs`.
- **R2-F: apariciones de la palabra** (`cincel-editor`). 150 ms después de
  detenerse, las demás apariciones de la palabra bajo el cursor (palabra
  completa, con mayúsculas) o de la selección de una fila se pintan con el
  color `selection` al 50 % (`OCCURRENCE_ALPHA`), por el mismo camino que las
  coincidencias de búsqueda; no con la búsqueda abierta, ni en solo lectura,
  ni en `Minimal`; temporizador de un solo disparo (en reposo no queda nada
  corriendo). `search.rs::OccurrenceQuery` / `find_occurrences`. Test:
  `occurrences_tests.rs`. Medido con `--bench typing` antes y después: sin
  cambio medible (ver "Verificación").
- **R2-G: limpieza al guardar** (`cincel-settings`, `cincel-editor`,
  `cincel-workspace`). Claves `files.trim_trailing_whitespace_on_save` y
  `files.ensure_final_newline_on_save` (`true`), con sus filas en
  Configuración → Archivos. `Center::save_path` llama
  `EditorView::clean_up_for_save` antes de escribir, en todas las vías de
  guardado, como una sola transacción `User` (un `Ctrl+Z` la deshace);
  no toca las filas de segmentos pendientes del agente, no recorta espacios
  en `.md`/`.markdown`, y no hace nada con un turno en curso sobre el
  archivo. Tests: `ops.rs` (`save_cleanup_edits`), `save_cleanup_tests.rs`.
- **R2-H: documentación de usuario**. `docs/usuario/editor.md` (nuevo,
  "Escribir en el editor"), secciones nuevas en `chat.md` y `conexiones.md`,
  referencias en `atajos.md`, `ajustes.md`, `docs/usuario/README.md` y el
  `README.md` raíz; `user_docs_tests.rs` ahora revisa también `chat.md` y
  `editor.md`.
- **J, arreglado por el orquestador antes de la ronda**: al retomar una
  conversación (`session/load`) las menciones `@archivo` volvían como un
  mensaje suelto "@a@b@c"; `ChatPanel::is_already_shown` descarta los
  `UserMessageChunk` con `ResourceLink`/`Resource` durante la reproducción
  (test `a_replayed_file_mention_is_not_shown_as_a_new_message`). **Segunda
  parte (2026-10-01, tras la prueba del autor)**: el adaptador de Claude
  convierte cada mención en el **texto** `[@nombre](file://…)` antes de
  guardar el prompt (`formatUriAsLink` en `promptToClaude`) y lo reproduce
  así, no como `ResourceLink`; además las versiones anteriores ya habían
  guardado esa burbuja en el archivo de la conversación. Ahora
  `is_already_shown` reconoce un fragmento de usuario hecho solo de esos
  enlaces cuando sus archivos están como bloques `File` de un mensaje
  guardado (`panel::mention_links_only`), y `load_conversation` descarta las
  burbujas de esa forma que quedaron guardadas (`is_stray_mention_bubble`).
  Tests: `a_replayed_mention_written_as_a_text_link_is_not_shown_as_a_new_message`,
  `a_stored_stray_mention_bubble_is_dropped_when_the_conversation_is_loaded`,
  `mention_links_only_accepts_only_the_adapter_shape`.

### Decisiones

Las R1–R17 de la spec 10 §2 se respetaron tal cual. Las que decidió el autor
durante la definición y no se reabren: la insignia del encabezado muestra solo
el estado de la conexión y la actividad va abajo (R1, R2); el tipo se muestra
siempre en las filas del menú (R3); el recorte de espacios al guardar no se
aplica a Markdown (R16); enviar un mensaje vuelve a seguir el final (R6). Se
corrigió en la spec la decisión R5, que había quedado contradiciendo §8 (la
fila sí se muestra, como "Escribiendo…", mientras llega la respuesta), y ocho
frases del resumen, los textos y la lista manual que todavía describían la
versión anterior del encabezado ("listo", "escribiendo…" arriba).

### Desviaciones (todas anotadas por el subagente que las tomó)

- **R2-A**: `agent_type_label` se eliminó (quedó sin uso; el workspace usa
  `visible_agent_type`). Selectores de prueba extra: `chat-header`,
  `chat-input`, `connection-icon-{i}`. `icon_button` ganó `flex_shrink_0`
  (a 280 px los botones del encabezado se encogían a 13 px). Los tooltips no
  se pueden leer en el arnés: se prueban por `header_badge()` y por el
  código fuente. Si `agent_name` viene vacío (lista guardada antigua) la
  segunda línea usa el nombre del proveedor.
- **R2-B**: cuando `is_scrolled_to_end()` devuelve `None` (alturas sin medir
  mientras llegan entradas con la vista subida), `shows_jump_to_end()`
  calcula lo mismo con las alturas conocidas; sin eso la flecha desaparecía
  justo cuando el agente escribe. La flecha tiene desvanecido de entrada, no
  de salida (previsto en §15).
- **R2-C**: sin desviaciones. Límite: una línea de nivel superior con cuatro
  o más espacios y ``` se trata como valla con sangría (no como código
  indentado); sin cierre propio podría impedir plegar un bloque posterior.
- **R2-D**: un archivo con menos de media pantalla de filas no se desplaza
  (su última línea ya está arriba de la mitad; la fórmula de §6.2 es la
  correcta y la frase de §6.1 sobre "archivo corto" vale desde media pantalla
  de filas). El clic debajo del texto sigue poniendo el cursor en la columna
  más cercana a la `x` del clic (conducta de siempre).
- **R2-E**: `cincel-editor` suma la dependencia `serde` (la marca JSON del
  portapapeles). La marca se lee de `ClipboardString::metadata_json`. En E4
  cuentan como blancos después del cursor espacios y tabs. En E5, con el
  cursor en la columna 0 de una fila de solo sangría, las dos filas quedan
  vacías (la sangría nueva es la anterior al cursor). En E6, con unidad de
  tab, el tope es de un carácter. Un portapapeles marcado pero sin `\n`
  final se pega como texto normal.
- **R2-F**: las marcas se suprimen en cualquier buffer de solo lectura (el
  editor no distingue el aviso de binario de la vista de un borrado). El
  estado lleva una clave (versión del texto, cursor, ancla) para que una
  edición del agente también borre las marcas en el mismo cuadro. Al abrir un
  archivo no hay marcas hasta el primer movimiento.
- **R2-G**: si la última fila está dentro de un segmento pendiente y falta el
  salto final, no se agrega (sería tocar una fila protegida). Durante la
  limpieza se pone a cero el intervalo de agrupación de deshacer de
  `cincel-text`, para que un `Ctrl+Z` no mezcle el tecleo previo con la
  limpieza. `selection_offsets` pasó a `pub(super)`.
- **Orquestador**: `tools/privacy-check.sh` encontró en
  `crates/cincel-connections/tests/login.rs` un correo de prueba en
  `example.test` (dominio reservado, pero fuera de la lista del script);
  pasó a `example.com`. El mismo script, en modo paquete, salta ahora
  `THIRD-PARTY-LICENSES.html` (licencias de terceros reproducidas tal cual,
  con los correos de sus autores); el resto de lo que marca en los paquetes
  está en la tabla de verificación.

Tests existentes ajustados (con `Edit`): cuatro de `connections_view_tests.rs`
(el botón ya no pinta tipo ni insignia; las filas son de dos líneas; el tipo
se pinta siempre), una aserción de `attachments_tests.rs` (miniaturas arriba
del texto), `review_tests::a_hunk_at_the_end_scrolls_as_far_as_the_document_allows`
→ `a_hunk_at_the_end_is_centred_with_the_end_margin`, cuatro usos de
`AgentText.view` en `cincel-chat/src/tests.rs` y uno en
`cincel-workspace/src/agents.rs` (→ `segments`), y las listas de
`user_docs_tests.rs`.

### Verificación (R2-I)

Máquina de referencia (Pop!_OS 24.04, COSMIC Wayland), 2026-10-01, siempre
con `--features cincel-editor/test-support,cincel-workspace/test-support,
cincel-chat/test-support` en los crates con GPUI:

| Comando | Resultado |
|---|---|
| `cargo fmt --all --check` | ok, limpio |
| `cargo clippy --workspace --all-targets --features … -- -D warnings` | ok, 0 avisos |
| `cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins` | ok |
| `cargo test --workspace --features …` | ok: 1658 pasados, 0 fallos, 6 ignorados (los de tiempo, en `slow-tests.yml`); los tres últimos son los del arreglo de las menciones reproducidas, agregados tras la prueba del autor, con los paquetes reconstruidos después |
| `cargo deny check licenses advisories bans sources` | ok |
| `cargo run -p cincel -- --smoke-test .` | ok |
| `cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .` (control de fugas) | ok |
| `cargo build --release -p cincel && target/release/cincel --smoke-test --bench .` | ok |
| `--bench typing` (`cinco-mil.rs`, 200 teclas, dos corridas) | p50 6,8 / 16,5 ms; p95 17,3 / 23,4 ms; máx. 23,3 / 24,6 ms (misma franja que antes de la ronda: R2-F midió 17–24 ms de p95 antes y después) |
| `--bench idle` (60 s) | 0 cuadros pintados, 0,017 % de CPU, 207 MB |
| `tools/privacy-check.sh` (árbol) | ok (0) tras la corrección de `login.rs`; historial informativo como siempre |
| `tools/privacy-check.sh` sobre los dos paquetes desempaquetados | sin nada del autor ni de la máquina. Quedan tres hallazgos esperados, no corregibles desde el repo: la firma `appro@openssl.org` del código de cifrado CRYPTOGAMS dentro del binario (viene de la dependencia de TLS; la 0.1.0 publicada ya la tenía), el `copyright` del `.deb` con el `Maintainer` que el autor puso en su `packaging/maintainer.txt` (no versionado, decisión suya de la Etapa 6) y, hasta hoy, los correos de los autores de las licencias de terceros en `THIRD-PARTY-LICENSES.html`, que el script ahora salta porque ese archivo se reproduce tal cual por exigencia de las licencias |
| `tools/check-links.sh` + tests `user_docs` | ok |
| `packaging/build.sh` + `packaging/verify.sh` (Ubuntu 22.04 y 24.04) | ok: `.deb` y `.tar.gz` 0.2.0 reconstruidos en `dist/` (2026-10-01), `verify.sh: todo bien` en 22.04 y 24.04; el smoke en Wayland (banco sway) se omite como siempre porque `cincel-perf` y su imagen no están en esta máquina |
| `pgrep -af "sleep\|cincel"; docker ps` | vacíos (en `pgrep` solo quedó el Cincel instalado del autor con su agente, anterior a la sesión) |

### Lista de comprobación manual (autor)

Tal como la deja la spec 10 §16, para recorrerla con el paquete de `dist/`
(o con `cargo run -p cincel --release`):

1. **Encabezado**: con una conexión activa, arriba del chat se ve el nombre
   completo y una sola etiqueta ("conectada"); no se ve "Conectada" ni el
   tipo. Mandá un mensaje: la etiqueta sigue diciendo "conectada" todo el
   turno (no cambia con lo que hace el agente). Cerrá el agente por fuera (o
   dejá vencer la sesión) y la etiqueta pasa a "desconectada" o "sesión
   vencida".
2. **Menú**: abrí "Conectar": cada fila tiene el icono a la izquierda, tu
   nombre arriba, el tipo (Claude/Codex/Antigravity) abajo en gris y la
   etiqueta de estado a la derecha; sin correo ni plan.
3. **Imágenes**: mandá un mensaje con texto y una imagen: en el mensaje
   enviado la imagen está arriba del texto. En la caja, al adjuntar, la
   miniatura aparece arriba de donde escribís.
4. **Actividad al pie**: al mandar un mensaje, abajo de todo en la
   conversación aparece "Pensando…" girando; pasa a "Trabajando…" cuando el
   agente usa una herramienta, a "Escribiendo…" cuando empieza la respuesta
   y a "Esperando permiso…" si pide uno; desaparece al terminar. Arriba no
   cambia nada.
5. **No te mueve mientras leés**: pedile al agente algo largo; mientras
   escribe, subí con la rueda a leer: la vista se queda quieta, también
   cuando termina. Aparece la flecha redonda abajo; tocala: baja al final y
   vuelve a seguir. Repetí bajando con la rueda hasta el final en vez de
   tocar la flecha.
6. **Bloques largos**: pedile "mostrame un archivo de 60 líneas en un bloque
   de código": se ve plegado a 12 líneas con difuminado y "Ver más (48
   líneas)". Tocalo: se despliega y dice "Ver menos". "Copiar" pega las 60
   líneas en el editor.
7. **Margen final**: abrí un archivo largo, bajá hasta el final: la última
   línea puede quedar a media pantalla; con un cambio del agente pendiente al
   final del archivo, la barra flotante ya no tapa las últimas líneas.
8. **Apariciones**: poné el cursor sobre una variable: un instante después
   se marcan suavemente sus otras apariciones visibles. Seleccioná un trozo
   de una línea: se marcan sus repeticiones.
9. **Copiar la línea**: sin seleccionar nada, `Ctrl+C` en una línea, bajá a
   otra y `Ctrl+V`: la línea entra entera arriba de la del cursor. `Ctrl+X`
   sin selección corta la línea; `Ctrl+Z` la devuelve.
10. **Sangría**: en un archivo con sangría de 4, después de `{` y `Enter`,
    escribí `}` en la línea vacía: queda alineado con la línea de la `{`. En
    una línea solo con sangría, `Enter`: la línea de arriba queda sin
    espacios. `Backspace` en la sangría borra 4 espacios de una vez.
11. **`Ctrl+↑`/`Ctrl+↓` y espacios**: `Ctrl+↓` varias veces: el texto se mueve
    y el cursor no. `Ctrl+Alt+W`: se ven los espacios en blanco en esa
    pestaña; otra vez, se ocultan.
12. **Guardar limpio**: escribí "hola   " (con espacios al final) en la
    última línea, sin `Enter`, y guardá: los espacios se van y el archivo
    termina con un salto de línea (`cat -A archivo` muestra `hola$`). `Ctrl+Z`
    lo devuelve. En Configuración → Archivos apagá las dos opciones nuevas,
    repetí: se guarda tal cual. Volvé a encenderlas.
13. **Retomar una conversación**: con una conversación donde mencionaste
    archivos con `@`, cerrá Cincel, abrilo y retomala desde el historial: no
    aparece un mensaje suelto "@a@b@c".
14. `cincel --version` dice `cincel 0.2.0`; instalar el `.deb` (o el
    comprimido) y repetir 1, 5 y 12 rápido.
15. Leer `docs/usuario/editor.md` y las partes nuevas de
    `docs/usuario/chat.md`; si algo no se entiende, avisar.
16. Commit, push, etiqueta `v0.2.0` y publicar el borrador (procedimiento de
    la lista de la ronda 1, punto 13, sin cambios). Recordatorio: el borrador
    de la **v0.1.0** también sigue sin publicar.

## Después de la 0.2.0

- **Etapa 8: terminal integrado** (decisión del autor, 2026-09-30). Pospuesto;
  tendrá su propia spec, todavía sin escribir.
- **Límite conocido de las imágenes**: un modelo de Codex que no acepta
  imágenes responde con error ("The current model does not support image
  input") aunque el adaptador las anuncie; el error se muestra como cualquier
  error del turno.
- **Ideas surgidas al escribir la spec y no acordadas (no entraron)**: listar
  los comentarios en el panel "Revisar todo"; recodificar los GIF animados
  reducidos conservando la animación (hoy se manda el primer cuadro, con
  aviso).
- **Arrastrar desde el árbol**: el árbol de Cincel no tiene arrastre real
  (gpui-kit no ofrece fuente de arrastre); si lo gana, soltar una imagen del
  árbol sobre la caja debería adjuntarla como hoy lo hace "Mencionar en el
  chat".
- Lo anotado en `docs/etapas/etapa-6.md` "Después de la 1.0" sigue abierto:
  los colores de git de `GitGutterColors` a `EditorTheme`, el residuo de
  ≈10 ms del escenario combinado de M11, adoptar el cambio de un solo archivo
  grande sin bloquear, el tamaño del binario, `tools/perf/run.sh cosmic` y
  medir el buscador sobre el corpus grande.
- Sin fecha, de `docs/specs/05-plan-etapas.md` "Después de v1": LSP,
  autenticación de agentes dentro de la app, multi-cursor y plegado,
  búsqueda en proyecto, paleta de comandos, OpenCode y agentes con API key o
  gateway, panel de git, ACP v2 y macOS.
