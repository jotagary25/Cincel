# Etapa 4: conexiones de agentes y renombrado a Cincel

Estado: **workstreams A, B y C terminados, y D (Gemini reemplazado por
Antigravity), sin commitear** (2026-09-26, D el 2026-09-27).
Spec: `docs/specs/06-etapa4-conexiones-y-cincel.md`. Este documento lo
escribió el workstream C (interfaz de conexiones e integración en el
workspace); A y B se describen tal como C los encontró en el árbol de
trabajo.

## Qué se construyó

### A — renombrado a Cincel, migración y log (recibido terminado)

- Producto, binario y crates `cincel-*`; textos y documentación con el
  nombre nuevo (la investigación histórica conserva una nota).
- `cincel_settings::migrate_xdg_dirs`: al primer arranque mueve
  `~/.config/asteroid`, `~/.local/state/asteroid`, `~/.local/share/asteroid`
  y `~/.cache/asteroid` a `cincel` (idempotente, nunca fatal) y el workspace
  muestra el toast "Migrada la configuración de Asteroid a Cincel".
- `cincel-log`: `tracing` a `~/.local/state/cincel/log/cincel.log` con
  rotación diaria (7 archivos) además de stderr, y `redact` para lo que
  captura el login.
- `settings.json`: la sección `agents` pasó a `connections`
  (`default_label`, `runtime.node_version`, `mcp_servers`); `agents.default`,
  `custom` y `registry_url` se retiraron con aviso.

### B — motor de conexiones y cambios de ACP (recibido terminado)

- `cincel-connections` (sin GPUI): índice sin credenciales, perfiles
  aislados 0700 con la variable de perfil por agente y las credenciales del
  sistema quitadas, Node LTS privado (SHA-256, reanudación, reintentos),
  adaptadores con npm privado, login en pty oculta con extracción de enlace,
  código y "pegá el código", serialización por proveedor, sondas de
  identidad y eliminación segura. Detalle en `docs/specs/modulos/connections.md`.
- `cincel-acp`: `NO_BROWSER` opcional, negociación real de
  `agentFileChangeReport`, `Logout`/`LoggedOut`, `Authenticate` fuera del
  loop con `AuthSucceeded`/`AuthFailed`, `_auth/status_update` →
  `AuthStatus`, `elicitation/complete` → `ElicitationCompleted`,
  `AgentConnection::start_with_env`. Detalle en `docs/specs/modulos/acp.md`.

### C — interfaz de conexiones (esta sesión)

1. **Sin conexión automática** (§1, §10). `Agents::new` lee el índice para
   llenar el popover y nada más: no lanza procesos, no descarga el registry
   ACP (antes lo hacía al arrancar; ahora solo al preparar un agente desde
   el modal), no ejecuta `node --version` ni lee `~/.claude`, `~/.codex` o
   `~/.gemini`. Se quitó `DEFAULT_AGENT_ID`. La barra de estado dice
   "Sin conexión". El log de arranque registra
   `conexiones leídas; no se lanza ningún agente hasta que el usuario elija una`.
