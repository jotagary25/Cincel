# Módulo `cincel-workspace` y binario `cincel`

## `cincel-workspace`
- `Workspace` (entidad raíz): `Project`, `BufferStore`, `ReviewStore`, `ChatPanel`, `Dock` con tres paneles (izquierda chat, centro pestañas, derecha árbol) usando el dock de `gpui-kit`; tamaños y colapsado persistidos en `layout.json`.
- **Pestañas**: lista con título, indicador sucio (●), indicador de revisión (`+N −M` en pequeño), previsualización (cursiva) vs. fijada; cerrar con `Ctrl+W` pregunta si hay cambios sin guardar; clic medio cierra; reordenar arrastrando (v1 opcional).
- **Vista previa de Markdown** (`workspace::toggle_markdown_preview`, `Ctrl+Shift+V`, solo con una pestaña de Markdown activa): alterna el cuerpo de la pestaña entre el editor de código y un renderizado del texto *actual* del buffer, reusando el renderizador del chat (`cincel_chat::markdown`) con su propio resaltado de bloques de código; se actualiza sola 150 ms después de cada cambio del buffer. El título, el punto de sucio y `Ctrl+S` siguen atados al buffer de siempre; la barra de estado muestra "Vista previa" en lugar de `Ln, Col`. Un botón con icono al final del breadcrumb hace lo mismo con el mouse. Cada pestaña recuerda si estaba en vista previa en `layout.json` y la restaura al reabrir el proyecto.
- **Árbol**: componente árbol de `gpui-kit` alimentado por `Worktree`; iconos por tipo; colores de estado del agente (`02-visual.md §6.4`) y de git (v2); clic simple previsualiza, doble fija; menú contextual con "Revelar en carpeta", "Copiar ruta", "Copiar ruta relativa"; teclado ↑↓←→ y `Enter`.
- **Barra de estado**: posición, EOL, lenguaje, chip de la conexión activa (icono del proveedor + etiqueta, o "Sin conexión"; clic abre el popover "Conectar" del chat y despliega el dock si estaba colapsado), pendientes (clic abre panel de revisión), zoom.
- **Conexiones** (Etapa 4, `docs/specs/06-etapa4-conexiones-y-cincel.md` §1–§4, §6; motor en `modulos/connections.md`): `Agents` es dueño de `Connections` (índice, perfiles, runtime privado, adaptadores) y de `ConnectionsModal`.
  - **Al arrancar no se conecta nada**: se lee `connections.json` para llenar el popover y nada más; no se lanza ningún proceso ni se lee `~/.claude`, `~/.codex` o `~/.gemini`, y el registry ACP ya no se descarga al arrancar (solo al preparar un agente desde el modal). `connections.default_label` solo preselecciona una fila.
  - **Elegir una conexión** (F1, `Agents::activate_connection`): se guarda la conversación en pantalla, se detiene el proceso de la anterior (nada se borra), `Connections::agent_launch` → `AgentConnection::start_with_env(raíz, env)` + `Spawn`, `touch`, y se abre una conversación vacía con `connection_id`. Una conexión con la sesión vencida o no disponible muestra su banner en vez de lanzar.
  - **Modal** (`connection_modal.rs`; `bg.elevated`, radio 6, borde 1 px, scrim 40 %, botones neutros iguales con anillo de foco en el de `Enter`, cursor de mano; contexto de teclas `ConnectionsModal`, `Enter`/`Esc`): "Conectar nuevo agente" (F2: Elegí un agente → Preparando… con dos barras alimentadas por `RuntimeProgress`/`AdapterProgress` → Iniciando sesión… → Abrí este enlace y aprobá el acceso, con el enlace en monoespaciada, "Copiar", "Abrir en el navegador" (`cx.open_url`), el código de un solo uso si llega `CodeDetected`, el campo "Pegá acá el código que te dio el navegador" + "Enviar" con `NeedsPastedCode`, "Ver detalles técnicos" con la salida ya redactada → Esperando que apruebes en el navegador… con spinner y "Cancelar" → Listo: "Listo: conectado como … (…)" o "Conectado" y "Nombre de la conexión" con `suggest_label`; "Guardar" → `finalize` y activación); errores con mensaje claro y "Reintentar"; `Esc` pide confirmación con un inicio de sesión en curso o una conexión sin guardar. "Volver a conectar" (F4, `start_relogin`, desde el paso 3, conserva etiqueta y conversaciones), "Reparar" (F5, `prepare` sin tocar credenciales), "Eliminar conexión" (F3: lista → "¿Eliminar «X»? Se cerrará la sesión, se borrarán sus credenciales de este equipo y sus N conversaciones." → se detiene su proceso si era la activa → `disconnect` → se borran sus conversaciones de todos los proyectos → resultado por paso si algo falló; nota "Para revocar el acceso, hacelo desde tu cuenta de Google" para Google Antigravity) y "Renombrar". Antigravity: "Preparando…" con una sola barra (el binario oficial, MB y porcentaje, nota de que Google no publica suma de verificación) y login por ACP `authenticate` sin campo de código.
  - **Eventos de autenticación**: `AuthRequired`, `AuthFailed` (toast + aviso en el chat, nunca se traga), `LoggedOut { ok: true }` y `AuthStatus { kind: none }` marcan la conexión como "Sesión vencida" (aunque el archivo de credenciales siga ahí); `AuthSucceeded` la limpia; `AuthStatus` con cuenta guarda la identidad con `set_identity`; `ElicitationCompleted` llega al chat como aviso.
  - **Conversaciones**: `Conversation.connection_id` e `IndexEntry.connection_id`; historial agrupado por la etiqueta actual; las viejas quedan bajo "(conexión anterior)" y se abren de solo lectura (no se reescriben). Abrir una conversación de otra conexión cambia a esa conexión.
