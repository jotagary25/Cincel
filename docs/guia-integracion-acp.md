# Guía de integración con agentes de programación vía ACP

Documento técnico y agnóstico (cualquier plataforma y lenguaje) para construir un cliente que conecte agentes de programación (Claude Code, Codex, Google Antigravity y cualquier otro del catálogo) usando el **Agent Client Protocol (ACP)** con las suscripciones del usuario. Condensa lo verificado al construir Cincel (septiembre de 2026): protocolo, catálogo, autenticación, aislamiento de cuentas, revisión de cambios y trampas. Está pensado para que otra IA lo lea y pueda implementar sin investigar ni experimentar de nuevo.

Referencias: https://agentclientprotocol.com (índice completo en `/llms.txt`), catálogo `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`, SDKs oficiales en Rust (`agent-client-protocol`), TypeScript (`@agentclientprotocol/sdk`) y otros.

## 1. Qué es ACP y en qué versión estar

- ACP es un protocolo **JSON-RPC 2.0 sobre stdio**: el cliente lanza al agente como proceso hijo y le habla por su entrada estándar; el agente responde por su salida estándar. Cada mensaje es una línea JSON terminada en `\n` (NDJSON), UTF-8, sin saltos de línea embebidos. La salida estándar del agente es **solo** para mensajes ACP; los logs van a stderr.
- **Versión 1 es la estable** y la única que hablan hoy todos los agentes y clientes relevantes (Zed, JetBrains, los adaptadores oficiales). La versión 2 está en borrador; la guía oficial es no enviarla por defecto. Implementar v1; aislar los tipos del protocolo detrás de una capa propia para migrar después.
- Ambos roles se pueden implementar sin SDK; los SDKs solo ahorran los tipos. Si se usa un SDK, revisar su changelog: la API cambia entre versiones menores.

## 2. Ciclo de vida de una conexión

1. **Lanzar el proceso** del agente (ver §3) con stdin/stdout/stderr en tuberías, en el directorio del proyecto, con el entorno controlado (§5).
2. **`initialize`** (cliente → agente): negocia versión y capacidades.
   ```json
   {"jsonrpc":"2.0","id":1,"method":"initialize","params":{
     "protocolVersion":1,
     "clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":false,
                           "auth":{"terminal":true},"elicitation":{"form":true,"url":true}},
     "clientInfo":{"name":"mi-cliente","version":"1.0"}}}
   ```
   Respuesta: `protocolVersion`, `agentInfo {name,title,version}`, `agentCapabilities` (por ejemplo `loadSession`, `auth.logout`, `promptCapabilities`, `sessionCapabilities`) y `authMethods[]` (§4).
