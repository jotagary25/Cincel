# Informe 05: cómo conectar y desconectar cuentas de agentes (UX de Zed, VS Code, JetBrains)

_Escrito cuando el proyecto se llamaba Asteroid (hoy Cincel)._

Fecha: 2026-09-26. Subagente Opus 5.5, con lectura de código fuente (zed@70e686c: `crates/agent_ui`, `agent_servers`, `acp_thread`, `project`; `claude-agent-acp`; `codex-acp`; `formulahendry/vscode-acp`) y spec ACP v1/v2.

## 1. Zed: cómo funciona y por qué no se puede "desconectar"

**Agregar un agente.** `agent: open settings` → **External Agents** → **Add Agent**: **Install from Registry** (marketplace con Install / Remove / Installed / Unavailable) o **Add Custom Agent** (entrada `agent_servers` en `settings.json`). `CustomAgentServerSettings` tiene `Registry { env }` y `Custom { command, args, env }`. Las entradas del registry se identifican por el id del agente: **no puede haber dos "Codex" del registry**; la única salida es una entrada Custom a mano con otro `CODEX_HOME`/`CLAUDE_CONFIG_DIR`, no documentada.

**Primera autenticación.** El agente responde `auth_required`; Zed (`handle_auth_required`) muestra el Callout "Authenticate to {Agente}" con un botón por método. Según el método:
- **`terminal`** (Claude: `claude-ai-login` "Claude Subscription", `console-login`): `spawn_external_agent_login` abre `claude --cli auth login --claudeai` en el **panel de terminal de Zed**; el usuario ve la URL, "press c to copy" y "Paste code here if prompted". Zed detecta el éxito buscando `"Login successful"` / `"Type your message"` en la salida (Claude y Gemini); para otros espera exit 0 y reinicia la conexión.
- **`agent`** (Codex `chat-gpt`): Zed manda `authenticate` y el adapter abre el navegador por su cuenta.
Issues abiertos de login colgado: zed#53000, zed#49216, zed#42653.

