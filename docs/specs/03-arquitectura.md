# Spec 03: Arquitectura

Estado: v1.0. Describe los módulos, sus fronteras, el modelo de hilos y las dependencias. Las decisiones están justificadas en `docs/00-sintesis-y-decisiones.md`.

## 1. Stack

| Capa | Elección | Motivo corto |
|---|---|---|
| Lenguaje | Rust, edition 2024, toolchain fijado en `rust-toolchain.toml` (≥ 1.90) | MSRV de `agent-client-protocol` y `tree-sitter-language` |
| Motor gráfico | GPUI, el motor de Zed, vía `gpui-pre` 0.3.5 y `gpui-pre-platform` 0.3.5 (la versión exacta que exige `gpui-kit` 0.6.1; `gpui-unofficial` queda como alternativa si gpui-kit migra) | aspecto Zed, Wayland/X11 nativos, renderizado subpíxel |
| Componentes UI | `gpui-kit` (dock, pestañas, árbol, inputs, markdown en streaming, popovers) | ahorra meses; Apache-2.0 |
| Editor de código | **propio** (`cincel-editor`), con un pipeline de display diseñado para el diff inline | ningún widget Rust existente inserta filas fantasma |
| Texto | `ropey` | estándar; lo usan Helix y gpui-kit |
| Sintaxis | `tree-sitter` 0.27 + `tree-sitter-highlight`, gramáticas estáticas por cargo feature, siempre desde crates.io | evita duplicar `tree-sitter-language` |
| Diff | `imara-diff` (o su fork `gix-imara-diff` si está más vivo), Histogram + postproceso de hunks; `similar` para diff de palabras | lo que usa Zed; el postproceso es requisito de corrección |
| Agentes | `agent-client-protocol` 2.2.x (ACP v1) sobre `tokio` | protocolo oficial; Claude, Codex, Google Antigravity (OpenCode en otra etapa) |
| Archivos | `notify` 9 + `notify-debouncer-full`, `ignore` para recorrer respetando `.gitignore` | evita agotar inotify |
| Git | binario `git` del sistema para `status`/`diff` | hereda credenciales y config |
| Diálogos | `rfd` vía portal xdg | funciona en COSMIC y GNOME |
| Config | `serde_json` + `json5`/`jsonc-parser` para comentarios | como Zed |
| Logs | `tracing` + `tracing-subscriber` + `tracing-appender`, a stderr y a `~/.local/state/cincel/log/` (rotación diaria, 7 archivos); redacción de secretos en `cincel-log` | diagnóstico, `docs/specs/06-etapa4-conexiones-y-cincel.md` §8 |

## 2. Workspace Cargo

```
cincel-editor/
  Cargo.toml                 workspace, lints compartidos, perfiles (release: lto="fat", codegen-units=1, strip=false)
  rust-toolchain.toml
  deny.toml                  cargo-deny: licencias permitidas (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, Unicode); prohibidas GPL/AGPL
  crates/
    cincel/                binario. main, ventana, arranque, CLI (`cincel [ruta]`), carga de settings/keymap/tema, wiring de todo
    cincel-workspace/      GPUI: layout de dock, pestañas, árbol de archivos, barra de estado, avisos, panel de revisión, comandos y keymap
    cincel-editor/         GPUI: el elemento editor (display map con diff, gutter, cursor, selección, búsqueda, pill y barra flotante de revisión)
    cincel-chat/           GPUI: panel de chat (transcript, tarjetas de herramientas, permisos, input, selectores)
    cincel-text/           sin GPUI: Buffer (rope + anclas + transacciones + undo + EditSource), codificación y fin de línea
    cincel-syntax/         sin GPUI: registro de lenguajes, parseo incremental, spans de resaltado, tema de sintaxis
    cincel-review/         sin GPUI: ReviewStore, snapshots base, hunks, aceptar/rechazar/rebase, persistencia, informe para el agente
    cincel-project/        sin GPUI: Worktree (árbol de archivos), watcher, BufferStore (archivos abiertos, estado sucio, reconciliación con disco), estado git
    cincel-acp/            sin GPUI: registro de agentes, lanzamiento de procesos, cliente ACP, modelo de sesión, eventos hacia la UI, handlers fs/permisos
    cincel-settings/       sin GPUI: tipos de settings, keymap y tema; carga, valores por defecto, validación, recarga en caliente
    cincel-log/            sin GPUI: arranque del log a archivo (rotación diaria) y redacción de secretos de login (`docs/specs/06-etapa4-conexiones-y-cincel.md` §8)
```

Regla de dependencias: los crates "sin GPUI" no dependen de nada gráfico y se testean con `cargo test` puro. Los crates GPUI dependen de ellos, nunca al revés. `cincel-editor` no conoce ACP; `cincel-acp` no conoce el editor. El pegamento vive en `cincel` y `cincel-workspace`.

## 3. Modelo de hilos