3. **`session/new`** con `cwd` (absoluto) y `mcpServers[]` (puede ir vacío). Respuesta: `sessionId`, y opcionalmente `modes` (`currentModeId`, `availableModes`) y `configOptions[]` (selectores de modo, modelo, nivel de razonamiento). Si el agente no está autenticado responde el error **`-32000` (`auth_required`)**; ese código es la señal para iniciar el login (§4). Alternativas: `session/load` y `session/resume` (con `sessionId` guardado) cuando el agente anuncia esas capacidades; devuelven el historial como notificaciones de actualización antes de la respuesta.
4. **`session/prompt`** con `prompt[]` de bloques de contenido: `{"type":"text","text":"…"}` y `{"type":"resource_link","uri":"file:///ruta/absoluta","name":"archivo.rs","mimeType":"text/x-rust"}` (menciones de archivos, intercalables con el texto en orden). La respuesta llega **al final del turno** con `stopReason` ∈ `end_turn | max_tokens | max_turn_requests | refusal | cancelled`.
5. Durante el turno el agente envía notificaciones **`session/update`** con `sessionUpdate` ∈ `agent_message_chunk` (texto de respuesta, en trozos), `agent_thought_chunk` (razonamiento), `user_message_chunk` (eco al cargar historial), `tool_call` (nueva herramienta: `toolCallId`, `title`, `kind` ∈ `read|edit|delete|move|search|execute|think|fetch|other`, `status` ∈ `pending|in_progress|completed|failed`, `content[]`, `locations[{path,line}]`, `rawInput`), `tool_call_update` (cambios de estado o contenido), `plan` (lista de tareas), `available_commands_update` (comandos slash del agente), `current_mode_update`, `config_option_update`, `session_info_update`, `usage_update`.
6. **Peticiones del agente al cliente** (el cliente debe responder):
   - `session/request_permission`: trae `toolCall` y `options[] {optionId,name,kind}` con `kind` ∈ `allow_once|allow_always|reject_once|reject_always`. Responder `{"outcome":{"outcome":"selected","optionId":"…"}}` o `{"outcome":{"outcome":"cancelled"}}`. **Responder siempre desde una tarea aparte**, nunca bloqueando el bucle que lee stdout: si se bloquea, el agente se cuelga.
   - `fs/read_text_file {path, line?, limit?}` y `fs/write_text_file {path, content}`: el cliente sirve el contenido **en memoria** (incluidos cambios sin guardar) y aplica escrituras. Restringir a rutas dentro del proyecto (responder `invalid_params` fuera). Ver §6 sobre qué agentes los usan de verdad.
   - `elicitation/create` (`mode: "form"` o `"url"`): el agente pide un dato o que el usuario abra una URL; responder `accept`/`decline`. Puede llegar después `elicitation/complete`.
   - `terminal/*` solo si el cliente anunció `terminal: true` (no hace falta para empezar).
7. **Cancelar**: notificación `session/cancel {sessionId}`. Después: responder `cancelled` a todo permiso pendiente, seguir aceptando actualizaciones, marcar como canceladas las herramientas que quedaron `pending`/`in_progress`, y esperar la respuesta del prompt con `stopReason: cancelled`.
8. **Cambiar modo/modelo**: `session/set_mode {sessionId, modeId}` y `session/set_config_option {sessionId, configId, value}` (la respuesta trae el estado completo de opciones porque unas dependen de otras).
9. **Cerrar**: cerrar stdin y matar el **grupo de procesos** (los lanzadores tipo `npx` crean procesos intermedios) tras un periodo de gracia.

## 3. Catálogo de agentes y cómo lanzarlos

- El catálogo oficial es un JSON público: `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`. Cada entrada tiene `id`, `name`, `version`, `distribution` y metadatos. `distribution` **no es excluyente**: un agente puede tener `npx` (paquete npm, a veces ya con `@versión`), `binary` (por plataforma `linux-x86_64`, `darwin-aarch64`, etc., con `archive`, `cmd`, `args`, `env` y `sha256` que **puede ser `null`**) y `uvx`. Hay una lista de cuarentena aparte. Cachear el catálogo (24 h) y tolerar entradas inválidas sin descartar el resto.
- Agentes con login por suscripción, hoy:
  | id del catálogo | Distribución | Comando efectivo |
  |---|---|---|
  | `claude-acp` (Claude Code) | npm `@agentclientprotocol/claude-agent-acp` | `npx -y @agentclientprotocol/claude-agent-acp@<versión>` |
  | `codex-acp` (Codex) | npm `@agentclientprotocol/codex-acp` | `npx -y @agentclientprotocol/codex-acp@<versión>` |
  | `antigravity-acp` (Google Antigravity) | binario zip por plataforma (≈333 MB, ≈1 GB extraído) | `./agy_acp_server.par` con los `args` del catálogo |
  Los paquetes `@zed-industries/*` están deprecados; usar `@agentclientprotocol/*`. **Gemini CLI** (`@google/gemini-cli --acp`) dejó de servir para cuentas personales: el backend responde "This client is no longer supported for Gemini Code Assist for individuals… migrate to Antigravity". OpenCode expone ACP pero su login por protocolo no hace nada (es un concentrador de proveedores).
