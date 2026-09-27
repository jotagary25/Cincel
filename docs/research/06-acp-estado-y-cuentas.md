# Informe 06: estado de ACP (sept 2026) y autenticación multi-cuenta

_Escrito cuando el proyecto se llamaba Asteroid (hoy Cincel)._

Fecha: 2026-09-26. Subagente Opus 5.5. Fuentes: código de `agent-client-protocol`, `rust-sdk`, `claude-agent-acp` 0.81.2, `codex-acp` 1.13.1, `registry`, Zed (HEAD 26-sep), gemini-cli 0.61.0, opencode 1.18.32, y docs oficiales. La prueba en vivo de `claude auth login` con un directorio temporal no se ejecutó (denegada por el clasificador de permisos): lo dicho sobre su salida viene de la documentación.

## 0. Hallazgos en nuestro crate (`asteroid-acp`), a corregir primero
1. **`NO_BROWSER=1` forzado** (`connection.rs:383`) empeora los métodos de login: Claude solo ofrece `claude-login` (TUI completa) en vez de `claude-ai-login` (`--cli auth login --claudeai`) y `console-login`; Codex oculta `chat-gpt`; Gemini falla el login con Google. Zed solo lo pone en proyectos remotos. → No inyectarlo por defecto.
2. **`agentFileChangeReport` nunca se negocia**: la forma real es `_meta.jetbrains.air = {"version":1,"capabilities":["agentFileChangeReport", …]}` (array de strings + `version`), anunciada por el agente en el `_meta` de nivel superior del `InitializeResponse`. Nosotros mandamos un objeto y buscamos mal (`protocol.rs:340-372`; `codex-acp/src/AirExtension.ts:47-60`; `claude-agent-acp/src/air-extension.ts:59-70`).
3. **Huecos para el flujo de cuentas**: `Authenticate` bloquea el loop de comandos (`connection.rs:846`) y no emite éxito; no hay comando `Logout`; no se procesa `_auth/status_update`; no se maneja `elicitation/complete`.

## 1. Versiones de ACP
- **v1 es la estable** y la que hablan todos: claude-agent-acp envía `protocolVersion: 1`; codex-acp y gemini-cli usan el SDK TS 1.5.0 (v1); Zed usa `ProtocolVersion::V1`.
- **v2 sigue en Draft** (schema `v2.0.0-alpha.5`, 18-sep). Guía oficial: gatear con negociación + feature flag, no enviar por defecto, no abandonar v1. Ningún agente del registry habla v2 en producción. RFDs activos: prompt lifecycle, diff-file-states, terminal-output, permission-requests.
- **Crates**: usamos 2.1.0 (schema 1.7.0). Existe **2.2.0 (18-sep, schema 1.9.1)**, que Zed fija: `ToolCall.name` estable, tolera `null`, conserva stderr al apagar, session notices y compaction unstable. → Subir a 2.2.0.
- **Qué daría v2**: diffs estructurados (`changes[]` add/delete/modify/move/copy, `fileType`, `mimeType`, `patch`), terminal propiedad del agente, **elimina `fs/*` y `terminal/*` del cliente** (pasan a MCP), auth `auth/login`/`auth/logout` con `methodId`, y prompts que confirman inserción (`messageId`) con steering.
- **Postura**: lanzar en v1; aislar tipos detrás de `protocol.rs`; spike de v2 tras feature flag; seguir el RFD `diff-delete` de v1.

## 2. Modelo de autenticación ACP (v1)
- `initialize` → `authMethods[]` con dos tipos estables: **`agent`** (el cliente llama `authenticate {methodId}`; el agente abre el navegador con callback local o pide una URL al cliente con `elicitation/create` modo `url`) y **`terminal`** (estable desde 20-ago; el cliente anuncia `clientCapabilities.auth.terminal: true`, relanza el mismo programa agregando los `args` del método y aplicando su `env`, lo muestra en una terminal interactiva, toma exit 0 como éxito, reconecta y reinicializa; sin `authenticate` ni señal de éxito en el protocolo). `registry/AUTHENTICATION.md` dice que los args "reemplazan"; la spec normativa dice que se agregan.
- **`env_var` eliminado** (27-jul); API keys por `_meta["api-key"]` en `authenticate` (Codex, Gemini).
- **`logout`** estable (21-may), solo si el agente anuncia `agentCapabilities.auth.logout: {}`.
- **`auth/status`**: RFD draft, sin implementar.
- **Identidad**: no estándar. Extensión **`_auth/status_update`** (Claude y Codex, anunciada con `agentCapabilities._meta.authStatus = {}`): `authStatus {kind: account|api_key|gateway|external|none, label, detail?, account {email, organization, plan}, vendor?}`, enviada tras `initialize`, `authenticate`, `logout`, cada `session/new` y (Claude) al inicio de cada prompt.
- **Un proceso = una identidad.** Varias cuentas = varios procesos con distinto directorio de configuración.

