# Etapa 2: chat y agentes

Estado: **completa en el workstream C** (2026-09-18). `asteroid-acp` (A) y
`asteroid-chat` (B) llegaron a esta sesión ya terminados y sin commitear; este
documento registra la integración en `asteroid-workspace` y `asteroid`
(workstream C) y deja constancia de A/B tal como se los encontró.

## Qué se construyó

### A — `asteroid-acp` (recibido terminado)

Cliente ACP completo: `AgentRegistry` (CDN + caché 24 h + `agents.custom`),
`AgentConnection` (hilo tokio propio, transporte tolerante a líneas no-JSON),
`Autonomy`/`AutonomyMode` con globs de rutas sensibles, protocolo (`AgentCommand`/
`AgentEvent`, `PromptBlock`, `McpServerSpec`, `AuthMethodView`,
`FileChangeReport` vía `_meta.jetbrains.air`), instalación de binarios
(`install.rs`) y un agente falso de test (`tests/fake_agent`). 32 tests
unitarios + 12 de integración contra el agente falso = 44 tests; no se tocó
nada de este crate en esta sesión.

### B — `asteroid-chat` (recibido terminado)

Panel de chat sobre GPUI: `ChatPanel` (transcript, popovers `@`/`/`,
selectores de pie, tarjetas de permiso/auth), `ChatEvent`, `ChatTheme`/
`ChatSettings` (structs `Default` sin dependencia de `asteroid-settings`),
`default_key_bindings`, persistencia (`export_transcript`/`import_transcript`
+ `TranscriptDump`). 40 tests. Tampoco se tocó en esta sesión.

### C — Integración en `asteroid-workspace` y `asteroid` (esta sesión)

1. **`ChatPanel` en el dock izquierdo**, reemplazando el placeholder:
   - `crates/asteroid-workspace/src/panels.rs`: `ChatDock` adapta
     `Entity<asteroid_chat::ChatPanel>` a `gpui-kit::dock::{Panel, BasePanel}`
     (la regla de huérfanos de Rust impide implementar el trait ajeno sobre el
     tipo ajeno directamente, así que se envuelve en vez de reimplementar).
   - `crates/asteroid-workspace/src/theme.rs`: `From<&ThemeColors> for
     asteroid_chat::ChatTheme` + `chat_theme`/`chat_settings`, con recarga en
     caliente cableada en `Workspace::new`'s settings-watcher (`chat.set_theme`
     / `chat.set_settings` en cada `SettingsEvent`).
   - `crates/asteroid-workspace/src/keymap.rs`: `asteroid_chat::default_key_bindings()`
     se suma a `built_in_bindings()` (no se llama a `asteroid_chat::init`
     directamente: ver "Desviaciones").
   - `Ctrl+L` (`workspace::focus_chat`) mueve el foco al compositor
     (`ChatPanel::focus_input`); `Ctrl+Shift+A` sigue colapsando el dock (sin
     cambios, ya funcionaba por posición de panel).