- **Panel de revisión**: `02-visual.md §6.3`.
- **Avisos**: cola de toasts.
- **Comandos globales**: `workspace::open_folder`, `toggle_chat`, `toggle_tree`, `focus_chat`, `next_change`, `prev_change`, `next_file_with_changes`, `accept_turn`, `reject_turn`, `undo_last_reject`, `open_review_panel`, `zoom_*`.
- **Keymap**: carga `keymap.json` del usuario sobre el predeterminado; formato `[{ "context": "Editor && review_hunk_under_cursor", "bindings": { "ctrl-enter": "review::accept_hunk" } }]` (mismo formato que Zed para que sea familiar).
- **Ciclo de revisión**: al `TurnEnded`, para cada archivo tocado en el turno: releer disco, actualizar buffer, `ReviewStore::file_written`; si el archivo no estaba abierto, abrirlo en un buffer sin pestaña; actualizar árbol y barra. Al recibir `FsWrite`: `BufferStore::apply_agent_write` + `ReviewStore`. Al recibir un `tool_call` con `locations` y `kind: edit` la primera vez para un path: `capture_base` desde el buffer/disco.
- **Diálogo de buffer sucio** antes de que el agente escriba un archivo con cambios sin guardar: Guardar / Descartar / Mantener (mantener = el agente escribe encima y el hunk incluirá tus cambios).

## Binario `cincel`
- CLI: `cincel [ruta] [--log-level ..]`. Sin ruta: último proyecto o pantalla vacía.
- Arranque: migrar directorios XDG de `asteroid` a `cincel` si hace falta (`docs/specs/06-etapa4-conexiones-y-cincel.md` §7) → iniciar log a archivo (`cincel-log`) → cargar settings/keymap/tema → crear app GPUI → ventana con tamaño/posición restaurados → `Workspace` → toast de migración si hubo alguna.
- GPU: intentar Vulkan; si falla, GL; si falla y `CINCEL_ALLOW_SOFTWARE_GPU=1`, software; si no, mensaje y salida con código 2.
- Recarga en caliente de settings/keymap/tema al cambiar los archivos.
- Una sola instancia por proyecto no es requisito en v1.

