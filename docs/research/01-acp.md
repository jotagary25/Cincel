# Informe 01: ACP (Agent Client Protocol) para Asteroid

Fecha: 2026-09-17. Investigación realizada con subagente Opus.

## 0. Estado del protocolo (sept 2026)

- **ACP v1 = estable y en producción.** Es lo que hay que implementar.
- **ACP v2 = Draft** desde 2026-07-20. La doc dice: no enviarlo por defecto en producción; soportar ambas versiones en paralelo.
- Gobernanza: org **`agentclientprotocol`** en GitHub, co-mantenida por Zed + JetBrains + Anthropic/OpenAI, con proceso de RFDs.
- Transporte: **stdio, JSON-RPC 2.0, NDJSON (mensajes delimitados por `\n`), UTF-8**. `stdout` solo mensajes ACP; logs a `stderr`. Existe draft de Streamable HTTP/WebSocket.
- Índice completo de docs: https://agentclientprotocol.com/llms.txt

## 1. HALLAZGO CLAVE: ¿los agentes escriben a través del cliente?

**No se puede asumir que el agente enrute sus escrituras por `fs/write_text_file`. Claude y Codex escriben directo a disco.**

### Claude — `@agentclientprotocol/claude-agent-acp` (ex `@zed-industries/claude-code-acp`)
CHANGELOG v0.18.0: "Switch over to built-in Claude tools. We no longer replicate specific ACP tools and just rely on sending updates based on Claude's internal tools. This means it won't use client capabilities for files or terminals."
Verificado en v0.79.0: `readTextFile`/`writeTextFile` existen como wrappers en `src/acp-agent.ts` pero nadie los llama. El adapter lanza el CLI real de Claude con `Read`/`Write`/`Edit` nativos. El diff ACP se emite a posteriori desde un hook `PostToolUse` usando `structuredPatch` (`src/diff.ts`, `src/tools.ts`).

### Codex — `@agentclientprotocol/codex-acp` v1.12.0
`read_text_file|write_text_file` solo aparecen en mocks de test. Escribe vía Codex App Server (`apply_patch`) directo a disco.

### Gemini CLI — sí lo hace
`gemini --acp`: "when the agent needs to read or write files, it does so through the ACP client". Es la implementación de referencia.

### Qué hace Zed realmente
Zed anuncia `fs.readTextFile(true)` + `fs.writeTextFile(true)` (`crates/agent_servers/src/acp.rs`), `terminal(true)`, `auth.terminal`, `elicitation.form/url`.
En `crates/acp_thread/src/acp_thread.rs::write_text_file`:
1. abre el `Buffer` del proyecto,
2. calcula `text_diff(old, new)` mínimo y lo aplica en memoria con `BufferEditSource::Agent`,
3. registra `action_log.buffer_edited(buffer)`,
4. opcionalmente formatea,
5. **llama `project.save_buffer` (guarda a disco).**
`read_text_file` devuelve el snapshot del buffer en memoria (incluye cambios sin guardar).

**Zed NO retiene la escritura sin guardar.** Su modelo es "escribir y poder revertir": `crates/action_log/src/action_log.rs` — `ActionLog` mantiene `TrackedBuffer { snapshot base, BufferDiff }` y expone `keep_edits_in_range`, `reject_edits_in_ranges`, `keep_all_edits`, `reject_all_edits`, `changed_buffers`, `undo_last_reject`. Reject restaura el contenido base y vuelve a guardar (o borra el archivo si el agente lo creó).

### Implicación para Asteroid: dos rutas, la realista es la B
- **A (shadow buffer puro):** solo con agentes que respeten `fs/write_text_file` (Gemini). Se intercepta la escritura, se aplica a un buffer sucio, `fs/read_text_file` devuelve contenido no guardado (el protocolo lo exige). No hay forma protocolar de decir "no" al agente: `fs/write_text_file` devuelve `null`.
- **B (modelo Zed: apply + track + revert):** única que funciona con Claude y Codex:
  1. Anunciar igual `fs.readTextFile/writeTextFile` (Gemini lo aprovecha).
  2. Al primer `tool_call` con `kind: "edit"` (o al inicio del turno), capturar el **base snapshot** del archivo.
  3. Usar el contenido `type: "diff"` de `tool_call`/`tool_call_update` como señal de UI, no como fuente de verdad.
  4. Vigilar el filesystem (inotify) y re-leer el archivo tras cada tool call de edición: ese es el `newText` real.
  5. Accept/reject por hunk contra el base snapshot; reject = reescribir el hunk original a disco.
  6. **Permission gate**: `session/request_permission` llega antes de ejecutar la tool, con `toolCallId` y `rawInput`. Único punto de bloqueo previo real (sin granularidad por hunk).

