# Etapa 5: productividad

Estado: **cerrada** (2026-09-28). Spec: `docs/specs/07-etapa5-productividad.md`
(v1.1). Reemplazó a `docs/etapas/pendientes-etapa-5.md` como fuente de verdad
del alcance.

Este documento lo escribió el orquestador (E5-K) integrando el trabajo de las
subetapas E5-A a E5-J, cada una construida por un subagente en el mismo árbol
de trabajo (sin worktrees por subagente: cada uno tocó crates o archivos
disjuntos, y solo `Edit` en los pocos archivos compartidos, ver "Desviaciones
· E5-K").

## Qué se construyó

### E5-A — Foco: zonas, rueda y acciones nuevas
`crates/cincel-workspace/src/focus.rs`. `FocusZone { Chat, Center, Files }` con
el orden de la rueda (`FocusZone::WHEEL`); `next_zone` es la función pura que
decide el destino de `Ctrl+L` (tabla completa con las tres zonas y `None` ×
cada combinación de visibilidad, testeada exhaustivamente).
`Workspace::toggle_focus` implementa la regla de §7.1 para
`Ctrl+Shift+A`/`Ctrl+Shift+E`: panel oculto → abrir y enfocar; panel visible
con el foco dentro → cerrar y devolver el foco al centro; panel visible con el
foco en otro lado → enfocar sin cerrar. `Workspace::is_modal_open` reúne todo
lo que congela estos atajos (modal de conexiones, diálogo de pestaña sin
guardar, panel de revisión, buscador de archivos, modal de atajos, "Nuevo
archivo", diálogo de salida, menú de la barra de título). Se declararon todas
las acciones `workspace::*` nuevas de §11.1 y sus bindings por defecto en
`cincel-settings/src/defaults.rs`.

### E5-B — `cincel-settings`: edición de JSONC, autoguardado y keymap efectivo
`crates/cincel-settings/src/edit.rs`: `SettingsEdit::{Set, Remove}`,
`apply_edit` (puro, sobre el CST de `jsonc-parser` 0.33 con la feature `cst`)
y `write_edit` (lectura-modificación-escritura atómica). Un archivo vacío o
inexistente arranca del documento con el comentario explicativo, no de `{}`
pelado. `Autosave::AfterDelay` + `FilesSettings.autosave_delay_ms` (100 ms a
60 000 ms, por defecto 1000). `KeymapOrigin::{Default, User}` y
`Keymap::effective_bindings()` para el modal de atajos. Tokens de tema
`git.added`, `git.modified`, `git.deleted`.

### E5-C — Conexiones: cancelar de verdad y actualizar adaptador/Node
`crates/cincel-connections/src/cancel.rs`: `CancelToken` (`Arc<AtomicBool>`)
con `check`, `sleep` (en tramos de 50 ms) y los adaptadores `CancelReader`/
`CancelWriter` para que `io::copy` de una descompresión se corte a mitad de
entrada. `WorkLock` serializa dos preparaciones del mismo agente para que una
segunda espere a que la cancelada termine de limpiar su `.part`. Los bucles de
descarga de `runtime.rs` y `adapters.rs` revisan el token cada 64 KiB, npm se
lanza en su propio grupo de procesos y se mata al cancelar. `Adapters::update`/
`prune_unused` y `Runtime::update_available`/`update`/`prune_old` para la
política de actualización (D13: no se borra la versión en uso por un proceso
vivo). `cincel-acp`: `AgentEvent::AuthRequired` ahora trae `message: Option<String>`
con el motivo que dio el agente.

### E5-D — Git en el margen del editor
`crates/cincel-project/src/git.rs`: `git --no-optional-locks status
--porcelain=v2 -z --untracked-files=all` (D10, evita el bucle sobre
`.git/index`); `diff_against_head` parsea `git diff --no-color --no-ext-diff
--no-renames -U0 HEAD` por archivo abierto; `GitDirWatcher` vigila `HEAD`,
`index`, `refs/heads/**` con debounce de 300 ms e ignora los eventos que el
propio `git status` produce. `crates/cincel-editor/src/git_gutter.rs`:
`EditorView::set_git_diff(Option<Vec<GitGutterHunk>>, cx)` recibe filas del
archivo **guardado**; el editor las traslada a la versión actual del buffer
(`carry_over_unsaved_edits`) y las ancla (`Buffer::track_anchor`) para que
sigan cada edición hasta el próximo cálculo. Geometría del gutter: columna de
git de 3 px repartiendo los 13 px que ya existían (`GUTTER_PADDING_LEFT` 4→2,
`GIT_BAR_GAP` nueva de 2, `GUTTER_BAR_GAP` de la barra del agente 6→3); el
ancho total y el origen del texto quedan exactamente iguales, con test de
regresión. Colores por `GitGutterColors`/`EditorView::set_git_colors` (ver
desviación).

### E5-E — Buscar y reemplazar con filas fantasma
`crates/cincel-editor/src/search.rs`: `MatchLocation::{Buffer, Phantom}` en
orden de pantalla (una fantasma de un hunk antes de la fila real donde se
inserta); `SearchState.replacement`, `field: SearchField`, `replace_open`,
`replacement_for` (literal o expansión de grupos regex). `view.rs`: acciones
`editor::find_replace` (`Ctrl+H`), `replace_next` (`Enter`), `replace_all`
(`Ctrl+Enter`), `search_next_field`/`search_prev_field` (`Tab`/`Shift+Tab`).
"Reemplazar todo" es una única transacción del buffer con `EditSource::User`
(un `Ctrl+Z` la deshace entera). Reemplazar sobre una coincidencia fantasma no
cambia nada, avisa en la barra y salta a la siguiente coincidencia real (D12).
La barra mantiene los 28 px de alto de siempre en los dos modos: con la
franja angosta los botones "Reemplazar"/"Reemplazar todo" se reducen a los
glifos ⇄/⇶ (ver desviación).

### E5-F — Buscador rápido de archivos (`Ctrl+P`)
`crates/cincel-workspace/src/file_finder.rs`: `FileFinder` (una por proyecto
abierto, como el panel de revisión) con el campo de búsqueda, la lista de
candidatos (refrescada al abrir y en `ProjectEvent::TreeChanged`) y el
historial de "más reciente" armado observando qué pestaña activa el
`CenterPanel`. `rank(candidates, query, recent, limit)` es la función pura:
consulta vacía → recientes (la activa al final) + resto del árbol; consulta no
vacía → `frizbee::Matcher::new` sobre la ruta relativa, orden por puntaje
descendente con desempate manual (ver desviación), tope 200. Las posiciones de
`frizbee` (en orden inverso y con posibles duplicados en caracteres
multibyte) se normalizan a un offset de byte ascendente por carácter
coincidente (`char_indices_to_byte_offsets`), probado con rutas con "ñ"/"á".
Por encima de 5000 candidatos el filtrado corre en `cx.background_executor()`
con descarte de resultados de una generación vieja. Panel flotante sin velo
oscuro, con capa transparente que cierra al clic afuera, igual estilo y
posición que "Nuevo archivo" (E5-I) y el modal de atajos.

### E5-G — Pestaña de configuración (`Ctrl+,`)
`crates/cincel-workspace/src/settings_view.rs` (`SettingsView`,
`SettingsSection::{Appearance, Fonts, Editor, Files, Review, Connections}`,
tabla `ROWS` con cada ajuste: clave JSON, control, descripción en español).
Cada control escribe con `cincel_settings::write_edit` y emite
`SettingsViewEvent::Written`; el workspace recarga sin el toast "Configuración
recargada". Un error de sintaxis deshabilita los controles y muestra un
banner con la línea/columna traducidas al español (`spanish_syntax_message`).
La sección Conexiones (`crates/cincel-workspace/src/agents/settings_bridge.rs`,
`SettingsBridge` dentro de `Agents`) nunca toca `cincel_connections`
directamente: consulta el registry una vez por sesión (`ConnectionsShown`),
corre las actualizaciones de adaptador/Node en un hilo propio con
`CancelToken` y refleja el progreso en la barra. Test que recorre
`SettingsView::visible_texts()` y falla si aparece un texto en inglés de
gpui-kit (D2).

### E5-H — Modal de atajos y botón de la barra de estado
`crates/cincel-workspace/src/shortcuts_modal.rs`: mezcla el keymap en vigor
(`crate::settings::keymap`) con una capa de solo lectura hecha de los
bindings Rust de los widgets (`cincel_editor::default_key_bindings`,
`cincel_chat::default_key_bindings`, `crate::keymap::built_in_bindings`),
reutilizando `Keymap::effective_bindings` en vez de reimplementar el
apilamiento de capas. `category()` clasifica cada comando por una tabla
explícita (no todo `workspace::*` es "Generales"); `description()` trae el
texto en español de cada comando, con test que exige que todo comando del
keymap por defecto y de los widgets tenga descripción.
`format_keystroke`/`format_key` traducen `ctrl-shift-a` → `Ctrl+Shift+A`,
`escape` → `Esc`, `delete` → `Supr`, `pageup`/`pagedown` →
`RePág`/`AvPág`, flechas como glifos. Búsqueda insensible a mayúsculas y
acentos. "Abrir keymap.json" crea el archivo con `[]` si no existe.

### E5-I — Barra de título: menú, "Nuevo archivo", salir
`crates/cincel-workspace/src/title_menu.rs`: botón de menú (`IconName::Menu`)
con `PopupMenu` de gpui-kit vía `Button::dropdown_menu`, `action_context` =
el foco del editor de la pestaña activa (o el del workspace), submenú
"Carpetas recientes" con "Borrar la lista". Botones de paneles (chat/archivos)
que llaman a la misma `Workspace::toggle_focus` que los atajos. "Salir"
(`Ctrl+Q`) y la `×` de la ventana convergen en `Workspace::should_close`:
sin archivos sin guardar cierra directo; con ellos abre el diálogo
"¿Guardar cambios?" (Guardar todo y salir / Salir sin guardar / Cancelar).
`crates/cincel-workspace/src/new_file.rs`: `NewFilePrompt` (campo flotante,
mismo estilo que el buscador de archivos, D16) con `validate_new_file_name`
(función pura: rechaza vacío, rutas absolutas y `..`, acepta subcarpetas).

### E5-J — Autoguardado, motivo de `AuthRequired`, cancelar en el modal
Autoguardado "tras N ms sin escribir" en `center.rs`: cada cambio del usuario
reprograma una tarea `cx.spawn` con `Timer::after(delay)`; si al vencer el
buffer sigue sucio se guarda, salvo que el archivo esté en un turno de
revisión activo. `ConnectionBanner::Expired` del chat muestra "El agente
dijo: «motivo»" cuando `AuthRequired.message` llega redactado desde
`agents.rs`. El botón "Cancelar" del modal de conexiones durante
"Preparando…" quedó cableado al `CancelToken` de E5-C.

### E5-K — Integración (orquestador)
Verificación completa del workspace (ver más abajo), corrección de una fuga
de `Workspace` en el gancho de cierre de ventana (capturaba una `Entity`
fuerte en vez de una `WeakEntity`, lo que impedía liberar la ventana al
cerrarla en los tests de fugas), corrección de un test intermitente
(`acp_login_fails_when_the_agent_exits`, en `cincel-connections/tests/login.rs`)
agregando una espera de 300 ms (`EXIT_GRACE`) a que el proceso del agente
falso termine de salir tras un `authenticate` fallido antes de comprobar el
resultado, neutralización de nombres de ejemplo en tests y documentación
(`ana@example.com` en vez de nombres o correos que pudieran confundirse con
datos reales) y esta actualización de documentación.

## Verificación

Máquina de referencia (Pop!_OS, COSMIC Wayland), 2026-09-28, siempre con
`--features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`:

| Comando | Resultado |
|---|---|
| `cargo fmt --all --check` | ok |
| `cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings` | ok, 0 avisos |
| `cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins` y `cargo test --workspace` con las mismas features | ok, 1130 tests, 0 fallos |
| `cargo deny check licenses` | ok |
| `cargo run -p cincel -- --smoke-test .` | salida 0; 7 frames pintados |
| `cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .` (control de fugas) | ok tras corregir la fuga de `Workspace` del gancho de cierre de ventana (ver E5-K arriba) |
| `pgrep -af sleep` | vacío, sin procesos colgados |

El test de "sin bucle de `git status`" de §6.4/§6.5 corre solo en
`cincel-project` (`GitDirWatcher` no emite al correr `git --no-optional-locks
status`): los tests GPUI no levantan hilos ajenos al ejecutor determinista de
GPUI, así que la comprobación de que la ventana real no repite `git status`
cada segundo queda en la lista manual (punto 6 de §14, más abajo).

## Desviaciones

Todas las anota el subagente responsable, con su motivo; ninguna cambia el
comportamiento visible descrito en la spec 07 más allá de lo aquí explicado.

- **E5-A**: `workspace::focus_chat` sigue registrada sin binding por defecto
  (D9): quien la tenga puesta en su `keymap.json` no la pierde. Cuatro tests
  viejos de `tests.rs` se reescribieron porque el comportamiento nuevo de
  abrir/cerrar/enfocar los paneles (§7.1) reemplaza al de "solo mostrar u
  ocultar" de las etapas anteriores.
- **E5-B**: `crate::jsonc::parse_options()` quedó compartido entre el parseo
  a `serde_json::Value` (carga de `settings.json`) y el CST de `edit.rs`, para
  no duplicar las opciones estrictas del parser. `center.rs` no necesitó
  ningún cambio para el autoguardado por pausa: ya comparaba el autoguardado
  con `!=` en vez de un `match` exhaustivo, así que la variante nueva
  `AfterDelay` entró sin tocar ese archivo.
- **E5-C**: "hay versión nueva" de un adaptador se compara contra el registry
  de agentes (que fija qué versión de npm corresponde a cada `AgentKind`), no
  contra el registry de npm directamente. Si el servidor deja de mandar datos
  a mitad de una descarga, la cancelación recién se nota en la próxima
  lectura (`ureq` 3 no corta una lectura a medias, solo la siguiente falla).
  El `npm install` en curso se vigila cada 100 ms desde un hilo propio, sin
  un mecanismo de espera más fino. `NodeVersion::Major` cubre políticas como
  `"22"` (un número de mayor sin versión completa). `prune_unused` lo cablea
  el workspace al arrancar (antes de lanzar nada, E5-G) y también cuando se
  detiene un agente o termina una actualización, no como una tarea periódica
  propia.
- **E5-D**: los colores de git entran por `GitGutterColors`/`set_git_colors`,
  un tipo propio de `cincel-editor`, y no por `EditorTheme` (para no tocar
  literales de tema en un archivo que otros subagentes tocaban en paralelo);
  llevar esos tres colores a `EditorTheme` junto con los demás tokens de tema
  queda como limpieza pendiente. `GitDiffChanged` es un evento aparte de
  `ProjectEvent`, no una variante nueva del mismo enum, para no romper el
  `match` exhaustivo de `tree_panel.rs` en paralelo con otro subagente. La
  barra del agente en el gutter se movió según la tabla de §6.3 (el gap antes
  de los números pasó de 4 px a 3 px, con la barra de git ocupando los 5 px
  liberados por el padding y el primer gap). D10 suma `--literal-pathspecs` y
  `ls-files` para detectar archivos no seguidos, y compara contra el árbol
  vacío en repositorios sin ningún commit todavía. `set_git_diff` recibe
  filas del texto **guardado**, y es el editor quien las traslada a la
  versión actual del texto con las mismas ediciones mínimas que usa un
  guardado del agente. La marca de línea borrada sigue la numeración que da
  git (fila que sigue a lo borrado), no una convención propia. El test de
  "sin bucle de `git status`" vive solo en `cincel-project` (los tests GPUI
  no corren hilos ajenos al ejecutor determinista): la comprobación real
  queda en la lista manual, punto 6.
- **E5-E**: los botones de la barra de búsqueda usan los glifos ⇄/⇶ en vez de
  iconos de Lucide cuando el ancho es menor a 720 px (`SEARCH_BAR_COMPACT_WIDTH`
  en `view.rs`), porque `cincel-editor` no depende de `gpui-kit` (que es
  quien trae `IconName`): sumar esa dependencia solo para dos iconos hubiera
  sido un cambio de mayor alcance que dos glifos Unicode. `keymap.rs` del
  workspace reinstala `cincel_editor::search_bar_bindings()` por encima del
  keymap del usuario salvo que este último asigne esa tecla en ese mismo
  contexto, porque el JSONC por defecto no trae secciones para `tab`/`enter`/
  `ctrl-enter` dentro de `Editor && searching`/`replacing`; agregarlas al
  documento por defecto (en vez de resolverlo en código) queda como mejora
  futura. Se eliminó el test `search_does_not_match_inside_phantom_rows`, que
  probaba justamente lo contrario de lo que pide esta spec (ahora sí se busca
  dentro de las filas fantasma).
- **E5-F**: sin `radix_sort_matches` de frizbee (esa función no conserva las
  posiciones de coincidencia, que el buscador necesita para resaltar), el
  orden se replicó a mano (puntaje descendente, empate por longitud de ruta y
  luego alfabético). El presupuesto "< 50 ms con 50 000 rutas" de §3.1 no se
  mide con un test automático de rendimiento: el ejecutor de fondo hace el
  trabajo pesado y `tracing::debug!` deja el tiempo real en el log para
  verificarlo a mano. El historial de "más reciente" lo arma `FileFinder`
  observando qué pestaña activa el panel central, porque `CenterPanel` no
  guarda ese historial por sí mismo y agregárselo hubiera tocado un archivo
  que otros subagentes editaban en paralelo. Dependencia nueva: `frizbee`
  0.13.0 (MIT).
- **E5-G**: `CenterItem` es un enum de **solo lectura** hacia afuera (las
  pestañas de archivo siguen viviendo en el `Vec<Tab>` de siempre) y la
  pestaña de configuración ocupa una posición aparte en la barra, no una fila
  más de ese vector. Eventos extra no pedidos por la spec:
  `SettingsViewEvent::Written`, `OpenSettingsFile` y `ConnectionsShown`
  (consulta al registry una vez por sesión, disparada por la vista misma en
  vez de por el workspace). Sin proyecto abierto, "Abrir settings.json" abre
  el archivo con la aplicación del sistema en vez de en una pestaña de
  Cincel (no hay `CenterPanel` para alojarla). Los errores de sintaxis de
  `jsonc-parser` (en inglés) se traducen con una tabla fija
  (`spanish_syntax_message`) en vez de un traductor genérico. El test de "sin
  textos en inglés" revisa `SettingsView::visible_texts()` (las cadenas que
  la vista construye), no lo efectivamente pintado en pantalla — una
  diferencia solo importa si un control de gpui-kit ignorara el texto que se
  le pasa, que no es el caso hoy. Sin test GPUI de punta a punta para
  "actualizar Node" ni para "cancelar una actualización" (se prueban por
  partes: el hilo de actualización en `cincel-connections`, y el cableado de
  eventos en `settings_bridge.rs`). El texto "Configuración" de la barra de
  estado no tiene un test propio.
- **E5-H**: módulo `shortcuts_modal.rs` en vez de `shortcuts.rs` (nombre más
  descriptivo, dado que también contiene la vista, no solo la categorización).
  Los chips de tecla son un `div` propio en vez del componente `Kbd` de
  gpui-kit, porque `Kbd` nombra las teclas en inglés y la spec pide español.
  "Abrir keymap.json" no hace nada sin un proyecto abierto (mismo límite que
  `CenterPanel::open_file`, que necesita un proyecto para alojar la pestaña).
  Sin test de clic sobre el botón de atajos de la barra de estado: llama a la
  misma función (`show_shortcuts`) que ya prueba el atajo `F1`.
- **E5-I**: sin tests de clic por coordenada de píxel (la feature
  `gpui-kit/test-support`, que habilitaría simular clics reales, no está
  encendida en el conjunto de features acordado para no multiplicar
  variantes de `target`): se prueba la función que cada botón invoca
  (`toggle_focus`, `request_quit`, etc.) en vez del clic en sí. Los bindings
  de teclado de los diálogos "Nuevo archivo" y "Salir" viven en
  `keymap::modal_bindings()` junto con los de los demás modales transitorios,
  no en `built_in_bindings()` (que alimenta la lista del modal de atajos):
  son diálogos efímeros cuya sola presencia en pantalla ya explica qué hacen
  `Enter`/`Esc`, a diferencia del buscador de archivos o el modal de atajos,
  que son paneles descubribles.
- **E5-J**: el test de cancelar una descarga no simula una descarga lenta
  real de principio a fin: prueba que el `CancelToken` se dispara y que el
  modal descarta un resultado que llega tarde, no el tiempo real de una
  descarga de red.
- **E5-K (orquestador)**: test intermitente
  `acp_login_fails_when_the_agent_exits` corregido en
  `cincel-connections/tests/login.rs` con una espera fija de 300 ms
  (`EXIT_GRACE`) a que el proceso del agente falso termine de salir después
  de un `authenticate` fallido, antes de comprobar el resultado (la falla
  intermitente era una carrera entre la salida del proceso y la lectura del
  resultado, no un error de lógica). Fuga de `Workspace` en el gancho de
  cierre de ventana corregida cambiando una `Entity<Workspace>` capturada por
  valor a una `WeakEntity<Workspace>` (el gancho no necesita mantener viva la
  ventana; mantenerla viva era justamente la fuga que el control de fugas de
  §15 detectó). Nombres de ejemplo en tests y documentación se neutralizaron
  (`ana@example.com` en vez de nombres que pudieran leerse como datos reales
  de una persona). Sin worktrees por subagente: cada worktree hubiera
  duplicado el directorio `target` (varios GB por copia, con el riesgo de
  llenar el disco que ya señala `pendientes-etapa-5.md` §B); en su lugar,
  todos los subagentes trabajaron en el mismo árbol sobre crates o archivos
  disjuntos, y solo `Edit` (nunca reescritura completa) en los pocos archivos
  que dos subetapas tocaban en paralelo (`defaults.rs` entre E5-A y E5-B,
  `keymap.rs` y `workspace.rs` entre varias).

## Lista de comprobación manual (autor)

Tal como la deja `docs/specs/07-etapa5-productividad.md` §14, para recorrerla
antes de dar por cerrada la etapa:

1. `Ctrl+P`, escribir parte de un nombre con una letra de más: aparece el
   archivo con las letras resaltadas; `Enter` lo abre en cursiva.
2. `Ctrl+,`: cambiar el tema a claro, la fuente del editor a 16 y apagar el
   ajuste de línea; abrir `settings.json` desde el botón y ver que tus
   comentarios siguen ahí. Editar a mano `ui_font_size` y ver el cambio en la
   pestaña. `Ctrl+W` la cierra.
3. En Configuración → Conexiones: renombrar una conexión; si aparece
   "Actualizar", actualizar el adaptador.
4. Conectar un agente nuevo que haya que descargar y cancelar a mitad de
   "Preparando…": el modal vuelve enseguida y en
   `~/.cache/cincel/downloads/` no queda el `.part`.
5. `F1` (o el botón del teclado en la barra de estado): buscar "chat"; si
   tenés un `keymap.json`, ver tus atajos marcados como "Personalizado".
6. En un archivo de un repositorio: cambiar una línea, agregar otras, borrar
   una y guardar: azul, verde y la marca roja. Hacer `git commit -am` en la
   terminal y ver que desaparecen. Dejar el proyecto abierto 30 s quieto y
   confirmar en `~/.local/state/cincel/log/cincel.log` que no se repite
   ningún `git status` (verificación manual de §6.4, sin test GPUI
   automático: ver "Desviaciones · E5-D").
7. `Ctrl+Shift+A` con el chat oculto (se abre y escribís ahí), otra vez (se
   cierra y volvés al editor); lo mismo con `Ctrl+Shift+E`. Probar los dos
   botones de la barra de título.
8. `Ctrl+L` varias veces: chat → editor → archivos; con el chat oculto, lo
   salta. `Ctrl+Shift+L` en el editor sigue seleccionando la línea.
9. El menú de la barra de título: abrir una carpeta reciente, "Nuevo
   archivo…" (y `Ctrl+N`), "Salir" con un archivo sin guardar; cerrar con la
   `×` con un archivo sin guardar (pregunta lo mismo).
10. Con una revisión pendiente y cambios de git en el mismo archivo, abrir y
    cerrar la búsqueda y el reemplazo: el código no se mueve de costado y la
    barra no crece.
11. `Ctrl+H` en un archivo con un cambio del agente pendiente: buscar una
    palabra que esté en una línea roja (la encuentra) y "Reemplazar todo"
    (no la toca y lo dice).
12. Poner `"files": { "autosave": "after_delay" }`, escribir y esperar un
    segundo: el punto de "sin guardar" desaparece.

## Deudas para la Etapa 6 o después

- Rendimiento, instalador, repositorio público, documentación de usuario y
  fuentes embebidas: ya estaban planificados para la Etapa 6
  (`docs/specs/05-plan-etapas.md`), sin cambios por esta etapa.
- Llevar `git.added`/`git.modified`/`git.deleted` de `GitGutterColors` a
  `EditorTheme`, junto con el resto de los tokens de tema del editor
  (E5-D).
- Agregar al `keymap.json` por defecto las secciones explícitas de
  `tab`/`enter`/`ctrl-enter` para `Editor && searching`/`replacing`, en vez de
  resolver la precedencia en código (`crate::keymap::search_bar_bindings`,
  E5-E).
- Medir automáticamente el presupuesto "< 50 ms con 50 000 rutas" del
  buscador de archivos, hoy solo registrado con `tracing::debug!` (E5-F).
- Test GPUI de punta a punta para "actualizar Node" y para "cancelar una
  actualización" en la pestaña de configuración (E5-G).
- Habilitar `gpui-kit/test-support` (o una alternativa) para tests de clic
  por coordenada real sobre los botones de la barra de título y de la barra
  de estado (E5-I, E5-H).
- El test de cancelar una descarga (E5-J) no ejercita una descarga de red
  lenta real; sería valioso un test de integración que sí lo haga, fuera del
  conjunto de tests que corre en cada verificación.
- Lo ya anotado en `docs/etapas/pendientes-etapa-5.md` §B que esta etapa no
  tocó (arrastrar imágenes al chat, servidores MCP sin interfaz propia,
  formularios de `elicitation`, etc.) sigue abierto tal cual estaba.

## Correcciones tras la prueba del autor (2026-09-28)

Ocho observaciones del autor sobre la primera prueba, resueltas antes del
commit de la etapa, todas con test:

1. **README.md "en blanco" al abrirlo la primera vez.** Causa real, hallada en
   el registro de la sesión: al arrancar, el escritorio informa primero
   apariencia clara y 0,6 s después oscura; `set_system_dark` cambiaba el tema
   global y la ventana, pero no avisaba a los editores ya abiertos (los
   restaurados del layout), que quedaban con texto del tema claro sobre fondo
   oscuro: invisible. Los archivos abiertos después nacían con el tema
   correcto, por eso "cambiar de carpeta y volver" lo arreglaba. Arreglo:
   `Workspace::follow_system_appearance` reaplica el tema a editores, vistas
   previas y chat (`refresh_theme_everywhere`) cuando cambia la apariencia.
   Test `appearance_tests::open_editors_follow_a_late_change_of_the_desktop_appearance`.
   Un subagente no pudo reproducirlo porque en su entorno la apariencia no
   cambia tras el arranque; dejó además `first_open_tests.rs` (dos tests de
   que la primera apertura pinta todas las líneas).
2. **Cambios sin decidir al cerrar.** Salir (`Ctrl+Q`), la `×`, "Abrir
   carpeta…" y "Carpetas recientes" preguntan "Hay N cambios de agente sin
   decidir en M archivos" con Aceptar todo / Rechazar todo / Cancelar; después,
   si hace falta, "¿Guardar cambios?". Con un turno en curso, primero se
   cancela. Usa `accept_all`/`reject_all` (todos los turnos), no `accept_turn`.
   `review_close.rs`, tests en `pending_review_close_tests.rs`.
3. **La rueda del mouse en los modales** (atajos, buscador, nuevo archivo) ya no
   mueve el editor de fondo: `.occlude()` en el contenedor flotante. Tests en
   `modal_scroll_tests.rs`.
4. **Progreso de "Actualizar adaptador"** ya no pisa la fila ni corta
   "Cancelar" (texto con elipsis, botón fijo). Test en `settings_view_tests.rs`.
5. **`review.jump_to_next_on_decide` pasa a `false` por defecto**, en
   `cincel-settings` y en `EditorSettings` del editor; los tests que necesitan
   el salto lo activan explícitamente.
6. **Barra flotante del editor**: "Aceptar todo" / "Rechazar todo" del turno
   completo (`Ctrl+Alt+↵` / `Ctrl+Alt+⌫`), con confirmación "Rechazar N cambios
   en M archivos" para rechazar; flechas, contador y "Revisar todo" siguen.
   Los controles por segmento y por línea quedan como única vía para decidir
   de a uno.
7. **Contadores**: no había error de conteo, pero contaban cosas distintas sin
   decirlo. Árbol: "1 archivo · 3 cambios · +0 −36"; barra: "cambio 1 de 3";
   barra de estado: "N cambios pendientes". Tests en `review_counter_tests.rs`.
8. **Saltar a un segmento lo centra** verticalmente (todos los caminos pasan por
   `jump_to_row`), salvo que ya esté en el tercio central; y **el puntero es la
   flecha normal** sobre la barra de scroll y el margen (los `+`/`−` del margen
   conservan la mano). Tests en `cincel-editor/src/review_tests.rs`.

Verificación tras las correcciones: fmt limpio; clippy 0 avisos; 1156 tests,
0 fallos; `cargo deny` ok; smoke test ok; control de fugas ok.

Nota de proceso: un subagente tomó una captura de pantalla completa de la
sesión del autor para intentar reproducir el punto 1 y la borró de inmediato;
no quedó en el repo ni en el scratchpad. Los briefs futuros prohíben capturas
de pantalla completa.

## Corrección de diseño de la revisión (2026-09-28)

**Qué pasaba.** La revisión solo seguía un archivo si el agente lo nombraba
por sus canales (`fs/read_text_file`, `fs/write_text_file` o un `tool_call`
edit/delete/move con ruta). Cuando el agente cambiaba archivos de otra manera
(el autor lo reprodujo dos veces: un `python3 - <<'EOF'` con `import re`
reescribió `calc.py` y `expr.py`), ningún tool call nombraba la ruta,
`Review::on_disk_changes` descartaba el cambio y el buffer se recargaba en
silencio: "0 cambios pendientes" con el disco ya modificado.

**La regla (la acordada desde el principio).** Cincel se guarda cómo está
cada archivo de la carpeta antes de mandar el mensaje; todo lo que cambie en
disco dentro del proyecto hasta que el agente termina es del agente, sin
importar con qué lo hizo, y se revisa en el editor en rojo y verde con
aceptar y rechazar por segmento y por línea. Lo que Cincel mismo escribe
durante el turno (un `Ctrl+S`, el autoguardado, un archivo nuevo, un rechazo)
no es del agente; lo que cambian otros programas, sí.

**Qué cambió** (spec: `03-arquitectura.md` §4 v2, `modulos/review.md`,
`modulos/workspace.md` "Ciclo de revisión"):

1. *Foto* (`cincel-project/src/snapshot.rs`): `ProjectSnapshot::capture`
   recorre el proyecto con el mismo `WalkBuilder` que el árbol
   (`project_walk_builder`, extraído de `Worktree`; mismas exclusiones y
   `files.exclude`) y guarda por archivo tamaño, mtime y copia: texto hasta
   `review.max_file_size_kb`, binarios con bytes y hash, y solo hash pasado el
   tope nuevo `review.snapshot_max_total_mb` (300 por defecto). `changes()` es
   el repaso: metadatos primero, contenido si difieren o si el mtime es
   reciente (ventana de 2 s, el problema "racy git").
2. *Turno* (`cincel-workspace/src/review_snapshot.rs`, módulo hijo de
   `review.rs`): `begin_prompt` saca la foto en el ejecutor de fondo;
   `Agents::forward_command` retiene el prompt hasta que está lista y, si pasa
   1 s, el chat muestra "Preparando la revisión…". Cancelar mientras tanto
   descarta el prompt. Con `ProjectOptions::inert()` la foto es síncrona.
3. *Vigilancia*: `Project::apply_fs_events` emite `FilesChanged` con cada
   lote del watcher (abierto o no); la revisión adopta cada ruta con la foto
   como base (`capture_base_text`, o `file_created` si no existía) y el disco
   como verdad; renombrar = borrar + crear; las carpetas se expanden.
4. *Repaso final* en `end_turn` (y `agent_gone`, y al encadenar turnos): suma
   lo que el watcher no avisó; recién después se libera la foto.
5. *Atribución*: `Project::save` y `note_host_write` (archivo nuevo) emiten
   `HostWrote`, que actualiza la foto; los rechazos de la revisión también.
   Un buffer con cambios sin guardar conserva los del usuario en la base.
6. *Pistas*: fs/read, fs/write y tool calls siguen capturando antes, como en
   v1; si el disco ya difiere de la foto, la base es la foto.
7. *Binarios* (no UTF-8): "archivo binario cambiado por el agente" en el
   árbol y el panel, aceptar o rechazar el archivo entero (rechazar = restaurar
   los bytes); sin copia previa, solo aceptar.
8. Agente falso: `FAKE_SHELL_EDITS` (`ruta=contenido;…`, `ruta=` borra,
   `ruta=@texto` crea, `ruta=>destino` renombra) escribe directo en disco y
   solo reporta un `tool_call` `execute` sin rutas.

**Tests.** `cincel-workspace/src/snapshot_review_tests.rs`: edición por shell
de un archivo abierto (segmentos y "1 archivo · 2 cambios · +3 −3"), de uno no
abierto, creado y borrado, renombrado, `Ctrl+S` del usuario durante el turno,
repaso final sin evento y con mismo tamaño y mtime, rechazo que restaura la
foto exacta, foto de 5 000 archivos / 50 MB (≈25 ms), Edit por tool call
idéntico a v1, turnos encadenados, binario restaurado, prompt que espera la
foto y cancelación durante la foto. `cincel-project/src/snapshot.rs`:
recorrido, exclusiones, topes de tamaño, binarios, repaso, `refresh`, 5 000
archivos. Ningún test existente cambió.

**Desviaciones.**
- La brief pedía §5 de `03-arquitectura.md`; el modelo de revisión es el §4
  (el §5 es el pipeline de display): se actualizó el §4.
- Los cambios binarios no se persisten al cerrar el proyecto y su rechazo no
  entra en `Alt+Shift+U`: el store (que persiste y deshace) solo guarda texto.
- Archivos de más de 64 MB no se hashean: se comparan por tamaño y mtime.
- El repaso final corre en el hilo principal (recorrido de metadatos; solo lee
  los archivos que difieren o son recientes).
- El aviso "Preparando la revisión…" no tiene test automático: en el
  ejecutor determinista de los tests la foto termina antes que el temporizador.
- `review.snapshot_max_total_mb` no tiene fila en la pestaña de
  configuración: `settings_view.rs` lo tenía otro subagente en paralelo.

### Arreglos reportados por el autor: la `×` y los archivos borrados

**1. La `×` de la barra de título cerraba sin preguntar.** *Causa:* Cincel
instala `on_window_should_close` (pregunta por cambios de agente sin decidir y
por archivos sin guardar), pero la `×` la dibuja `gpui_component::TitleBar`,
que sin manejador propio llama a `window.remove_window()` directamente; el
gancho solo corría con el cierre del escritorio (`Alt+F4`). *Arreglo:*
`Workspace::request_close` (`title_menu.rs`) hace `should_close` + guardar
`WindowState` y la usan los dos caminos: el gancho devuelve su resultado y
`TitleBar::on_close_window` llama a `Workspace::close_from_title_bar`, que
quita la ventana solo si `request_close` dijo que sí. *Test:*
`title_bar_tests::the_title_bar_close_button_asks_before_closing` y
`deleted_file_review_tests::the_close_dialog_counts_the_deletion`.
*Limitación:* gpui-kit solo dibuja los controles de ventana con decoraciones
del cliente y la ventana de test de GPUI siempre reporta decoraciones del
servidor, así que el botón no está en pantalla en los tests: se prueba la
función exacta que llama su clic.

**2. Un archivo borrado por el agente se veía como uno normal y al abrirlo
salía "no se pudo leer … No such file or directory".** *Causa:* el árbol
pintaba `FileState::Deleted` igual que un modificado (color de aviso,
`+0 −N`) y, al hacer clic, `CenterPanel::open_file` intentaba leerlo del
disco. *Arreglo (como Antigravity):*
- *Árbol* (`tree_panel.rs`): el borrado pendiente va tachado, en
  `status.error`, con `−N` (N = líneas del contenido, `FileReview::stats`
  ya no cuenta la fila vacía tras el último salto) y el tooltip "Borrado por
  el agente · pendiente"; el creado, en verde con `+N` (antes `+N −0`). El
  árbol lo sigue mostrando aunque el watcher ya lo haya quitado del
  `Worktree` (y también la carpeta si se fue con él) hasta que se decide.
- *Pestaña* (`center.rs`, `Tab::deleted_review`): clic, `Alt+L` o el panel
  "Revisar todo" abren una pestaña de solo lectura sobre un buffer vacío
  fuera del store; `Review::view_for_tab` le pinta todo el contenido previo
  como una única sección de filas fantasma en rojo (el mismo
  `DiffTransformMap` del editor, hunk `DELETED_FILE_HUNK`), y la barra
  flotante suma "✓ Aceptar archivo" / "✗ Rechazar archivo"
  (`ReviewView::file_actions`, `AcceptFile`/`RejectFile`) delante de
  "Aceptar todo"/"Rechazar todo" y "Revisar todo". La píldora del hunk decide
  el archivo entero. Aceptar cierra la pestaña y el archivo sale del árbol;
  rechazar restaura el archivo y la pestaña pasa a ser un editor normal
  (`CenterPanel::sync_deleted_reviews`, que la revisión pide después de cada
  refresco que lo necesite). Una pestaña limpia ya abierta cuando el agente
  borra el archivo pasa a ser la vista del borrado; una con cambios sin
  guardar se deja como está. Un binario borrado (o sin copia previa) muestra
  una sola fila que lo explica.

*Tests* (`deleted_file_review_tests.rs`, con `FAKE_SHELL_EDITS`): árbol
tachado con `−6` y creado con `+1`; clic abre la pestaña en rojo sin toast de
error y no se puede escribir; rechazar restaura y la pestaña queda editable
con el contenido; aceptar quita archivo, pestaña y fila; `Alt+L` y el panel
llegan al borrado; el diálogo de cierre lo cuenta como cambio pendiente.
Además `tree_panel::tests::pending_deletions_stay_in_the_tree` y
`cincel-editor` `the_bar_offers_the_file_decision_when_asked`; en
`cincel-review` el test que fijaba `(0, 2)` para un borrado de una línea pasa
a `(0, 1)`.
