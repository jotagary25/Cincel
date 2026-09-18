# Módulo `asteroid-chat`

Panel de chat sobre GPUI. Consume `AgentEvent` y emite `AgentCommand` (`03-arquitectura.md §3`). Diseño en `02-visual.md §7`.

## Estructura
- `ChatPanel` (entidad): `agents: Vec<AgentDescriptor>`, `active: Option<AgentHandle>`, `sessions`, `transcript: Vec<Entry>`, `input`, `config_options`, `autonomy`.
- `Entry`: `UserMessage { blocks }`, `AgentText { markdown, streaming }`, `AgentThought { text, collapsed, duration }`, `ToolCall { id, kind, title, status, content: Vec<ToolContent>, locations, stats: Option<(u32,u32)> }`, `Plan { items }`, `Permission { request_id, tool_call, options, answered }`, `AuthRequired { methods }`, `Notice { level, text }`, `TurnSeparator { stop_reason, duration }`.
- Markdown: componente de `gpui-kit` en modo streaming; código con resaltado de `asteroid-syntax`; botón copiar por bloque.
- Tarjeta de herramienta de edición: muestra path y `+N −M` (de `ReviewStore::stats`), botón "Ver en el editor" → `workspace::open_and_jump(path, first_pending_hunk)`. **No renderiza el diff completo**: como mucho las 3 primeras líneas de contexto al desplegar, con la nota "Revisá en el editor".
- Permisos: botones en el orden que envía el agente; `Enter` elige el primero; `Esc` elige la primera opción `reject_*`; al responder, la tarjeta se compacta a una línea "Permitido / Rechazado".
- Input: `Textarea` de `gpui-kit`, crece hasta 8 líneas; `Enter` envía, `Shift+Enter` salto; `@` abre el selector de archivos del proyecto (filtro por subcadena, v1 sin fuzzy); `/` abre comandos del agente; arrastrar desde el árbol inserta mención; pegar imagen no soportado en v1 (aviso).
- Selectores de pie: autonomía (siempre), y uno por `ConfigOption` de categoría `mode`, `model`, `thought_level`; también `availableModes` legacy. Cambiar uno envía `SetConfigOption`/`SetMode` y refleja la respuesta.
- Estado del agente en cabecera y en la barra de estado.
- Persistencia: el transcript de la sesión actual se guarda en estado XDG por proyecto para restaurarlo al abrir (solo lectura hasta que el agente soporte `session/load`).

## Criterios de aceptación
- [ ] Con el agente falso de `asteroid-acp`, el transcript muestra texto en streaming sin parpadeo y las tarjetas cambian de estado.
- [ ] Un pedido de permiso bloquea el envío de nuevos mensajes hasta responderlo; `Esc` lo rechaza.
- [ ] Cambiar el modelo desde el selector se refleja en la siguiente `configOptions` que devuelve el agente.
- [ ] `@` inserta `resource_link` con la ruta absoluta del archivo en el prompt enviado.
- [ ] El chat no muestra nunca un diff completo; la tarjeta enlaza al editor.