2. **Modelo de conexión en la UI** (`cincel-chat`). El selector de agente del
   encabezado es ahora el control "Conectar": botón sin conexión activa;
   icono del proveedor + etiqueta + identidad en gris + caret con una. El
   popover lista `ChatConnection`s (icono, etiqueta, identidad, "Usado hace
   2 h", insignia Conectada / Sesión vencida / No disponible), con "Volver a
   conectar" o "Reparar" en las filas que lo necesitan, menú contextual
   "Renombrar" (clic derecho) y "Conectar nuevo agente…" / "Eliminar
   conexión…" al pie. Elegir una fila (`Agents::activate_connection`): guarda
   la conversación, detiene el proceso anterior (sin borrar nada),
   `agent_launch` → `AgentConnection::start_with_env` + `Spawn`, `touch`, y
   abre una conversación vacía con `connection_id`. Chip en la barra de estado
   (icono + etiqueta; clic abre el popover y despliega el dock del chat).
   `Conversation.connection_id` / `IndexEntry.connection_id` con
   `#[serde(default)]`; las conversaciones viejas quedan bajo
   "(conexión anterior)" y se abren de solo lectura; el historial se agrupa por
   la etiqueta actual de cada conexión. Abrir una conversación de otra conexión
   cambia a esa conexión y reabre su sesión si el agente puede.
3. **Modal "Conectar nuevo agente"** (`crates/cincel-workspace/src/connection_modal.rs`):
   los seis pasos con la redacción de la spec (grilla Claude/Codex/Gemini —hoy
   Google Antigravity, ver D— con
   "instalado"/"se descargará"; "Preparando…" con dos barras alimentadas por
   `RuntimeProgress`/`AdapterProgress`; "Iniciando sesión…"; "Abrí este enlace
   y aprobá el acceso" con el enlace en monoespaciada, "Copiar", "Abrir en el
   navegador" (`cx.open_url`), el código de un solo uso con `CodeDetected`, el
   campo "Pegá acá el código que te dio el navegador" + "Enviar" (`send_code`)
   con `NeedsPastedCode` y el desplegable "Ver detalles técnicos" con las líneas
   `Output` ya redactadas; "Esperando que apruebes en el navegador…" con
   spinner y "Cancelar" (`cancel`); "Listo: conectado como … (…)" o
   "Conectado", "Nombre de la conexión" con `suggest_label` y "Guardar" →
   `finalize` + activación). Errores (tiempo agotado, salida ≠ 0, sin red, sin
   navegador, proceso que no arranca, sin sesión al terminar) con mensaje claro
   y "Reintentar"; el login encolado detrás de otro del mismo proveedor dice
   "Esperando que termine otro inicio de sesión de …". `Esc` cierra solo si no
   se pierde nada; con un login en curso o una conexión sin guardar pregunta
   ("Seguir acá" / "Cerrar igual"). Estilo del diálogo de guardar:
   `bg.elevated`, radio 6, borde 1 px, scrim 40 %, botones neutros iguales con
   anillo de foco en el de `Enter`, cursor de mano. Los hilos (preparar,
   login, eliminar) informan por `async_channel` a una tarea de primer plano;
   una generación descarta mensajes viejos.
4. **Modal "Eliminar conexión"** (F3): lista → "¿Eliminar «X»? Se cerrará la
   sesión, se borrarán sus credenciales de este equipo y sus N
   conversaciones." ("su 1 conversación" en singular; N cuenta las de todos
   los proyectos) → "Eliminar" emite `WillDelete`, el workspace detiene el
   proceso si era la activa ("Sin conexión") y recién entonces corre
   `disconnect` (el logout nunca convive con el agente vivo sobre el mismo
   perfil) → se borran las conversaciones de esa conexión en todos los
   proyectos (archivo + fila de `index.json`) → sale de la lista. Si algún
   paso falló se muestra una línea por paso; si todo salió bien, un toast.
   Antigravity (antes Gemini): nota "Para revocar el acceso, hacelo desde tu
   cuenta de Google." en la confirmación y en el toast.
5. **Sesión vencida y No disponible** (F4, F5). `AuthRequired`, `AuthFailed`,
   `LoggedOut { ok: true }`, `AuthStatus { kind: none }` o `status()` =
   `SessionExpired` → insignia ámbar, banner "La sesión de «X» venció" con
   "Volver a conectar" → `start_relogin` y el modal desde el paso 3 → al
   terminar, `set_identity`, el proceso se relanza y la etiqueta y las
   conversaciones quedan. Un agente que dice que no hay sesión gana aunque el
   archivo de credenciales siga ahí (token revocado). "No disponible" →
   banner/fila con "Reparar" → `prepare` (sin tocar credenciales) → al
   terminar se relanza si era la activa.