## 3. Por adaptador

### Claude (`claude-agent-acp` 0.81.2, Agent SDK 0.3.280)
- Métodos `terminal`: `claude-ai-login` (`--cli auth login --claudeai`), `console-login`; `--cli` delega en el binario nativo empaquetado o `CLAUDE_CODE_EXECUTABLE`.
- `claude auth login [--claudeai|--console] [--email X] [--sso]`: imprime la URL, espera el callback local y **lee de stdin el código pegado** si el callback no llega → manejable por pipe/pty (no verificado en vivo).
- `claude auth status --json`: `loggedIn, authMethod, apiProvider, email, orgName, subscriptionType` (verificado localmente). `claude auth logout`: no interactivo, idempotente.
- Almacenamiento: `$CLAUDE_CONFIG_DIR/.credentials.json` (0600); macOS Keychain con clave por directorio. La doc de `CLAUDE_CONFIG_DIR` dice que sirve para varias cuentas lado a lado.
- ACP `logout` anunciado (ejecuta `claude auth logout`).
- Trampas: un directorio nuevo no tiene `settings.json`, plugins, skills, MCP ni historial (se puede enlazar `settings.json`; nunca copiar credenciales). Limpiar del entorno del hijo: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_PROFILE`, `CLAUDE_CODE_USE_*`. "another Claude Code process is refreshing" viene de procesos que comparten directorio y lock: con un directorio por conexión y un solo proceso por conexión, desaparece.

### Codex (`codex-acp` 1.13.1, TypeScript sobre `codex app-server`, codex 0.156.1; `CODEX_PATH`)
- `authMethods`: `api-key`; `chat-gpt` (solo sin `NO_BROWSER`: abre navegador, callback `localhost:1455`); **`chat-gpt-device-code`** (si el cliente anuncia `elicitation.url`): el agente envía `elicitation/create {mode:"url", url, message:"…enter this code: XXXX"}`, espera `account/login/completed` y manda `elicitation/complete`. **Es exactamente el flujo de producto, dentro del protocolo.** Requiere device code habilitado en la configuración de seguridad de ChatGPT (beta). `gateway` opt-in.
- `logout` anunciado (`account/logout`). CLI: `codex login`, `--device-auth`, `--with-api-key`/`--with-access-token`, `codex login status`, `codex logout`.
- Almacenamiento: `$CODEX_HOME/auth.json` o keyring (`cli_auth_credentials_store = file|keyring|auto|ephemeral`; clave derivada de la ruta). Fijar `file` en el `config.toml` de cada perfil.
- Trampas: refresh tokens de un solo uso (copiar `auth.json` o dos procesos sobre el mismo home → `refresh_token_reused`, openai/codex#6498); puerto 1455 fijo → serializar logins.

### Gemini CLI (0.61.0, `--acp`)
- `authMethods`: `oauth-personal`, `gemini-api-key`, `vertex-ai`, `gateway`. Sin `logout` ni métodos terminal. `authenticate(oauth-personal)` abre el navegador solo; **el cliente no recibe la URL**.
- Almacenamiento: `$GEMINI_CLI_HOME/.gemini/oauth_creds.json` (o cifrado con `GEMINI_FORCE_ENCRYPTED_FILE_STORAGE=true`).
- Workaround para mostrar la URL: `gemini` interactivo en pty con `NO_BROWSER=true` (imprime la URL y lee el código de stdin).

### OpenCode (1.18.32, `opencode acp`)
- `authMethods`: `opencode-login` tipo `agent`, pero su `authenticate` no hace nada; el login real es `opencode auth login [-p provider] [-m method]` (imprime `Go to: <url>`, prompts interactivos → pty). `opencode auth logout`, `opencode auth list`.
- Almacenamiento: `$XDG_DATA_HOME/opencode/auth.json`. Aislar solo `XDG_DATA_HOME` (las `XDG_*` las heredan git, gh…).

## 4. Qué significa "desconectar"
Tres niveles: matar el proceso (no toca credenciales); ACP `logout` (Claude y Codex borran su almacén); borrar el directorio del perfil (limpieza local, sin revocar en el servidor). Secuencia segura: lanzar el adaptador con el env de **esa** conexión → `initialize` → `logout` si está anunciado → matar el grupo → borrar `connections/<id>/` validando la ruta. Gemini/OpenCode: borrado directo (u `opencode auth logout`). **Nunca ejecutar logout sin la variable de directorio puesta**: cerraría la sesión global del usuario.

## 5. Viabilidad del flujo de producto

| Paso | Claude | Codex | Gemini | OpenCode |
|---|---|---|---|---|
| a) Arrancar sin agente | Posible | Posible | Posible | Posible |
| b) Listar cuentas conectadas | Posible: registro propio + `claude auth status --json` o `_auth/status_update` | Posible: registro + `_auth/status_update` o `codex login status` | Workaround: existencia de `oauth_creds.json` | Posible: `opencode auth list` |
| c) Nuevo agente con URL, aprobar y detectar fin | Workaround: método terminal por pty, capturar URL, campo para pegar código, exit 0 | **Posible hoy** con `chat-gpt-device-code` (elicitation URL); si no, pty con `codex login` | Workaround: pty con `NO_BROWSER=true` | Workaround: pty con `opencode auth login` |
| d) Desconectar | Posible: ACP `logout` + borrar directorio | Posible: idem | Workaround: borrar directorio | Workaround: `opencode auth logout` o borrar |
| e) Varias por proveedor | Posible (`CLAUDE_CONFIG_DIR`, documentado) | Posible (`CODEX_HOME`) | Posible (`GEMINI_CLI_HOME`, poco documentado) | Posible (`XDG_DATA_HOME`, con efectos colaterales) |

## Arquitectura recomendada
- **Una conexión = un perfil = un directorio**: `~/.local/share/asteroid/connections/<uuid>/` (0700) + `connections.json` con `{id, agent_id, display_name, env_var, config_dir, created_at, last_status}`. Sin tokens en el JSON.
- Variable por agente: Claude `CLAUDE_CONFIG_DIR`, Codex `CODEX_HOME`, Gemini `GEMINI_CLI_HOME`, OpenCode `XDG_DATA_HOME`. Limpiar del hijo las variables de credenciales del proveedor. Sembrar en el perfil solo configuración no secreta (symlink a `settings.json`, copia de `config.toml`).
- Opción **"Usar la sesión del sistema"**: conexión que apunta a `~/.claude` / `~/.codex` (acepta contención de refresh con la CLI del usuario).
- **Login**: 1) método `agent` con elicitation URL (Codex) → `authenticate` y diálogo "abrí esta URL e ingresá el código"; 2) método `terminal` → comando derivado (`LaunchSpec` + args + env del perfil) en un **pty** (`portable-pty`) dentro de un panel de la app, URLs por regex con botón "Abrir", input para pegar código, exit 0 = éxito; 3) reconectar y confirmar con `_auth/status_update` o comando de estado; recién entonces persistir.
- Un proceso adaptador por conexión activa. Serializar logins por proveedor. Nunca copiar credenciales entre perfiles.
- Riesgos: cambios de flags de las CLIs; device code de Codex en beta; Gemini sin URL por protocolo; el navegador logueado en otra cuenta (sugerir otro perfil de navegador o `--email` de Claude); términos de Anthropic sobre uso de suscripción desde terceros (el adaptador corre el Claude Code oficial, como Zed).

## 6. Otros cambios desde el 17-sep
- claude-agent-acp 0.79.0 → 0.81.2 (SDK 0.3.280, session notices experimentales, deltas de terminal, `model` en `usage_update`, `--hide-claude-auth`). codex-acp 1.12.0 → 1.13.1.
- Deprecados en npm: `@zed-industries/claude-code-acp`, `@zed-industries/claude-agent-acp`, `@zed-industries/codex-acp` → `@agentclientprotocol/*`.
- Registry: schema `version:"1.0.0"`, 41 agentes, clave `extensions: []`, `quarantine.json`, política: solo agentes con auth `agent` o `terminal`.
- `agentFileChangeReport` vigente en ambos (`declaredComplete:false`); doc en `codex-acp/docs/`; en Claude el código sigue en `src/file-change-audit.ts`.
- Spec: tool call name, elicitation y terminal auth estables; session notices y compaction en Preview; MCP-over-ACP con alcance por request (unstable).

Fuentes: [auth v1](https://agentclientprotocol.com/protocol/v1/authentication) · [RFD auth-methods](https://agentclientprotocol.com/rfds/auth-methods) · [RFD get-auth-state](https://agentclientprotocol.com/rfds/get-auth-state) · [logout](https://agentclientprotocol.com/announcements/logout-method-stabilized) · [v2 draft](https://agentclientprotocol.com/announcements/acp-v2-draft) · [migración v2](https://agentclientprotocol.com/protocol/v2/migration) · [rust-sdk CHANGELOG](https://github.com/agentclientprotocol/rust-sdk/blob/main/src/agent-client-protocol/CHANGELOG.md) · [claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp) · [codex-acp](https://github.com/agentclientprotocol/codex-acp) · [registry AUTHENTICATION.md](https://github.com/agentclientprotocol/registry/blob/main/AUTHENTICATION.md) · [Claude auth](https://code.claude.com/docs/en/authentication) · [Claude env-vars](https://code.claude.com/docs/en/env-vars) · [Codex auth](https://learn.chatgpt.com/docs/auth) · [openai/codex#6498](https://github.com/openai/codex/issues/6498)

## Anexo (2026-09-27): Gemini CLI retirado, Google Antigravity en su lugar

- **Por qué**: con gemini-cli 0.61.0 el login personal de Google falla con `IneligibleTierError`: "This client is no longer supported for Gemini Code Assist for individuals… migrate to Antigravity". Cincel reemplaza Gemini por el servidor ACP oficial de Antigravity.
- **Registry**: `antigravity-acp` "Google Antigravity" 1.2.1, distribución `binary` por plataforma (`darwin-*`, `linux-*`, `windows-*`). `linux-x86_64`: `https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-1.2.1-linux-x86_64.zip` (333.590.110 bytes, acepta `Range`), `cmd ./agy_acp_server.par`, `args ["--uid="]`, `sha256: null` (Google no publica suma). El zip trae `agy_acp_server.par` (ELF de 920 MB, Python empaquetado) y `localharness_external` (133 MB).
- **Protocolo** (binario real, `GEMINI_HOME` temporal): ACP v1. `initialize` → `agentInfo {name: "antigravity-acp", title: "Google Antigravity", version: "1.2.1"}`, `authMethods`: `oauth-personal` "Log in with Google", `oauth-business` (Gemini Enterprise), `gemini-api-key`, `agent-platform`; `agentCapabilities`: `loadSession`, `sessionCapabilities.{list, resume}`, `auth.logout = {}`. **No** anuncia `_meta.authStatus` ni envía `_auth/status_update`. Sin autenticar, `session/new` responde `auth_required` ("No authentication method selected…").
- **Login**: `authenticate {methodId: "oauth-personal"}` imprime por **stderr** `Open the following link to authenticate the ACP server: https://accounts.google.com/o/oauth2/v2/auth?…redirect_uri=http%3A%2F%2F127.0.0.1%3A<puerto>%2F…` (con `state` y `code_challenge`), levanta un servidor de redirección en 127.0.0.1 y termina solo cuando el usuario aprueba (sin código). Además llama a `webbrowser.open` de Python: con `BROWSER=/bin/true` no abre nada por su cuenta. El servidor de redirección espera 300 s; después `authenticate` responde error. Al terminar bien guarda `auth.type` en `<GEMINI_HOME>/antigravity-acp/settings.json`, así que los procesos siguientes no necesitan `authenticate`. Con `auth.type` guardado y un token inválido, `session/new` inicia de nuevo el login en el navegador (imprime el enlace) en vez de responder `auth_required`.
- **Aislamiento**: todo cuelga de `$GEMINI_HOME` (nombra el propio directorio `.gemini`; `paths.py`: "setting it relocates the whole tree"). Token personal en `<GEMINI_HOME>/antigravity-acp/acp_token.json` (`client_id`, `client_secret`, `refresh_token`, `token_uri`, `scopes`, `project_id`: **sin email**), el de empresa en `acp_business_token.json`, conversaciones en `antigravity-acp/conversations/`. En Linux siempre archivo (el llavero solo en macOS). Variables a quitar: `GOOGLE_*`, `GEMINI_*` (salvo el `GEMINI_HOME` propio), `CLOUDSDK_*`, `GCLOUD_*`, y por las suyas `ANTIGRAVITY_*` (`ANTIGRAVITY_HARNESS_PATH`) y `AGY_*` (`AGY_ACP_CCPA_PROJECT`, `AGY_ACP_ENABLE_GATEWAY_AUTH`).
- **Logout**: ACP `logout` borra las credenciales del método en uso y el `auth.type` guardado.
- **Lanzamiento**: `localharness_external` se busca en `dirname(argv[0])` (o `ANTIGRAVITY_HARNESS_PATH`), así que basta la ruta absoluta del `.par`; funciona desde cualquier directorio de trabajo.