**Extensión no estándar implementada por ambos adapters: `agentFileChangeReport`** (`_meta.jetbrains.air`). Se negocia en `initialize`; el cliente mete `agentFileChangeReportRequest {version:1, requestId}` en `session/prompt` y el agente responde con `session_info_update` con la lista de paths modificados en el turno. Claude la deriva de checkpoints (`Query.rewindFiles`), Codex de `turn/diff/updated`. Ambos publican `declaredComplete: false` (no capturan cambios de shell). Docs: `docs/agent-file-change-report.md` en ambos repos.

## 2. Protocolo v1: superficie necesaria

**Métodos Agent (el cliente llama):** `initialize`, `authenticate`, `session/new`, `session/load`, `session/resume`, `session/close`, `session/delete`, `session/list`, `session/prompt`, `session/set_mode`, `session/set_config_option`, `logout`. Notificación: `session/cancel`.

**Métodos Client (el cliente implementa):** `session/request_permission` (obligatorio), `fs/read_text_file`, `fs/write_text_file`, `terminal/*`, `elicitation/create`. Notificación: `session/update`.

**`initialize`** negocia `protocolVersion` (entero `1`), `clientCapabilities`, `agentCapabilities`, `authMethods`, `agentInfo`.

**`session/update` — `sessionUpdate`:** `agent_message_chunk`, `agent_thought_chunk`, `user_message_chunk`, `tool_call`, `tool_call_update`, `plan`, `available_commands_update`, `current_mode_update`, `config_option_update`, `session_info_update`, `usage_update`.

**ToolCall:** `toolCallId`, `title`, `name`, `kind` (`read|edit|delete|move|search|execute|think|fetch|other`), `status` (`pending|in_progress|completed|failed`), `content[]`, `locations[]` (`{path, line}`), `rawInput`, `rawOutput`.

**Contenido `diff`:**
```json
{ "type": "diff", "path": "/abs/path/src/config.json",
  "oldText": "{\n  \"debug\": false\n}", "newText": "{\n  \"debug\": true\n}" }
```
`oldText` es `null` si es archivo nuevo. **No hay rename/delete en v1** (solo `kind: "delete"`/`"move"`). v2 (RFD `diff-file-states`) reemplaza por `changes[]` con ops `add|delete|modify|move|copy`, `fileType`, `mimeType`, `patch` en `git_patch`.

**Permisos:**
```json
{"method":"session/request_permission","params":{
  "sessionId":"…","toolCall":{"toolCallId":"call_001"},
  "options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},
             {"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}
```
`kind` ∈ `allow_once | allow_always | reject_once | reject_always`. Respuesta: `{"outcome":{"outcome":"selected","optionId":"…"}}` o `{"outcome":{"outcome":"cancelled"}}` (obligatorio si se canceló el turno).

