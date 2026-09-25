# Módulo `asteroid-workspace` y binario `asteroid`

## `asteroid-workspace`
- `Workspace` (entidad raíz): `Project`, `BufferStore`, `ReviewStore`, `ChatPanel`, `Dock` con tres paneles (izquierda chat, centro pestañas, derecha árbol) usando el dock de `gpui-kit`; tamaños y colapsado persistidos en `layout.json`.
- **Pestañas**: lista con título, indicador sucio (●), indicador de revisión (`+N −M` en pequeño), previsualización (cursiva) vs. fijada; cerrar con `Ctrl+W` pregunta si hay cambios sin guardar; clic medio cierra; reordenar arrastrando (v1 opcional).
- **Árbol**: componente árbol de `gpui-kit` alimentado por `Worktree`; iconos por tipo; colores de estado del agente (`02-visual.md §6.4`) y de git (v2); clic simple previsualiza, doble fija; menú contextual con "Revelar en carpeta", "Copiar ruta", "Copiar ruta relativa"; teclado ↑↓←→ y `Enter`.
- **Barra de estado**: posición, EOL, lenguaje, agente y estado, pendientes (clic abre panel de revisión), zoom.
- **Panel de revisión**: `02-visual.md §6.3`.
- **Avisos**: cola de toasts.
- **Comandos globales**: `workspace::open_folder`, `toggle_chat`, `toggle_tree`, `focus_chat`, `next_change`, `prev_change`, `next_file_with_changes`, `accept_turn`, `reject_turn`, `undo_last_reject`, `open_review_panel`, `zoom_*`.
- **Keymap**: carga `keymap.json` del usuario sobre el predeterminado; formato `[{ "context": "Editor && review_hunk_under_cursor", "bindings": { "ctrl-enter": "review::accept_hunk" } }]` (mismo formato que Zed para que sea familiar).
- **Ciclo de revisión**: al `TurnEnded`, para cada archivo tocado en el turno: releer disco, actualizar buffer, `ReviewStore::file_written`; si el archivo no estaba abierto, abrirlo en un buffer sin pestaña; actualizar árbol y barra. Al recibir `FsWrite`: `BufferStore::apply_agent_write` + `ReviewStore`. Al recibir un `tool_call` con `locations` y `kind: edit` la primera vez para un path: `capture_base` desde el buffer/disco.
- **Diálogo de buffer sucio** antes de que el agente escriba un archivo con cambios sin guardar: Guardar / Descartar / Mantener (mantener = el agente escribe encima y el hunk incluirá tus cambios).

## Binario `asteroid`
- CLI: `asteroid [ruta] [--log-level ..]`. Sin ruta: último proyecto o pantalla vacía.
- Arranque: cargar settings/keymap/tema → crear app GPUI → ventana con tamaño/posición restaurados → `Workspace`.
- GPU: intentar Vulkan; si falla, GL; si falla y `ASTEROID_ALLOW_SOFTWARE_GPU=1`, software; si no, mensaje y salida con código 2.
- Recarga en caliente de settings/keymap/tema al cambiar los archivos.
- Una sola instancia por proyecto no es requisito en v1.

## Criterios de aceptación
- [ ] `asteroid .` abre la ventana con el árbol del proyecto en < 400 ms en la máquina de referencia.
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
- **El indicador de actividad del agente en una pestaña es un sufijo "◆"**
  fijo en el título (`Tab::agent_touched`, `CenterPanel::note_agent_write`/
  `reload_after_agent_edit`), no el indicador de revisión `+N −M` que describe
  este documento — ese llega con `asteroid-review` en la Etapa 3.
- **`ChatSettings` no tiene sección propia en `settings.json`**: ninguno de
  sus campos (filas del input, líneas de contexto del diff, …) tiene
  contraparte en `asteroid_settings::Settings` todavía, así que
  `theme::chat_settings` siempre proyecta los valores por defecto del crate;
  el cableado de recarga en caliente ya está listo para cuando esa sección
  se agregue.
- Detalle completo (incluida la razón de cada una) en
  `docs/etapas/etapa-2.md`, secciones "Desviaciones" y "Deseos".
