# Etapa 6: rendimiento, distribución y publicación (cierre = versión 1.0)

Estado: **cerrada** (2026-09-29). Spec: `docs/specs/08-etapa6-cierre-1-0.md`.
Esta etapa cierra la versión **1.0.0** de Cincel.

Este documento lo escribió el orquestador (E6-L) integrando el trabajo de las
subetapas E6-A a E6-K, cada una construida por un subagente en el mismo árbol
de trabajo (sin worktrees por subagente, por la misma razón que en la
Etapa 5: cada worktree duplicaría `target`, varios GB por copia). Cada
subagente tocó crates o archivos disjuntos, con `Edit` (nunca reescritura
completa) en los pocos archivos que dos subetapas de la misma ola tocaban en
paralelo (`main.rs`, `Cargo.toml` raíz, `review.rs`, `keymap.rs`,
`settings_view.rs`, `defaults.rs`).

## Qué se construyó

### E6-A — `cincel --bench`: mediciones internas
`crates/cincel-workspace/src/bench.rs` y el CLI de `crates/cincel/src/main.rs`
(sección "Diagnóstico" de `USAGE`). Ocho escenarios de medición
(`startup`, `open`, `typing`, `scroll`, `idle`, `finder`, `sweep`,
`review-1mb`), `all` (los ocho en orden) y `demo` (solo para capturas, no
mide). Sin `--bench`, los ganchos (`frame_begin`, `frame_probe`) leen una
bandera global una vez y no hacen nada. "Fin de cuadro" se toma en una tarea
de primer plano lanzada desde el `paint` de `FrameProbe`, un elemento de
tamaño cero que el workspace agrega como último hijo de su elemento raíz —
GPUI 0.3.5 no ofrece un callback tras `present`, así que esa tarea, que corre
justo después de que GPUI vuelve al bucle principal, es el punto más cercano
disponible. `CINCEL_TRACE_TIMINGS=1` agrega `tracing::info_span!` en los
tramos que E6-G usó para ubicar el costo de M11 y M8 (§9 de
`docs/rendimiento.md`). Tests en `crates/cincel-workspace/src/
bench_tests.rs` (cada escenario sobre un proyecto temporal chico en
`TestAppContext`) y `crates/cincel/src/bench_cli_tests.rs` (`Cli::parse`
con `--bench` y cada escenario; uno desconocido da error de uso).

### E6-B — `cincel-perf`: banco de medición externo
Crate nuevo `crates/cincel-perf` (binario, sin GPUI, `publish = false`),
cliente Wayland propio: detecta la ventana de la app medida
(`wlr-foreign-toplevel-management` o, para la comprobación cruzada en
COSMIC, `ext-foreign-toplevel-list-v1`), captura cuadros con marca de tiempo
(`wlr-screencopy`, `copy_with_damage`) e inyecta teclas y rueda del mouse
(`zwp_virtual_keyboard_v1`, `zwlr_virtual_pointer_v1`). Subcomandos:
`corpus`, `evict`, `launch`, `idle`, `quick-open`, `type`, `scroll`, `key`,
`stop`, `probe`, `wait-wayland`, `report` (detalle y ejemplos en
`docs/specs/modulos/perf.md` y `tools/perf/README.md`). `tools/perf/`: imagen
Docker (`sway` sin pantalla a 240 Hz sobre `ubuntu:24.04`), `run.sh` (arma la
imagen, genera el corpus, lanza el contenedor), perfiles aislados por app
(§3.5 de la spec). Tests unitarios de corpus determinista, p50/p95, detección
de "primer cuadro con contenido" y "cuadro estable", suma de RSS/PSS sobre un
`/proc` falso, y armado de la tabla de `report`.

### E6-D — Fuentes Inter y JetBrains Mono embebidas
`crates/cincel-workspace/assets/fonts/` (diez TTF estáticos, sin recortar,
más `OFL.txt` de cada familia y `SOURCES.md` con versión, URL de la release
oficial y SHA-256 de cada archivo, calculado al descargar porque ninguno de
los dos proyectos publica hash por archivo). `crates/cincel-workspace/src/
fonts.rs`: `UI_FAMILY = "Inter"`, `BUFFER_FAMILY = "JetBrains Mono"`,
`embedded()` (`include_bytes!`), `register_embedded(cx)` (`cx.text_system().
add_fonts`, con el tiempo y las familias en el log). `main.rs` lo llama antes
de `cincel_workspace::init` y de abrir la ventana; un error se registra y el
arranque sigue con los respaldos del sistema. `fonts_tests.rs` comprueba
cabeceras TrueType de los diez archivos; como `TestAppContext` usa
`NoopTextSystem` (no resuelve fuentes reales), la comprobación de que
`all_font_names()` y `font_id` realmente resuelven Inter y JetBrains Mono
queda en `--smoke-test`, que las registra y falla si no resuelven.

### E6-E — Binarios persistidos, deshacer unificado y repaso en fondo
Dos deudas de revisión, en `crates/cincel-workspace/src/`:
- `review_binaries.rs`: cambios binarios (imágenes, PDF, cualquier archivo
  no UTF-8) persistidos en `review/<hash>/binaries.json` +
  `review/<hash>/binaries/<sha256>`, aparte de `state.json`/`objects/` del
  store de texto (que borra lo que no referencia: compartir `objects/`
  borraría los binarios). Restaurar al abrir el proyecto compara el hash de
  lo que hay en disco contra `current_hash`; si no coincide, se descarta con
  el mismo aviso que un archivo de texto que cambió por fuera, y nunca se
  escribe un archivo al restaurar. `RejectLog` lleva el orden de los
  rechazos (texto y binario mezclados) y `Alt+Shift+U` deshace el último,
  sea cual sea; `ReviewStore::undo_depth` (nuevo, `cincel-review`) le dice al
  workspace si el store apiló algo. Tests: `review_binaries_tests.rs`
  (persistencia y deshacer) y `crates/cincel-review/tests/undo_depth.rs`
  (`ReviewStore::undo_depth`).