- Los adaptadores npm necesitan **Node.js ≥ 22**. Para no depender del Node del sistema, el cliente puede descargar un Node LTS propio (tarball oficial de nodejs.org, verificar `SHASUMS256.txt`) en su carpeta de datos y ejecutar `node <npm-cli.js> install --cache <caché propia>` con un `userconfig` y un `globalconfig` **distintos** (npm rechaza el mismo archivo para ambos: "double-loading config") y sin variables `npm_config_*` heredadas. Los adaptadores traen dentro sus propios binarios (Claude Code nativo, `codex` nativo): no hacen falta los CLIs instalados en el sistema.
- Actualizaciones: comparar la versión instalada con la del catálogo y ofrecer actualizar; no actualizar en silencio.

## 4. Autenticación

`initialize` devuelve `authMethods[]` con `id`, `name`, `description` y `type`:
- **`agent`** (predeterminado si falta `type`): el cliente llama `authenticate {methodId}` y el agente hace el flujo completo: abre el navegador él mismo (callback en `localhost`) o pide al cliente que muestre una URL vía `elicitation/create {mode:"url"}`. Ejemplos: Codex `chat-gpt` (abre navegador) y `chat-gpt-device-code` (URL + código por elicitation, requiere activar "código de dispositivo" en la cuenta de ChatGPT); Antigravity `oauth-personal` (imprime el enlace en **stderr** con el prefijo `Open the following link to authenticate the ACP server:` y completa solo por callback local).
- **`terminal`**: requiere `clientCapabilities.auth.terminal: true`. El cliente relanza el **mismo programa** del agente agregando los `args` del método y aplicando su `env`, en una terminal interactiva; **código de salida 0 = éxito**; luego reinicia la conexión. Ejemplo: Claude `claude-ai-login` = `<adaptador> --cli auth login --claudeai`. No hay `authenticate` ni señal de éxito en el protocolo.
- **`logout`** (estable desde mayo de 2026): solo si el agente anuncia `agentCapabilities.auth.logout`. Claude y Antigravity lo anuncian; Codex también.
- **Identidad de la cuenta** (no estándar): extensión `_auth/status_update` (notificación) que Claude y Codex envían tras `initialize`, `authenticate`, `logout` y cada `session/new`, con `authStatus {kind: account|api_key|gateway|external|none, label, account {email, organization, plan}}`. Antigravity no la envía y su token no incluye correo: permitir siempre que el usuario nombre la conexión.
- **Cómo mostrar "abrí este enlace" sin terminal visible**: ejecutar el comando de login en una **pseudo-terminal** (pty) oculta, leer su salida, detectar la URL con una expresión regular (`https://claude.com/cai/oauth/authorize…`, `https://auth.openai.com/…`, `https://accounts.google.com/…`), mostrarla con "Copiar" y "Abrir en el navegador", y:
  - Claude imprime `If the browser didn't open, visit: <url>` y luego `Paste code here if prompted >`: si el callback local no llega, el navegador muestra un código que el usuario pega; enviarlo por stdin del pty.
  - Codex (`codex login`) imprime `If your browser did not open, navigate to this URL to authenticate:` y completa solo por callback en `localhost:1455` (puerto fijo → serializar logins de Codex). `codex login --device-auth` da URL + código para máquinas sin navegador.
  - Antigravity: en vez de pty, usar `authenticate` por el protocolo y leer el enlace de stderr.
  - **Neutralizar el navegador automático** poniendo `BROWSER=/bin/true` (o equivalente) en el entorno del login: los tres CLIs lo respetan y así solo se abre lo que el usuario decida. Tiempo de espera de 15 minutos. Redactar en los logs `code=`, `state=`, `code_challenge=`, tokens y códigos de un solo uso.
  - Confirmar el éxito por código de salida 0 y, después, por `_auth/status_update` o por la existencia del archivo de credenciales del perfil.
