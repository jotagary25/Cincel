# Spec 06: Etapa 4, conexiones de agentes y renombrado a Cincel

Estado: v1.1 (2026-09-27), definida con el autor. Fuentes: `docs/research/05-conexiones-ux.md`, `docs/research/06-acp-estado-y-cuentas.md`, y las pruebas en vivo del 26-sep (login de Claude y Codex con perfiles vacíos aislados: funcionan de cero e imprimen el enlace; las sesiones del sistema no se ven afectadas).

**v1.1 (2026-09-27): Gemini CLI reemplazado por Google Antigravity.** Google rechaza el login personal de Gemini CLI ("This client is no longer supported for Gemini Code Assist for individuals… migrate to Antigravity", `IneligibleTierError` en gemini-cli 0.61.0). El tercer agente por suscripción pasa a ser el servidor ACP oficial de Antigravity (`antigravity-acp` del registry, distribución `binary`). Detalle y evidencia en `docs/etapas/etapa-4.md` y en el anexo de `docs/research/06-acp-estado-y-cuentas.md`.

## 1. Principios
1. **Aislamiento total.** Una conexión de Cincel no depende de nada del sistema: ni de sesiones iniciadas en la terminal, ni de `~/.claude`/`~/.codex`/`~/.gemini`, ni de un Node instalado. Todo vive bajo el directorio de datos de Cincel.
2. **Al arrancar, nada conectado.** El usuario elige explícitamente con qué conexión trabajar.
3. **Solo suscripciones** en esta etapa (login OAuth del proveedor). API keys y gateways quedan para otra etapa. OpenCode queda para otra etapa (es un concentrador de proveedores).
4. **Una conexión activa a la vez** en el chat. Cambiar de conexión detiene el proceso de la anterior y abre conversación nueva; no borra nada.
5. **Eliminar es olvidar todo**: cierre de sesión en el agente, borrado del perfil con sus credenciales y de sus conversaciones.

## 2. Modelo