- `review_snapshot.rs`: el repaso final del turno (`end_turn`, `agent_gone`,
  turnos encadenados) ya no bloquea la ventana. `PhotoState::Sweeping`
  lanza `ProjectSnapshot::changes()` en el ejecutor de fondo y el turno sigue
  activo hasta que vuelve; entonces se adoptan en el hilo principal, en
  tramos de a lo sumo `SWEEP_SLICE`, solo los archivos que difieren
  (`docs/rendimiento.md` §9.2 documenta el ajuste posterior de este valor,
  hecho por E6-G). Mientras tanto, los lotes del vigilante se siguen
  adoptando (`WatchQueue`, en tramos de `WATCH_SLICE` cuando el lote supera
  4 rutas) y lo que Cincel escribe se descuenta al volver el resultado. Un
  prompt nuevo espera el repaso igual que hoy espera la foto
  (`turn_waiter`, antes `photo_waiter`); pasado 1 s, la barra de estado
  muestra "Revisando los cambios del agente…". Test:
  `sweep_background_tests.rs`, `watch_slice_tests.rs`.

### E6-F — Deudas menores y tests que faltaban
- **Tope de memoria de la foto en Configuración** (`settings_view.rs`):
  fila "Memoria para la foto del proyecto (MB)" en la sección Revisión,
  clave `review.snapshot_max_total_mb`, control `number(16., 4096., 16., 0)`;
  validación de rango en `cincel-settings`. Tests sumados a
  `settings_view_tests.rs` (fila visible, escritura, búsqueda, restablecer),
  no en un archivo aparte: la spec proponía `settings_view_tests_e6.rs`,
  pero extender el archivo existente evitó duplicar el montaje de la vista
  que ya usan los demás tests de esta pantalla.
- **Teclas de la barra de búsqueda al keymap por defecto** (D14,
  `cincel-settings/src/defaults.rs`): secciones `Editor && searching`
  (`tab`/`shift-tab`) y, nueva, `Editor && searching && replacing`
  (`enter`/`ctrl-enter`), después de la sección `Editor`. Se borraron
  `cincel_editor::search_bar_bindings` y
  `cincel_workspace::keymap::search_bar_bindings`: la precedencia ya no se
  resuelve en código, sino por el orden del propio `keymap.json`. Test:
  `keymap_search_tests_e6.rs`.
- **Test del buscador con 50 000 rutas**:
  `crates/cincel-workspace/src/file_finder_perf_tests.rs`
  (`ranks_fifty_thousand`, `#[ignore]`; corre aparte, con la máquina
  tranquila, `-- --ignored`, porque bajo carga de compilaciones concurrentes
  el presupuesto de 50 ms falla).
- **Tests que faltaban**: `settings_update_e2e_tests.rs` (actualizar Node y
  cancelar una actualización de punta a punta, adaptador y runtime);
  `click_tests.rs` (clic real, con `gpui-kit/test-support` sumado a la
  feature `test-support` de `cincel-workspace`, D16, sobre menú, chat,
  archivos, atajos y conexión de la barra de estado; el botón `×` de la
  ventana no tiene clic real porque gpui-kit solo dibuja decoraciones de
  cliente y la ventana de test de GPUI siempre informa decoraciones de
  servidor — sigue cubierto por su función exacta,
  `close_from_title_bar`); `crates/cincel-connections/tests/
  slow_download.rs` (cancelar una descarga lenta real contra un servidor
  HTTP local que entrega 1 MB cada 100 ms, `#[ignore]`, fuera de toda
  verificación regular).
- §5.7 (llevar los colores de git a `EditorTheme`) no se hizo: quedó como
  limpieza opcional para después de la 1.0 (ver más abajo).

### E6-C — Primera medición completa
Corrida de `tools/perf/run.sh` con las tres apps sobre el binario de release
de ese momento; borrador de `docs/rendimiento.md` con las metas no
cumplidas para que E6-G las corrigiera. M10 se midió sobre el corpus mediano
(2 009 archivos), no sobre 50 000, porque `cincel --bench all` comparte una
única ruta entre los ocho escenarios; `run.sh cosmic` no se corrió;
columna de VS Code omitida (no instalado en la máquina de referencia).

### E6-G — Corrección de rendimiento
Detalle completo, con causa, cambio y tabla antes/después, en
`docs/rendimiento.md` §7–§9. Resumen:
- **M11** (repaso del turno): el bloqueo del hilo principal bajó de
  26–41 ms a 4–7 ms adoptando el vigilante en tramos (`WatchQueue`,
  `WATCH_SLICE`) y bajando `SWEEP_SLICE` de 4 a 3 ms; soltar la foto del
  proyecto grande se movió al ejecutor de fondo. Queda un residuo de
  ≈10 ms solo en el escenario combinado `all` (recargar la pestaña abierta
  cuesta más que la revisión en sí, que aporta 0,09 ms), anotado para
  después de esta etapa.
- **M8** (CPU en reposo): de ≈93 despertares por segundo a ≈2. Causa:
  `notify-debouncer-full` dormía en un bucle sin fin cada 25/25/75 ms para
  los tres vigilantes (proyecto, `.git`, configuración); reemplazado por el
  crate nuevo `cincel-watch` (`docs/specs/modulos/watch.md`). Además, la
  tarea de parpadeo del cursor de cada editor ahora termina a los 5 s sin
  escribir en vez de seguir despertando cada 500 ms.