- **Hilo principal**: GPUI. Toda entidad de UI y todo `Buffer` abierto viven aquí (GPUI usa `Entity<T>` no `Send`).
- **Ejecutor de fondo de GPUI** (`cx.background_executor()`): cálculo de diffs, parseo de tree-sitter, recorrido de directorios, lectura de archivos.
- **Hilo tokio** dedicado (`cincel-acp`): procesos de agentes y conexiones ACP. Nunca toca UI.

Comunicación UI ⇄ ACP por dos canales asíncronos (`async-channel`):
- `AgentCommand` (UI → ACP): `Spawn(agent_id, cwd)`, `NewSession`, `Prompt(session, blocks)`, `Cancel(session)`, `SetConfigOption(...)`, `RespondPermission(request_id, option_id)`, `Shutdown`.
- `AgentEvent` (ACP → UI): `Connected{agent_info, auth_methods, capabilities}`, `AuthRequired{methods}`, `SessionCreated{id, modes, config_options, commands}`, `Update(session, SessionUpdate)`, `PermissionRequest{id, tool_call, options}`, `FsRead{path, line, limit, reply: oneshot}`, `FsWrite{path, content, reply: oneshot}`, `TurnEnded{stop_reason}`, `Stderr(line)`, `Exited{code}`, `Error(String)`.

Los `FsRead`/`FsWrite` son peticiones del agente que la UI contesta desde el `BufferStore` (contenido en memoria, incluidos cambios sin guardar) a través del `oneshot` incluido en el evento. Este es el mismo patrón que usa Zed para cruzar del mundo `Send` de tokio al hilo `!Send` de la UI.

## 4. Flujo de datos de la revisión (el corazón)

```
 agente escribe archivo ──► disco ──► notify ──┐
 agente: tool_call kind=edit (pending)         ├──► cincel-review: ReviewStore
   └─ captura base_text si es 1er contacto ────┤        base_text por archivo
 agente: tool_call_update (completed) ─────────┤        hunks = diff(base_text, texto actual)
   └─ relee archivo, actualiza buffer ─────────┘        accept/reject/rebase
 fs/write_text_file (agentes que lo usan) ─────► BufferStore.apply_agent_edit ─► guarda ─► ReviewStore
 fs/read_text_file ────────────────────────────► BufferStore (contenido en memoria) ─► marca base si es 1er contacto
 usuario teclea ───────────────────────────────► Buffer (EditSource::User) ─► ReviewStore.rebase_user_edit
 usuario acepta/rechaza ───────────────────────► ReviewStore ─► (reject) Buffer edit + guardar
 ReviewStore.changed ──────────────────────────► cincel-editor (DiffTransformMap) y cincel-workspace (árbol, panel, barra)
```

**v2 (2026-09-28): la base viene de la foto del proyecto; las herramientas son solo una pista.** La regla del producto es una sola: Cincel se guarda cómo está cada archivo de la carpeta antes de mandar el mensaje; todo lo que cambie en disco dentro del proyecto hasta que el agente termina es del agente, sin importar con qué lo hizo (Edit, un script, un comando, otro programa), y se revisa en el editor igual que siempre: rojo y verde, aceptar y rechazar por segmento y por línea. Lo único que no es del agente es lo que el propio Cincel escribe durante el turno (un `Ctrl+S` del usuario, el autoguardado, un archivo nuevo creado desde Cincel, un rechazo).

```
 prompt ─► Review::begin_prompt ─► foto del proyecto (fondo) ─► recién entonces sale el prompt
             (mismo recorrido y exclusiones que el árbol; texto hasta review.max_file_size_kb;
              binarios: hash + bytes; por encima de review.snapshot_max_total_mb: solo hash)
 durante el turno:
   watcher (notify, cualquier ruta del proyecto, abierta o no) ─► FilesChanged ─► Review
     ruta sin seguimiento: base = copia de la foto (capture_base_text / file_created si no estaba)
     luego file_written / file_deleted con lo que hay en disco (renombrar = borrar + crear)
   pistas (fs/read, fs/write, tool_call edit/delete/move) ─► capturan base antes, como antes
   Cincel guarda (Ctrl+S, autoguardado, nuevo archivo, rechazo) ─► HostWrote ─► la foto se actualiza
 TurnEnded ─► repaso: toda la foto contra el disco (tamaño/mtime; contenido si difieren o si el
              mtime es reciente) ─► suma lo que el watcher no avisó ─► se libera la foto
```