**Logout y cambio de cuenta.** El `main` actual tiene en el menú "…" del Agent Panel **"Reauthenticate"**, **"Log Out"** (solo si el agente anuncia `agentCapabilities.auth.logout`; el método `logout` es estable en ACP desde 2026-05-21) y "Reload Agent". Por qué no se pudo desconectar: (1) en versiones anteriores no existía o está escondido; (2) **Zed no maneja credenciales**: viven en `~/.claude/.credentials.json` y `~/.codex/auth.json`, y "Log Out" ejecuta `claude auth logout`, **global**, que también desloguea el CLI de la terminal; (3) no existe el concepto "cuenta": una identidad por agente, sin selector ni email; la doc dice "To change authentication, use the Codex thread's native login/logout flow"; (4) bug: `/logout` + `/login` deja al agente cargando para siempre (zed#47922).

## 2. VS Code / Copilot y JetBrains

**Menú Accounts de VS Code** (modelo de referencia): icono en la Activity Bar con las cuentas por proveedor, cada una con **Sign out**, más **Manage Accounts** y **Manage Extension Account Preferences** (preferencia por workspace/perfil). No permite dos cuentas de Copilot a la vez por perfil (vscode#276028). API `vscode.authentication`: `getSession(provider, scopes, {createIfNone | forceNewSession | account})`, `getAccounts`, `onDidChangeSessions`, `registerAuthenticationProvider`: las extensiones piden sesiones y VS Code es dueño del token. **Esa inversión es la que Asteroid necesita.**
Agentes de terceros en VS Code (Claude/Codex en Agent Sessions desde 1.109) van por la suscripción de Copilot. **Copilot CLI `--acp`** anuncia `copilot-login` por terminal y tiene multi-cuenta nativa (`/user list`, `/user switch`, `/logout`, keyring). **vscode-acp**: QuickPick con métodos + modal "Authenticate"; sin login por terminal, sin logout, sin cuentas.

**JetBrains AI Assistant**: "Install From ACP Registry" o "Add Custom Agent" (`~/.jetbrains/acp.json`). Auth: suscripción JetBrains AI, API key u OAuth del proveedor; agentes custom "en una terminal". Quitar autorización: Settings | Tools | AI Assistant | Providers & API keys → **Revoke**. Sin multi-cuenta.

**Cursor / Antigravity**: solo sign out / sign in; multi-cuenta con instancias con otro `user-data-dir` o extensiones de terceros.

## 3. Multi-cuenta por proveedor

**Ningún editor grande soporta dos cuentas de Claude o de Codex lado a lado** (pedidos abiertos: claude-code#30031, #28585). Lo que sí existe: **aislar el directorio de credenciales por conexión**:
- **Claude**: `CLAUDE_CONFIG_DIR` → `$DIR/.credentials.json` (en macOS la entrada del Keychain queda asociada al directorio). `claude-agent-acp` la respeta (`CLAUDE_CONFIG_DIR ?? ~/.claude`).
- **Codex**: `CODEX_HOME` → `$CODEX_HOME/auth.json`.
- **Gemini**: `~/.gemini/oauth_creds.json`; sin variable documentada para cambiar el directorio. **Verificar.**
Productos chicos que ya lo hacen: codex-multi (un `CODEX_HOME` por perfil compartiendo config, skills y MCP); intent#5720 propone lo mismo y advierte que fijar `CLAUDE_CONFIG_DIR` saca a macOS de la cuenta del Keychain por defecto. Reporte no verificado: `CODEX_HOME` "ya no confiable" para varias instancias en Mac.

## Tabla comparativa

| | Zed | VS Code | JetBrains | Cursor / Antigravity |
|---|---|---|---|---|
| Agregar agente | Registry o `agent_servers` custom | Extensión / Agent Sessions vía Copilot; vscode-acp: 11 predefinidos + custom | Registry o `~/.jetbrains/acp.json` | Solo el propio |
| Autenticar | Callout con botón por método; terminal en el panel de Zed | Accounts / `authentication.getSession` (OAuth) | Suscripción JB, API key u OAuth; custom: terminal | Login propio |
| Logout | "Log Out" en "…" si el agente lo anuncia; global al CLI | "Sign out" por cuenta | "Revoke" | Sign out |
| Cambiar cuenta | Logout + login, con bugs | Preferencias por workspace/perfil | Revoke y activar | Sign out / sign in |
| Multi-cuenta | No | Varias GitHub, una activa por perfil | No | No |

## 4. UX recomendada para Asteroid

### Modelo
**Conexión** = `id`, `agente` (id del registry), `etiqueta` (del usuario), `identidad` (email/plan), `perfil` (directorio aislado bajo `~/.local/share/asteroid/connections/<id>/`), `último uso`, `estado`. Asteroid lanza el adapter con `CLAUDE_CONFIG_DIR` o `CODEX_HOME` apuntando al perfil: dos Codex conviven, "Desconectar" no toca el `~/.codex` de la terminal, y Asteroid es dueño de la lista (como VS Code con sus cuentas).
**Identidad**: `claude-agent-acp` la empuja por `_auth/status_update` (`kind` account/api_key/none, `account.email`, `organization`, `plan`); `codex-acp` tiene `AuthStatusMeta`. Si no llega, se usa la etiqueta.

### Estados
```
[Sin conexiones] --Conectar nuevo agente--> [Elegir agente]
[Elegir agente] --elige--> [Iniciando inicio de sesión]
[Iniciando] --URL/código detectado--> [Esperando aprobación]
[Esperando] --exit 0 / auth OK--> [Verificando cuenta] --identidad--> [Nombrar conexión] --> [Conectada]
[Esperando] --error--> [Error] --Reintentar--> [Iniciando]
[Esperando] --Cancelar--> [Elegir agente]
[Conectada] --auth_required / expiró--> [Sesión vencida] --Volver a conectar--> [Iniciando] (mismo perfil)
[Conectada] --Desconectar--> [Confirmar] --Sí--> auth/logout + borrar perfil
[Conectada] --sin binario / adapter caído--> [No disponible] --Reintentar-->
```

### Pantallas y textos
- **Estado vacío** (panel de chat): "No hay ningún agente conectado" + botón **"Conectar"**.
- **Popover "Conectar"** (encabezado del chat): una fila por conexión (icono, **etiqueta**, identidad en gris "ana@… · Plan Max", "Usado hace 2 h", insignia Conectado / Sesión vencida / No disponible); al pie **"Conectar nuevo agente"** y **"Desconectar agente…"**.
- **Modal "Conectar nuevo agente"**: 1) "Elegí un agente" (grilla del registry); 2) "Estamos iniciando el inicio de sesión…"; 3) "Abrí este enlace y aprobá el acceso" con URL en monoespaciada, **"Copiar"**, **"Abrir en el navegador"**, código si lo hay y campo "Pegá aquí el código que te mostró el navegador" si hace falta; 4) "Esperando que apruebes en el navegador…" + **"Cancelar"**; 5) "Listo: conectado como ana@… (Claude Max)"; 6) "Nombre de la conexión" con sugerencia ("Claude – personal") + **"Guardar"**.
- **Modal "Desconectar agente"**: lista + confirmación "¿Desconectar «Codex – trabajo»? Se cerrará la sesión y se borrarán sus credenciales de este equipo." **"Desconectar"** (destructivo) / **"Cancelar"**; avisar si hay threads activos.
- **Sesión vencida**: insignia ámbar, banner en el thread y **"Volver a conectar"** (repite el paso 3 sobre el mismo perfil, conserva nombre y threads).
- **Dónde vive**: lista y elección en el encabezado del chat; chip en la barra de estado con la conexión del thread activo; gestión completa en **Configuración → Agentes**; cada thread guarda el `id` de su conexión.