- **M9** (tamaño del binario): no se pudo bajar de 74,9 MB a 20–60 MB sin
  perder velocidad o funciones (`strip = true` ahorraría 8,6 MB pero
  perdería nombres de función en un pánico, contra D11; `panic = "abort"`
  rompería el desenrollado que las tareas de tokio del ACP necesitan;
  `opt-level` más bajo cede rendimiento). **Decisión del autor
  (2026-09-29): la meta de M9 se ajusta a ≤ 80 MB** (era 20–60 MB,
  `01-producto.md §2`); el binario final (empaquetado, E6-H) pesa
  75,7 MB (79 410 024 bytes): 41,0 MB de código (`.text`), 20,7 MB de datos
  de solo lectura (de los cuales 13,0 MB son las tablas de las 18
  gramáticas de tree-sitter y ≈3,5 MB las fuentes embebidas), 8,6 MB de
  símbolos (D11), 4,5 MB de tablas de desenrollado de pánicos.

### E6-H — Empaquetado
Versión `1.0.0` en `[workspace.package]`, `publish = false` en los 13
crates (los 12 de la Etapa 5 más `cincel-perf`). `packaging/`: `install.sh`
(POSIX `sh`, sin `sudo`, `~/.local` por defecto, comprueba bibliotecas con
`ldconfig -p`, `--uninstall` no toca configuración ni conexiones),
`docker/Dockerfile.build` + `build.sh` (compilan en `ubuntu:22.04`, arman
tarball y `.deb` con `cargo-deb` en `dist/`), `verify.sh` (instala y
corre ambos paquetes en contenedores limpios `ubuntu:22.04` y `ubuntu:24.04`,
comprueba `ldd`, símbolos `GLIBC_`, tamaño), `tests/install_test.sh`. Icono
propio (`packaging/icons/`, SVG + PNG generados con `render.sh`),
`.desktop` con `app_id` `dev.cincel.Cincel` (D10), sin `MimeType`.
`CHANGELOG.md` (formato Keep a Changelog, resumen en español dentro de la
entrada `1.0.0`); `Cargo.lock` regenerado por el salto de versión (≈6
dependencias transitivas actualizadas). `about.toml`/`about.hbs` para
`cargo about generate` → `THIRD-PARTY-LICENSES.html`. El `maintainer` del
`.deb` queda genérico
(`Cincel contributors <cincel@users.noreply.github.com>`) hasta que el
autor ponga el suyo (`packaging/README.md` explica cómo, con
`CINCEL_MAINTAINER` o `packaging/maintainer.txt`, nunca commiteado).
`verify.sh` instala `fonts-dejavu-core` y las bibliotecas de D9 en los
contenedores pelados antes de correr el smoke test.

### E6-I — Repositorio público y CI
README (inglés + sección "En español"), `CONTRIBUTING.md`,
`CODE_OF_CONDUCT.md` (Contributor Covenant 2.1 por referencia),
`SECURITY.md`, plantillas de issue y de pull request,
`tools/privacy-check.sh` (D20). Tres workflows de GitHub Actions: `ci.yml`
(fmt, clippy `-D warnings`, tests con las features fijas, `cargo deny`,
`shellcheck`), `slow-tests.yml` (el test de descarga lenta, semanal y
manual), `release.yml` (arma los paquetes y deja un **borrador** de
release; lo publica el autor). `docs/publicacion.md`: guía paso a paso para
el autor. `docs/guia-integracion-acp.md` había quedado fuera del repositorio
por un error de una etapa anterior; el orquestador la recuperó y la puso al
día para esta etapa (enlazada desde el README, ver E6-L). §13 de la spec
decía que `install.sh` vivía en la raíz del paquete; en el repositorio de
Cincel el script fuente vive en `packaging/` (se copia a la raíz del
tarball al empaquetar, `packaging/build.sh`).

### E6-J — Documentación de usuario y capturas
`docs/usuario/` (siete archivos + índice): instalación, primer arranque,
conexiones, revisión de cambios, atajos, ajustes, solución de problemas,
todo en español y lenguaje llano. `docs/capturas/` (`revision.png`,
`chat.png`, `configuracion.png`, `buscador.png`), tomadas en el banco (D21)
con `grim` sobre un proyecto de demostración generado por
`cincel-perf corpus --demo`, sin nombres reales.
`crates/cincel-workspace/src/user_docs_tests.rs` ata cada atajo del keymap
por defecto y de los widgets a su descripción en `atajos.md`. Un segundo
subagente (Sonnet) siguió `instalacion.md`, `primer-arranque.md` y
`revision.md` al pie de la letra en el banco y corrigió cada paso que no
coincidía.

### E6-K — Medición final
Repite la medición completa (§3.6 de la spec) con los **paquetes** de E6-H
(el binario extraído del tarball, no `target/release/cincel`) y deja
`docs/rendimiento.md` en su forma definitiva: las once metas cumplidas
(M2 es informativa), con las dos salvedades de escala y de residuo ya
conocidas por E6-C y E6-G documentadas en su §6. Además corrió `sweep` con
la receta exacta de M11 (corpus grande, sin ninguna pestaña abierta), que
E6-C había dejado pendiente.

### E6-L — Integración (orquestador)
Verificación completa del workspace (ver más abajo), `cargo deny` con los
avisos RUSTSEC "unmaintained" transitivos ignorados con motivo (`deny.toml`,
ver "Verificación"), recuperación y actualización de
`docs/guia-integracion-acp.md` (había quedado fuera del árbol del
repositorio, ver E6-I), esta actualización de documentación
(`docs/etapas/etapa-6.md`, `docs/specs/modulos/*.md`,
`docs/specs/05-plan-etapas.md`, `docs/etapas/pendientes-etapa-5.md`,
`README.md`).