**Stop reasons:** `end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, `cancelled`.

**Modes / models:** legacy `currentModeId`/`availableModes` + `session/set_mode` + `current_mode_update`. Moderno: **Session Config Options** `ConfigOption {id, name, type: "select"|"boolean", currentValue, options}` con categorías `mode`, `model`, `model_config`, `thought_level`; `session/set_config_option` devuelve el estado completo; el agente puede empujar `config_option_update`. Soportar ambos.

**Slash commands:** `available_commands_update` con `[{name, description, input?: {hint}}]`; se envían como texto en `session/prompt`.

**MCP passthrough:** `session/new` recibe `mcpServers[]` (stdio siempre; HTTP/SSE opcional).

**Cancelación:** `session/cancel` (notificación por turno) y `$/cancel_request` con error `-32800`. El cliente debe marcar tool calls incompletas como canceladas y seguir aceptando updates.

**Elicitation** (`elicitation/create`): input estructurado, capabilities `form` y `url`. Claude lo usa para OAuth de MCP.

## 3. Terminales

`terminal/create` → `{terminalId}`; `terminal/output`, `terminal/wait_for_exit`, `terminal/kill`, `terminal/release`. Params: `command`, `args[]`, `env[]`, `cwd`, `outputByteLimit`. Capability opcional; **claude-agent-acp no la usa**. Recomendación: no implementar en v1 de Asteroid. En ACP v2 se reemplaza por "Terminal Output" agent-owned.

## 4. Rust: crates

SDK Rust: **`github.com/agentclientprotocol/rust-sdk`** (reescrito).

| Crate | Versión | Para qué |
|---|---|---|
| `agent-client-protocol` | 2.1.0 | Core: roles `Client`/`Agent`/`Proxy`/`Conductor`, builders |
| `agent-client-protocol-schema` | 1.7.0 | Tipos de wire |
| `agent-client-protocol-http` | 2.1.0 | HTTP/SSE + WebSocket |
| `agent-client-protocol-rmcp` | 3.1.0 | MCP vía `rmcp` |
| `agent-client-protocol-cookbook` | — | Patrones |

- Edition 2024, **MSRV 1.88** (ojo: el equipo tiene Rust 1.85, hay que actualizar), Apache-2.0.
- **Ya no es `!Send`/LocalSet**: handlers `impl Future + Send`, tokio multi-thread normal.
- Features unstable: `unstable_protocol_v2`, `unstable_mcp_over_acp`, `unstable_session_fork`, etc.

Spawn del agente con `AcpAgent` / `AcpAgentConfig` (`acp_agent.rs`):
```rust
let agent = AcpAgent::new(AcpAgentConfig::new("npx")
    .arg("-y").arg("@agentclientprotocol/claude-agent-acp"));

agent_client_protocol::Client.builder()
    .on_receive_notification(async move |n: SessionNotification, _cx| { Ok(()) },
        agent_client_protocol::on_receive_notification!())
    .on_receive_request(async move |req: RequestPermissionRequest, responder, _conn| {
        responder.respond(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id))))
    }, agent_client_protocol::on_receive_request!())
    .connect_with(agent, |c: ConnectionTo<Agent>| async move {
        c.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
        let s = c.send_request(NewSessionRequest::new(cwd)).block_task().await?;
        c.send_request(PromptRequest::new(s.session_id, vec![])).block_task().await?;
        Ok(())
    }).await?;
