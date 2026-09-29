# Módulo `cincel-connections`

Conexiones de agentes aisladas (`docs/specs/06-etapa4-conexiones-y-cincel.md` §1–§4, §8). Sin GPUI. Una conexión = un agente + un perfil privado + una etiqueta. Nada del sistema: ni `~/.claude`/`~/.codex`/`~/.gemini`, ni el Node del sistema, ni la caché de npm del usuario.

## Disposición en disco

| Qué | Dónde |
|---|---|
| Índice (sin tokens) | `~/.local/share/cincel/connections.json` (0600, escritura atómica) |
| Perfil de cada conexión | `~/.local/share/cincel/connections/<uuid>/` (0700) |
| Node privado | `~/.local/share/cincel/runtime/node-<versión>/` + `runtime/current` |
| Adaptadores (npm) y agentes binarios (Antigravity) | `~/.local/share/cincel/agents/<agent_id>/<versión>/` + `agents/<agent_id>/current` |
| Caché de npm | `~/.cache/cincel/npm` (con `npmrc` vacío propio en `~/.local/share/cincel/npmrc`) |
| Descargas parciales | `~/.cache/cincel/downloads/` (reanudables; Node y archivos `binary`, p. ej. `antigravity-acp-1.2.1-linux-x86_64.zip.part`) |

Todas las rutas cuelgan de `CincelPaths { data_dir, cache_dir }` (`CincelPaths::from_xdg()` en producción, `CincelPaths::under(tmp)` en tests).

## API pública

### Fachada
```rust
Connections::new(paths) / .with_runtime(Runtime) / .with_adapters(Adapters)
connections.store() / .runtime() / .adapters()
connections.status(&Connection) -> ConnectionStatus            // Connected | SessionExpired | Unavailable { reason }
connections.list_with_status() -> Result<Vec<(Connection, ConnectionStatus)>>
connections.is_ready(AgentKind) -> bool                          // "instalado" en la grilla
connections.prepare(Option<&AgentRegistry>, AgentKind, &mut dyn FnMut(PrepareProgress)) -> Result<(Option<NodePaths>, AdapterInstall)>  // None: agente binario, sin Node
connections.agent_launch(&Connection) -> Result<AgentLaunch { launch: LaunchSpec, env: ProcessEnv }>
connections.start_login(&PendingConnection) -> Result<LoginSession>   // F2 (cancelar borra el perfil pendiente)
connections.start_relogin(Uuid) -> Result<LoginSession>                // F4 (cancelar NO borra el perfil)
connections.disconnect(Uuid) -> Result<DisconnectReport>               // F3
```

### Índice (`ConnectionStore`)
`Connection { id: Uuid, agent_id, label, identity: Option<Identity { email, plan, organization }>, created_at, last_used_at }` (segundos Unix). Métodos: `list` (más reciente primero), `get`, `create_pending(agent_id) -> PendingConnection { id, kind, profile }` (crea y siembra el perfil, **no** lo indexa), `finalize(&pending, label, identity)`, `discard_pending(id)`, `rename`, `touch`, `set_identity`, `delete` (borra perfil validado + índice), `profile_dir`, `profile`. Helpers: `suggest_label(kind, &existing)` → `"Claude · personal"` (`… 2` si está tomada), `Identity::summary()` → `"ana@… · Max"`.