- **Cierre de sesión seguro**: lanzar el agente con el entorno de **esa** conexión → `initialize` → `logout` si está anunciado → matar el grupo de procesos → borrar el directorio del perfil (validando que esté dentro de la raíz de perfiles) → borrar sus datos locales. Nunca ejecutar un logout sin la variable de perfil puesta: cerraría la sesión global del usuario.

## 5. Aislamiento de cuentas (varias cuentas por proveedor, sin tocar el sistema)

Un proceso de agente = una identidad. Para tener varias cuentas del mismo proveedor y no depender de las sesiones del sistema, cada **conexión** es un **perfil**: un directorio propio (permisos 0700) al que el agente escribe sus credenciales, indicado por una variable de entorno:

| Agente | Variable | Archivo de credenciales dentro del perfil | Notas |
|---|---|---|---|
| Claude Code | `CLAUDE_CONFIG_DIR=<perfil>` | `.credentials.json` | documentado por Anthropic para varias cuentas; en macOS el Keychain se indexa por directorio |
| Codex | `CODEX_HOME=<perfil>` | `auth.json` | sembrar `config.toml` con `cli_auth_credentials_store = "file"`; los refresh tokens son de un solo uso: nunca copiar `auth.json` ni compartir un home entre dos procesos |
| Antigravity | `GEMINI_HOME=<perfil>` | `antigravity-acp/acp_token.json` | `GEMINI_HOME` nombra el directorio `.gemini` completo; el servidor escribe registros en el directorio temporal: fijar `TMPDIR=<perfil>/tmp` |