## Criterios de aceptación
- [ ] `cincel .` abre la ventana con el árbol del proyecto en < 400 ms en la máquina de referencia.
- [ ] Cerrar y reabrir restaura pestañas, tamaños de paneles y posición del scroll de cada pestaña.
- [ ] `Alt+J` recorre todos los hunks pendientes de todos los archivos abriendo pestañas según haga falta, con wrap.
- [ ] Editar `settings.json` cambia el tamaño de fuente sin reiniciar.

## Desviaciones (Etapa 2, integración del chat)

- **"Mencionar en el chat" en vez de arrastrar**: `gpui-kit` 0.6.1 no expone
  una fuente de arrastre en su árbol virtualizado (`tree()`/`TreeItem`), a
  diferencia de las pestañas del centro, que arrastran con primitivas `div`
  crudas (`on_drag`/`drag_over`/`on_drop`). El menú contextual del árbol
  ganó el ítem "Mencionar en el chat" (`workspace::mention_in_chat`), que
  hace exactamente lo que haría soltar el archivo: inserta el chip vía
  `ChatPanel::insert_mention`.
- ~~**El indicador de actividad del agente en una pestaña es un sufijo "◆"**~~
  Resuelto en la Etapa 3: la pestaña muestra `+N −M` de la revisión.
- **`ChatSettings` no tiene sección propia en `settings.json`**: ninguno de
  sus campos (filas del input, líneas de contexto del diff, …) tiene
  contraparte en `cincel_settings::Settings` todavía, así que
  `theme::chat_settings` siempre proyecta los valores por defecto del crate;
  el cableado de recarga en caliente ya está listo para cuando esa sección
  se agregue.
- Detalle completo (incluida la razón de cada una) en
  `docs/etapas/etapa-2.md`, secciones "Desviaciones" y "Deseos".

## Desviaciones (Etapa 3, revisión)

Detalle y motivos en `docs/etapas/etapa-3.md`, sección "Desviaciones":

- La relectura del disco entra al buffer como `EditSource::Load`
  (`BufferStore::reload_from_disk` no recibe fuente) y se informa al
  `ReviewStore` como `Agent`.
- `review.max_file_size_kb`/`max_lines` se aplican en el host; no pueden
  subir los límites fijos del store (2 MB / 50 000 líneas).
- Archivos no UTF-8 o binarios no entran en la revisión (no hay buffer).
- `turn_active` es por archivo (el que el turno en curso está tocando).
- El diálogo de buffer sucio siempre pregunta antes de un `FsWrite`; al
  arrancar un tool call solo si el permiso no se concedió solo (si no, se
  guarda); sin atajos de teclado.
- "Aceptar/Rechazar todo" del panel decide todo lo pendiente;
  `workspace::accept_turn`/`reject_turn` el último turno con pendientes.

## Etapa 4 (workstream C): conexiones en la interfaz

Detalle, verificación, desviaciones y lista de comprobación manual en
`docs/etapas/etapa-4.md`. Resumen de desviaciones:

- El icono del proveedor es un monograma (C/X/G): `gpui-kit` no trae
  logos de marcas y el proyecto no incluye esos recursos.
- "No se encontró un navegador" se detecta por heurística (`$BROWSER` o
  `xdg-open` en el `PATH`): `cx.open_url` no informa fallos.
- Cerrar el modal durante "Preparando…" no corta una descarga en curso: el
  hilo termina (las descargas son reanudables) y su resultado se descarta.
- `AuthRequired` ya no muestra la tarjeta "Hace falta autenticarse" del
  chat: lo reemplaza el banner de sesión vencida con "Volver a conectar".
- Detener el proceso a propósito (cambio de conexión, eliminar) ya no deja
  el aviso "El agente se cerró" en la conversación; solo la píldora pasa a
  `desconectado`.