### Mecánica por agente

| Agente | Cómo se loguea | Qué hace Asteroid |
|---|---|---|
| Codex (`codex-acp`) | `chat-gpt-device-code` (URL + código por **URL elicitation** de ACP); alternativa `chat-gpt` (abre el navegador y espera callback local) | Si Asteroid anuncia la capability de URL elicitation, el flujo ideal es nativo, sin terminal |
| Claude (`claude-agent-acp`) | método `terminal` → `claude --cli auth login --claudeai`; Asteroid anuncia `clientCapabilities.auth.terminal` | Lo corre en una **pseudo-terminal embebida y oculta dentro del modal**; extrae la URL con regex; si aparece "Paste code here if prompted", manda el código al stdin; éxito = exit 0; luego reinicializa el adapter |
| Todos | ACP `auth/logout` | Llamarlo y además borrar el directorio del perfil |

No conviene lanzar la terminal del sistema: en Wayland no hay forma portable de elegirla, no se puede leer la URL ni saber cuándo termina. Para el desplegable "Ver detalles técnicos" sirve `alacritty_terminal`; para el login oculto alcanza `portable-pty` + regex.

### A decidir con el autor
1. **Qué comparte el perfil aislado**: agentes limpios (sin `CLAUDE.md`, skills, MCP ni `settings.json` de `~/.claude`) o enlazar lo compartido y aislar solo la credencial (como codex-multi).
2. **Importar la cuenta existente** ("Usar la cuenta ya iniciada en este equipo", perfil `~/.claude`): desconectarla desloguea también el CLI; el modal debe advertirlo.
3. **Dos opciones al desconectar**: "Olvidar en Asteroid" (conserva credenciales) o "Cerrar sesión y borrar credenciales".
4. **API key y gateway**: dentro o fuera de v1.
5. **Gemini**: confirmar aislamiento de directorio; si no se puede, una conexión por proveedor.
6. **Riesgos de multi-cuenta**: términos del proveedor; aviso sobre `CODEX_HOME` en Mac.
7. **Versión de ACP**: v1 con `auth.logout` o v2 (`auth/logout` implícito si hay `authMethods`; el método `env_var` se eliminó).

Fuentes: [ACP auth v2](https://agentclientprotocol.com/protocol/v2/authentication) · [claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp) · [codex-acp](https://github.com/zed-industries/codex-acp) · [vscode-acp](https://github.com/formulahendry/vscode-acp) · [Codex auth](https://developers.openai.com/codex/auth) · [Copilot CLI ACP](https://docs.github.com/en/copilot/reference/copilot-cli-reference/acp-server) · [ACP en JetBrains](https://blog.jetbrains.com/ai/2026/02/acp-jetbrains-ide-ai) · [Zed external agents](https://zed.dev/docs/ai/external-agents) · [VS Code Accounts](https://code.visualstudio.com/docs/setup/copilot)
