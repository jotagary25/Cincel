# Módulo `cincel-workspace` y binario `cincel`

## `cincel-workspace`
- `Workspace` (entidad raíz): `Project`, `BufferStore`, `ReviewStore`, `ChatPanel`, `Dock` con tres paneles (izquierda chat, centro pestañas, derecha árbol) usando el dock de `gpui-kit`; tamaños y colapsado persistidos en `layout.json`.
- **Pestañas**: lista con título, indicador sucio (●), indicador de revisión (`+N −M` en pequeño), previsualización (cursiva) vs. fijada; cerrar con `Ctrl+W` pregunta si hay cambios sin guardar; clic medio cierra; reordenar arrastrando (v1 opcional).
- **Vista previa de Markdown** (`workspace::toggle_markdown_preview`, `Ctrl+Shift+V`, solo con una pestaña de Markdown activa): alterna el cuerpo de la pestaña entre el editor de código y un renderizado del texto *actual* del buffer, reusando el renderizador del chat (`cincel_chat::markdown`) con su propio resaltado de bloques de código; se actualiza sola 150 ms después de cada cambio del buffer. El título, el punto de sucio y `Ctrl+S` siguen atados al buffer de siempre; la barra de estado muestra "Vista previa" en lugar de `Ln, Col`. Un botón con icono al final del breadcrumb hace lo mismo con el mouse. Cada pestaña recuerda si estaba en vista previa en `layout.json` y la restaura al reabrir el proyecto.
- **Árbol**: componente árbol de `gpui-kit` alimentado por `Worktree`; iconos por tipo; colores de estado del agente (`02-visual.md §6.4`) y de git (v2): modificado en `status.warning` con `+N −M`, creado en `status.ok` con `+N`, borrado pendiente tachado en `status.error` con `−N` y tooltip "Borrado por el agente · pendiente" (sigue en el árbol aunque el `Worktree` ya no lo liste, hasta que se decide); clic simple previsualiza, doble fija; clic en un borrado pendiente abre una pestaña de solo lectura con todo su contenido previo en rojo (una única sección de filas fantasma) y la barra flotante con "Aceptar archivo"/"Rechazar archivo" además de las del turno: aceptar cierra la pestaña, rechazar restaura el archivo y la pestaña pasa a ser un editor normal; menú contextual con "Revelar en carpeta", "Copiar ruta", "Copiar ruta relativa"; teclado ↑↓←→ y `Enter`.
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
- **Ciclo de revisión** (**v2 (2026-09-28): la base viene de la foto del proyecto; las herramientas son solo una pista**; regla en `03-arquitectura.md` §4):
  1. *Foto al empezar el turno*: `Agents` llama a `Review::begin_prompt`, que saca la foto del proyecto (`cincel_project::ProjectSnapshot::capture`, el recorrido del árbol con sus exclusiones y `files.exclude`; texto hasta `review.max_file_size_kb`; binarios con hash y bytes; por encima de `review.snapshot_max_total_mb` solo hash, y la base sale del buffer si está abierto) en el ejecutor de fondo. El prompt sale cuando la foto está lista; si tarda más de 1 s el chat muestra "Preparando la revisión…". Con `ProjectOptions::inert()` (tests) la foto es síncrona.
  2. *Vigilancia*: `Project` emite `FilesChanged(Vec<FsEvent>)` por cada lote del watcher, para cualquier ruta del proyecto (abierta o no). Para una ruta sin seguimiento la base es la copia de la foto (`capture_base_text`; `file_created` si no existía) y luego `file_written`/`file_deleted` con el disco; renombrar = borrar + crear. Si el buffer tiene cambios sin guardar, se suman a la base (no son del agente).
  3. *Repaso al terminar* (`end_turn`, también en `agent_gone` y al encadenar turnos): la foto entera contra el disco (tamaño/mtime; contenido si difieren o si el mtime es reciente) y se suma lo que el watcher no avisó; recién después se libera la foto. **Desde la Etapa 6, el repaso corre en el ejecutor de fondo** (`PhotoState::Sweeping`): el turno sigue activo hasta que el resultado vuelve y recién entonces se adoptan, en el hilo principal y en tramos cortos, solo los archivos que difieren; si tarda más de 1 s, la barra de estado muestra "Revisando los cambios del agente…". Un prompt nuevo espera ese fin igual que hoy espera la foto (`turn_waiter`). Mientras tanto, los lotes del vigilante que llegan de a muchos (más de 4 rutas) también se adoptan en tramos cortos (`WatchQueue`), para que ningún bloqueo del hilo principal supere los 8 ms (M11, `docs/rendimiento.md`).
  4. *Atribución*: lo que Cincel escribe durante el turno (`Project::save`, nuevo archivo, rechazos) emite `HostWrote(path)` y actualiza la foto: no es del agente; `classify` sigue mandando para los buffers en seguimiento. Lo que cambian otros programas durante el turno se atribuye al agente.
  5. *Pistas*: `FsWrite` (`BufferStore::apply_agent_write` + `ReviewStore`), `FsRead` y el primer `tool_call` edit/delete/move con `locations` capturan la base antes, como en v1; si el disco ya difiere de la foto, la base es la foto.
  6. *Turnos encadenados*: cada turno saca su foto; lo pendiente de turnos anteriores conserva su base original.
  7. *Binarios* (no UTF-8, o un NUL en los primeros 8 KB: `cincel_project::looks_binary`): **quedan fuera de la revisión** (correcciones tras la prueba de la 1.0). Lo que el agente les haga (crearlos, cambiarlos, borrarlos, o convertir un texto en binario) se aplica sin preguntar y no aparece como pendiente: ni en el árbol, ni en "Revisar todo", ni en la barra de estado, ni en el diálogo de cierre. La foto guarda de ellos solo tamaño y hash. Una pestaña de un binario abierto desde el árbol es de solo lectura, dice "Archivo binario: no se muestra" y no tiene botones de revisión. Se retiraron `review_binaries.rs`, `binaries.json`/`binaries/<sha256>` (se borran al abrir el proyecto si quedaron de una versión anterior) y el `RejectLog`: `Alt+Shift+U` usa la pila del store. Un archivo de *texto* sin copia en la foto sigue en revisión, entero y solo aceptar ("cambiado por el agente, sin copia previa"), en memoria.
  8. *Botones de un segmento*: aparecen con el cursor en el segmento o el mouse encima; "encima" es las filas del segmento **más el rectángulo de los propios botones** (`EditorView::hover_pill_zone`), porque se dibujan en su esquina superior derecha y pueden salir del segmento (la fila anterior, o su borde superior). Llegar a ellos desde arriba ya no los esconde. La barra flotante es la misma para todos los archivos: "Aceptar todo" · "Rechazar todo" del turno · flechas · "cambio N de M" · "Revisar todo"; un archivo borrado por el agente se decide con los botones de su único segmento o desde "Revisar todo".