6. **Eventos**: `AuthSucceeded` limpia el vencimiento; `AuthFailed` se muestra
   (toast + aviso de error en el chat, nunca se traga); `LoggedOut`,
   `AuthStatus` (identidad con `set_identity`, se ve en el popover y el
   encabezado) y `ElicitationCompleted` (aviso en el chat). Las tarjetas de
   permiso de rutas sensibles siguen igual (política fija, tests).
7. **Limpieza**: fuera `ChatAgent`, `set_agents`/`set_active_agent`,
   `ChatEvent::AgentSelected`, `Popover::Agents`, la carga del registry al
   arrancar, `install_registry`, las pistas de "Instalá Node 22+" / "uv" y el
   texto "hace falta Node 18" del estado vacío. `connections.default_label`
   solo preselecciona la fila (también en recarga en caliente, junto con
   `mcp_servers`). El toast de migración sigue.
8. **Arreglo previo**: `cargo check -p cincel-workspace -p cincel-chat
   --all-targets` sin features vuelve a compilar (el test GPUI de `theme.rs`
   y los ayudantes de test quedan detrás de `test-support`).
9. **Cambios mínimos fuera de los crates de C**: solo comentarios en
   `cincel-log` (`redact.rs` y `lib.rs` decían que la captura del login era
   "futura").

### D — Gemini reemplazado por Antigravity (2026-09-27)

**Causa raíz.** Google dejó de aceptar el inicio de sesión personal de
Gemini CLI: con gemini-cli 0.61.0 (la versión del registry ACP) el login
"Log in with Google" termina en `IneligibleTierError` con el mensaje "This
client is no longer supported for Gemini Code Assist for individuals…
migrate to Antigravity". Era la "sesión vencida" que el autor veía con
Gemini el 26-sep (la investigación de ese día dejó el rechazo en el log vía
`recent_stderr`). No hay arreglo del lado de Cincel: el cliente quedó fuera.

**Reemplazo.** El tercer agente por suscripción es el servidor ACP oficial
de Google Antigravity (`antigravity-acp` 1.2.1 en el registry, distribución
`binary`: un zip de 333 MB con `agy_acp_server.par`, 920 MB, y
`localharness_external`; `cmd ./agy_acp_server.par`, `args ["--uid="]`,
`sha256: null`). Verificado con el binario real en un `GEMINI_HOME`
temporal: habla ACP v1, anuncia `oauth-personal` "Log in with Google",
`auth.logout` y `loadSession`, **no** anuncia `_meta.authStatus`, y
`authenticate` imprime el enlace por stderr y completa solo por su servidor
local de redirección. Hechos completos en el anexo de
`docs/research/06-acp-estado-y-cuentas.md`.

1. **`cincel-connections`**: distribuciones `binary` en `Adapters`
   (`binary_plan`, `install_binary`: zip y tar.gz, descarga con progreso y
   `Range`, SHA-256 si está publicado, `verifiable: false` si no, chequeo de
   espacio de 1,4 GB antes de descargar y con el tamaño exacto antes de
   descomprimir, `chmod +x` del `cmd`, staging + rename; `launch_spec` lanza el
   `cmd` con los `args` del registry, sin Node). `AgentKind::Antigravity`
   reemplaza a `Gemini` (perfil `GEMINI_HOME=<perfil>`, token
   `antigravity-acp/acp_token.json`, prefijos `GOOGLE_*`/`GEMINI_*`/
   `CLOUDSDK_*`/`GCLOUD_*`/`ANTIGRAVITY_*`/`AGY_*` quitados, `BROWSER=/bin/true`);
   fuera el `settings.json` sembrado de Gemini y su login por pty. Nuevo modo
   de login **ACP authenticate** en `LoginSession` (`spawn_acp`): `initialize`
   → `authenticate { methodId: "oauth-personal" }`, enlace por stderr o por
   elicitación `url` → `UrlDetected`, `Completed` con la respuesta de
   `authenticate` y el token en el perfil, `Failed` por error, salida del
   proceso o 15 min, `cancel` mata el grupo, stderr redactado; lo usa también
   "Volver a conectar". Eliminar hace `logout` ACP con el perfil. `prepare`,
   `status`, `agent_launch` y el login no piden Node para Antigravity. Una
   conexión vieja de Gemini queda "No disponible" con el motivo y se puede
   eliminar (sin lanzar nada).