### Perfil (`Profile`, `AgentKind`)
`AgentKind::{Claude, Codex, Antigravity}` ↔ `"claude-acp" | "codex-acp" | "antigravity-acp"`; `source()` (`AdapterSource::Npm { package, command }` o `Binary`), `npm_package()`, `needs_node()`, `display_name()` (`"Antigravity"`, para etiquetas), `full_name()` (`"Google Antigravity"`), `description()`, `profile_var()` (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_HOME`), `login_method()` (`LoginMethod::Pty` o `AcpAuthenticate("oauth-personal")`), `login_needs_code()` (solo Claude). `RETIRED_AGENTS`/`retired_reason("gemini")`: el motivo que muestra una conexión vieja de Gemini. `Profile::process_env(node_bin)` → `ProcessEnv` con la variable de perfil, `STRIPPED_VARIABLES` quitadas (incluye los prefijos `GOOGLE_*`, `GEMINI_*`, `CLOUDSDK_*`, `GCLOUD_*`, `ANTIGRAVITY_*`, `AGY_*`; el `GEMINI_HOME` propio se pone después de quitar), `BROWSER=/bin/true` para Antigravity y el `bin/` del Node privado al frente del `PATH` (solo npm). `seed()`: Codex `config.toml` con `cli_auth_credentials_store = "file"`; Claude y Antigravity nada (Antigravity guarda `auth.type` en `antigravity-acp/settings.json` al autenticarse; sembrarlo antes haría que `session/new` abra un login en el navegador en vez de responder `auth_required`). `credentials_file()` (`.credentials.json`, `auth.json`, `antigravity-acp/acp_token.json`), `has_credentials()`, `clear_credentials()`, `offline_identity()` (Antigravity: campo `email` del token si existiera; el token real de 1.2.1 no lo trae).

### Runtime (`Runtime`, `NodePaths`)
`Runtime::new(paths)`, `.with_version(NodeVersion::parse("lts" | "24.11.1"))`, `.with_downloader(Box<dyn Downloader>)`, `.with_base_url`, `.with_retry(intentos, pausa)`. `installed() -> Option<NodePaths>` (sin red), `ensure(&mut dyn FnMut(RuntimeProgress)) -> Result<NodePaths>`. `NodePaths { version, root, bin_dir, node, npm /* npm-cli.js */, npx /* npx-cli.js */ }`. Resolución: primera LTS ≥ 22 de `https://nodejs.org/dist/index.json` con la plataforma (`linux-x64`, `linux-arm64`, `darwin-*`); SHA-256 de `SHASUMS256.txt`; descarga a `.part` con `Range` para reanudar, reintentos (3 por defecto); un parcial con SHA incorrecto se descarta; extracción de `.tar.xz` (lzma-rs, puro Rust) conservando solo `bin/node`, `bin/npm`, `bin/npx`, `lib/node_modules/npm/**` y `LICENSE`; instalación en staging + rename atómico; se borran runtimes viejos. `RuntimeProgress::{Resolving, Downloading { version, done, total }, Retrying { attempt, reason }, Verifying, Extracting, Done}`.

### Adaptadores y agentes binarios (`Adapters`)
`Adapters::new(paths)`, `.with_installer`, `.with_downloader(Box<dyn Downloader>)`, `.with_platform("linux-x86_64")`, `.with_retry`, `.with_free_space(fn)`, `.with_space_needed(bytes)` (tests). `install(kind, version, args, &node, progress)` (npm), `install_binary(kind, &BinaryPlan, progress)`, `binary_plan(&registry, kind) -> BinaryPlan { version, platform, archive, cmd, args, env, sha256 }` (`verifiable()`, `archive_name()`), `ensure(Option<&AgentRegistry>, kind, Option<&NodePaths>, progress)` (usa lo instalado si existe; sin red y sin nada instalado → `AdapterMissing`; npm sin Node → `RuntimeMissing`), `installed(agent_id) -> Option<String>`, `installed_adapter`, `update_available(&registry, kind) -> Option<String>` ("Actualizar"), `launch_spec(agent_id, Option<&NodePaths>) -> LaunchSpec` (npm: `node <paquete>/<bin> [args]`; binario: `<instalación>/<cmd> [args]`, p. ej. `agy_acp_server.par --uid=`), `command_spec(agent_id, &node, &[..])` (solo npm), `bin_path`.

- **npm**: `node npm-cli.js install --prefix <staging> --cache ~/.cache/cincel/npm --no-save`, `npm_config_*` del usuario quitadas, `userconfig`/`globalconfig` vacíos, Node privado primero en `PATH`.
- **binario** (`distribution.binary.<plataforma>` del registry; clave `current_platform_key()`, p. ej. `linux-x86_64`): chequeo de espacio libre en `downloads/` y `agents/` (`install_space_needed`: Antigravity 1,4 GB, menos lo ya descargado) → `NotEnoughSpace { agent, path, needed, available }`; descarga a `.part` con `Range` para reanudar y 3 intentos, informando `AdapterProgress::Downloading { version, done, total, verifiable }` cada MB; SHA-256 si el registry lo publica (`Verifying`; si no coincide se borra el parcial y se reintenta; `sha256: null` → instalación `verifiable: false`); segundo chequeo de espacio con el tamaño descomprimido exacto del zip; `Extracting`: zip (entrada por entrada con `enclosed_name`, conservando los bits del zip más `u+rw`) o tar.gz (`unpack_in`), nunca fuera del destino; `chmod +x` del `cmd`; staging + rename; se borra el archivo descargado y las versiones viejas.

Un marcador `.cincel-adapter.json` guarda `distribution` (`npm` | `binary`), paquete o nombre del archivo, versión, `bin`, `args`, `env` y `verifiable` para lanzar sin registry (los marcadores viejos sin `distribution` se leen como npm).

### Login (`LoginSession`)
```rust
LoginSession::start(&Profile, Option<&NodePaths>, &Adapters, Option<PendingCleanup>) -> Result<LoginSession>
LoginSession::spawn(LoginCommand, LoginOptions) -> LoginSession   // motor pty genérico (tests)
LoginSession::spawn_acp(AcpLogin { launch, env, cwd, method_id }, LoginOptions) -> LoginSession  // modo ACP authenticate
plan_login(&Profile, Option<&NodePaths>, &Adapters) -> Result<(LoginPlan::{Pty, Acp}, LoginOptions)>
session.events() -> &async_channel::Receiver<LoginEvent>
session.send_code(&str) -> Result<()>   // solo pty
session.cancel()          // mata el grupo de procesos; borra el perfil pendiente si hay PendingCleanup
```
Login por proveedor (entorno del perfil, `cwd` = perfil):

| Agente | Modo | Comando | Fin |
|---|---|---|---|
| Claude | pty oculta (1000 columnas) | `node <claude-agent-acp> --cli auth login --claudeai` | exit 0 (tras pegar el código) |
| Codex | pty oculta | `node <codex-acp> cli login` con `BROWSER=/bin/true` (codex empaquetado en el adaptador) | exit 0 (callback local en :1455) |
| Antigravity | ACP authenticate | `agy_acp_server.par --uid=` con `GEMINI_HOME=<perfil>`, `BROWSER=/bin/true`; `initialize` (anuncia `elicitation.url`) → `authenticate { methodId: "oauth-personal" }` | respuesta correcta de `authenticate` + `antigravity-acp/acp_token.json` presente |

Modo ACP authenticate: el agente se lanza con `AgentConnection::start_with_env` (grupo de procesos propio). Cada línea de stderr se limpia (`strip_ansi`), se buscan enlaces de login (`find_login_urls`; `accounts.google.com` está en la lista) → `UrlDetected`, y se emite como `Output` redactada (`redact_line`: query de URLs, `cincel_log::redact`). Una `elicitation/create` en modo `url` también da `UrlDetected` (se responde `accept`: el usuario abre el enlace desde el modal); una en modo formulario se responde `cancel`. Si el agente no ofrece el método → `Failed { Rejected }`; error de `authenticate` → `Failed { Rejected }` con el mensaje redactado; el proceso termina → `Failed { Exit(código) }`; 15 min → `Failed { Timeout }`. Pase lo que pase, al final `shutdown` mata el grupo de procesos (y el servidor de redirección local del agente). "Volver a conectar" usa el mismo modo.

Luego se confirma la identidad: Claude y Codex, se lanza el adaptador una vez con el perfil y se lee `_auth/status_update` (`AcpIdentityProbe`); si no llega, `claude --cli auth status --json` / `codex cli login status` por el binario empaquetado (`CliStatusProbe`). Antigravity no anuncia `_meta.authStatus` (verificado con el binario real 1.2.1): `ProfileProbe` (token presente; email solo si el token lo trajera, y el de 1.2.1 no lo trae, así que "Conectado" sin identidad). Si el probe dice "sin sesión" el login termina en `Failed { reason: NotLoggedIn }`.

Serialización: un candado por `agent_id`; un segundo login del mismo proveedor emite `Queued` y espera (cancelable). Tiempo máximo 15 min (`LoginOptions::timeout`).

## Protocolo para la UI (modal "Conectar nuevo agente")

1. **Elegí un agente** → `AgentKind` (`full_name()` + `description()`). Mostrar "instalado" con `is_ready(kind)`.
2. **Preparando…** → `prepare(Some(&registry), kind, cb)` en un hilo de fondo; `PrepareProgress::Runtime(Downloading { done, total })` y `PrepareProgress::Adapter(Downloading { done, total, verifiable })` alimentan las barras (Antigravity: una sola barra, MB decimales y porcentaje; con `verifiable: false`, la nota "Google no publica una suma de verificación para este paquete"). Sin red: `prepare(None, …)`; `RuntimeMissing`/`AdapterMissing` → "hace falta conexión para instalar el agente"; `NotEnoughSpace` → cuánto hace falta y dónde.
3. **Iniciando sesión…** → `store().create_pending(id)` y `start_login(&pending)`.
4. Consumir `session.events()`:

| Evento | UI |
|---|---|
| `Queued` | "Esperando que termine otro inicio de sesión de este agente…" |
| `Started` | spinner |
| `Output(línea)` | "Ver detalles técnicos" (ya redactada: sin enlaces con query, códigos ni tokens; apta para el log) |
| `UrlDetected(url)` | paso 4: enlace monoespaciado, **Copiar**, **Abrir en el navegador** (portal xdg). **Nunca** loguear `url`. |
| `CodeDetected(código)` | mostrar el código que hay que escribir en el navegador (Codex device code). No loguear. |
| `NeedsPastedCode` | campo "Pegá acá el código que te dio el navegador" → `send_code(texto)` (nunca en Antigravity) |
| `Completed { identity }` | paso 6: "Conectado como …" / "Conectado"; campo nombre con `suggest_label`; **Guardar** → `store().finalize(&pending, nombre, identity)` y activar (F1) |
| `Failed { message, reason }` | mensaje + **Reintentar** (nuevo `create_pending`; descartar el anterior con `discard_pending`) |
| `Cancelled` | volver a "Elegí un agente" |

**Cancelar** → `session.cancel()`. Soltar la `LoginSession` también cancela.

F1 (activar): `agent_launch(&conn)` → `AgentConnection::start_with_env(raíz_del_proyecto, launch.env)` + `AgentCommand::Spawn { launch: launch.launch, cwd }`; `store().touch(id)`.
F3 (eliminar): `disconnect(id)` → `DisconnectReport { logout: LoggedOut | NotSupported | Skipped(_) | Failed(_), process_stopped, profile_removed, index_removed, profile_error }`; borrar las conversaciones de la conexión es del chat. Antigravity anuncia `logout` (borra su token) → `LoggedOut`; el modal agrega "Para revocar el acceso, hacelo desde tu cuenta de Google". Una conexión vieja de Gemini → `NotSupported` sin lanzar nada.
F4 (sesión vencida): `status == SessionExpired` → `start_relogin(id)`; al `Completed`, `store().set_identity(id, identity)`.
F5 (no disponible): `status == Unavailable` → **Reparar** = `prepare(Some(&registry), kind, cb)` (no toca credenciales).

## Etapa 5: cancelar de verdad y actualizar adaptador/Node

Detalle, verificación y desviaciones en `docs/etapas/etapa-5.md`. Spec:
`docs/specs/07-etapa5-productividad.md` §10.3 y §10.4.

- **`CancelToken`** (`cancel.rs`, `Arc<AtomicBool>`): `new`, `cancel`,
  `is_cancelled`, `check` (`Err(Cancelled)` una vez cancelado), `sleep`
  (duerme en tramos de 50 ms, cancelable). `CancelReader`/`CancelWriter`
  fallan con un error de E/S una vez cancelado, así que `io::copy` de una
  entrada de archivo se corta a mitad. Toda la cadena de preparación pasa a
  recibir `&CancelToken`: `Connections::prepare`, `Runtime::ensure`,
  `Adapters::ensure`, `install`, `install_binary`,
  `PackageInstaller::install`. Los bucles de descarga (`runtime.rs`,
  `adapters.rs`) revisan el token cada 64 KiB, entre reintentos y antes de
  verificar/descomprimir; al cancelar se suelta la respuesta, se borra el
  `.part` y el directorio de staging. `npm install` corre en su propio grupo
  de procesos (`rustix`), con un hilo que lo espera y otro que revisa el
  token cada 100 ms; cancelar mata el grupo. `WorkLock::acquire(key, token)`
  serializa dos preparaciones del mismo recurso compartido (una segunda
  espera, cancelablemente, a que la primera termine de limpiar).
- **`Adapters::update(registry, kind, node, progress, &CancelToken) ->
  Result<AdapterInstall>`**: instala la versión del registry en staging y
  hace `commit(keep: Option<&str>)`, que no borra la versión en uso por una
  conexión activa (D13). `Adapters::prune_unused(in_use: &[(&str, &str)])`
  borra lo que no es `current` ni está en uso; lo llama el workspace al
  arrancar (antes de lanzar nada) y al detener una conexión o terminar una
  actualización.
- **`Runtime::update_available() -> Result<Option<String>>`** resuelve
  contra `index.json` según `connections.runtime.node_version` (`"lts"` → la
  LTS más nueva ≥ 22; un mayor como `"22"` → la más nueva de esa línea; una
  versión completa nunca ofrece actualización). `Runtime::update(progress,
  &CancelToken) -> Result<NodePaths>` descarga, verifica, instala y mueve
  `current`; la versión anterior se conserva hasta `Runtime::prune_old()` en
  el próximo arranque.
- Reemplaza la deuda de la Etapa 4 "cerrar el modal durante «Preparando…» no
  corta una descarga en curso" y el deseo "botón Actualizar cuando el
  registry publica una versión más nueva".

## Etapa 6: descarga lenta real y política de actualización de Node

Detalle, verificación y desviaciones en `docs/etapas/etapa-6.md`. Spec:
`docs/specs/08-etapa6-cierre-1-0.md` §5.6.3.

- **`Runtime::update_available`/`update` solo ofrecen actualizar dentro de
  la política configurada, nunca a un mayor distinto**: con
  `connections.runtime.node_version: "lts"`, la LTS más nueva ≥ `MIN_NODE_
  MAJOR`; con un mayor fijo (`"22"`), la más nueva **de esa misma línea**
  (nunca salta a la 24); con una versión completa (`"24.11.1"`), nunca
  ofrece nada (está fijada, sin red). Ya se comportaba así desde que se
  agregó el botón en la Etapa 5; esta etapa lo deja probado de punta a
  punta contra un servidor real.
- **`crates/cincel-connections/tests/slow_download.rs`** (`#[ignore]`,
  §5.6.3): servidor HTTP local (`std::net::TcpListener`, sin ningún
  framework) que entrega 1 MB cada 100 ms; `Runtime::ensure` corre en su
  propio hilo con el `HttpDownloader` real (no uno en memoria, a diferencia
  del resto de los tests del crate) y se cancela a los 300 ms: el hilo
  termina en menos de 500 ms (× `CINCEL_PERF_BUDGET_FACTOR`, D15), sin
  `.part` ni carpeta de staging, y el servidor ve la conexión cerrada. Se
  corre a mano (`cargo test -p cincel-connections --test slow_download --
  --ignored`) o en el job semanal/manual de CI, nunca en cada push: es el
  único test del crate que de verdad tarda y usa la red (loopback).
- **`settings_update_e2e_tests.rs`** (`cincel-workspace`): actualizar Node y
  cancelar una actualización de punta a punta, desde `Ctrl+,` → Conexiones,
  con `ConnectionsFixture` y un descargador en memoria; antes se probaba
  por partes (el hilo de actualización en este crate, y el cableado de
  eventos en `settings_bridge.rs` por separado).

## Seguridad
- El índice nunca guarda credenciales (test `index_never_contains_credentials`).
- Borrado de perfiles solo dentro de `connections/` (canonicalize + `starts_with`, rechaza la raíz y symlinks).
- El `logout` de F3 se lanza siempre con la variable de perfil construida por el crate (tests: el agente falso recibe `CLAUDE_CONFIG_DIR` / `GEMINI_HOME` = ese perfil).
- `Output` pasa por `cincel_log::redact` + recorte de query de URLs + los códigos detectados o enviados (la pty hace eco de lo pegado). En el modo ACP, lo mismo para cada línea de stderr y para el mensaje de error de `authenticate`.
- Descomprimir nunca escribe fuera de la carpeta de instalación (test con una entrada `../fuera.txt` en el zip).

## Tests
107 en total más 2 ignorados: 47 unitarios, 26 del motor de login (`tests/login.rs`: pty con `cincel-connections-fake-login`, y modo ACP authenticate con `cincel-connections-fake-agent`, que imprime el enlace por stderr o lo manda como elicitación `url`, responde tras una demora, falla, sale o no escribe el token según variables de entorno), 34 de motor (`tests/engine.rs`: runtime con descargador en memoria y tar.xz generado, npm falso, distribuciones binarias con zip/tar.gz generados en memoria (sin sha256, con sha256 correcto e incorrecto, reanudación, falta de espacio, zip roto, entrada que intenta salir del destino), flujo completo pty y Antigravity login → finalize → lanzamiento → volver a conectar → eliminar, sondas de identidad y eliminación). Ninguno usa la red ni agentes reales. Los 2 ignorados corren contra el binario real de Antigravity si se les da la ruta (`CINCEL_REAL_AGY=<carpeta con agy_acp_server.par>`: `authenticate` imprime el enlace y se cancela sin completar el login; `CINCEL_REAL_AGY_ZIP=<zip>`: instala el zip real y comprueba `initialize` como `antigravity-acp`, con `logout` y sin `_meta.authStatus`). Ambos pasaron el 2026-09-27 con 1.2.1.

## Desviaciones
- **`LoginSession::start` recibe también `&Adapters`** (y `Option<PendingCleanup>`), no solo `(agent_id, profile, runtime)`: hace falta para resolver el script del adaptador; el `agent_id` sale del `Profile`.
- **`launch_spec(agent_id, Option<&NodePaths>)`** en vez de `(agent_id, profile)`: el perfil no cambia el comando, solo el entorno; el entorno va en `ProcessEnv` (`Profile::process_env` / `Connections::agent_launch`). `None` alcanza para los agentes binarios.
- **Directorio de trabajo de los agentes binarios**: el brief pedía `cwd` = carpeta de extracción. `LaunchSpec` no tiene `cwd` (lo pone el `Spawn` de quien lanza: el proyecto en el chat, el perfil en login/logout) y no hace falta: se ejecuta el `cmd` por ruta absoluta y Antigravity busca `localharness_external` en `dirname(argv[0])` (`main.py` del `.par`); verificado lanzándolo desde otra carpeta (`initialize` responde). Mantener el `cwd` del proyecto es además lo correcto para las herramientas del agente.
- **Espacio en disco**: 1,4 GB en vez de "~1,3 GB": 333 MB del zip + 1,05 GB descomprimido, y el zip se borra al final.
- **`NodePaths.npm`/`npx` son los `.js` de npm** (se ejecutan con `node`), no los symlinks de `bin/`, para no depender del shebang `#!/usr/bin/env node`. `bin_dir` va al frente del `PATH`.
- **Variables quitadas**: se quita la unión de todas las del §2 (más `NO_BROWSER`) para cualquier agente, no una lista por proveedor: más estricto con el principio de aislamiento.
- **Eventos extra**: `LoginEvent::Queued`, `Started`, `Cancelled` y `Failed.reason` (`LoginFailure`), además de los pedidos.
- **Gemini retirado (2026-09-27)**: Google rechaza su login personal; lo reemplaza Antigravity. `LoginOptions::success_file` queda como opción genérica del motor pty (ya no la usa ningún agente) y como verificación del token en el modo ACP.
- **Identidad de Antigravity**: no hay. El binario 1.2.1 no anuncia `_meta.authStatus` y su token (`client_id`, `client_secret`, `refresh_token`, `token_uri`, `scopes`, `project_id`) no trae email; el email solo se obtendría llamando a `userinfo` de Google con el token, cosa que Cincel no hace.
- **Sin nuevo `initialize` + `session/new` al terminar el login de Antigravity**: se verifica el archivo de token. Un `session/new` real dispara el onboarding del proyecto de Google, que puede tardar y no agrega información útil al modal.
- **Tiempos**: `created_at`/`last_used_at` en segundos Unix (`u64`) para no sumar una dependencia de fechas.
- **Borrar conversaciones** (F3) lo hace el workspace tras `disconnect` (workstream C): las de esa conexión en todos los proyectos, solo si salió del índice.