- **Diálogo de buffer sucio** antes de que el agente escriba un archivo con cambios sin guardar: Guardar / Descartar / Mantener (mantener = el agente escribe encima y el hunk incluirá tus cambios).

## Binario `cincel`
- CLI: `cincel [ruta] [--log-level ..] [--bench ESCENARIO [--bench-file ARCHIVO]]`. Sin ruta: último proyecto o pantalla vacía. `--bench` (Etapa 6, D6, `docs/specs/modulos/perf.md`): corre uno de los ocho escenarios de medición interna (`startup`, `open`, `typing`, `scroll`, `idle`, `finder`, `sweep`, `review-1mb`), `all` (los ocho en orden) o `demo` (solo para capturas); imprime JSON por línea a la salida estándar. Sin la bandera, los ganchos de medición (`cincel_workspace::bench`) no hacen nada.
- Arranque: migrar directorios XDG de `asteroid` a `cincel` si hace falta (`docs/specs/06-etapa4-conexiones-y-cincel.md` §7) → iniciar log a archivo (`cincel-log`) → **registrar las fuentes embebidas Inter y JetBrains Mono** (`cincel_workspace::fonts::register_embedded`, antes de `cincel_workspace::init` y de abrir la ventana; un error se registra y el arranque sigue con los respaldos del sistema) → cargar settings/keymap/tema → crear app GPUI → ventana con tamaño/posición restaurados → `Workspace` → toast de migración si hubo alguna.
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
- ~~Archivos no UTF-8 o binarios no entran en la revisión (no hay buffer).~~
  Desde la v2 de la revisión (2026-09-28) entraban aparte del store, solo
  por archivo; desde las correcciones tras la prueba de la 1.0 vuelven a
  quedar fuera, por decisión del autor: se aplican sin revisar.
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
- `AuthRequired` ya no muestra la tarjeta "Hace falta autenticarse" del
  chat: lo reemplaza el banner de sesión vencida con "Volver a conectar".