2. **Ciclo de vida del agente**: `crates/asteroid-workspace/src/agents.rs`
   (nuevo, entidad `Agents`):
   - `AgentRegistry::load()` en el executor de fondo, fusionado con
     `settings.agents.custom` (`with_custom`); estado "instalado" por canal
     (`node` para `npx`, `uv` para `uvx`, `is_installed` para `binary`, `PATH`
     para `custom`) → `ChatPanel::set_agents`; `settings.agents.default` se
     selecciona si está en la lista.
   - Seleccionar un agente lanza `AgentConnection::start(sandbox_root)` +
     `AgentCommand::Spawn`, drena sus `AgentEvent`s en una tarea `cx.spawn` y,
     al `Connected`, envía `session/new` automáticamente
     (`Agents::start_session`) para que el chat tenga sesión sin un paso extra.
   - `FsRead` → `BufferStore::read_for_agent` (valida `line > total` acá, como
     documenta `modulos/acp.md` §Handlers); `FsWrite` →
     `BufferStore::apply_agent_write` (crea el buffer si no estaba abierto),
     guarda, refresca git y nudgea el editor abierto.
   - `PermissionRequest` → `Autonomy::decide` (modo de settings +
     `review.sensitive_paths`); autoresponde con `pick_allow_option` cuando
     corresponde, si no delega a `ChatPanel::request_permission`.
   - Cada `ToolCallUpdate` que llega a `completed` con `kind: edit` (o el
     `FileChangeReport` de fin de turno) recarga el buffer abierto desde disco
     si está limpio y marca la pestaña.
   - `Exited`/`Error` → tarjeta en el chat (ya la pinta `ChatPanel::handle_event`)
     + un toast con la cola de stderr y un botón "Reiniciar" que relanza el
     mismo agente.
   - `ChatEvent::OpenTerminalWithCommand` → intenta `cosmic-term`,
     `x-terminal-emulator`, `gnome-terminal`, `xterm` en ese orden;
     `CopyToClipboard` → portapapeles; `RequestFileList` → substring sobre
     `Worktree::entries()` (archivos, máx. 50); `AutonomyChanged` → reconstruye
     la política (persistencia: perezosa, ver más abajo).
3. **Arrastrar desde el árbol al chat**: `gpui-kit` no expone una fuente de
   arrastre en su árbol virtualizado (no hay `on_drag` en `tree()`/`TreeItem`,
   a diferencia de las pestañas del centro, que arrastran con primitivas
   `div` crudas). Se implementó el atajo que la tarea ya autorizaba: ítem de
   menú contextual "Mencionar en el chat"
   (`crates/asteroid-workspace/src/tree_panel.rs`, acción
   `workspace::mention_in_chat`) que emite `WorkspaceEvent::MentionFile` y
   `Workspace` lo resuelve en `ChatPanel::insert_mention`.
4. **Barra de estado**: el lado derecho muestra `"<agente> · <estado>"` (o
   "Sin agente") leyendo directamente `chat.active_agent()`/`chat.status()` —
   son la misma fuente que pinta la cabecera del chat, así que están siempre
   sincronizados sin lógica propia; un clic mueve el foco al compositor
   (`crates/asteroid-workspace/src/workspace.rs::render_status_bar`).
5. **Persistencia del transcript** por proyecto:
   `~/.local/state/asteroid/workspaces/<hash>/chat.json` (mismo hash de
   `layout.json`). Se importa al abrir el proyecto (solo lectura: no toca
   `session_id`, así que la primera consulta real dispara sesión nueva) y se
   guarda cada 30 s si cambió y una vez más al cerrar la ventana
   (`Agents::save_transcript_now`, atado a `cx.on_app_quit`).
   `ChatPanel::set_sessions` recibe como mucho un resumen (título = primer
   mensaje del usuario, fecha = mtime del archivo), porque el formato actual
   (`chat.md`, "Persistencia") solo guarda **una** sesión por proyecto.
6. **Estados vacíos y bordes**: sin Node → agentes `npx` atenuados con
   "Instalá Node 22+"; sin registro (sin red y sin caché) → toast de aviso y
   solo los agentes de `agents.custom`; sin proyecto abierto → sandbox en un
   `tempfile::TempDir` con el aviso exacto "Abrí una carpeta para que el
   agente vea tus archivos.".
7. **Tests**: 12 tests nuevos en
   `crates/asteroid-workspace/src/agents.rs` (feature `test-support`, agente
   falso de `asteroid-acp` vía `CustomAgent` apuntando al binario) + 2 en
   `crates/asteroid-workspace/src/tests.rs` (`Ctrl+L`, mención por menú
   contextual) = **14 tests nuevos** (pedido: ≥ 12).
8. Este documento.

## Verificación

Todo corrido desde la raíz del repo (`cargo build -p asteroid-acp --bin
asteroid-acp-fake-agent` antes de los tests, como pide la tarea):