Reglas del entorno del hijo: quitar las variables de credenciales del proveedor heredadas de la shell (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_PROFILE`, `CLAUDE_CODE_USE_*`, `OPENAI_API_KEY`, `CODEX_API_KEY`, `GOOGLE_*`, `GEMINI_*` salvo la propia, `CLOUDSDK_*`, `GCLOUD_*`, `NO_BROWSER`), poner el Node privado primero en `PATH`, **no** inyectar `NO_BROWSER=1` (cambia los métodos de login ofrecidos, a peor), y usar **un solo proceso por perfil** (el error "another Claude Code process is refreshing" viene de dos procesos compartiendo directorio). El perfil arranca vacío: nada de la configuración global del usuario se copia ni se enlaza. Guardar en el índice de conexiones solo metadatos (id, agente, etiqueta, fechas, identidad), nunca tokens.

## 6. Ediciones del agente y revisión de cambios

Hecho central: **Claude y Codex escriben los archivos directamente en disco** con sus herramientas nativas y **no** usan `fs/write_text_file`; solo lo usan algunos agentes. Zed también aplica y guarda de inmediato. Por tanto, un cliente no puede "retener" la escritura hasta que el usuario apruebe. El modelo que funciona es **aplicar, registrar, revertir**, con una regla que no depende de las herramientas del agente:

**Regla:** el cliente se guarda cómo está cada archivo del proyecto antes de enviar el prompt (la *foto*), y **todo lo que cambie en disco dentro del proyecto hasta que termina el turno es del agente**, sin importar cómo lo hizo: con su herramienta de edición, con un script, con un comando de shell o con otro programa. Lo único que no es del agente es lo que el propio cliente escribe durante el turno (un guardado del usuario, un autoguardado, un rechazo).

Por qué la regla tiene que ser así: en la práctica los agentes editan a veces **sin** su herramienta de edición (por ejemplo, Claude Code con un script de Python que hace `open(p).read()` / `write_text()` para un refactor grande). Esas escrituras no producen ningún `tool_call` de tipo `edit` con `locations`, así que un cliente que solo siga los archivos que el agente "nombra" por el protocolo **se los pierde** y los aplica sin revisión. Cincel arrancó con ese diseño y tuvo que corregirlo; no lo repitas.

1. **Foto al empezar el turno.** Antes de enviar `session/prompt`, recorrer los archivos del proyecto (con las mismas exclusiones que el árbol: carpetas de compilados y dependencias, y un tope de tamaño por archivo) y guardar en memoria el contenido de cada archivo de texto con su tamaño y fecha; de los binarios, solo hash y tamaño, para distinguirlos (quedan fuera de la revisión, ver el punto 6). En un proyecto de 5 000 archivos y 50 MB tarda decenas de milisegundos en segundo plano. El prompt sale cuando la foto está lista.
2. **Vigilancia durante el turno.** Un vigilante de disco (`notify` o equivalente, con debounce) reporta cada creación, modificación, borrado o renombrado dentro del proyecto, esté el archivo abierto o no. Para cada ruta afectada: si no está en seguimiento, la **base** es la copia de la foto (o "no existía" si es nueva); después se relee el disco y se calculan los segmentos. Renombrado = borrado + creación.
3. **Repaso al terminar el turno.** Con `stop_reason` del prompt, comparar toda la foto contra el disco (tamaño y fecha primero, contenido si difieren) y sumar lo que el vigilante no haya reportado; recién después liberar la memoria de la foto. Hacerlo en segundo plano y entregar los resultados en tramos cortos para no congelar la interfaz.
4. **Las señales del protocolo son pistas, no requisitos.** `fs/read_text_file`, `fs/write_text_file`, el `oldText` del primer `diff` y el primer `tool_call` de tipo `edit` con esa ruta en `locations` sirven para capturar la base **antes**, con mejor precisión; el resultado con ellas debe ser idéntico al de la foto. Los `diff` de las herramientas (`{"type":"diff","path","oldText","newText"}`, `oldText` `null` si es archivo nuevo) son **señal de interfaz, no fuente de verdad**: son parciales y `oldText` puede no coincidir con el buffer. La verdad son los bytes en disco. Si el agente lo soporta, el reporte de archivos tocados al final del turno ayuda (extensión `agentFileChangeReport`: el cliente anuncia `clientCapabilities._meta.jetbrains.air = {"version":1,"capabilities":["agentFileChangeReport"]}`, el agente confirma en el `_meta` de nivel superior de la respuesta a `initialize` con un array de strings, y por turno se envía `agentFileChangeReportRequest {version:1, requestId}` en el prompt; el reporte llega en `session_info_update`), pero tampoco reemplaza a la foto.
5. **Segmentos.** Diff de líneas Histogram **con postproceso de deslizamiento** (como git), sin ignorar espacios; diff de palabras dentro del segmento solo si ambos lados son cortos (≤ 5 líneas).
6. **Decidir.** Aceptar un segmento = copiar el texto nuevo en la base (sin tocar disco). Rechazar = escribir el texto de la base sobre el buffer y guardar. Por línea: partir el segmento en pares de líneas. Un archivo borrado por el agente se muestra como borrado pendiente (todo su contenido base en rojo, solo lectura); rechazar lo restaura. Un archivo creado se muestra entero en verde. **Los binarios quedan fuera de la revisión:** la revisión es de los archivos de texto; lo que el agente haga con un binario (un `.pyc` de `__pycache__`, una imagen: crearlo, cambiarlo, borrarlo) se aplica sin revisar y no aparece como pendiente en ningún lado. Cincel considera binario un archivo que no es UTF-8 o que tiene un byte NUL en sus primeros 8 KB. Ediciones manuales del usuario fuera de los segmentos se aplican también a la base para que no aparezcan como cambios del agente; dentro de un segmento, quedan como parte del lado nuevo.
7. **Al cerrar** el proyecto o la ventana con cambios sin decidir, preguntar (aceptar todo / rechazar todo / cancelar) en vez de dejarlos pendientes en silencio.
8. **Feedback al agente.** En el siguiente prompt, informar qué se rechazó o modificó: un parche unificado por archivo entre lo que el agente dejó y lo que quedó, antepuesto como bloque de texto (`<user_review_feedback>`), separando los cambios de formateo.
9. **Persistencia.** Guardar solo la base (direccionada por hash) y metadatos; al restaurar, **nunca reescribir el archivo**: si el archivo cambió por fuera, descartar esa revisión. Los binarios no se guardan: no se revisan.
10. `session/request_permission` llega **antes** de ejecutar la herramienta: es el único punto de bloqueo previo, útil para rutas sensibles (`.env`, lockfiles); rechazar por permiso descarta la herramienta entera, no un segmento.

## 7. Trampas comprobadas

- NDJSON estricto: leer stdout por líneas sin límite de tamaño (los `tool_call_update` con diffs grandes son líneas de varios MB) y **drenar stderr siempre**, o el agente se bloquea al llenar la tubería. Tolerar líneas no JSON en stdout (registrarlas y seguir).
- `-32000` en `session/new` puede significar "no autenticado" **o** un rechazo del backend (caso Gemini/Antigravity): conservar el `message` del error y las últimas líneas de stderr para mostrar el motivo.
- Con `initialize` Codex ofrece `chat-gpt` solo si no hay `NO_BROWSER`; Claude ofrece `claude-ai-login` y `console-login` solo si no hay `NO_BROWSER` (con la variable ofrece solo su TUI completa).
- `authenticate` y los permisos deben responderse fuera del bucle de lectura; un `authenticate` lento no debe impedir otros comandos.
- `session/prompt` que falla llega como error de la petición, no como `stopReason`: cerrar el turno igual en la interfaz.
- Cambiar de proyecto: cancelar el turno, cerrar el agente y relanzarlo con el nuevo `cwd`; el sandbox de `fs/*` sigue al proyecto.
- Al matar el agente, matar el grupo (`npx` deja un proceso intermedio); en Linux, `SIGTERM` al grupo y `SIGKILL` tras la gracia.
- Descargas: verificar SHA-256 cuando el catálogo lo publique; cuando sea `null`, avisar al usuario. Antigravity necesita ≈1,4 GB libres durante la instalación y no publica suma.
- Navegador ya logueado en otra cuenta: al crear una segunda conexión del mismo proveedor, sugerir otro perfil de navegador o una ventana privada.
- Registros: nunca escribir enlaces de login, códigos ni tokens; redactar antes de guardar.

## 8. Lista de comprobación para implementar

1. Transporte NDJSON con lector sin límite y drenaje de stderr; matar por grupo.
2. `initialize` con capacidades `fs`, `auth.terminal`, `elicitation.url`; leer `authMethods` y `agentCapabilities`.
3. Sesiones (`new`/`load`/`resume`), prompts con `text` + `resource_link`, cancelación completa, modos y opciones.
4. Handlers no bloqueantes: permisos, `fs/*` sandboxed al proyecto, elicitation.
5. Catálogo cacheado; instalación de npm con Node privado y de binarios; actualizaciones explícitas.
6. Perfiles aislados por conexión con la variable de cada agente, entorno limpio, `BROWSER` neutro en logins, `TMPDIR` para Antigravity.
7. Login: pty + regex para Claude y Codex, `authenticate` + stderr para Antigravity; campo para pegar código; 15 min; éxito por exit 0 y credencial presente; identidad por `_auth/status_update` si existe.
8. Eliminar conexión: `logout` con el perfil puesto, matar, borrar perfil y datos.
9. Revisión: base al primer contacto, relectura de disco tras cada edición, hunks con Histogram + postproceso, aceptar/rechazar por segmento y línea, rebase de ediciones del usuario, persistencia sin reescribir, informe al agente.
10. Registros con redacción; nada de secretos en el repositorio ni en índices.

Verificado con: `agent-client-protocol` 2.2.0 (schema 1.9.1), `@agentclientprotocol/claude-agent-acp` 0.79–0.81, `@agentclientprotocol/codex-acp` 1.12–1.13, `antigravity-acp` 1.2.1, `@google/gemini-cli` 0.61 (retirado para cuentas personales), Zed 1.20 como cliente de referencia.