## Decisiones tomadas en la construcción

- **M9 ajustada a ≤ 80 MB** (autor, 2026-09-29). La meta original de
  `01-producto.md §2` era 20–60 MB. E6-G revisó todas las palancas
  razonables (`strip`, `panic = "abort"`, `opt-level` más bajo, quitar
  features de dependencias, deduplicar crates) sin poder bajar de 74,9 MB
  sin ceder velocidad o funciones del producto (regla fija de
  `docs/etapas/pendientes-etapa-5.md` §B: "no sacrificar rendimiento por
  disco"). El binario final, con todas las optimizaciones que no cuestan
  velocidad ya aplicadas, pesa **75,7 MB**: 41 MB de código y 13 MB de
  tablas de gramáticas de sintaxis (tree-sitter, 18 lenguajes). El autor
  aceptó ajustar la meta en vez de perder rendimiento o funciones.
- **`cincel-watch` reemplaza a `notify-debouncer-full`** en los tres
  vigilantes (proyecto, `.git`, configuración): la biblioteca de terceros
  despertaba un hilo por vigilante cada 25–75 ms para siempre, aun sin
  ningún evento, lo que representaba ≈93 despertares por segundo y tres
  cuartos de la CPU en reposo de Cincel. El crate propio bloquea el hilo de
  lotes en su canal mientras no pasa nada (detalle en
  `docs/specs/modulos/watch.md`).
- **Repaso final del turno en el ejecutor de fondo**, con el turno
  permaneciendo activo hasta que el resultado vuelve: mismo patrón que ya
  usaba la foto (`turn_waiter`, antes `photo_waiter`), para no introducir
  una segunda forma de esperar algo asincrónico en el ciclo de revisión.
- **Deshacer unificado de texto y binarios** por orden real de rechazo
  (`RejectLog`), en vez de dos pilas independientes: es lo que pedía la
  deuda (`Alt+Shift+U` tiene que deshacer "lo último", sin importar el
  tipo), y evita que el usuario tenga que recordar qué pila usar.
- **Las teclas de la barra de búsqueda se resuelven por el orden del
  archivo del keymap por defecto**, no en código (D14): consecuencia
  aceptada y documentada, si el usuario reasigna `tab`/`enter`/`ctrl-enter`
  en su propia sección `Editor` (sin `searching`), esa gana también con la
  barra de búsqueda abierta — igual que en Zed, y es justamente lo que la
  deuda original pedía ("nada resuelto a mano en código").

## Desviaciones

Todas las anota el subagente responsable, con su motivo; ninguna cambia el
comportamiento visible descrito en la spec 08 más allá de lo aquí explicado.

- **E6-A**: `docs/rendimiento.md` (la publicación de los resultados) la
  escribieron E6-C y E6-K, no E6-A (que solo construyó el instrumento). Una
  corrida con `--bench` manda `state`, `data` y `cache` a una carpeta
  temporal propia para no tocar la configuración, los proyectos recientes
  ni las conexiones reales del autor. En la sesión real de COSMIC (no en el
  banco), el escenario `finder` dio ≈1 s por tecla tras `idle`: el
  compositor frena la ventana tapada por otras ventanas del escritorio; se
  mide sin ese efecto en el banco, que no tiene pantalla.
- **E6-B**: `~/.local/bin/antigravity` es la app de gestión de agentes
  ("Antigravity 2.0"), no el editor: el banco usa `antigravity-ide` (el IDE
  derivado de VS Code) y rechaza la otra con un aviso si se confunden.
  "Primer cuadro con contenido" exige además ≥ 16 colores distintos (no
  solo el 2 % de píxeles distintos del fondo), para no contar como
  contenido un rectángulo liso que Electron pinta mientras acomoda la
  ventana. Ajustes por app: Zed necesita `session.trust_all_worktrees: true`
  (si no, abre el diálogo de "modo restringido" y se come las teclas);
  Antigravity IDE necesita una clave de bienvenida sembrada a mano
  (`antigravityOnboarding` en una base SQLite mínima) y el cierre de su
  panel de agente (sin red, muestra un indicador animado sin fin); Electron
  corre con `--no-sandbox` (su sandbox necesita un ayudante *setuid* que un
  usuario sin privilegios dentro del contenedor no tiene); sway arranca con
  `--unsupported-gpu` (el módulo NVIDIA del kernel del anfitrión lo hace
  negarse a arrancar aunque el contenedor no use esa GPU). COSMIC sí ofrece
  a un cliente común captura (`ext_image_copy_capture_manager_v1`) y
  teclado virtual (`zwp_virtual_keyboard_manager_v1`) — al revés de lo que
  suponía D1 —, comprobado con `cincel-perf probe`; queda anotado, sin
  usarse (mantener el escritorio invisible aparte sigue siendo la regla:
  nunca capturar la sesión real). `shellcheck` y `actionlint` se corrieron
  en un contenedor porque no están instalados en el host. `Cargo.lock`
  cambió por la dependencia nueva `wayland-protocols-misc`.
- **E6-C**: M10 y M11 se midieron sobre el corpus mediano (2 009 archivos),
  no el grande (20 000), porque `--bench all` comparte una sola `RUTA`
  entre los ocho escenarios y esa ruta tiene que contener los tres archivos
  de `open`/`typing`/`scroll`. `run.sh cosmic` no se corrió en esta
  subetapa. VS Code no estaba instalado en la máquina de referencia: esa
  columna se omitió.
- **E6-D**: los hashes SHA-256 de `SOURCES.md` se calcularon al descargar,
  porque ni Inter ni JetBrains Mono publican un hash por archivo en su
  release oficial. `TestAppContext` usa `NoopTextSystem`, así que
  `fonts_tests.rs` solo verifica cabeceras TrueType y que el registro no
  falle; la resolución real de las familias (`all_font_names`, `font_id`)
  la prueba `--smoke-test`.
- **E6-E**: la barra de estado muestra "Revisando…" recién tras 1 s sin que
  el repaso termine, con el ejecutor determinista de los tests (la
  comprobación automática fuerza el fondo con `set_photo_in_background`).
  Nombres de archivos de test tal como los pide la spec. Toques mínimos en
  `workspace.rs` y `agents.rs` (los imprescindibles para conectar
  `PhotoState::Sweeping` con el resto del ciclo de vida del turno).
- **E6-F**: §5.7 (colores de git de `GitGutterColors` a `EditorTheme`,
  deuda de la Etapa 5) no se hizo: es opcional y queda para después de la
  1.0. Los botones "atajos" y "pendientes" de la barra de estado no tienen
  `debug_selector` propio: el clic real ya está cubierto en menú, chat,
  archivos y conexión, y esos dos llaman a la misma función que ya prueba
  su atajo de teclado (F1 y "abrir panel de revisión"). El test de 50 000
  rutas se marcó `#[ignore]` y se corre aparte, con la máquina tranquila
  (falla bajo carga de compilaciones concurrentes en paralelo, no por un
  problema del buscador). Un subagente corrió `cargo sweep` en medio de la
  ola de construcción: la regla de la spec (§10.1.5) dice que esa limpieza
  la hace solo el orquestador, entre subetapas; quedó anotado para que no
  se repita.
- **E6-G**: en el escenario combinado `all`, el tramo `sweep` de M11 deja
  un residuo de ≈10 ms por la recarga de la pestaña abierta (la revisión en
  sí aporta 0,09 ms de ese tiempo); con la receta exacta de la spec
  (`sweep` solo, sin pestañas abiertas) los tres tramos cumplen con margen
  (confirmado por E6-K). Adoptar el cambio del agente sobre un archivo de
  1 MB bloquea 31 ms al hilo principal (57–74 ms sobre uno de 50 000
  líneas): no es parte de M11 (es un solo archivo, no se puede partir en
  tramos), quedó anotado para después. `[profile.release]` no cambió:
  `lto = "fat"` y `codegen-units = 1` ya estaban. `panic = "abort"` se
  descartó porque las tareas de tokio que hablan con los agentes (ACP)
  dependen del desenrollado para aislar un pánico sin cerrar todo el
  editor. `strip = true` se descartó por contradecir D11 (perdería los
  nombres de función en una traza de pánico). `target/debug/incremental`
  se borró en un momento de disco lleno, práctica ya prevista en las reglas
  de construcción. `notify-debouncer-full` se reemplazó por `cincel-watch`
  y su entrada se retiró del `Cargo.toml` raíz por el orquestador (para no
  dejar una dependencia sin usar en un archivo que varias subetapas
  compartían).
- **E6-H**: el binario final pesa 75,7 MB, dentro de la meta ajustada de
  M9. `CHANGELOG.md` lleva un resumen en español dentro del formato
  estándar de Keep a Changelog (que es en inglés). `Cargo.lock` se
  regeneró por el salto a la versión 1.0.0 (unas 6 dependencias
  transitivas se actualizaron solas). `verify.sh` instala
  `fonts-dejavu-core` y las bibliotecas de D9 en los contenedores pelados,
  porque una imagen `ubuntu:22.04`/`24.04` limpia no las trae. El
  `Maintainer` del `.deb` queda genérico
  (`Cincel contributors <cincel@users.noreply.github.com>`) hasta que el
  autor decida poner el suyo.
- **E6-I**: `docs/guia-integracion-acp.md` estaba fuera del árbol del
  repositorio por un error de una etapa anterior; el orquestador la
  recuperó y la puso al día para esta etapa. §13 de la spec ubicaba
  `install.sh` en la raíz del repositorio; el script fuente vive en
  `packaging/` (se copia a la raíz del tarball al empaquetar). El disco
  llegó al 100 % por la caché de compilación de Docker durante esta
  subetapa; se liberó con `docker builder prune`. La historia de git de
  este repositorio lleva el correo real del autor en sus commits, por
  decisión suya (documentado en `docs/publicacion.md` §1–§2, que le da la
  opción de publicar una historia nueva de un solo commit si prefiere no
  exponerlo).
- **E6-J**: la captura `chat.png` muestra el panel de chat vacío (sin
  conexión con ningún agente, ni siquiera el falso), porque conectar un
  agente real expondría una cuenta del autor. Las capturas se tomaron con
  `grim` y un script ad hoc que no forma parte del repositorio. Dos
  funciones de `shortcuts_modal.rs` pasaron de privadas a `pub(crate)` para
  que el test del manual de usuario pudiera comparar contra
  `atajos.md`.
- **E6-K**: no se amplió la medición de `finder` al corpus grande (20 000
  archivos) ni se corrió `run.sh cosmic`: no estaban en el alcance
  específico de esta subetapa (repetir la medición con los paquetes y
  cerrar el pendiente puntual de M11 con la receta exacta). Por indicación
  explícita para esta subetapa no se corrió `cargo sweep` (regla general
  de §10.1.5 de la spec), para no gastar tiempo en un paso de limpieza que
  no aporta a la medición ni a la verificación de paquetes.
- **Orquestador (E6-L)**: `cargo deny check` completo exigió ignorar 5
  avisos RUSTSEC de mantenimiento ("unmaintained"), todos transitivos de
  `gpui`/`gpui-kit` o de `ureq`, sin reemplazo disponible todavía; cada uno
  tiene su motivo en `deny.toml` (`instant`, `paste`, `rustls-pemfile`,
  `rustybuzz`, `ttf-parser`). La guía `docs/guia-integracion-acp.md` se
  recuperó de fuera del árbol del repositorio (ver E6-I) y se puso al día
  para esta etapa.

## Verificación

Máquina de referencia (Pop!_OS 24.04, COSMIC Wayland), 2026-09-29, siempre
con `--features cincel-editor/test-support,cincel-workspace/test-support,
cincel-chat/test-support` en los crates con GPUI:

| Comando | Resultado |
|---|---|
| `cargo fmt --all --check` | ok |
| `cargo clippy --workspace --all-targets --features …` | ok, 0 avisos |
| `cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins` + `cargo test --workspace --features …` | ok, 1292+ tests, 0 fallos |
| `cargo deny check licenses advisories bans sources` | ok completo, con 5 avisos RUSTSEC "unmaintained" transitivos ignorados con motivo en `deny.toml` (ver "Desviaciones · Orquestador") |
| `cargo run -p cincel -- --smoke-test .` | ok |
| `cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .` (control de fugas) | ok |
| `cargo build --release -p cincel && target/release/cincel --smoke-test --bench .` | ok |
| `cargo test -p cincel-connections --test slow_download -- --ignored` | ok |
| `cargo test … ranks_fifty_thousand -- --ignored` (máquina tranquila) | ok |
| `packaging/build.sh && packaging/verify.sh` | ok: tarball 25,4 MB, `.deb` 17,0 MB, binario 75,7 MB; `verify.sh` completo en `ubuntu:22.04` y `ubuntu:24.04` |
| `tools/perf/run.sh` (medición final, E6-K) | ok: 11 metas de rendimiento cumplidas, tabla completa en `docs/rendimiento.md` |
| `tools/privacy-check.sh` | ok (0) sobre el árbol de trabajo y sobre el contenido de los dos paquetes |
| `shellcheck install.sh packaging/*.sh packaging/**/*.sh tools/*.sh tools/**/*.sh` | ok |
| `pgrep -af "sleep\|cincel\|zed\|antigravity\|sway"; docker ps` | vacíos |

Medición de rendimiento final: **11 metas cumplidas** (M2 es informativa),
con las dos salvedades ya conocidas (M10 medido a menor escala; residuo de
M11 solo en el escenario combinado `all`) documentadas con causa en
`docs/rendimiento.md` §6. Tabla completa, metodología y cómo repetirla:
`docs/rendimiento.md`.

## Lista de comprobación manual (autor)

Tal como la deja `docs/specs/08-etapa6-cierre-1-0.md` §12, para recorrerla
antes de dar por publicada la versión 1.0 (usando los paquetes que dejó
E6-H en `dist/`, o el borrador del release si ya se publicó):

1. **Tarball**: descomprimir, `./install.sh`. Buscar "Cincel" en el
   lanzador de COSMIC: aparece con su icono. Abrirlo: la ventana y el panel
   muestran el mismo icono. `cincel --version` dice `cincel 1.0.0`.
2. Abrir tu proyecto real: el árbol, las pestañas y el editor se ven con
   Inter y JetBrains Mono (en el chat, una respuesta con **negrita** se
   nota más gruesa).
3. Etapa 1: abrir, editar con acentos, buscar y reemplazar, guardar,
   deshacer; cerrar y reabrir con las mismas pestañas.
4. Etapas 2 y 4: conectar Claude (o usar la conexión que ya tenés),
   pedirle que explique un archivo y que corra un comando (tarjeta de
   permiso); repetir con Codex y Antigravity si los usás.
5. Etapa 3 y deudas: pedirle a Claude un cambio en 3 archivos que incluya
   **una imagen o un archivo binario**; rechazar un segmento, una línea y
   el binario; `Alt+Shift+U` dos veces (vuelven en orden inverso). Cerrar
   Cincel con cambios sin decidir, reabrir: siguen ahí, también el
   binario.
6. Etapa 5: `Ctrl+P`, `Ctrl+,` (en Revisión aparece "Memoria para la foto
   del proyecto"), `F1` (las teclas de la búsqueda figuran), `Ctrl+L`, git
   en el margen, el menú y la `×` con un archivo sin guardar.
7. `./install.sh --uninstall`: Cincel desaparece del lanzador y tu
   configuración sigue en `~/.config/cincel`.
8. **`.deb`**: doble clic (o `sudo apt install ./cincel_1.0.0-1_amd64.deb`),
   abrir Cincel desde el lanzador y repetir 2 y 5 rápido. Desinstalar con
   `sudo apt remove cincel`.
9. Leer `docs/rendimiento.md` y `docs/usuario/README.md`; si algo no se
   entiende, avisar.
10. Seguir `docs/publicacion.md` cuando quieras publicar.

## Correcciones tras la prueba de la 1.0

Tres cambios en la revisión que el autor pidió después de probar la 1.0.

1. **Los archivos binarios quedan fuera de la revisión.**
   - *Causa:* E6-E (y antes la v2 de la revisión) metía en revisión todo
     archivo no UTF-8 que el agente tocara; en un proyecto Python, cada
     ejecución del agente llenaba la revisión de `.pyc` de `__pycache__`
     "cambiados por el agente" que nadie quiere revisar. Decisión del autor:
     la revisión es de todos los archivos de texto y solo se excluyen los
     binarios.
   - *Cambio:* binario = no UTF-8 o un NUL en los primeros 8 KB
     (`cincel_project::looks_binary`, el criterio del store). La foto guarda
     de ellos solo tamaño y hash (`SnapshotContent::Binary` ya no lleva los
     bytes ni gasta el tope de memoria); `Review::adopt_change` deja pasar
     cualquier cambio que tenga un binario de un lado u otro (crear, cambiar,
     borrar, o convertir un texto en binario): se aplica sin preguntar y no
     aparece en el árbol, en "Revisar todo", en la barra de estado ni en el
     diálogo de cierre. Se retiraron `review_binaries.rs` (persistencia en
     `binaries.json` + `binaries/<sha256>`, `RejectLog`, deshacer de
     binarios), la etiqueta "archivo binario cambiado por el agente" y
     `review_binaries_tests.rs`; al abrir un proyecto se borran el
     `binaries.json` y la carpeta `binaries/` que haya dejado una versión
     anterior. `Alt+Shift+U` vuelve a la pila del store; `ReviewStore::
     undo_depth` se queda (lo usa `tests/undo_depth.rs`). Un binario abierto
     desde el árbol muestra "Archivo binario: no se muestra" (solo lectura,
     sin botones) en vez de los bytes como texto ilegible.
   - *Tests:* `review_binary_exclusion_tests.rs` (el agente falso, por
     shell durante el turno, reescribe un `.pyc` de `__pycache__`, crea una
     imagen y cambia `a.txt`: solo `a.txt` queda pendiente, el árbol no
     marca los binarios, disco y `git status` muestran el cambio; rechazar
     el turno no toca el binario), `snapshot_review_tests.rs`
     (`binary_files_changed_by_the_agent_stay_out_of_the_review`: cambiado,
     creado y borrado, más la pestaña con el aviso) y `cincel-project`
     (`binaries_keep_only_size_and_hash`, `binary_sniff`).
2. **Sin "Aceptar archivo" / "Rechazar archivo" en la barra flotante.**
   - *Causa:* el arreglo de archivos borrados (Etapa 5) los agregó
     (`ReviewView::file_actions`) sin que el autor lo pidiera.
   - *Cambio:* se quitó `file_actions`; la barra vuelve a ser "Aceptar
     todo" · "Rechazar todo" del turno · flechas · "cambio N de M" ·
     "Revisar todo" en cualquier archivo. Un archivo borrado se decide con
     los botones de su único segmento (el host traduce el segmento
     `DELETED_FILE_HUNK` a aceptar o rechazar el archivo) o con la ✓ y la ✗
     de su fila en "Revisar todo".
   - *Tests:* `cincel-editor/src/review_tests.rs`
     (`the_bar_of_a_deleted_file_has_no_file_buttons`: barra sin botones de
     archivo, la píldora del segmento aparece con el mouse y su clic
     decide) y `deleted_file_review_tests.rs` (la pestaña de solo lectura:
     la barra no los tiene; con el mouse sobre las filas rojas aparecen los
     botones del segmento y su "✗ Rechazar", pulsado con el mouse, restaura
     el archivo).
3. **Los botones de un segmento ya no se esconden al acercarse desde
   arriba.**
   - *Causa:* los botones se muestran mientras el mouse está sobre las filas
     del segmento, pero se dibujan en su esquina superior derecha y salen de
     él (en la fila anterior cuando la primera fila no tiene lugar, o por su
     borde superior: miden 24 px). Al subir el mouse hasta ellos, la fila
     bajo el mouse dejaba de ser del segmento y desaparecían antes del clic.
   - *Cambio:* la zona que los mantiene visibles es las filas del segmento
     más el rectángulo de los propios botones (`EditorView::
     hover_pill_zone`, que el manejador de movimiento del mouse llena con
     los rectángulos pintados en el cuadro anterior). El texto no se mueve.
   - *Test:* `cincel-editor/src/review_tests.rs`
     (`the_pill_stays_while_the_mouse_reaches_it_from_above`: el mouse
     entra al segmento desde una fila anterior, sube hasta el "Aceptar" de
     la píldora, que está en la fila de arriba, y hasta su borde superior;
     la píldora sigue y el clic emite `AcceptHunk`).

Desviación: un archivo de *texto* que la foto no copió (por encima de
`review.max_file_size_kb` o del tope total) sigue en revisión fuera del
store, entero y solo aceptar, como antes de E6-E; al retirar
`binaries.json` ese caso deja de persistir entre reinicios (vive en
memoria). Es un caso raro y persistirlo pediría otra vez un archivo aparte
del store.

## Después de la 1.0

Lo que esta etapa dejó anotado para más adelante, sin fecha asignada:

- **Llevar `git.added`/`git.modified`/`git.deleted` de `GitGutterColors` a
  `EditorTheme`** junto con el resto de los tokens de tema del editor
  (deuda de la Etapa 5, §5.7 de esta spec no la tomó por no bloquear el
  cierre).
- **El residuo de ≈10 ms del escenario combinado `all`** en M11: recargar
  una pestaña abierta durante el repaso final del turno cuesta más que la
  revisión en sí (que aporta 0,09 ms); con la receta exacta de M11 (sin
  pestañas abiertas) no hay problema, pero bajarlo pediría sacar del hilo
  principal la recarga de los archivos abiertos o abaratar el primer
  cuadro después de una recarga (`cincel-project`/`cincel-editor`).
- **Adoptar el cambio del agente sobre un solo archivo grande** (1 MB o
  50 000 líneas) bloquea 31–74 ms al hilo principal, porque es un solo
  archivo y no se puede partir en tramos como el resto del repaso; M5 y M6
  igual cumplen, pero calcular ese diff en el ejecutor de fondo evitaría el
  bloqueo.
- **Tamaño del binario**: si en el futuro se necesita bajarlo más allá de
  los 75,7 MB actuales, las palancas ya revisadas y descartadas por esta
  etapa (`strip`, `panic = "abort"`, quitar gramáticas o formatos de
  imagen) siguen documentadas en `docs/rendimiento.md` §9.4 como punto de
  partida.
- **`tools/perf/run.sh cosmic`** (comprobación cruzada del arranque en
  COSMIC, D2) y **medir `finder` sobre el corpus grande** (50 000 rutas
  reales, no solo el test unitario `ranks_fifty_thousand`) quedaron sin
  ampliar en E6-C y E6-K por alcance; siguen siendo la forma más completa
  de cerrar del todo M10 y la comprobación cruzada de M1.
- Todo lo que ya estaba anotado como "Después de v1" en
  `docs/specs/05-plan-etapas.md` y no era parte del alcance acordado para
  esta etapa (LSP, terminal integrado, autenticación de agentes dentro de
  la app, comentarios sobre segmentos, multi-cursor y plegado, búsqueda en
  proyecto, paleta de comandos, OpenCode y agentes con API key o gateway,
  panel de git, ACP v2, macOS) sigue abierto tal cual estaba.

### Además, en la misma ronda (2026-09-29)

4. **El logo de Cincel** (`packaging/icons/dev.cincel.Cincel.svg`, copiado a
   `crates/cincel-workspace/assets/logo/`) reemplaza al icono genérico de
   "destello" de gpui-kit en la pantalla de inicio (48 px) y aparece a 16 px
   en la barra de título (`logo.rs`, `logo_tests.rs`).
5. **Avisos del inicio de sesión.** Hallazgo: el programa de Antigravity
   espera la vuelta del navegador 5 minutos (`_LOGIN_TIMEOUT_SECONDS = 300`
   en `oauth/credential_manager.py`) y después responde "Onboarding failed:
   Timed out waiting for the authentication flow to complete"; el autor
   había dejado el enlace 10–20 minutos sin abrir. Codex no tiene límite
   propio (`codex-rs/login/src/server.rs`) y Claude no tiene uno conocido; en
   los tres rige el límite de Cincel de 15 minutos. Ahora junto al enlace se
   indica cuánto tiempo hay ("5 minutos" / "15 minutos"), el error de
   vencimiento de Antigravity se explica en español con "Reintentar", el
   enlace sigue a la vista aunque llegue un error, y `login.rs` registra cada
   paso del login sin enlaces, códigos ni identidad
   (`login_event_log_line`, `login_modal_tests.rs`).
6. **`--smoke-test` en carpeta temporal** (`bench::isolate_state_in_temp`):
   el autochequeo abría el repo como proyecto y lo dejaba en los recientes
   del autor, por lo que el Cincel instalado arrancaba con el repo.
7. **Test `status_does_not_rewrite_the_index`** hecho tolerante al primer
   refresco legítimo del índice ("racily clean"): lo que vigila es que el
   estado estable no lo reescriba.
8. **Paquetes en `dist/`** (raíz del repo, ignorado por git) en vez de
   `target/dist`, con el tarball ya extraído al lado para que el instalador
   esté a la vista.

## Publicación (2026-09-29 y 30): lo que enseñó el primer release real

El repositorio público es `github.com/jotagary25/Cincel`; la primera versión
publicada es la **0.1.0** (el autor prefirió no arrancar en 1.0.0; el
número vive solo en `Cargo.toml`, el resto lo lee de ahí). Lo que falló en
GitHub y no había fallado acá, con su causa:

1. **Tests que asumían herramientas del entorno** (`cincel-acp::registry`):
   tres tests daban por hecho `uvx` y `node` en `PATH`; la máquina de GitHub
   no trae `uvx`. Ahora comprueban el comando si la herramienta está y el
   error `UvMissing`/`NodeMissing` si no.
2. **Presupuesto de tiempo sin factor** (`cincel-syntax`): el test de 5 000
   líneas no leía `CINCEL_PERF_BUDGET_FACTOR` (el CI lo fija en 4) y falló
   por 0,8 ms en la máquina compartida.
3. **Test que no esperaba a un hilo real** (`settings_update_e2e_tests`): la
   comprobación de actualizaciones corre en un hilo del sistema; el test
   miraba el resultado tras un solo `run_until_parked`. Ahora espera hasta
   10 s.
4. **El workflow de release estaba copiado a mano** de `packaging/build.sh`
   y nunca se había ejecutado: tres rutas mal (archivo de licencias de
   terceros, carpeta del tarball, ruta relativa de `verify.sh` que Docker
   rechaza). Se corrigieron y el job entero se auditó ejecutando cada bloque
   `run:` en el mismo orden, primero en la máquina de referencia y después en
   un clon limpio dentro de la imagen Ubuntu 22.04 del empaquetado.
   Regla nueva (`asteroid-work-rules`): nada que "solo corre en GitHub" se
   entrega sin ejecutarlo localmente paso por paso.

Procedimiento que quedó validado: push a `main` → CI en verde → etiqueta
`vX.Y.Z` → Release (build, verify, borrador) → el autor instala el `.deb` del
borrador y publica. Para mover una etiqueta tras un arreglo:
`git push origin --delete vX.Y.Z && git tag -d vX.Y.Z && git tag -a vX.Y.Z -m … && git push origin vX.Y.Z`.