| Comando | Resultado |
|---|---|
| `cargo fmt --all --check` | limpio |
| `cargo clippy -p asteroid -p asteroid-workspace --all-targets --features asteroid-workspace/test-support -- -D warnings` | limpio |
| `cargo test -p asteroid -p asteroid-workspace --features asteroid-workspace/test-support` | 67/0 (60 en `asteroid-workspace`, 7 en `asteroid`, + doctest) |
| `cargo run -p asteroid -- --smoke-test .` | exit 0, 5 cuadros dibujados |
| `cargo run -p asteroid --features asteroid-workspace/test-support -- --smoke-test .` (detector de fugas) | exit 0, sin abort |
| Corrida real 5 s+ sobre este repo | sin panics |
| `pgrep -af sleep` | vacío |

No se corrió contra agentes reales (Claude/Codex/Gemini): la tarea lo
prohíbe explícitamente para esta sesión y los tests usan
`asteroid-acp-fake-agent`.

## Desviaciones

- **`asteroid_chat::init` no se llama**: en vez de invocar la función (que
  hace `cx.bind_keys(default_key_bindings())` una sola vez), sus bindings se
  suman dentro de `crate::keymap::built_in_bindings()`. Motivo: `keymap::install`
  hace `cx.clear_key_bindings()` en cada recarga de `keymap.json`/`settings.json`
  y reconstruye todo desde `BaseBindings` + `built_in_bindings()` + el keymap
  del usuario; si los bindings del chat se instalaran una sola vez por fuera de
  ese pipeline, la primera recarga en caliente los borraría (es exactamente el
  motivo por el que `asteroid_editor::default_key_bindings()` ya vive ahí
  desde la Etapa 1). Efecto observable: ninguno, el resultado es el mismo
  conjunto de atajos, con la garantía extra de sobrevivir a una recarga.
- **`AgentCommand::NewSession`/`LoadSession`/`ResumeSession` recalculan `cwd`**:
  `ChatPanel::new_session()` llena `cwd` con `std::env::current_dir()`, que no
  tiene por qué ser la carpeta del proyecto (es el directorio de trabajo del
  proceso `asteroid`, casi nunca el mismo). `Agents::forward_command`
  reconstruye el comando reemplazando `cwd` por el `sandbox_root` real de la
  conexión antes de reenviarlo.
- **Selección de agente sin evento**: `ChatPanel::set_active_agent` solo hace
  `cx.notify()`, no emite un `ChatEvent`. `Agents::on_chat_notify` es un
  `cx.observe` que compara `active_agent().id` contra el último valor visto en
  cada notificación (que en la práctica es casi cualquier cambio del panel) y
  solo actúa cuando cambió. Funciona, pero es un diff de instantáneas donde un
  evento explícito sería más barato y más claro.
- **`RequestFileList` no usa su propio `reply`**: GPUI entrega los eventos de
  `cx.subscribe`/`cx.observe` por referencia (`&ChatEvent`), y el campo `reply`
  es un `Box<dyn FnOnce(Vec<PathBuf>)>` que no se puede invocar a través de una
  referencia compartida. `Agents::answer_file_list` ignora el closure y llama
  directamente a `ChatPanel::set_file_candidates`, ya que este crate ya tiene
  un handle fuerte al panel. Mismo problema de fondo con `ChatEvent::Command`
  (envuelve un `AgentCommand` que no es `Clone`): se reconstruye campo por
  campo en vez de clonar el enum entero.
- **`ChatSettings` no tiene contraparte en `settings.json`**: ningún campo de
  `asteroid_chat::ChatSettings` (`input_min_rows`/`max_rows`,
  `diff_preview_lines`, `collapse_thoughts`, `max_entries`) existe en
  `asteroid_settings::Settings` (no hay sección `chat`). `theme::chat_settings`
  siempre devuelve `ChatSettings::default()`; el cableado de recarga en
  caliente está listo para cuando esa sección exista.
