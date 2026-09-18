# Módulo `asteroid-acp`

Cliente ACP: descubrir, lanzar y hablar con agentes. Sin GPUI. Corre en un hilo tokio propio y se comunica con la UI por canales (`03-arquitectura.md §3`).

## Responsabilidades
- **Registro**: descarga y cachea (24 h, en estado XDG) `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`; expone `AgentDescriptor { id, name, version, distribution: Npx { package, args } | Binary { url, sha256, cmd, args }, icon }` para `linux-x86_64`/`linux-aarch64`. Los agentes definidos por el usuario en `settings.json` (`agents.custom: [{ id, name, command, args, env }]`) se suman. Distribución `binary`: descarga a `~/.local/share/asteroid/agents/<id>/<version>/` verificando sha256 solo tras confirmación del usuario. Sin red: usa la caché o solo los agentes custom.
- **Disponibilidad**: comprueba `node --version` ≥ 22 para `npx`; para binarios, si ya está descargado.
- **Lanzamiento**: proceso hijo con stdin/stdout/stderr en pipes, `cwd` del proyecto, entorno del usuario más `NO_BROWSER` sin tocar. Lectura de stdout línea a línea sin límite de tamaño; stderr drenado a log y a los últimos 64 KiB para diagnósticos. Matar el grupo de procesos al cerrar.
- **Conexión**: `agent-client-protocol` 2.1.x, `ProtocolVersion::V1`. Capacidades anunciadas: `fs.readTextFile = true`, `fs.writeTextFile = true`, `terminal = false`, `elicitation` si el SDK lo permite, `auth.terminal = true` (Asteroid delega al terminal del sistema pero declara la capacidad para que el agente ofrezca esos métodos). Negociar `agentFileChangeReport` vía `_meta` si el agente lo anuncia.
- **Sesión**: `session/new` con `cwd` y `mcpServers` de settings; guardar `sessionId`; soporte de `session/load` si el agente lo anuncia (historial). Modos y `configOptions` (ambos), `available_commands_update`, `current_mode_update`, `config_option_update`.
- **Prompt**: bloques `text` y `resource_link` para menciones `@archivo`; antes de enviar, si `ReviewStore::report_for_agent` devuelve texto, se antepone como bloque `text` propio marcado con `<user_review_feedback>`. Tras `end_turn`, si el agente soporta `agentFileChangeReport`, procesar la lista de rutas.
- **Handlers**: `session/request_permission` → evento a la UI con `oneshot`; timeout ninguno; si se cancela el turno, responder `cancelled`. `fs/read_text_file` y `fs/write_text_file` → eventos con `oneshot`; rutas fuera del proyecto → error `invalid_params`; `line > total` → `invalid_params`.
- **Cancelación**: `session/cancel`; marcar tool calls `pending`/`in_progress` como canceladas al recibir `stopReason: cancelled`.
- **Autenticación**: si `initialize` devuelve `authMethods` y `session/new` falla con `auth_required`, emitir `AuthRequired { methods }`; la UI muestra el comando (`args` del método `terminal`) y ofrece reintentar.
- **Modo de autonomía** (`01-producto.md §F5`): `Revisar después` responde automáticamente la opción `allow_once` a permisos cuyo tool call es `kind: edit|read|search|think`, salvo rutas sensibles; `Pedir antes` reenvía todo a la UI; `Aplicar siempre` responde `allow_once` a todo salvo rutas sensibles. Las opciones `reject_*` nunca se autoseleccionan. Los tool calls `execute` siempre se preguntan salvo en `Aplicar siempre`.

## Criterios de aceptación
- [ ] Test de integración con un agente falso (binario en `tests/fake_agent`) que: negocia, crea sesión, hace streaming de texto, emite un tool call con `diff`, pide permiso, llama `fs/read_text_file` y `fs/write_text_file`, y termina. Se verifica la secuencia de `AgentEvent`.
- [ ] Un `stdout` con una línea no-JSON no rompe la conexión.
- [ ] Matar el proceso del agente produce `Exited` y la UI puede relanzarlo.
- [ ] Ejemplo `cargo run -p asteroid-acp --example chat -- --agent claude-acp "di hola"` imprime la respuesta usando la sesión del CLI del sistema.