2. **Modal**: la grilla muestra Claude, Codex y **Google Antigravity** ("Tu
   suscripción de Google, vía el agente oficial de Antigravity"), cada una con
   su descripción; "Preparando…" de Antigravity tiene una sola barra ("Agente
   oficial") con MB y porcentaje ("Descargando Google Antigravity 1.2.1: 120 de
   333 MB (35 %)") y la nota "Google no publica una suma de verificación para
   este paquete…"; el paso del enlace muestra "Copiar" / "Abrir en el
   navegador", "Esperando que apruebes en el navegador…" y "Cuando apruebes el
   acceso, Google vuelve a Cincel solo: no hay código que pegar." (sin campo de
   código). Eliminar una conexión de Antigravity agrega "Para revocar el
   acceso, hacelo desde tu cuenta de Google." en la confirmación y el toast.
   Nuevos mensajes: `authenticate` rechazado y falta de espacio en disco.
3. **Chat / agentes**: nombre "Antigravity" y monograma "A"; si el agente de
   la conversación imprime un enlace de login por stderr (Antigravity con un
   token guardado que dejó de valer abre un login en el navegador dentro de
   `session/new` en vez de responder `auth_required`), se muestra el banner de
   sesión vencida, y las líneas de stderr que se guardan para el log van con
   la consulta de las URL redactada.

### E — correcciones (2026-09-27)

1. **"Elegí un agente" cortaba la tercera tarjeta.** El diálogo medía 520 px
   fijos y la grilla era una fila de tarjetas `flex_1` sin ancho mínimo ni
   envoltura, así que "Google Antigravity" se salía por la derecha y los
   textos se partían mal. Ahora el diálogo sigue la ventana entre 420 y 560 px
   (`w_full` + `min_w`/`max_w`, 24 px de margen, `overflow_x_hidden`), la
   grilla es un flex con `flex_wrap` y `items_stretch`, y cada tarjeta tiene
   base y mínimo de 160 px, crece (`flex_grow`) y recorta (`overflow_hidden`):
   cualquier cantidad de agentes se acomoda en filas. Tarjeta: monograma,
   nombre 14 px peso 500, descripción 12 px `text.muted` que se ajusta
   dentro, y estado "instalado" / "se descargará (333 MB)" (tamaño conocido
   solo para la distribución binaria: `download_size_hint`). El diálogo
   "¿Cerrar?" interno pasó de 380 px fijos a `max_w` 380. Test
   `the_agent_grid_never_overflows_the_dialog` (900×700, 1400×900 y 480×700:
   diálogo entre 420 y 560, cada tarjeta dentro del diálogo, una fila en las
   dos primeras y dos filas en la angosta) y `agent_cards_say_installed_or_the_download_size`.
   Captura real (COSMIC, `cosmic-screenshot`): las tres tarjetas en una fila,
   de igual alto, con el texto ajustado dentro y "Cancelar" abajo a la derecha.
2. **El código pasaba "debajo" del árbol de archivos.** Diagnóstico: los
   límites del editor ya terminaban donde empieza el dock y la pintura ya iba
   con máscara (medido en test y en la captura real), pero con el ajuste de
   línea desactivado por defecto las líneas largas quedaban cortadas en el
   borde del dock y arrastrar el borde no las volvía a acomodar, que es lo que
   se veía como "dibuja por debajo". Cambios: `editor.soft_wrap` pasa a
   `true` por defecto (`cincel-settings`, `cincel-editor`,
   `default_settings_jsonc`; `Alt+Z` sigue alternando, ahora vía
   `EditorView::set_soft_wrap`); el elemento fija `min_size.width = 0` y
   `overflow.x = hidden` (su ancho es el del padre, nunca el del contenido);
   el ajuste se recalcula cuando cambia el ancho (las columnas de
   `ensure_wrap` salen del ancho del layout); el cuerpo de la pestaña en
   `center.rs` es `flex_1` + `min_w_0` + `w_full` + `overflow_hidden` y el
   panel central `min_w_0` + `overflow_hidden`. La sonda de render ahora
   guarda `bounds`, `text_width`, `wrap_rows` y la máscara efectiva de la
   pintura (`paint_clip`). Tests: `a_long_line_stays_inside_a_narrow_container_and_rewraps`
   (línea de 400 caracteres en 300 px: el elemento mide 300 px, la máscara no
   sale de él, con y sin ajuste; a 180 px hay más filas y a 300 vuelve),
   `soft_wrap_is_the_default_and_alt_z_turns_it_off`,
   `the_editor_ends_at_the_right_dock_and_follows_its_resize` (sin ajuste,
   dock de 240/420/180 px: el editor termina donde empieza el dock y lo
   sigue) y `resizing_the_right_dock_rewraps_the_editor` (con ajuste: el
   editor pierde exactamente lo que gana el dock y suma filas).
3. **El login de Claude abría el navegador solo.** `BROWSER` neutro se ponía
   solo para Antigravity (y como extra para Codex); el CLI de Claude ejecuta
   `$BROWSER <url>` antes de `xdg-open` (verificado en el binario 0.81.2),
   así que abría una pestaña. Nuevo `Profile::login_env`: el entorno del
   perfil más `BROWSER=/bin/true` para **todo** login (pty de Claude y Codex,
   `authenticate` de Antigravity); el proceso del chat de Claude y Codex no lo
   lleva. Tests `claude_and_codex_logins_run_with_a_neutral_browser`
   (`plan_login` real), `every_login_env_mutes_the_browser_but_chat_env_does_not`
   y `claude_link_is_detected_with_a_neutral_browser_and_colors` (el enlace
   de "If the browser didn't open, visit:" se detecta con colores o en la
   línea siguiente).

Verificación de E: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets` con las features de test `-D warnings`, `cargo test
--workspace` con las mismas features (931 tests, 0 fallos),
`cargo run -p cincel -- --smoke-test .` (salida 0) y `pgrep -af sleep` vacío.

## Verificación

Máquina de referencia (Pop!_OS, COSMIC Wayland), 2026-09-26 (A–C; la tabla
de D está al final de esta sección):

| Comando | Resultado |
|---|---|
| `cargo fmt --all --check` | ok |
| `cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings` | ok |
| `cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins` y `cargo test --workspace` con las mismas features | ok, 890 tests (`cincel-workspace` 129, `cincel-chat` 97) |
| `cargo check -p cincel-workspace -p cincel-chat --all-targets` (sin features) | ok, sin avisos |
| `cargo deny check licenses` | ok |
| `cargo run -p cincel -- --smoke-test .` | salida 0; bajo `strace -f -e trace=%file,execve` no hay ningún acceso a `.claude`, `.codex` ni `.gemini` y los únicos `execve` son el binario y `git` (estado del proyecto); el log dice "no se lanza ningún agente" |
| `cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .` (fugas) | salida 0 |
| Ejecución real de 5 s (`timeout 5 cincel .`) | sin pánicos ni errores; captura por XWayland: "Conectar" en el encabezado, "No hay ningún agente conectado" con su botón, "Sin conexión" en la barra |
| `pgrep -af sleep` | vacío |

No se hicieron inicios de sesión reales ni se lanzaron agentes reales: todo
va contra el programa de login falso de `cincel-connections` y el agente ACP
falso de `cincel-acp`.

### Tests nuevos

- `cincel-workspace/src/agents.rs` (fixture `test_support::FakeEnv`: un
  directorio de datos con un "Node" que es un script de shell y adaptadores
  que ejecutan `cincel-connections-fake-login` para el login y
  `cincel-acp-fake-agent` para ACP): arranque sin procesos; `default_label`
  preselecciona sin conectar; popover con estados (Conectada / Sesión vencida /
  No disponible), identidad y "Usado…"; activar lanza con el perfil de la
  conexión (el agente ve `CLAUDE_CONFIG_DIR` = su perfil); cambiar de conexión
  detiene el proceso anterior y conserva sus conversaciones; reabrir una
  conversación vuelve a su conexión; conversación vieja de solo lectura; alta
  completa por el modal (enlace, código pegado, completado, nombre, activación,
  conversación ligada); Codex sin código; cancelar borra el perfil pendiente;
  `Esc` con login en curso pregunta; código incorrecto → error + "Reintentar";
  sin red ni runtime → mensaje; eliminar la activa borra perfil, índice y
  conversaciones (solo las suyas) y queda "Sin conexión"; sesión vencida →
  banner → "Volver a conectar" conserva etiqueta y conversaciones;
  `AuthRequired` marca vencida; `AuthFailed` se muestra; `AuthStatus`
  actualiza la identidad y `kind: none` vence; `LoggedOut`/`ElicitationCompleted`
  llegan al chat; "Reparar" sin tocar credenciales; renombrar (y nombre
  vacío). Los tests de ACP de siempre se portaron al arnés de conexiones.
- `connection_modal.rs`: redacción exacta de la confirmación, mensajes de
  error, barras de progreso, informe por pasos, línea "Listo".
- `cincel-chat/src/tests.rs`: control "Conectar" y estado vacío, etiqueta en
  el encabezado, filas con insignias, preselección sin conectar, pie y menú
  contextual, banner con "Volver a conectar"/"Reparar", `AuthFailed` visible,
  avisos de `LoggedOut`/`ElicitationCompleted`, historial agrupado por
  conexión, conversación vieja sin `connection_id`, nombres/monogramas.
- `cincel-workspace/src/tests.rs`: la ventana abre sin conexión, el chip
  sigue a la elección, clic abre el popover y eliminar vuelve a "Sin conexión".

## Criterios de aceptación (spec §10)

- [x] Al arrancar no se lanza ningún proceso de agente ni se lee nada de
  `~/.claude`, `~/.codex` ni `~/.gemini` (test de arranque + `strace` del
  smoke test).
- [ ] Crear una conexión de Claude y otra de Codex con perfiles vacíos y
  comprobar `claude auth status` / `codex login status` del sistema: manual
  (punto 2–3 de la lista de abajo); cubierto con el login falso en los tests.
- [ ] Dos conexiones de Codex con cuentas distintas: manual (punto 4).
- [x] Eliminar borra perfil y conversaciones sin afectar a las demás (test);
  el sistema, manual (punto 6).
- [x] Sesión vencida simulada (credenciales borradas) → insignia, banner y
  "Volver a conectar" sin perder conversaciones (test).
- [ ] Sin Node en el sistema: el runtime privado lo cubre B (test de B con
  `PATH` vacío); con agentes reales, manual.
- [x] `agentFileChangeReport` negociado (B, test de integración de ACP).
- [x] Ningún log contiene enlaces, códigos ni tokens: el modal solo muestra y
  nunca registra el enlace ni el código; "Ver detalles técnicos" muestra las
  líneas ya redactadas (test del alta lo comprueba).
- [x] Renombrado y migración (A).

## Desviaciones

- **Icono del proveedor**: monograma (C, X, A) en un cuadrado redondeado.
  `gpui-kit` trae Lucide, sin logos de marcas, y el proyecto no incluye esos
  recursos.
- **"Sin navegador"** se detecta por heurística (`$BROWSER` o `xdg-open` en el
  `PATH`): `cx.open_url` no informa si falló. El mensaje invita a copiar el
  enlace.
- **Paso 5** ("Esperando que apruebes en el navegador…") aparece como línea
  con spinner en el paso 4 y como pantalla propia después de enviar el código;
  Codex (sin código) se queda en el paso 4 hasta terminar.
- **Cerrar durante "Preparando…"** no corta una descarga en curso: el hilo
  termina (las descargas son reanudables) y su resultado se descarta.
- **"Volver a conectar" termina con "Continuar"** (no hay nombre que elegir:
  se conserva); "Reparar" termina con "Cerrar".
- **Renombrar** abre un modal chico con el campo, no edición en línea dentro
  del popover (el popover no tiene campos de texto).
- **`AuthRequired` ya no pinta la tarjeta "Hace falta autenticarse"** del
  chat (que ofrecía abrir una terminal): la reemplaza el banner con "Volver a
  conectar", que repite el login aislado. La tarjeta sigue en `cincel-chat`
  para quien la use directamente.
- **Detener a propósito** (cambio de conexión, eliminar, relanzar) no deja el
  aviso "El agente se cerró" en la conversación; solo la píldora pasa a
  `desconectado`. Una caída real sigue mostrando el aviso y el toast con
  "Reiniciar".
- **Una conexión vencida o no disponible no se lanza** al elegirla: muestra el
  banner. Elegirla de nuevo es un intento nuevo (se olvida el vencimiento que
  había dicho el agente, no el del perfil).
- **Conversaciones de una conexión eliminada**: se borran en todos los
  proyectos y solo si `disconnect` la sacó del índice (si no pudo borrar el
  perfil, la conexión sigue listada con sus conversaciones).
- **En los tests** preparar y eliminar corren en línea y el login se alimenta
  a mano (el ejecutor determinista de GPUI no tolera hilos ajenos que
  despierten tareas); preparar usa `registry = None` (lo instalado). En la app
  real todo corre en hilos propios.
- **Verificación visual**: COSMIC no expone captura de pantalla a la sesión
  (`grim` no tiene `wlr-screencopy`) y no hay herramienta para hacer clic, así
  que solo se capturó la pantalla inicial por XWayland; el popover y los
  modales se verificaron por tests (estructura y `debug_bounds`), no a ojo.
  Queda en la lista manual.
  (27-sep: `cosmic-screenshot --interactive=false` sí captura la pantalla
  completa; así se miraron el modal de agentes y el editor con ajuste de
  línea de las correcciones E.)
- **Tamaño de descarga en la tarjeta**: solo Antigravity dice "se descargará
  (333 MB)"; Claude y Codex dicen "se descargará" sin tamaño, porque lo que
  baja npm depende de lo que resuelva (el registry no publica tamaños) y un
  número inventado sería peor que ninguno.

## Deseos

- Logos de proveedor como recursos propios (licencia a revisar).
- Botón "Actualizar" cuando el registry publica un adaptador más nuevo
  (`Adapters::update_available` ya existe; spec §3), fuera del alcance de C.
- Cancelar una descarga en curso desde "Preparando…".
- Porcentaje real de `npm install` (hoy la barra del adaptador es
  indeterminada hasta terminar).
- Mostrar las elicitaciones de tipo URL del agente (hoy solo se informa su
  cierre).
- Un modo de captura de pantalla para tests visuales de GPUI.

## Lista de comprobación manual (autor)

Antes de empezar: `cargo build --release -p cincel` y, en una terminal,
anotar `claude auth status` y `codex login status` del sistema.

1. **Arranque sin conexión.** `cincel .`: el encabezado del chat muestra el
   botón "Conectar", el chat dice "No hay ningún agente conectado" y la barra
   de estado "Sin conexión". En `~/.local/state/cincel/log/cincel.log` debe
   aparecer "no se lanza ningún agente hasta que el usuario elija una" y
   `ps -ef | grep -E "claude|codex|gemini"` no muestra procesos.
2. **Claude.** "Conectar" → "Conectar nuevo agente…" → Claude (la primera vez
   dice "se descargará"): "Preparando…" con las dos barras (Node, adaptador) →
   "Iniciando sesión…" → aparece el enlace en monoespaciada; probar "Copiar"
   (pegarlo en un editor) y "Abrir en el navegador"; aprobar; pegar el código
   en "Pegá acá el código que te dio el navegador" y "Enviar" → "Esperando que
   apruebes…" → "Listo: conectado como tu@correo (plan)" → dejar o cambiar
   "Claude · personal" → "Guardar". El encabezado y la barra muestran la
   conexión; mandar un mensaje y recibir respuesta. Abrir "Ver detalles
   técnicos" en algún momento y comprobar que no se ve el enlace completo ni el
   código.
3. **Codex y Google Antigravity.** Repetir con Codex (se aprueba en el
   navegador, sin código; termina solo). Luego Google Antigravity: la primera
   vez "Preparando…" descarga 333 MB con MB y porcentaje y muestra "Google no
   publica una suma de verificación para este paquete"; aparece un enlace de
   `accounts.google.com` → "Abrir en el navegador" → aprobar con tu cuenta de
   Google → el modal pasa solo a "Listo" / "Conectado" (sin código ni correo:
   Antigravity no informa identidad) → "Antigravity · personal" → "Guardar" →
   chatear. En la terminal, `claude auth status` y `codex login status` deben
   seguir igual que antes, y `~/.gemini` no debe cambiar.
4. **Dos cuentas de Codex.** Crear una segunda conexión de Codex con otra
   cuenta (el nombre sugerido será "Codex · personal 2"; ponerle, por ejemplo,
   "Codex · trabajo"). Alternar entre las dos desde el popover y, en el
   historial (reloj), ver que cada conversación aparece bajo la etiqueta de su
   conexión.
5. **Cambiar de conexión.** Con una conversación en curso, elegir otra fila:
   la anterior se detiene (`ps` ya no la muestra), se abre una conversación
   vacía, y la anterior sigue en el historial. Volver a abrirla desde el
   historial devuelve también su conexión.
6. **Eliminar.** "Eliminar conexión…" → elegir una: la confirmación dice
   "¿Eliminar «…»? Se cerrará la sesión, se borrarán sus credenciales de este
   equipo y sus N conversaciones." con el número correcto → "Eliminar". La
   conexión desaparece del popover, su carpeta
   `~/.local/share/cincel/connections/<id>/` ya no existe, sus conversaciones
   no están en el historial, y si era la activa la barra dice "Sin conexión".
   `codex login status` en tu terminal sigue igual. Con Antigravity aparece la
   nota de la cuenta de Google.
7. **Sesión vencida.** Cerrar Cincel, borrar
   `~/.local/share/cincel/connections/<id>/.credentials.json` de una conexión
   de Claude, abrir Cincel: la fila dice "Sesión vencida" en ámbar; elegirla
   muestra "La sesión de «…» venció" con "Volver a conectar" → el modal arranca
   en "Iniciando sesión…" → completar → "Continuar": la etiqueta y las
   conversaciones siguen y se puede chatear.
8. **No disponible.** Cerrar Cincel, borrar `~/.local/share/cincel/agents/codex-acp/current`,
   abrir: la fila de Codex dice "No disponible"; "Reparar" vuelve a instalar el
   adaptador y la conexión funciona sin volver a iniciar sesión.
9. **Renombrar.** Clic derecho en una fila → "Renombrar" → cambiar el nombre →
   "Guardar": cambia en el popover, el encabezado, la barra y los grupos del
   historial.
10. **Migración.** Con carpetas `~/.config/asteroid` y `~/.local/*/asteroid`
    de antes, el primer arranque las mueve a `cincel` con ajustes y
    conversaciones, y aparece el toast "Migrada la configuración de Asteroid a
    Cincel". Las conversaciones de antes quedan bajo "(conexión anterior)" y se
    abren de solo lectura.
11. **Reporte de archivos tocados.** Terminar un turno en el que el agente
    edite un archivo y ver en el log el reporte de `agentFileChangeReport`.
12. **Teclado y aspecto.** En cada modal: `Esc` cierra si no hay nada en
    curso y pregunta si hay un inicio de sesión abierto; `Enter` activa el
    botón con el anillo; los botones son todos iguales y neutros; el fondo
    queda oscurecido al 40 %.