- Detener el proceso a propósito (cambio de conexión, eliminar) ya no deja
  el aviso "El agente se cerró" en la conversación; solo la píldora pasa a
  `desconectado`.

## Etapa 5: productividad

Detalle, verificación, desviaciones y lista de comprobación manual en
`docs/etapas/etapa-5.md`. Spec: `docs/specs/07-etapa5-productividad.md`.

- **Foco** (`crate::focus`): `FocusZone { Chat, Center, Files }`.
  `Workspace::toggle_focus(zone, …)` es la regla de `Ctrl+Shift+A`/
  `Ctrl+Shift+E`: panel oculto → abrir y enfocar; panel visible con el foco
  dentro → cerrar y devolver el foco al centro; panel visible con el foco en
  otro lado → enfocar sin cerrar. `Workspace::focus_next_zone` (`Ctrl+L`)
  recorre las zonas visibles Chat → Centro → Archivos → Chat, sin abrir ni
  cerrar nada; `next_zone(current, visible)` es la función pura que decide el
  destino. `workspace::focus_chat` se conserva registrada, sin binding por
  defecto. `Workspace::is_modal_open` reúne todo lo que congela estos atajos.
- **Buscador rápido de archivos** (`crate::file_finder`, `Ctrl+P`):
  `FileFinder` (una instancia por proyecto abierto) con `rank(candidates,
  query, recent, limit)` como función pura de coincidencia difusa
  (`frizbee` 0.13, MIT) y desempate. Sin proyecto, toast; con el buscador
  abierto, `Ctrl+P` lo cierra. Abre en pestaña de previsualización
  (`CenterPanel::open_file(path, pin: false)`).
- **Pestaña de configuración** (`crate::settings_view`, `Ctrl+,`):
  `SettingsView`, con secciones Apariencia, Fuentes, Editor, Archivos,
  Revisión y Conexiones. Cada control escribe con
  `cincel_settings::write_edit` (conserva comentarios, orden y claves
  desconocidas) y emite `SettingsViewEvent::Written`, que el workspace aplica
  sin el toast "Configuración recargada". La sección Conexiones
  (`crate::agents::settings_bridge::SettingsBridge`) nunca toca
  `cincel_connections` directamente: emite eventos que `Agents` decide,
  incluidas las actualizaciones de adaptador y de Node con `CancelToken`
  cancelable desde el propio botón "Cancelar".
- **Modal de atajos y botón de la barra de estado** (`crate::shortcuts_modal`,
  `F1`): mezcla el keymap en vigor con los bindings Rust de los widgets,
  agrupa por categoría (Generales, Editor, Revisión, Chat, Conexiones), marca
  "Personalizado" lo que el usuario cambió en `keymap.json` y "Desactivado en
  tu keymap.json" lo que puso en `null`.
- **Barra de título** (`crate::title_menu`): botón de menú (Abrir carpeta,
  Carpetas recientes, Nuevo archivo, Guardar, Guardar todo, Configuración,
  Atajos de teclado, Conexiones, Salir) y los dos botones de mostrar/ocultar
  chat y archivos, que llaman a la misma `Workspace::toggle_focus` que los
  atajos. "Salir" (`Ctrl+Q`), la `×` de la barra de título y el cierre del
  escritorio (`Alt+F4`) convergen en `Workspace::should_close` (la `×` y
  `Alt+F4` a través de `Workspace::request_close`, que además guarda la
  geometría; la `×` pasa por `TitleBar::on_close_window`, porque sin
  manejador gpui-kit quita la ventana sin preguntar): primero preguntan por
  los cambios de agente sin decidir (borrados incluidos); sin archivos sin
  guardar cierran directo; con ellos, abren el diálogo "¿Guardar cambios?"
  (Guardar todo y salir / Salir sin guardar / Cancelar).
- **Nuevo archivo** (`crate::new_file`, `Ctrl+N`): campo flotante (mismo
  estilo y posición que el buscador de archivos) que crea el archivo en disco
  bajo la carpeta seleccionada en el árbol (o la raíz) y lo abre fijado.
  `validate_new_file_name` es la función pura que rechaza nombres vacíos,
  absolutos o con `..`.