Reglas:
0. **La foto.** `Review::begin_prompt` saca la foto (`cincel_project::ProjectSnapshot`) en el ejecutor de fondo y el prompt se envía cuando está lista; si tarda más de 1 s, el chat muestra "Preparando la revisión…". Presupuesto: 5 000 archivos / 50 MB en menos de 1 s. Cada turno saca su foto; un archivo que ya está en revisión conserva su base original hasta que el usuario decide (la foto nueva no lo re-basa). Archivos no UTF-8: "archivo binario cambiado por el agente", solo por archivo, rechazar = restaurar los bytes. Archivos sin copia (más grandes que `review.max_file_size_kb` o por encima del tope total): la base sale del buffer si estaba abierto al sacar la foto; si no, solo se puede aceptar. Los cambios hechos por otros programas durante el turno se atribuyen al agente.
1. La **base** de un archivo se fija en el primer contacto del turno: la copia de la foto (si el buffer no tiene cambios sin guardar), `fs/read_text_file`, `oldText` del primer `diff` del tool call, o lectura de disco al ver el primer `tool_call` de tipo `edit` con ese path en `locations`. Si el archivo está abierto con cambios sin guardar del usuario, se pregunta (guardar / descartar / mantener) antes de que el agente lo toque; si no se puede preguntar (permiso ya concedido), se guarda. Si el agente lo cambió por otra vía (sin pista), la base es la foto y los cambios sin guardar del usuario se suman a la base.
2. Los `diff` que manda el agente son **señal de interfaz**, no fuente de verdad. La verdad son los bytes en disco releídos al completarse cada tool call de edición (y al llegar el `agentFileChangeReport` del final del turno, si el agente lo soporta), las notificaciones del watcher y el repaso final contra la foto.
3. Los hunks se recalculan en el ejecutor de fondo con debounce de 50 ms cada vez que cambia el buffer o la base, descartando resultados de versiones viejas.
4. Aceptar = mover la base. Rechazar = editar el buffer con el texto de la base y guardar. Ninguna otra operación toca disco.

## 5. Pipeline de display del editor

```
Buffer (rope)  ──►  DiffTransformMap  ──►  WrapMap  ──►  (FoldMap, v2)  ──►  BlockMap  ──►  Elemento
                    inserta filas del        ajuste de                      pill de botones,
                    base_text por cada       línea                          separadores
                    hunk con eliminaciones                                  (bloques de UI, no texto)
```

- `DiffTransformMap` (modelo Zed): mapea filas de display ↔ filas del buffer, con filas fantasma que apuntan a rangos del `base_text`. Las filas fantasma son texto real para el pipeline (se resaltan, se seleccionan, se copian, se buscan) pero **no son editables**: cualquier edición con el cursor en una fila fantasma se redirige a la primera fila real siguiente.
- `BlockMap` solo aloja bloques de interfaz (pill de aceptar/rechazar por hunk); no texto.
- El elemento pinta únicamente las filas visibles (virtualización), pide el layout de cada línea al sistema de texto de GPUI y guarda los layouts en caché por versión de buffer.
- Implementa `InputHandler` de GPUI para IME, dead keys y composición.

## 6. Persistencia

Directorios XDG:
- Config: `~/.config/cincel/{settings.json, keymap.json, themes/}`.
- Estado: `~/.local/state/cincel/{log/, workspaces/<hash de ruta>/layout.json}`.
- Datos: `~/.local/share/cincel/review/<hash de ruta>/{state.json, objects/<sha256>}`.
- Caché: `~/.cache/cincel/registry.json`.

Al arrancar, antes de leer cualquiera de estos directorios: si existe `<raíz>/asteroid` y no `<raíz>/cincel`, se renombra (o se copia y se deja el original si el renombrado falla, por ejemplo entre sistemas de archivos), con un aviso en el log y un toast una vez abierta la ventana (`docs/specs/06-etapa4-conexiones-y-cincel.md` §7).

`state.json` de revisión: por archivo `{path, base_hash, current_hash, status, turn_id, created_at}`. Al restaurar: leer el archivo tal cual está en disco, comparar su hash con `current_hash`; si coincide, recalcular hunks contra `objects/<base_hash>`; si no, descartar la entrada y avisar. **Nunca se reescribe un archivo al restaurar.** Los objetos huérfanos se borran al aceptar/rechazar todo.

## 7. Errores y resiliencia

- Proceso de agente muerto: evento `Exited`, tarjeta en el chat, el turno se marca cancelado, los pendientes de revisión se conservan.
- stdout del agente con basura: el parser ignora líneas que no son JSON y las manda al log.
- Cancelación: `session/cancel` + responder `cancelled` a todo permiso pendiente + marcar tool calls colgadas.
- Archivo cambiado por fuera mientras hay pendientes: se recalculan hunks contra la misma base; si el archivo desaparece, el estado pasa a `Deleted` y el panel ofrece restaurarlo desde la base.
- Sin Vulkan: reintento con backend GL de wgpu; sin adaptador: mensaje claro y salida, salvo `CINCEL_ALLOW_SOFTWARE_GPU=1`.

## 8. Calidad

- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo deny check licenses` en cada etapa.
- Tests unitarios en todos los crates sin GPUI (obligatorios para `cincel-review` y `cincel-text`: cobertura de los casos de borde listados en `modulos/review.md`).
- Tests de integración de `cincel-acp` contra un agente falso (binario de test que habla ACP) para no depender de red ni de suscripciones.
- Smoke test gráfico manual por etapa (lista de comprobación en `05-plan-etapas.md`).