- **Tests sin drenaje automático de eventos**: `Agents::new` recibe un
  `background: bool` (se pasa `project_options.watch_files`, la misma señal
  que ya apaga `Watcher`/`GitStatusWatcher` en `ProjectOptions::inert`). En
  `false`, no se lanza la tarea `cx.spawn` que drena `AgentConnection::events()`
  automáticamente: esa tarea es exactamente el patrón ("hilo ajeno despierta
  una tarea en primer plano") que el detector de determinismo de GPUI no
  tolera bajo `#[gpui::test]` (`docs/etapas/etapa-1.md`, mismo motivo que
  `ProjectOptions::inert`). Los tests drenan a mano con
  `connection.events().recv_blocking()` (una llamada bloqueante normal en el
  hilo del test, no una tarea de GPUI) y empujan cada evento con
  `Agents::handle_agent_event`. El proceso del agente falso y su hilo tokio son
  reales en ambos modos; lo único que cambia es quién lee el canal de eventos.

## Deseos (API que otro crate debería exponer)

- `asteroid-chat`: que `ChatPanel::set_active_agent` emita
  `ChatEvent::AgentSelected(id)` (o similar) en vez de depender de que el
  llamador note la diferencia entre notificaciones.
- `asteroid-chat`: que `ChatEvent::Command`/`RequestFileList` no dependan de
  recibirse por valor (o que el crate ofrezca una variante ya pensada para
  `cx.subscribe`, que en GPUI entrega todo por referencia).
- `asteroid-chat`: que `ChatPanel::new_session()` no fije `cwd`, o que lo
  reciba como parámetro — hoy asume `std::env::current_dir()`, que solo la
  capa de integración sabe si es correcto.
- `asteroid-settings`: una sección `chat` en `Settings` (los cinco campos de
  `ChatSettings`) para que la Etapa 2 tenga algo real que recargar en
  caliente además del tema.
- `asteroid-project`/`asteroid-workspace` (para la propia Etapa 3, no
  bloqueante): una forma de insertar una entrada nueva en `Worktree` sin
  esperar al watcher, para que un archivo creado por el agente aparezca en el
  árbol incluso con `watch_files = false`.

## Gaps conocidos

- El indicador de pestaña "◆" es un sufijo de texto fijo (no un ícono, no se
  puede limpiar); la Etapa 3 lo reemplaza por el indicador real de revisión
  (`+N −M`).
- "Ver en el editor" de una tarjeta de edición abre la pestaña pero no salta
  al primer segmento pendiente (no existe "segmento" hasta la Etapa 3); es
  literal lo que pedía la tarea ("jumping to a hunk is E3").
- Un archivo que el agente crea vía `fs/write_text_file` sin que el watcher
  esté activo (los tests, o una corrida con `ProjectOptions::inert`) no
  aparece en el árbol hasta el próximo refresco manual: solo se refresca git,
  no el worktree (ver "Deseos").
- El diálogo de "buffer sucio" antes de que el agente escriba encima de
  cambios sin guardar (`modulos/workspace.md`, "Ciclo de revisión") es
  explícitamente de la Etapa 3; hoy `apply_agent_write` sigue adelante.

## Lista de comprobación manual (autor)

Cierra la Etapa 2 de `docs/specs/05-plan-etapas.md`. Con sesiones reales de
Claude, Codex y Gemini ya autenticadas en la máquina (nada de esto lo corre
un agente).

```bash
cd asteroid-editor
cargo build -p asteroid            # una vez
./target/debug/asteroid .          # abre este repo
```

1. **Elegir un agente.** Abrí el selector de la cabecera del chat: los
   agentes sin `node` (o sin el ejecutable, si son personalizados) aparecen
   atenuados con la pista de instalación. Elegí Claude: la barra de estado
   (abajo a la derecha) pasa de "Sin agente" a "Claude · listo" en cuanto
   conecta.
2. **Pedile que explique un archivo.** Escribí `@` y elegí `src/main.rs`
   (o el que exista): se inserta como chip. Pedile "explicá este archivo" y
   `Enter`. La respuesta llega en streaming, sin parpadeo.
3. **Pedile que ejecute `ls`.** Con la autonomía en "Pedir antes" (selector
   del pie del chat), pedile que corra `ls`: aparece la tarjeta de permiso con
   los botones del agente; `Enter` acepta el primero, `Esc` rechaza el
   primer `reject_*`. Contestala y mirá que la barra de estado diga "esperando
   permiso" mientras espera y vuelva a "listo" después.
4. **Cambiar de modelo.** Si el agente anuncia un selector de modelo en el
   pie del chat, cambialo: la próxima respuesta de `configOptions` debería
   reflejar el nuevo valor en el mismo selector.
5. **Cancelar a mitad de turno.** Pedile algo largo y apretá `Esc` (con el
   chat enfocado) antes de que termine: el turno se marca cancelado, las
   tarjetas de herramienta en curso pasan a "falló", y podés escribir de
   nuevo enseguida.
6. **Repetir con Codex y con Gemini.** Elegí cada uno del selector: Asteroid
   lanza el proceso correspondiente (puede pedir autenticación con una
   tarjeta y un comando para copiar/abrir en terminal — probá el botón "Abrir
   terminal"). Repetí los pasos 2 a 5 con cada uno.
7. **Mencionar arrastrando (o el atajo).** `gpui-kit` no tiene arrastre desde
   el árbol; abrí el menú contextual de un archivo en el árbol y elegí
   "Mencionar en el chat": debería aparecer como chip arriba del input, igual
   que si lo hubieras tecleado con `@`.
8. **Comando slash.** Escribí `/` en el chat: debería aparecer la lista que
   anuncia el agente (si anuncia alguno) con su descripción; `Tab` o `Enter`
   la inserta.
9. **Cambiar la autonomía.** Cambiala a "Aplicar siempre" y pedile una
   edición chica: no debería aparecer tarjeta de permiso (salvo que el
   archivo matchee `review.sensitive_paths`, en cuyo caso sí debe
   preguntarte incluso en este modo). Cerrá y reabrí Asteroid en el mismo
   proyecto: el selector debería recordar el valor elegido.
10. **La barra de estado en vivo.** Mientras el agente piensa, la barra debe
    decir "pensando…"; con un permiso pendiente, "esperando permiso"; si
    matás el proceso del agente a mano (`pkill -f claude-agent-acp` o
    similar), debe pasar a "desconectado" y aparecer un aviso con botón
    "Reiniciar" que, al tocarlo, vuelve a conectar.

## Correcciones tras la prueba manual del autor (2026-09-18)

Observaciones recibidas y su resolución:

| Observación | Resolución |
|---|---|
| Las menciones mostraban la ruta absoluta | Chip con el nombre del archivo; carpeta relativa en gris en el selector y como tooltip. El agente sigue recibiendo el `file://` absoluto. El workspace informa la raíz del proyecto con `ChatPanel::set_project_root`. |
| `@`/`/`: Enter enviaba el mensaje, las flechas no navegaban, el clic enviaba | La tecla se procesaba dos veces (acción del chat + `PressEnter` del textarea). Nuevo contexto `Chat > Input` con `up/down/enter/tab/escape` que gana al del componente y propaga cuando no hay popover. Tests con teclado y mouse reales. |
| Los comandos ejecutados no se distinguían | Tarjeta de terminal: icono, comando monoespaciado, estado, plegada si salió bien, salida recortada a 20 líneas con "ver más". |
| Mensajes propios y del agente sin diferencia clara | Burbuja a la derecha con etiqueta "Vos"; respuesta del agente a ancho completo con etiqueta del agente y regla de acento a la izquierda. |
| "turno terminado · X s" innecesario | Eliminado del render (queda en el modelo). |
| No se ve el pensamiento | Fila "Pensando… (N s)" plegada y desplegable, spinner mientras llega; `tracing::debug!` por chunk para saber si el agente lo envía. |
| Modos de autonomía incomprensibles | "Aplicar y revisar después" / "Preguntar antes de cada cambio" / "Aplicar sin revisar", con tooltip explicativo. |
| Input y barra de opciones indistinguibles; placeholder inútil | Selectores como chips en su fila, separados por una línea; caja de input con borde propio y botón adentro; placeholder "Escribí un mensaje para {agente}…" y pista debajo. |
| El editor "parpadea" al escribir | Causa: el estado de sintaxis se mudaba al hilo de fondo en cada tecla y el texto se pintaba sin colores un cuadro. Ahora reparseo síncrono con presupuesto de 2 ms, colores acarreados si no llega a tiempo, y caché de líneas por contenido: solo la fila editada se vuelve a maquetar. Medido: 0 filas pierden color (antes hasta 300 por 180 teclas). |
| Al cambiar de carpeta el agente seguía en el proyecto anterior | `Agents::set_project` cancela el turno en curso, guarda el transcript, cierra la conexión, carga el transcript del nuevo proyecto y relanza el agente con la nueva raíz. 4 tests. |

Verificación tras las correcciones: fmt y clippy limpios, 477 tests, `cargo deny` ok, smoke y detección de fugas exit 0.

## Correcciones tras la segunda prueba con agentes reales (2026-09-19)

Tres observaciones del autor después de usar el chat con Claude y Codex de
verdad, y cómo quedaron resueltas.

### 1. Conversaciones como objetos de primera clase (modelo Antigravity)

Un agente, una conversación. El hilo dejó de ser "el transcript del proyecto"
para ser una lista de conversaciones que se pueden abrir, borrar y retomar.

- **Modelo** (`asteroid-chat`): `Conversation { version, id, agent_id,
  agent_name, session_id, cwd, created_at, updated_at, title, autonomy,
  entries }` y `ConversationSummary` para las filas de la popover. El título es
  el primer mensaje del usuario recortado a 60 caracteres
  (`conversation_title`).
- **Almacenamiento** (`asteroid-workspace::conversations`):
  `~/.local/state/asteroid/workspaces/<hash>/conversations/<id>.json` más un
  `index.json` (`id`, `agent_id`, `agent_name`, `title`, `updated_at`,
  `session_id`) para pintar el historial sin leer cada transcript. El `id` es
  con forma de UUID (reloj + contador). El `chat.json` del formato anterior se
  migra a una conversación la primera vez que se abre el proyecto y se
  renombra a `chat.json.migrado` (no se borra: una migración mala se recupera
  a mano).
- **Comportamiento**: elegir otro agente en la cabecera **siempre** abre una
  conversación vacía para ese agente y guarda la anterior; el botón `+`
  ("Nueva conversación") hace lo mismo con el agente actual. La popover del
  reloj lista las conversaciones del directorio agrupadas por agente (el
  nombre del agente es el encabezado del grupo), más nuevas primero, con
  título, tiempo relativo ("hace 5 min", "ayer") y una ✕ que borra con
  deshacer de 5 s (`toast::undo`).
- **Retomar la sesión ACP**: al abrir una conversación guardada se manda
  `LoadSession { session_id }` si el agente anunció
  `agentCapabilities.loadSession`, `ResumeSession` si anunció
  `sessionCapabilities.resume`, y si no anuncia ninguna de las dos se muestra
  arriba del transcript, en `text.muted`: *"Historial de solo lectura: el
  agente no puede retomar esta sesión; tu próximo mensaje abre una sesión
  nueva"*, se abre una sesión nueva y el historial queda visible igual.
  Durante el replay de `session/load`, `ChatPanel::begin_replay` descarta lo
  que ya está en pantalla (por contenido, no por posición: un agente repite un
  mensaje entero donde el vivo mandó veinte chunks) y agrega lo que falte;
  `SessionCreated` cierra el filtro.
- **`asteroid-chat` ahora avisa**: `ChatEvent::AgentSelected`,
  `NewConversation`, `OpenConversation` y `DeleteConversation`. Con eso
  desapareció el *workaround* de `Agents::on_chat_notify`, que comparaba
  snapshots del panel en cada repintado para enterarse de un cambio de agente.
  La suscripción a los eventos del chat vive ahora dentro de `Agents::new`.

#### Capacidades verificadas de los agentes reales (2026-09-19)

Medido con `cargo run -p asteroid-acp --example chat -- --agent <id>
--capabilities-only`, una bandera nueva del ejemplo (único cambio aditivo en
`asteroid-acp`, y sólo en el ejemplo) que negocia `initialize`, imprime
`agentCapabilities` y sale: no crea sesión ni manda prompt, así que no gasta
tokens del modelo.

| Agente | Versión | `loadSession` | `sessionCapabilities.resume` | `session/list` |
|---|---|---|---|---|
| `claude-acp` (`@agentclientprotocol/claude-agent-acp`) | 0.79.0 | **sí** | **sí** | sí |
| `codex-acp` (`@agentclientprotocol/codex-acp`) | 1.12.0 | **sí** | **sí** | sí |

Los dos soportan `session/load`, así que en la práctica abrir una conversación
vieja con Claude o con Codex retoma la sesión de verdad; el aviso de solo
lectura es para los agentes que no lo anuncien.

### 2. Menciones en línea

El autor quiere escribir `mira este archivo @calculadora.py, que te parece`.

- Elegir un archivo en la popover `@` (o "Mencionar en el chat" del árbol)
  inserta el token `@calculadora.py` **en el cursor**, como texto plano
  (`gpui-kit`'s `Textarea` no tiene widgets en línea), y guarda el mapeo
  token → ruta absoluta del borrador.
- Al enviar, `split_mentions` corta el texto alrededor de los tokens y emite
  `PromptBlock::Text` / `PromptBlock::ResourceLink` **en orden**. Un token que
  el autor editó o borró a mano simplemente no se encuentra y deja de ser una
  mención.
- Dos archivos con el mismo nombre pasan a `@src/main.rs` y `@tests/main.rs`
  (se reescribe también el token ya escrito, para que nunca queden dos
  `@main.rs` idénticos).
- En la burbuja del usuario la mención se pinta como chip en línea, donde fue
  escrita, con el nombre del archivo y la ruta relativa como tooltip.
- Desapareció la tira de chips arriba del input.

### 3. Tarjeta de `execute`

- La salida pierde las vallas de markdown (```` ```console ````) antes de
  pintarse (`strip_code_fences`); sólo se saca la valla que envuelve todo el
  bloque.
- La cabecera muestra `$ ls -la` en monoespaciada, con prompt.
- La salida sigue en monoespaciada sobre `text.muted`, recortada a 20 líneas
  con "ver más".
- Si falló y la salida dice el código de salida (`exit code: 2`, `exit status
  127`, `código de salida: 3`), la cabecera lo muestra en rojo: `salió 2`.
- La tarjeta tiene borde `border` de 1 px y radio 6 (`COMMAND_CARD_RADIUS`).

### Desviaciones y decisiones

- Abrir un proyecto arranca con una conversación **vacía**, no con la última:
  el historial está a un clic y así el panel nunca arranca mostrando algo que
  el agente ya no puede continuar.
- Una conversación sin entradas no se guarda, para que cambiar de agente dos
  veces seguidas no deje archivos vacíos.
- El desduplicado del replay compara por contenido (`contains`): un chunk
  replayado muy corto que esté contenido en un mensaje ya guardado se
  descarta. Es el precio de aceptar que el replay venga con otra granularidad
  que el streaming en vivo.
- `toast::undo` es un aviso con botón **y** temporizador (5 s), a diferencia
  de `toast::ask`, que espera una respuesta para siempre.

Verificación: `cargo fmt --all --check` limpio, `cargo clippy` con `-D
warnings` limpio, tests de `asteroid` + `asteroid-chat` + `asteroid-workspace`
en verde (7 + 64 + 70), `cargo run -p asteroid -- --smoke-test .` y
`cargo run -p asteroid-chat --example chat_demo -- --smoke-test` exit 0.