- **Git en el margen del editor**: `Project` arranca un `GitDirWatcher` junto
  al `GitStatusWatcher` y emite `ProjectEvent::GitDiffChanged(PathBuf)`; el
  workspace recalcula el diff de los archivos abiertos en el ejecutor de
  fondo y llama a `EditorView::set_git_diff` (detalle del pintado en
  `modulos/editor.md`).
- **Autoguardado tras una pausa** (`center.rs`): con `files.autosave =
  "after_delay"`, cada cambio del usuario reprograma un `Timer::after
  (autosave_delay_ms)`; si el buffer sigue sucio al vencer, se guarda, salvo
  que el archivo esté en un turno de revisión activo. Reemplaza la
  limitación de etapas anteriores de solo autoguardar "al perder el foco".
- **Cancelar una descarga de verdad** (`docs/specs/07-etapa5-productividad.md`
  §10.3): "Cancelar" en "Preparando…" dispara un `CancelToken` real: la
  descarga se corta entre lecturas de 64 KiB, se borra el `.part` y el
  proceso de `npm install` (en su propio grupo) se mata. Reemplaza la
  desviación de la Etapa 4 que dejaba terminar el hilo y descartaba su
  resultado.
- **Motivo de `AuthRequired`**: `AgentEvent::AuthRequired` trae ahora
  `message: Option<String>` con el motivo que dio el agente (redactado y
  recortado a 240 caracteres), que el banner de sesión vencida del chat
  muestra como "El agente dijo: «motivo»".
- **Botón de actualizar adaptador y Node**: la sección Conexiones de la
  pestaña de configuración muestra "Actualizar a X.Y.Z" cuando
  `Adapters::update_available`/`Runtime::update_available` devuelven una
  versión nueva, con barra de progreso y "Cancelar"; `prune_unused` borra las
  versiones que ya no se usan al arrancar y al detener un agente.

## Etapa 6: rendimiento, deudas de revisión y fuentes embebidas

Detalle, verificación y desviaciones en `docs/etapas/etapa-6.md`. Spec:
`docs/specs/08-etapa6-cierre-1-0.md`.

- **`bench.rs`** (`crate::bench`, D6, `docs/specs/modulos/perf.md`): los
  ocho escenarios de `cincel --bench` corren dentro de la ventana real
  (`Workspace`, `CenterPanel`, `Review`); sin la bandera, los ganchos
  (`frame_begin`, `frame_probe`) leen un valor global una sola vez y no
  hacen nada. `FrameProbe` es el elemento de tamaño cero que marca "fin de
  cuadro" (GPUI 0.3.5 no ofrece un callback tras `present`).
- **Fuentes embebidas** (`crate::fonts`, D7, `docs/specs/08-etapa6-cierre-
  1-0.md` §4): Inter (interfaz) y JetBrains Mono (código), diez TTF
  estáticos registrados una vez al arrancar (`main.rs`, antes de `init`),
  respaldo del sistema solo para otra familia elegida por el usuario.
- ~~**Binarios persistidos y deshacer unificado** (`review_binaries.rs`,
  D13, §5.2): un cambio del agente en un archivo no UTF-8 (imagen, PDF)
  sigue pendiente aunque se cierre y reabra Cincel; `Alt+Shift+U` deshace
  el último rechazo, de texto o binario, en el orden real en que ocurrió
  (`RejectLog`).~~ Retirado en las correcciones tras la prueba de la 1.0
  (`docs/etapas/etapa-6.md`): los binarios quedan fuera de la revisión.
- **Repaso final del turno en el ejecutor de fondo** (`review_snapshot.rs`,
  D12, §5.3): ver "Ciclo de revisión" más arriba, puntos 2 y 3.
- **Tope de memoria de la foto en Configuración** (`settings_view.rs`,
  §5.1): fila "Memoria para la foto del proyecto (MB)" en la sección
  Revisión, clave `review.snapshot_max_total_mb` (`modulos/settings.md`).
- **Teclas de la barra de búsqueda en el keymap por defecto** (D14, §5.4):
  `crate::keymap::search_bar_bindings` se eliminó; la precedencia entre el
  keymap del usuario y las teclas de búsqueda ahora la resuelve el orden de
  las secciones dentro de `keymap.json` (`modulos/settings.md`), no código.
- **`gpui-kit/test-support`** (D16, §10.1 de la spec): se sumó a la feature
  `test-support` de `cincel-workspace` para habilitar clic real por
  coordenada sobre los botones de la barra de título y de la barra de
  estado (`click_tests.rs`), sin agregar una variante nueva de `target`.