```
Captura stderr (64 KiB), callbacks de debug por línea, `SHUTDOWN_GRACE_PERIOD` 1s. Ejemplos: `examples/yolo_one_shot_client.rs`, `examples/v2_one_shot_client.rs`, binario `yopo`. Docs: https://agentclientprotocol.github.io/rust-sdk/

## 5. Adaptadores y cómo lanzarlos

Registry oficial: `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json` (41 agentes). Schema: `{id, name, version, description, repository, distribution: {npx: {package, args?} | binary: {<os>-<arch>: {archive, cmd, args, sha256}}}, icon}`.

**Claude** — `npx -y @agentclientprotocol/claude-agent-acp` (v0.79.0, Node ≥22). Usa Claude Agent SDK 0.3.274. **Honra la suscripción**: `authMethods` incluye
```
id: "claude-ai-login", name: "Claude Subscription", type: "terminal", args: ["--cli","auth","login","--claudeai"]
id: "console-login",   name: "Anthropic Console",  args: ["--cli","auth","login","--console"]
```
Auth tipo `terminal`: el cliente abre un terminal y corre el comando (capability `auth.terminal`). Si ya se hizo `claude login`, hereda la sesión. Env: `CLAUDE_CODE_EXECUTABLE`.

**Codex** — `npx -y @agentclientprotocol/codex-acp` (v1.12.0). Auth: ChatGPT login (suscripción), API key, o gateway. Env: `CODEX_PATH`, `CODEX_CONFIG`, `NO_BROWSER=1`, `INITIAL_AGENT_MODE` (`read-only|agent|agent-full-access`), `MODEL_PROVIDER`.

Otros: `npx @google/gemini-cli@0.60.0 --acp` · `opencode acp` · `goose acp` · `npx @github/copilot@1.0.85 --acp` · `cursor-agent acp` · `npx @qwen-code/qwen-code --acp` · `npx cline --acp` · `npx @augmentcode/auggie --acp` · `junie` · `kimi` · `amp-acp` · `npx @kilocode/cli acp` · `antigravity-acp` · etc.

## 6. Otros clientes ACP
Zed · JetBrains AI Assistant (nativo) · Emacs agent-shell.el · Neovim: CodeCompanion, avante, agentic.nvim · Qt Creator · Sublime Text · VSCode (varios) · Obsidian · marimo. Lista: https://agentclientprotocol.com/get-started/clients

## 7. Pitfalls

1. **Diffs parciales por hunk, no archivo entero.** Claude emite diff "optimista" desde `rawInput` (`Edit`: `oldText = old_string`; `Write`: `oldText: null` siempre) y luego lo corrige con `tool_call_update` desde `structuredPatch`. Nunca reconstruir el archivo concatenando `newText`.
2. **`oldText` puede no matchear el buffer.** Usar `locations[].line` como pista, fuzzy anchoring; si falla, re-leer disco.
3. **Sin rename/delete en v1.** Planificar migración a `changes[]` de v2.
4. **Binarios**: `fs/*` es solo texto.
5. **Archivos grandes**: `fs/read_text_file` acepta `line`+`limit` (1-based). Si `line > max` responder `invalid_params`.
6. **Cancel**: responder `cancelled` a permisos pendientes, seguir aceptando updates, marcar tool calls colgadas.
7. **stdio**: NDJSON estricto; leer stdout por líneas sin límite pequeño (líneas de MBs) y drenar `stderr` siempre.
8. **Crash del agente**: `npx` añade proceso intermedio; matar el process group.
9. **Linux**: sin problemas; ojo con `npx` y musl para el binario de Claude.
10. **Race disco↔buffer**: Claude/Codex escriben a disco; si el archivo está abierto y sucio hay que reconciliar. Política recomendada: al empezar un turno, guardar o bloquear edición manual de los archivos tocados.
11. **`format_on_save`** puede descuadrar offsets de hunk; Zed re-registra `buffer_edited` tras formatear.

## 8. Recomendación

Implementar **v1** con `agent-client-protocol` 2.1.0, anunciar `fs.readTextFile` + `fs.writeTextFile` (+ `elicitation`, + `session.configOptions`), **no** anunciar `terminal`. Arquitectura de edición **híbrida**: shadow buffer cuando la escritura llega por `fs/write_text_file`, y *apply + track + revert* estilo `ActionLog` (base snapshot + diff + keep/reject por rango) para Claude y Codex, complementado con `agentFileChangeReport` y watching de FS. Usar `session/request_permission` como preview pre-escritura.

**Fuentes:** [Protocolo v1](https://agentclientprotocol.com/protocol/v1/overview) · [File System](https://agentclientprotocol.com/protocol/v1/file-system) · [Tool Calls](https://agentclientprotocol.com/protocol/v1/tool-calls) · [v2 draft](https://agentclientprotocol.com/announcements/acp-v2-draft) · [RFD diff-file-states](https://agentclientprotocol.com/rfds/v2/diff-file-states) · [Rust SDK](https://github.com/agentclientprotocol/rust-sdk) · [claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp) · [codex-acp](https://github.com/agentclientprotocol/codex-acp) · [registry.json](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json) · [Zed acp_thread.rs](https://github.com/zed-industries/zed/blob/main/crates/acp_thread/src/acp_thread.rs) · [Zed action_log.rs](https://github.com/zed-industries/zed/blob/main/crates/action_log/src/action_log.rs) · [Gemini ACP mode](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/acp-mode.md)

---

# Adenda 01-b: correcciones tras la implementación de E0

Verificado con `agent-client-protocol` 2.1.0 y el `registry.json` real (2026-09-18):
- El **registro** no es "uno de" `npx`/`binary`: algunos agentes traen ambos canales, existe `uvx`, `sha256` falta en 9 agentes, `npx.package` ya incluye `@version` y hay `env` en `npx` y en targets `binary`.
- Método de login de Claude: `claude-login` ("Log in with Claude", `args: ["--cli"]`), no `claude-ai-login`. Codex expone `api-key` pero funciona con la sesión ChatGPT del CLI sin autenticar.
- `AcpAgentConfig` no acepta `cwd`; para lanzar en el directorio del proyecto hay que usar `tokio::process` y armar el transporte NDJSON a mano (lo que además permite tolerar líneas no-JSON).
- Los handlers `session/request_permission` y `fs/*` corren dentro del dispatch loop: responder desde una tarea aparte, o la conexión se bloquea.
- `on_receive_request` debe encadenarse por tipo concreto (el enum `AgentRequest` acepta todo y devuelve `Value`).
- `AuthCapabilities` vive en `schema::v1`.
- `SessionCreated` no trae comandos; llegan siempre por `available_commands_update`.