```rust
Connection {
  id: Uuid,                 // nombre del directorio del perfil
  agent_id: String,         // id del registry ACP: "claude-acp" | "codex-acp" | "antigravity-acp"
  label: String,            // obligatorio, elegido por el usuario; sugerencia "Claude · personal"
  identity: Option<Identity { email: Option<String>, plan: Option<String>, organization: Option<String> }>, // solo si el agente la informa
  created_at, last_used_at,
  status: Connected | SessionExpired | Unavailable { reason }  // calculado al listar, no persistido como verdad
}
```
- Almacenamiento: `~/.local/share/cincel/connections/<id>/` (modo 0700) con el perfil del agente adentro, y `~/.local/share/cincel/connections.json` con los metadatos. **Nunca tokens ni credenciales en el JSON**: las credenciales son las que escribe el propio agente en su perfil.
- Variable de perfil por agente: Claude `CLAUDE_CONFIG_DIR=<perfil>`, Codex `CODEX_HOME=<perfil>` (+ `cli_auth_credentials_store = "file"` en `<perfil>/config.toml`), Antigravity `GEMINI_HOME=<perfil>` (nombra el propio directorio `.gemini`: token en `<perfil>/antigravity-acp/acp_token.json`, ajustes y conversaciones bajo `<perfil>/antigravity-acp/`).
- Entorno del proceso del agente: se **quitan** las variables de credenciales del proveedor heredadas del sistema (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_PROFILE`, `CLAUDE_CODE_USE_*`, `OPENAI_API_KEY`, `CODEX_API_KEY`, y para Google los prefijos completos `GOOGLE_*`, `GEMINI_*`, `CLOUDSDK_*`, `GCLOUD_*`, `ANTIGRAVITY_*`, `AGY_*`; el `GEMINI_HOME` propio se pone después) y **no** se inyecta `NO_BROWSER`. Para Antigravity se pone `BROWSER=/bin/true`: el agente llama a `webbrowser.open` de Python y abriría el navegador por su cuenta; Cincel muestra el enlace en el modal. **Todo login** (Claude `auth login --claudeai`, `codex login`, `authenticate` de Antigravity) corre con `BROWSER=/bin/true`: el CLI de Claude ejecuta `$BROWSER <url>` antes de caer a `xdg-open`, y sin esto abría una pestaña por su cuenta. El proceso del chat de Claude y Codex no lo lleva.
- Perfil limpio: no se enlaza ni copia nada de la configuración global del usuario (decisión del autor). Cincel siembra solo lo mínimo que el agente necesita para funcionar sin interacción (por ejemplo el `config.toml` de Codex con el almacén de credenciales en archivo).
- Conversaciones: cada una guarda `connection_id`; el historial se lista por conexión.

## 3. Runtime privado (Node), adaptadores y distribuciones binarias
- Los adaptadores de Claude y Codex (`@agentclientprotocol/claude-agent-acp`, `@agentclientprotocol/codex-acp`) son paquetes npm y necesitan Node.js. Cincel **no** usa el Node del sistema: descarga un Node LTS propio (~50 MB) en `~/.local/share/cincel/runtime/node-<versión>/` la primera vez que se conecta un agente, desde nodejs.org, verificando el SHA-256 publicado, con barra de progreso y reintento. Zed hace lo mismo para sus servidores de lenguaje.
- Los adaptadores se instalan con ese Node en `~/.local/share/cincel/agents/<agent_id>/<versión>/` (`npm install` con caché propia bajo `~/.cache/cincel/npm`), nunca en la caché de npm del sistema. La versión es la del registry ACP; hay "Actualizar" cuando el registry publica una nueva.
- **Distribuciones binarias** (Antigravity): el registry publica un archivo por plataforma (`distribution.binary.<os>-<arch>`: `archive`, `cmd`, `args`, `env`, `sha256`). Cincel lo descarga a `~/.cache/cincel/downloads/<agent_id>-<versión>-<plataforma>.<zip|tar.gz>.part` con progreso (MB y porcentaje), reanudación por `Range` y tres intentos; verifica el SHA-256 si el registry lo publica y, si es `null` (caso de Google), marca la instalación `verifiable: false` y el modal dice "Google no publica una suma de verificación para este paquete". Antes de descargar comprueba el espacio libre (Antigravity: 1,4 GB, porque el zip de 333 MB se descomprime a 1,05 GB y se borra al final) y, antes de descomprimir un zip, vuelve a comprobarlo con el tamaño exacto del directorio central. Descomprime (zip o tar.gz, sin escribir fuera del destino) en un directorio de staging, hace `chmod +x` del `cmd` y lo mueve a `~/.local/share/cincel/agents/<agent_id>/<versión>/`. Se lanza el `cmd` por ruta absoluta con los `args` del registry (Antigravity: `./agy_acp_server.par --uid=`), sin Node; el proceso conserva el directorio del proyecto (el servidor encuentra `localharness_external` junto a su propio ejecutable).
- Sin red: se usan las versiones ya instaladas; si no hay ninguna, el modal explica que hace falta conexión para instalar el agente.

## 4. Flujos de usuario

### F1. Conectar
Botón **"Conectar"** en el encabezado del chat (donde hoy está el selector de agente). Abre un popover con:
- Una fila por conexión: icono del proveedor, **etiqueta**, identidad en gris si existe ("gary@… · Max"), "Usado hace 2 h", insignia de estado (Conectada / Sesión vencida / No disponible).
- Al pie: **"Conectar nuevo agente…"** y **"Eliminar conexión…"**.
Elegir una fila: se lanza el proceso del agente con el perfil de esa conexión, se abre una conversación nueva, la barra de estado muestra el chip de la conexión. Si estaba activa otra, se detiene su proceso (sin borrar nada).

### F2. Conectar nuevo agente (modal)
1. **"Elegí un agente"**: Claude, Codex y **Google Antigravity** ("Tu suscripción de Google, vía el agente oficial de Antigravity"), del registry ACP; solo los que tienen login por suscripción. Muestra si el agente ya está instalado o se va a descargar.
2. **"Preparando…"**: Claude y Codex: descarga de Node privado y del adaptador si hacen falta, con progreso. Antigravity: solo su binario oficial (333 MB), con MB descargados y porcentaje, y la nota "Google no publica una suma de verificación para este paquete".
3. **"Iniciando sesión…"**, según el agente:
   - **Claude** (pty oculta): `claude --cli auth login --claudeai` vía el adaptador.
   - **Codex** (pty oculta): `codex login` con `BROWSER` neutralizado para que imprima el enlace.
   - **Antigravity** (modo **ACP authenticate**): se lanza el propio servidor con el perfil (`GEMINI_HOME`, `BROWSER=/bin/true`), `initialize` anunciando `elicitation.url` y `authenticate { methodId: "oauth-personal" }`. El enlace llega por stderr (`Open the following link to authenticate the ACP server: https://accounts.google.com/…`, con `redirect_uri=http://127.0.0.1:<puerto>/`) o, si algún día lo manda, como `elicitation/create` en modo `url`; las líneas de stderr pasan por la redacción antes de mostrarse.
   En la pty el enlace se extrae con una expresión regular sobre la salida.
4. **"Abrí este enlace y aprobá el acceso"**: enlace en monoespaciada, botones **"Copiar"** y **"Abrir en el navegador"** (portal xdg). Según el agente: Claude muestra además el campo **"Pegá acá el código que te dio el navegador"** (se envía al proceso); Codex termina solo por su callback local; Antigravity termina solo por su servidor local de redirección, sin campo de código ("Cuando apruebes el acceso, Google vuelve a Cincel solo: no hay código que pegar"). Un desplegable **"Ver detalles técnicos"** muestra la salida (redactada) del login.
5. **"Esperando que apruebes en el navegador…"** con spinner y **"Cancelar"** (mata el grupo de procesos y borra el perfil a medio crear). Éxito: Claude y Codex, el proceso termina con código 0 (y, si el agente la manda, la identidad por `_auth/status_update`); Antigravity, la respuesta correcta de `authenticate` más el archivo de token en el perfil (sin identidad: no anuncia `_auth/status_update` y su token no trae email). El propio Antigravity corta su servidor de redirección a los 5 minutos: `authenticate` responde con error y el modal ofrece "Reintentar".
6. **"Listo"**: "Conectado como gary@… (Claude Max)" si hay identidad; si no, solo "Conectado". Campo **"Nombre de la conexión"** con sugerencia; botón **"Guardar"**. Recién ahí se persiste la conexión y se activa (F1).
Errores: tiempo de espera agotado (15 min), proceso terminado con error, `authenticate` rechazado, sin red, sin espacio en disco, sin navegador. Cada uno con mensaje claro y **"Reintentar"**.
Los logins de un mismo proveedor se serializan (Codex usa un puerto local fijo).

### F3. Eliminar conexión (modal)
Lista de conexiones; al elegir una: "¿Eliminar «Codex · trabajo»? Se cerrará la sesión, se borrarán sus credenciales de este equipo y sus N conversaciones." Botones **"Eliminar"** (neutro pero con confirmación) y **"Cancelar"**. Secuencia: lanzar el agente con el perfil → `initialize` → `logout` si lo anuncia → matar el grupo de procesos → borrar `connections/<id>/` (validando que la ruta esté dentro de la raíz) → borrar sus conversaciones → quitar del índice. Los tres agentes anuncian `logout` (Antigravity: `agentCapabilities.auth.logout = {}`, que además borra su token). Para Antigravity el modal y el aviso final agregan "Para revocar el acceso, hacelo desde tu cuenta de Google". Una conexión vieja de Gemini (agente retirado) aparece "No disponible" con el motivo y se borra directamente, sin lanzar nada.

### F4. Sesión vencida
Cuando el agente responde que requiere autenticación con una conexión existente (token revocado, contraseña cambiada, caducidad): insignia ámbar "Sesión vencida" en la lista, banner en el chat "La sesión de «Claude · personal» venció" con **"Volver a conectar"**, que repite F2 desde el paso 3 sobre el mismo perfil, conservando etiqueta y conversaciones (para Antigravity, el mismo modo ACP authenticate). Antigravity, con el token guardado pero inválido, no responde `auth_required` sino que abre un login en el navegador dentro de `session/new` e imprime el enlace por stderr: Cincel lo detecta (sin guardar la consulta del enlace) y muestra el mismo banner.

### F5. No disponible
Si falta el adaptador, el binario o el Node privado (por ejemplo tras un borrado manual), la conexión aparece "No disponible" con **"Reparar"**, que vuelve a descargar lo que falte sin tocar las credenciales. Antigravity no necesita Node.

## 5. Cambios en el módulo ACP (`cincel-acp` → `cincel-acp`)
1. Quitar `NO_BROWSER=1` por defecto (opción por agente).
2. Corregir la negociación de `agentFileChangeReport`: `_meta.jetbrains.air = {"version":1,"capabilities":["agentFileChangeReport"]}` en `initialize`, y leer el array de strings del `_meta` de nivel superior del `InitializeResponse`.
3. `AgentCommand::Logout` → `logout` cuando `agentCapabilities.auth.logout` está anunciado; evento `LoggedOut`.
4. `Authenticate` no bloquea el loop de comandos; emite `AuthSucceeded`/`AuthFailed`.
5. Procesar la extensión `_auth/status_update` → `AgentEvent::AuthStatus { kind, label, account { email, organization, plan } }`.
6. Manejar `elicitation/complete`.
7. `AgentConnection::start` recibe `env` del perfil (variable de directorio + variables a quitar) y el Node privado en el `PATH` del hijo.
8. Subir `agent-client-protocol` a 2.2.0. Seguir en ACP v1; tipos aislados detrás de `protocol.rs`; spike de v2 detrás de feature flag, sin activar.
9. Nuevo crate `cincel-connections` (sin GPUI): índice de conexiones, perfiles, runtime privado, instalación de adaptadores, flujo de login en pty (`portable-pty`) con extracción de enlace y código, eliminación segura. Tests con un "agente de login falso" que imprime un enlace y espera un código.

## 6. Interfaz
- Encabezado del chat: botón **"Conectar"** (o el nombre de la conexión activa con caret cuando hay una); sin conexión, el estado vacío dice "No hay ningún agente conectado" con el botón.
- Barra de estado: chip con icono del proveedor + etiqueta de la conexión activa; clic abre el popover de F1.
- Los modales siguen el estilo del diálogo de guardar: `bg.elevated`, radio 6, borde 1 px, botones neutros iguales, foco con anillo, scrim al 40%.
- El modal de conexiones sigue el ancho de la ventana entre 420 y 560 px (con 24 px de margen), y nada se sale de él. La grilla de agentes es un flex que se envuelve: tarjetas de al menos 160 px que crecen para repartirse la fila, así que cualquier cantidad de agentes se acomoda en filas. Cada tarjeta: monograma, nombre (14 px, peso 500), descripción (12 px, `text.muted`, se ajusta dentro de la tarjeta) y estado ("instalado" o "se descargará", con el tamaño cuando se conoce de antemano: "se descargará (333 MB)" para Antigravity).
- Renombrar una conexión: desde el popover (menú contextual "Renombrar").
- El selector de modo/modelo/esfuerzo del agente y todo lo demás del chat no cambia.

## 7. Renombrado a Cincel
- Nombre de producto "Cincel"; binario y comando `cincel`; crates `cincel-*`; identificadores, textos de interfaz y documentación. `Cargo.toml`: `repository` con la URL real que indique el autor.
- Directorios: `~/.config/cincel`, `~/.local/state/cincel`, `~/.local/share/cincel`, `~/.cache/cincel`. **Migración automática** al primer arranque: si existen los directorios `cincel` y no los `cincel`, se mueven (o copian y se deja un aviso) conservando ajustes, keymap, temas, layouts, conversaciones y revisiones pendientes.
- `.gitignore`, README, LICENSE (sin cambios de licencia), specs y etapas: reemplazo completo del nombre; la investigación histórica puede mantener "Cincel" con una nota al inicio.

## 8. Log a archivo
`tracing` a `~/.local/state/cincel/log/cincel.log` con rotación diaria (7 archivos), nivel `info` por defecto y `RUST_LOG` para subirlo; además de stderr. Nunca se escriben tokens, enlaces de login ni códigos (filtro en el capturador de salida del login).

## 9. Ajustes
`settings.json`: sección `agents` pasa a `connections` (`default_label: null`, `runtime: { node_version: "lts" }`), se quita `agents.default` (no hay conexión por defecto). `review.sensitive_paths` se mantiene.

## 10. Criterios de aceptación
- [ ] Al arrancar no se lanza ningún proceso de agente ni se lee nada de `~/.claude`, `~/.codex` ni `~/.gemini` (test: esos directorios pueden no existir).
- [ ] Antigravity: se instala desde el archivo `binary` del registry (zip o tar.gz) con progreso, reanudación, chequeo de espacio y `verifiable: false` sin `sha256`; inicia sesión por ACP `authenticate` (enlace por stderr o elicitación `url`), sin código; su token queda en `<perfil>/antigravity-acp/acp_token.json`; eliminar hace `logout` con `GEMINI_HOME` del perfil y borra el perfil.
- [ ] Crear una conexión de Claude y otra de Codex con perfiles vacíos: ambas funcionan; `claude auth status` y `codex login status` del sistema no cambian.
- [ ] Dos conexiones de Codex con cuentas distintas conviven y se pueden alternar.
- [ ] Eliminar una conexión borra su perfil y conversaciones, y no afecta a las demás ni al sistema.
- [ ] Sesión vencida simulada (borrar el archivo de credenciales del perfil) → insignia, banner y "Volver a conectar" recupera sin perder conversaciones.
- [ ] Sin Node en el sistema (test con `PATH` vacío) todo funciona con el runtime privado.
- [ ] `agentFileChangeReport` negociado: el agente falso lo anuncia y Cincel recibe el reporte al final del turno.
- [ ] Ningún log contiene enlaces de login, códigos ni tokens.
- [ ] Renombrado: `cincel .` arranca, migra las carpetas `cincel`, y no queda "cincel" en código, textos ni docs (salvo la nota histórica en `docs/research`).

## 11. Lista de comprobación manual (autor)
1. Arrancar `cincel .`: nada conectado, botón "Conectar".
2. "Conectar nuevo agente" → Claude: descarga (solo la primera vez), enlace, abrir en el navegador, pegar código, nombrar. Chatear.
3. Repetir con Codex (aprobar en el navegador, sin código) y con **Google Antigravity**: la primera vez descarga 333 MB (ver MB y porcentaje, y la nota "Google no publica una suma de verificación para este paquete"); aparece el enlace de `accounts.google.com`; "Abrir en el navegador", aprobar con tu cuenta de Google; el modal pasa solo a "Listo" (sin código); nombrar ("Antigravity · personal") y chatear. Comprobar que `~/.gemini` no cambió.
4. Crear una **segunda** conexión de Codex con otra cuenta; alternar entre las dos y ver que cada conversación pertenece a su conexión.
5. Cambiar de conexión: la anterior se detiene, nada se borra.
6. Eliminar una conexión: confirmación con el número de conversaciones; desaparece; `codex login status` en tu terminal sigue igual.
7. Provocar "Sesión vencida" (borrar `.credentials.json` del perfil, o `antigravity-acp/acp_token.json` para Antigravity) y usar "Volver a conectar".
8. Verificar que las carpetas `~/.config/cincel` y `~/.local/*/cincel` migraron a `cincel` con tus ajustes y conversaciones.
9. Confirmar que el reporte de archivos tocados llega (en el log) al terminar un turno con edición.
