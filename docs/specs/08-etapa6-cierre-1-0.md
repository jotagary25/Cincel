# Spec 08: Etapa 6, cierre de la versión 1.0 (rendimiento, instalador, publicación, documentación, fuentes y deudas)

Estado: v1.0 (2026-09-29), definida con el autor. Fuente de verdad del alcance de la Etapa 6 (`05-plan-etapas.md`).

Modelo de formato: `07-etapa5-productividad.md`. Cada ítem tiene primero un **resumen en lenguaje llano** (para el autor) y después el **diseño técnico** (para los subagentes). Las palabras técnicas que aparecen en los resúmenes se explican la primera vez.

Esta spec corrige otras en estos puntos (el orquestador los aplica al cerrar, E6-L; no se tocan antes):
- `01-producto.md §5`: cada meta pasa a tener una forma de medirla (§3.1 de esta spec) y un enlace a `docs/rendimiento.md` con los resultados.
- `02-visual.md §3`: Inter y JetBrains Mono vienen dentro del programa; los respaldos (`system-ui`, `Zed Mono`, `DejaVu Sans Mono`…) solo se usan si el usuario elige otra familia que no está instalada.
- `03-arquitectura.md §4`: el repaso final del turno corre en segundo plano (§5.3); `§6`: la revisión persiste también los cambios de archivos binarios (§5.2); `§8`: la calidad se verifica también en GitHub Actions (§7).
- `modulos/review.md`, `modulos/workspace.md`, `modulos/settings.md`, `modulos/editor.md`, `modulos/connections.md`: lo que cambia en cada deuda (§5).

---

## 1. Qué trae esta etapa (resumen en lenguaje llano)

1. **Rendimiento medido y comparado.** Se mide cuánto tarda Cincel en abrir, en abrir un archivo, en mostrar una letra al escribir, cómo se comporta con archivos enormes, cuánta memoria usa y si gasta procesador cuando no hacés nada. Lo mismo se mide, con el mismo método, en **Zed** y en **Antigravity** (que es un VS Code modificado), para tener con qué comparar. El método queda escrito para sumar VS Code cuando esté instalado. Los resultados se publican en `docs/rendimiento.md` en una tabla con las metas del producto; lo que no cumpla se corrige en esta etapa. Todo lo mide el orquestador con subagentes, sin que tengas que hacer nada, y **sin grabar tu pantalla**: las mediciones visuales se hacen en un escritorio invisible, aparte, que solo contiene la aplicación medida.
2. **Instalador liviano.** Dos formas de instalar: un archivo comprimido con un `install.sh` que copia Cincel en tu carpeta personal (sin pedir contraseña) y agrega el icono al menú de aplicaciones; y un paquete `.deb` para instalar con doble clic o con `apt`. Se compilan en los servidores de GitHub sobre Ubuntu 22.04, así funcionan en Ubuntu y Pop!_OS 22.04 o más nuevos. Los agentes (Claude, Codex, Antigravity) no vienen adentro: se siguen descargando la primera vez que conectás cada uno, como hoy. Incluye un icono propio: un cincel estilizado.
3. **Repositorio público.** Lo creás vos en GitHub siguiendo una lista paso a paso (`docs/publicacion.md`). Todo lo demás queda listo antes: README en inglés con una sección en español y capturas, guía para colaborar, plantillas para reportar errores, y los controles automáticos de GitHub (formato, avisos del compilador, tests, licencias) más el armado automático del instalador al publicar una versión. Antes de publicar se revisa que no quede ningún dato personal (tu usuario, tus rutas, tu correo) en ningún archivo.
4. **Manual de uso en español** (`docs/usuario/`): instalar, primer arranque, conectar agentes, revisar cambios, atajos, ajustes y qué hacer cuando algo falla.
5. **Fuentes dentro del programa.** Inter (la de la interfaz) y JetBrains Mono (la del código) van adentro del ejecutable. Hoy, si no las tenés instaladas, Cincel usa otras y se ve distinto (por ejemplo, la negrita del chat no cambia de grosor). Desde esta etapa se ve igual en cualquier equipo.
6. **Seis deudas pendientes**, cada una con lo que te pasa hoy si no se arregla:
   - **Tope de memoria de la foto en la configuración.** *Hoy:* antes de cada mensaje al agente, Cincel se guarda una copia de tus archivos (la "foto") para poder mostrarte y deshacer lo que cambie; el máximo de memoria de esa copia solo se cambia escribiendo en `settings.json`, y nadie sabe que existe.
   - **Cambios del agente en archivos binarios** (imágenes, PDF, cualquier archivo que no es texto). *Hoy:* si cerrás Cincel sin decidir, al volver ese cambio ya no aparece para revisar; y si lo rechazás por error, `Alt+Shift+U` (deshacer el último rechazo) no lo recupera.
   - **Repaso final del turno sin congelar la ventana.** *Hoy:* cuando el agente termina, Cincel compara toda la foto con el disco; en un proyecto grande eso puede dejar la ventana sin responder un instante.
   - **Teclas de la barra de búsqueda en el mapa de teclas.** *Hoy:* `Tab`, `Enter` y `Ctrl+Enter` dentro de la búsqueda funcionan, pero no figuran en el mapa de teclas por defecto (`keymap.json`), así que no se ven ni se pueden copiar desde ahí para cambiarlas.
   - **Prueba automática de velocidad del buscador de archivos.** *Hoy:* nadie avisaría si un cambio futuro vuelve lento `Ctrl+P` en proyectos grandes.
   - **Pruebas que faltaban.** *Hoy:* algunas funciones (actualizar Node desde la configuración, cancelar una actualización, los clics en los botones de las barras, cancelar una descarga lenta de verdad) se probaron por partes; un error en la unión entre las partes pasaría sin que nadie lo note.

A vos solo te quedan dos cosas: crear el repositorio siguiendo `docs/publicacion.md` y, al final, la lista de comprobación manual (§12): instalar desde el comprimido y desde el `.deb` en tu máquina y repetir lo esencial de las etapas 1 a 5.

---

## 2. Decisiones técnicas de esta spec

| # | Decisión | Motivo |
|---|---|---|
| D1 | **Banco de medición**: contenedor Docker (`ubuntu:24.04`) con `sway` sin pantalla (*headless*) a 240 Hz, y una herramienta propia, `cincel-perf`, que habla Wayland directamente: lanza la app, detecta su ventana (`wlr-foreign-toplevel-management`), captura cuadros del escritorio invisible con marca de tiempo (`wlr-screencopy`, `copy_with_damage`) e inyecta teclas y rueda del mouse (`zwp_virtual_keyboard_v1`, `zwlr_virtual_pointer_v1`). | Mide las tres apps sin instrumentarlas y con la misma vara. Docker ya está en la máquina y el usuario pertenece al grupo `docker` (no hace falta `sudo`); `sway`, `wf-recorder` y `wtype` no están instalados en el sistema y no se instalan: van dentro de la imagen. El escritorio invisible solo contiene la app medida, así que ninguna captura toca la sesión del autor (regla "nada de capturas de pantalla completa"). COSMIC no ofrece a programas comunes, hasta donde se sabe, captura de pantalla sin diálogo ni inyección de teclas; E6-B lo confirma y lo anota. |
| D2 | **Comprobación cruzada en COSMIC**: `cosmic-comp` anidado (una ventana en el escritorio del autor durante la medición) solo para "arranque hasta ventana mapeada" con `ext-foreign-toplevel-list-v1`, si ese compositor lo expone a clientes comunes. Si no, se documenta y se omite. RSS y CPU se miden igual en el banco y, como control, en la sesión real por `/proc`, sin capturas. | Que los números del banco no sean un artefacto de `sway`. Nunca se captura la pantalla real. |
| D3 | **Memoria**: se publican **RSS** del proceso principal y **PSS sumado** de todo el árbol de procesos (`/proc/<pid>/smaps_rollup`). La meta de Cincel (< 300 MB) se juzga con **RSS sumado del árbol** (lo más conservador); la comparación entre apps, con PSS. | Zed y sobre todo Antigravity son multiproceso; sumar RSS cuenta varias veces las bibliotecas compartidas, PSS las reparte. Para Cincel (un proceso) RSS y PSS casi coinciden. |
| D4 | **Arranque "en frío" sin root**: antes de cada corrida, `cincel-perf evict` desaloja de la caché de páginas del sistema los archivos de la app (ejecutable, bibliotecas propias, recursos) con `posix_fadvise(POSIX_FADV_DONTNEED)`. Se publican frío y tibio (segunda ejecución), mediana y p90 de 10 corridas. | `drop_caches` necesita root. Desalojar los archivos propios de cada app es lo que un usuario ve al abrirla por primera vez en el día; las bibliotecas del sistema (Mesa, Vulkan) se dejan calientes para todas por igual. |
| D5 | **Apps en perfiles aislados y comparables**: carpeta de datos propia por corrida, sin extensiones, sin servidores de lenguaje, sin telemetría ni actualización automática, cursor sin parpadeo, Wayland nativo (Electron con `--ozone-platform=wayland`). | Que Zed no arranque `rust-analyzer` y Antigravity no cargue extensiones: se mide el editor, no lo que cada uno instala. No se toca ningún perfil ni dato del autor. El parpadeo del cursor ensuciaría la detección de cambios en pantalla. |
| D6 | **Mediciones internas de Cincel** con `cincel --bench <escenario>` en el **binario de release** (no hay binario aparte), con salida JSON por línea; `--smoke-test --bench` imprime además los tiempos de arranque. | Separa cuánto tarda Cincel en sí de lo que agrega el compositor, y localiza qué corregir. Sin la opción, los ganchos no cuestan nada (una bandera leída una vez). |
| D7 | **Fuentes**: TTF **estáticas** (no variables) de Inter 4.1 (Regular, Italic, Medium, SemiBold, Bold, BoldItalic) y JetBrains Mono 2.304 (Regular, Italic, Bold, BoldItalic), **sin recortar** (sin *subsetting*), en `crates/cincel-workspace/assets/fonts/`, registradas con `cx.text_system().add_fonts` desde `main.rs` antes de abrir la ventana, **no** desde `cincel_workspace::init`. | Estáticas: cada peso que usa la interfaz (400, 500, 600, 700 e itálicas) existe tal cual, sin depender de cómo resuelve pesos variables `cosmic-text`. Sin recortar: la licencia OFL trata el recorte como modificación. Fuera de `init`: los más de 1 100 tests existentes miden texto con las fuentes del sistema y no deben cambiar. |
| D8 | **Licencia de las fuentes**: el crate sigue `license = "MIT"`; las fuentes no son crates, `cargo deny` no las ve y `deny.toml` **no cambia**. Los textos OFL viajan con cada paquete (`share/doc/cincel/`) y en el repo junto a las fuentes. | La OFL permite incluir las fuentes dentro de un programa si acompañan el aviso de copyright y la licencia. Agregar `OFL-1.1` a `deny.toml` sin ningún crate que la use solo daría un aviso de "licencia permitida sin usar". |
| D9 | **Sin `cargo-dist` y sin vendorizar bibliotecas**: el binario se compila en `ubuntu-22.04` (glibc 2.35) y depende solo de `libc6`, `libgcc-s1`, `libxcb1`, `libxkbcommon0` y `libxkbcommon-x11-0`, presentes en cualquier escritorio Ubuntu o Pop!_OS 22.04+. Wayland (`libwayland-client`) y Vulkan (`libvulkan`) ya se cargan en tiempo de ejecución (`dlopen`), no aparecen en `ldd`. | `ldd` del binario actual (hecho al escribir esta spec) muestra exactamente esas cinco más sus dependencias de X (`libXau`, `libXdmcp`, `libbsd`, `libmd`, `libxcb-xkb`). `cargo-dist` no arma `.deb`; un script propio es más corto que configurarlo. El `.deb` las declara con `$auto`; `install.sh` las comprueba con `ldconfig -p` y, si falta alguna, dice qué instalar. |
| D10 | Identificador de la app **`dev.cincel.Cincel`** (el `app_id` que ya usa la ventana, `workspace.rs`) para el `.desktop`, el icono y el `StartupWMClass`. | En Wayland el escritorio asocia ventana, icono y entrada de menú por ese nombre. |
| D11 | Perfil `release`: se agrega `strip = "debuginfo"` (se conservan los nombres de funciones para que un error tenga una traza legible). Meta de tamaño del binario: 20–60 MB (`01-producto.md §2`); si se pasa, E6-G decide con números (primero revisar qué ocupa: `cargo bloat`/`size -A`). | El perfil ya no genera información de depuración propia; la que trae la biblioteca estándar solo suma tamaño. |
| D12 | **Repaso final en segundo plano**: la foto pasa a `Arc<ProjectSnapshot>`; `end_turn` lanza `changes()` en el ejecutor de fondo y el turno **sigue activo** hasta que el resultado vuelve; recién entonces se adoptan los cambios (en el hilo principal, solo los archivos que difieren) y se cierra el turno. Un prompt nuevo espera ese fin igual que hoy espera la foto. | Es el mismo patrón que la foto (`photo_waiter`): no cambia la regla de atribución y los botones de decidir siguen deshabilitados mientras el turno no terminó del todo. |
| D13 | **Binarios persistidos** en `review/<hash>/binaries.json` + `review/<hash>/binaries/<sha256>`, aparte de `state.json`/`objects/` del store de texto. **Deshacer unificado**: el workspace lleva el orden de los rechazos (`RejectLog`) y `Alt+Shift+U` deshace el último, sea de texto (store) o binario. | `ReviewStore::save` borra los objetos que el store no referencia: compartir `objects/` borraría los binarios. El store (`cincel-review`) no sabe nada de binarios y no tiene por qué saberlo. |
| D14 | Las teclas de la barra de búsqueda pasan al `keymap.json` por defecto, en secciones `Editor && searching` y `Editor && searching && replacing` **después** de la sección `Editor`; se borran `cincel_editor::search_bar_bindings` y `cincel_workspace::keymap::search_bar_bindings`. Consecuencia aceptada y documentada: si el usuario reasigna `tab`, `enter` o `ctrl-enter` en su propia sección `Editor` (sin `searching`), gana la suya también con la barra abierta, igual que en Zed. | En GPUI, entre secciones que coinciden en el mismo elemento gana la instalada después; el documento por defecto se instala después de los bindings de los widgets, así que el orden del archivo alcanza. Es lo que pedía la deuda: nada resuelto "a mano" en código. |
| D15 | Tests con tiempo límite: el límite se multiplica por `CINCEL_PERF_BUDGET_FACTOR` (por defecto 1; el CI de GitHub usa 4). | En la máquina de referencia se exige la meta real; en los servidores compartidos de GitHub, que son más lentos y variables, se evita un fallo falso sin dejar de detectar una regresión grande. |
| D16 | Clics reales en tests: la feature `test-support` de `cincel-workspace` suma `gpui-kit/test-support`. El conjunto de features de siempre no cambia de nombre (sigue siendo `cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`). | Habilita la simulación de clics sobre componentes de gpui-kit sin crear una variante nueva del directorio `target` (se recompila una vez). Si no alcanza para un botón, alternativa: envolverlo en un `div` con `debug_selector` (función de GPUI que en tests guarda dónde se pintó un elemento). |
| D17 | CI en GitHub Actions sobre `ubuntu-22.04`, solo acciones oficiales de GitHub más `Swatinem/rust-cache`, **fijadas por hash de commit**; herramientas (`cargo-deny`, `cargo-deb`, `cargo-about`) con `cargo install --locked --version X`. El workflow de release crea un **borrador** de release; lo publica el autor. | Menos terceros con acceso al repo; ninguna publicación automática sin que el autor mire. |
| D18 | Versión **1.0.0** en `[workspace.package]`; `publish = false` en todos los crates; `CHANGELOG.md` en inglés (formato *Keep a Changelog*), como el README. | No se publica en crates.io; el changelog es para el público del repositorio. |
| D19 | Avisos de licencias de dependencias: `THIRD-PARTY-LICENSES.html` generado con `cargo-about` en el release e incluido en los dos paquetes; `about.toml` acepta las mismas licencias que `deny.toml`. | MIT y Apache-2.0 piden acompañar el aviso de copyright al distribuir binarios. |
| D20 | Control de datos personales: `tools/privacy-check.sh` busca en el árbol **y en la historia de git** el usuario del sistema, el nombre del equipo y el correo de `git config`, leídos **en el momento** (nunca escritos en el repo), más patrones genéricos (rutas `/home/<nombre>/` que no sean los nombres de ejemplo del repo, correos que no sean `@example.com`/`@example.invalid`, prefijos de tokens conocidos). | Buscar el nombre real exige saberlo; escribirlo en el script sería justamente publicarlo. |
| D21 | Capturas del README tomadas **en el banco** (D1) con `grim` sobre la salida del escritorio invisible, con un proyecto de demostración y un turno sintético (`cincel --bench demo`, §3.4: rojo y verde sin ningún agente real). Nunca de la sesión del autor. | Sin datos personales y reproducibles. El chat con un agente real no se captura: necesitaría una cuenta del autor. |
| D22 | Nombre sugerido del repositorio: `cincel`. La URL (`https://github.com/jotagary25/cincel`) queda con el marcador `jotagary25` en un solo lugar por archivo, listado en `docs/publicacion.md`, y la completa el autor (o el orquestador cuando el autor le diga su usuario). | No se adivina ni se escribe la cuenta del autor. |

---

## 3. Rendimiento (A.1)

### 3.1 Métricas, metas y cómo se mide cada una

| # | Métrica | Meta (`01-producto.md §5`) | Externa (banco, las 3 apps) | Interna (`cincel --bench`) |
|---|---|---|---|---|
| M1 | Arranque en frío hasta la primera ventana pintada | < 400 ms | lanzamiento → primer cuadro con contenido de la ventana (§3.2); también → ventana mapeada | inicio del proceso → fin del primer cuadro |
| M2 | Arranque tibio (segunda ejecución) | informativa | ídem | ídem |
| M3 | Abrir un archivo de 5 000 líneas | < 50 ms hasta el primer pintado | `Ctrl+P`, nombre, `Enter` → primer cuadro estable con el archivo (§3.2) | llamada a `open_file` → fin del primer cuadro con el archivo dibujado |
| M4 | Latencia de tecleo (tecla → cambio en pantalla), archivo de 5 000 líneas | < 16 ms | tecla inyectada → primer cuadro con el cambio; p50, p95, máx. de 100 teclas | despacho de la tecla → fin del cuadro que la muestra; p50, p95, máx. de 200 |
| M5 | Archivo de 1 MB: apertura, scroll y tecleo | "edición fluida; revisión inline activa" → apertura < 200 ms; scroll p95 del intervalo entre cuadros ≤ 16,7 ms y ninguno > 33 ms; tecleo p95 < 16 ms | como M3, M4 y scroll (§3.2) | ídem + **revisión**: 50 cambios del agente aplicados → segmentos visibles < 100 ms; tecleo con segmentos pendientes p95 < 16 ms |
| M6 | Archivo de 50 000 líneas (≈ 2 MB): apertura, scroll y tecleo | "edición fluida; revisión solo por archivo" → mismas cifras que M5 (sin la parte de revisión inline) | como M5 | como M5; comprueba que no se pintan segmentos inline |
| M7 | Memoria en reposo | < 300 MB | RSS y PSS del árbol a los 30 s, con el proyecto mediano abierto y un archivo abierto | `VmRSS` de `/proc/self/status` a los 30 s |
| M8 | CPU en reposo | 0 % (sin redibujo continuo) | CPU del árbol (`utime+stime`) durante 60 s tras los 30 s de M7; cuadros producidos en esos 60 s | cuadros pintados en esos 60 s (debe ser 0) y CPU propia |
| M9 | Tamaño del binario | ≤ 80 MB (`01-producto.md §2`, meta ajustada por el autor el 2026-09-29; la original era 20–60 MB) | — | `stat` del binario de release empaquetado |
| M10 | Buscador de archivos con 50 000 rutas | < 50 ms tecla → resultados (spec 07 §3.1) | — | tecla → fin del cuadro con resultados, en el corpus grande |
| M11 | Repaso final del turno en un proyecto grande | la ventana no se congela: ningún bloqueo del hilo principal > 8 ms atribuible al repaso | — | foto, repaso total y bloqueo máximo del hilo principal (§5.3) |

"0 % de CPU" se juzga como: media ≤ 0,1 % en 60 s **y** 0 cuadros producidos por la app en ese lapso (con el cursor sin parpadeo; Cincel ya lo detiene a los 5 s sin escribir).

### 3.2 Banco de medición externo (`cincel-perf` + Docker)

**Resumen llano.** Es un escritorio invisible dentro de un contenedor (una "caja" aislada del sistema) donde se abre una sola aplicación por vez. Un programa pequeño hecho para esto aprieta teclas por ella, mueve la rueda del mouse, y anota el instante exacto en que cada cuadro nuevo aparece en ese escritorio. Como el escritorio invisible refresca 240 veces por segundo, el error de cada medición es de unos 4 ms como máximo.

**Diseño técnico.**
- Crate nuevo `crates/cincel-perf` (binario `cincel-perf`, `publish = false`, sin GPUI), miembro del workspace. No se empaqueta ni se instala. Dependencias: `wayland-client` 0.31, `wayland-protocols` 0.32 (feature `client`, y `staging` para `ext-foreign-toplevel-list-v1`), `wayland-protocols-wlr` 0.3 (`client`), `wayland-protocols-misc` (`client`, para `zwp_virtual_keyboard_v1`; única dependencia nueva, MIT), `rustix` (ya está; `posix_fadvise`), `serde_json`, `anyhow`. Todas MIT.
- Subcomandos (salida: una línea JSON por medición, a stdout):
  - `corpus <dir>`: genera el corpus de §3.4 (determinista, semilla fija).
  - `evict <ruta>…`: `posix_fadvise(DONTNEED)` recursivo sobre archivos y carpetas (D4).
  - `launch --name <app> [--settle-ms 30000] -- <comando…>`: marca `t0` (`CLOCK_MONOTONIC`) justo antes de `spawn`, pasa `CINCEL_BENCH_T0=<ns>` en el entorno (lo usa Cincel si está en `--bench`, las otras apps lo ignoran), registra `mapped` (primer `toplevel` nuevo con `app_id` esperado) y `first_content` (primer cuadro cuya región de la ventana tiene al menos 2 % de píxeles distintos del color de fondo del primer cuadro mapeado). Luego espera `settle`, mide RSS/PSS del árbol (M7), y 60 s de CPU y cuadros (M8). Los procesos del árbol se encuentran por `/proc/*/stat` (padre) desde el pid lanzado.
  - `quick-open --file <nombre>`: con la app ya abierta, inyecta `Ctrl+P`, escribe el nombre, espera 1 s (que el buscador termine), inyecta `Enter` y mide hasta el **primer cuadro estable** (el primero tras el cual no hay más daño durante 100 ms) — M3, y apertura de M5/M6.
  - `type --count 100 --interval-ms 150 [--key a]`: inyecta la tecla y su borrado alternados (el archivo no crece), mide cada tecla → primer cuadro dañado cuya área cambiada intersecta la ventana. p50/p95/máx (M4, M5, M6).
  - `scroll --seconds 3 --every-ms 8`: rueda del mouse continua sobre el centro de la ventana; intervalos entre cuadros producidos (M5, M6).
  - `report <archivos.jsonl>…`: arma la tabla Markdown de §3.7.
- Tiempos: los eventos `ready` de `zwlr_screencopy_frame_v1` traen la marca de tiempo de presentación (`CLOCK_MONOTONIC` en sway); las inyecciones se marcan con `clock_gettime(CLOCK_MONOTONIC)` justo antes del `flush` de Wayland. Misma base de tiempo, sin sincronizar relojes.
- Imagen `tools/perf/Dockerfile` (`ubuntu:24.04` fijado por *digest*): `sway`, `xwayland`, `grim`, `mesa-vulkan-drivers`, `libvulkan1`, `libgl1-mesa-dri`, `fonts-dejavu-core`, `ca-certificates`, `jq`, `git`. Sin credenciales, sin red durante las mediciones (`--network none`).
- `tools/perf/run.sh`: arma la imagen, arranca el contenedor con `--user $(id -u):$(id -g)` (así nada queda de root en disco), `--device /dev/dri`, la carpeta del corpus y las apps montadas **solo lectura**, `--security-opt seccomp=unconfined` solo si Electron lo necesita (se anota). Dentro: `WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_RENDER_DRM_DEVICE=<nodo de la GPU AMD>` (la GPU NVIDIA no es usable en el contenedor sin su kit propio: se mide con la AMD y se anota), salida `HEADLESS-1` en `1920x1080@240Hz` (`swaymsg output … mode --custom`; si sway no acepta 240 Hz, 144 Hz y el margen se recalcula). Las rutas de las apps entran por variables (`CINCEL_BIN`, `ZED_BIN`, `ANTIGRAVITY_BIN`, `VSCODE_BIN` opcional), nunca escritas en el repo.
- **VS Code**: `VSCODE_BIN` apunta al ejecutable `code` y usa el mismo perfil que Antigravity (§3.5). Sin la variable, la columna se omite con la nota "no instalado en la máquina de referencia".
- Comprobación cruzada en COSMIC (D2): `cosmic-comp` anidado con `WAYLAND_DISPLAY` propio; `cincel-perf launch --toplevel-protocol ext` mide solo `mapped`. Si el compositor no expone `ext-foreign-toplevel-list-v1` a clientes comunes o no corre anidado, se anota en `docs/rendimiento.md` y se omite.
- Margen de error (se publica): cuadro a 240 Hz = 4,17 ms → cada muestra ±4,2 ms; M1 además ±1 ms por el `spawn`. M4 externo incluye hasta un cuadro de espera del compositor: la meta de 16 ms se juzga con la medición **interna**, y la externa se acepta con p95 ≤ 16 ms + 1 cuadro (20,2 ms). El banco mide hasta que el compositor tiene el cuadro, no hasta que la luz sale del monitor (la pantalla real suma su propio retardo, igual para las tres apps).

### 3.3 Mediciones internas (`cincel --bench`)

- CLI (`crates/cincel/src/main.rs`, en `USAGE` bajo "Diagnóstico"): `cincel --bench ESCENARIO [RUTA] [--bench-file ARCHIVO]`. Escenarios: `startup`, `open`, `typing`, `scroll`, `idle`, `finder`, `sweep`, `review-1mb`, `all` (los 8 anteriores en orden), y `demo` (solo para capturas, §3.4; no mide). Imprime JSON por línea a stdout y sale con 0 (o 1 si el escenario no pudo correr, con el motivo). `--smoke-test --bench` = `startup` + el smoke test de siempre.
- Código en `crates/cincel-workspace/src/bench.rs` (acceso a `Workspace`, `CenterPanel`, `Review`), con una bandera global leída al arrancar; sin `--bench`, ningún gancho hace nada.
- Marcas: `t0` = `CINCEL_BENCH_T0` si viene (lo pone `cincel-perf`), si no el arranque del proceso de `/proc/self/stat` (resolución 10 ms, se indica en la salida). "Fin del cuadro": la marca se toma en el primer punto que GPUI permita después de pintar (preferentemente un callback tras el `present`; si no existe, al final del `paint` del elemento del editor); E6-A documenta cuál usó en el código y en `docs/rendimiento.md`.
- `startup`: `process_start→main`, `main→config`, `config→window_open`, `window_open→first_frame`, total.
- `open`: abre `--bench-file` con `CenterPanel::open_file(path, pin: true)` 10 veces (cerrando la pestaña entre una y otra, esperando reposo) → p50/p95/máx.
- `typing`: 200 teclas despachadas con `window.dispatch_keystroke` (letra y `backspace` alternados), una por cuadro → p50/p95/máx.
- `scroll`: un `ScrollWheelEvent` por cuadro durante 3 s → duración de cada cuadro.
- `idle`: espera 30 s, luego 60 s contando cuadros pintados (`FrameCounter`), CPU propia (`utime+stime`) y `VmRSS`.
- `finder`: sobre `RUTA` = corpus grande, abre `Ctrl+P`, escribe 5 consultas de 3 a 8 letras → tecla → fin del cuadro con resultados.
- `sweep`: sobre el corpus grande, turno sintético (`Review::begin_prompt`, cambios en disco de 200 archivos, `end_turn`) → duración de la foto, del repaso y **bloqueo máximo del hilo principal** (una tarea del hilo principal que se reprograma cada 1 ms registra el mayor hueco entre dos ejecuciones).
- `review-1mb`: sobre el archivo de 1 MB, 50 ediciones del agente vía la misma API que usa `FAKE_SHELL_EDITS` → tiempo hasta segmentos visibles; luego `typing` con segmentos pendientes.
- Además: con `CINCEL_TRACE_TIMINGS=1`, `tracing` registra la duración de los tramos marcados (`tracing::info_span!` en carga de configuración, carga de temas, registro de fuentes, apertura de proyecto, primer escaneo, restauración de revisión) para ubicar dónde se va el tiempo sin un perfilador (en Ubuntu, `perf` está restringido para usuarios comunes).

### 3.4 Corpus

`cincel-perf corpus <dir>` crea, siempre igual (semilla fija; fecha y autor de git fijos: `Cincel Bench <bench@example.invalid>`):
- `medio/`: repo git con un commit, 2 000 archivos en 120 carpetas, ≈ 200 000 líneas (Rust, TypeScript, Python, Markdown y JSON con aspecto real), 5 imágenes PNG pequeñas; es el "proyecto mediano" de M7.
- `grande/`: 20 000 archivos, ≈ 400 MB, 50 binarios de 1–5 MB; para M10 y M11.
- `archivos/`: `cinco-mil.rs` (5 000 líneas), `un-mega.rs` (1 MB, ≈ 25 000 líneas), `cincuenta-mil.rs` (50 000 líneas, ≈ 2 MB); copiados dentro de `medio/` para abrirlos con `Ctrl+P`.
- `--demo`: `demo/`, proyecto chico con nombres inventados (una calculadora en Rust con 8 archivos y un README) y un archivo `demo-turn.txt` con los cambios de un turno de ejemplo (mismo formato que `FAKE_SHELL_EDITS`: 3 archivos modificados y uno creado). Para las capturas de D21: `cincel --bench demo demo/` (escenario extra de §3.3, no cuenta como medición) hace un turno sintético con esos cambios —igual que `sweep`, por la regla de la foto— y deja la ventana abierta, con los segmentos rojo y verde pendientes, hasta recibir `SIGTERM`; mientras tanto `grim` captura la salida del escritorio invisible.

El corpus vive fuera del repo (scratchpad del orquestador) y se regenera en segundos; no se versiona.

### 3.5 Perfiles de las apps (D5)

Una carpeta de perfil nueva por corrida, dentro del corpus, borrada al terminar:
- **Cincel**: `CINCEL_CONFIG_DIR`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` propios; `settings.json` con `editor.cursor_blink: false` y el resto por defecto.
- **Zed**: `--user-data-dir <perfil>` (si la versión instalada no lo soporta, `XDG_CONFIG_HOME`/`XDG_DATA_HOME` propios); `settings.json` con `enable_language_server: false`, `auto_update: false`, `telemetry: { diagnostics: false, metrics: false }`, `cursor_blink: false`, sin asistentes de IA activados.
- **Antigravity / VS Code**: `--user-data-dir <perfil> --extensions-dir <perfil>/ext --disable-extensions --disable-workspace-trust --skip-welcome --ozone-platform=wayland`; `settings.json` con `editor.cursorBlinking: "solid"`, `telemetry.telemetryLevel: "off"`, `update.mode: "none"`.
- Se registran las versiones exactas de las tres apps en `docs/rendimiento.md`.

### 3.6 Procedimiento

1. Construir `cincel` en release (`cargo build --release -p cincel`) — el binario que se mide es el mismo que se empaqueta (§6).
2. Generar corpus; armar la imagen.
3. Por app: 10 corridas frías (`evict` + `launch`) y 10 tibias; 1 corrida larga de reposo (M7, M8); `quick-open` de los tres archivos (5 veces cada uno); `type` y `scroll` sobre cada archivo.
4. Cincel: `cincel --bench all` dentro del banco (y `startup` también en la sesión real, que no captura nada).
5. Control en la sesión real (sin capturas): `launch` de cada app con `--toplevel-protocol none` para RSS/CPU por `/proc` (D2).
6. Se anotan: CPU, RAM total y libre, GPU usada, kernel, versión de COSMIC y sway, carga del sistema (`/proc/loadavg`) al empezar; si la carga es > 2, se espera o se repite.

### 3.7 Publicación: `docs/rendimiento.md`

Estructura: (1) resumen en lenguaje llano con las conclusiones; (2) equipo y versiones; (3) tabla principal — fila por métrica M1–M11, columnas **Meta · Cincel · Zed · Antigravity · VS Code · ¿Cincel cumple?**, con mediana y p95 donde aplique; (4) mediciones internas de Cincel; (5) metodología resumida y margen de error (§3.2), con los comandos exactos para repetirla; (6) lo que se corrigió (§3.8) con el antes y el después; (7) lo que no se pudo medir y por qué. Sin rutas personales, sin nombre de equipo.

### 3.8 Corrección de lo que no cumpla (E6-G)

Por cada meta no cumplida: localizar con `CINCEL_TRACE_TIMINGS` y los escenarios de `--bench`, corregir, volver a medir la métrica completa (externa e interna) y anotar antes/después. Candidatos conocidos a revisar primero (sin suponer que fallan): escaneo de fuentes del sistema al arrancar (`fontdb`), carga del catálogo completo de iconos (`AllAssets`), carga de temas y gramáticas, apertura del proyecto anterior en el camino del primer cuadro, reconstrucción de layouts de línea en archivos grandes. Si una meta resulta inalcanzable por algo fuera de Cincel (por ejemplo, la inicialización del controlador Vulkan), se documenta con números como desviación y se consulta al autor antes de cerrar; nunca se ajusta la meta en silencio.

### 3.9 Criterios de aceptación
- [ ] `cincel-perf` mide las tres apps (VS Code si `VSCODE_BIN` está) sin modificar ninguna, y ninguna captura sale del escritorio invisible.
- [ ] `cincel --bench all` corre en el binario de release y devuelve JSON válido para los 8 escenarios de medición; `cincel --bench demo` deja la ventana con segmentos pendientes.
- [ ] `docs/rendimiento.md` publicado con la tabla completa, versiones, equipo, margen de error y comandos para repetir.
- [ ] Cincel cumple M1, M3, M4 (interna), M5, M6, M7, M8, M9, M10 y M11, o cada incumplimiento está documentado con números, motivo y aprobación del autor.
- [ ] Ningún archivo de perfil, corpus o captura del banco queda en el repo.

### 3.10 Tests
- Unitarios `cincel-perf`: generación del corpus determinista (dos corridas → mismos hashes); cálculo de p50/p95; detección de "primer cuadro con contenido" y "cuadro estable" sobre imágenes sintéticas; suma de PSS/RSS con un `/proc` falso (carpeta con `smaps_rollup` de ejemplo); armado de la tabla de `report`.
- Unitarios `cincel`: `Cli::parse` con `--bench` y sus escenarios; escenario desconocido → error de uso.
- `TestAppContext` (`crates/cincel-workspace/src/bench_tests.rs`): cada escenario corre en un proyecto temporal chico y devuelve los campos esperados (sin juzgar tiempos).

---

## 4. Fuentes Inter y JetBrains Mono embebidas (A.5)

### 4.1 Comportamiento
- Cincel se ve igual en cualquier equipo: la interfaz en Inter y el código en JetBrains Mono, con los pesos reales (normal, medio, seminegrita, negrita) e itálicas.
- Si el usuario elige otra familia en `settings.json` o en la pestaña de configuración y está instalada, se usa esa. Si no está instalada, el respaldo es la embebida correspondiente (Inter para la interfaz, JetBrains Mono para el código), no una fuente del sistema al azar.
- El selector de fuentes de la configuración lista las embebidas aunque no estén instaladas en el sistema (ya salen de `all_font_names()` al estar registradas).

### 4.2 Diseño técnico
- `crates/cincel-workspace/assets/fonts/inter/{Inter-Regular,Inter-Italic,Inter-Medium,Inter-SemiBold,Inter-Bold,Inter-BoldItalic}.ttf` + `OFL.txt`; `crates/cincel-workspace/assets/fonts/jetbrains-mono/{JetBrainsMono-Regular,JetBrainsMono-Italic,JetBrainsMono-Bold,JetBrainsMono-BoldItalic}.ttf` + `OFL.txt`; `crates/cincel-workspace/assets/fonts/SOURCES.md` con versión, URL de la release oficial de cada proyecto y SHA-256 de cada archivo. Se descargan solo de las releases oficiales (GitHub de cada proyecto) y se comprueba el hash contra el publicado cuando exista.
- `crates/cincel-workspace/src/fonts.rs`: `pub const UI_FAMILY: &str = "Inter"`, `pub const BUFFER_FAMILY: &str = "JetBrains Mono"`, `pub fn embedded() -> Vec<Cow<'static, [u8]>>` (`include_bytes!`), `pub fn register_embedded(cx: &mut App) -> anyhow::Result<()>` (`cx.text_system().add_fonts(embedded())`, registra en el log las familias y el tiempo).
- `main.rs` llama a `cincel_workspace::fonts::register_embedded(cx)` antes de `cincel_workspace::init` y de abrir la ventana; un error se registra y el arranque sigue (respaldo del sistema).
- Respaldos: `cincel-chat/src/panel.rs` (`PROSE_FALLBACKS`, lista monoespaciada) y `cincel-editor/src/settings.rs` ponen `Inter` / `JetBrains Mono` primero (ya lo hacen) y se quita del respaldo la variante "Nerd Font" solo si su presencia hiciera elegir otra fuente antes que la embebida (E6-D lo comprueba; si no cambia nada, se deja).
- Si el sistema tiene su propia Inter o JetBrains Mono instalada con otra versión, `cosmic-text` puede elegir cualquiera de las dos: E6-D comprueba cuál resuelve (`font_weight_and_style`/`font_id`) y, si elige la del sistema, se deja constancia (la diferencia entre versiones es mínima); no se desinstala nada del sistema.
- Tamaño esperado: ≈ 3,5 MB más en el binario (se registra en M9).

### 4.3 Criterios de aceptación
- [ ] En un contenedor `ubuntu:22.04` sin Inter ni JetBrains Mono instaladas, `cincel --smoke-test` registra en el log que resolvió "Inter" y "JetBrains Mono" (no un respaldo).
- [ ] La negrita del Markdown del chat se ve con peso 700 y la seminegrita de la interfaz con 600 (comparación en el banco, D21, contra una captura con las fuentes del sistema).
- [ ] `cargo deny check licenses` sigue pasando sin cambiar `deny.toml`; los dos `OFL.txt` están en los paquetes (§6).
- [ ] Los tests existentes no cambian de resultado (las fuentes no se registran en `init`).

### 4.4 Tests
- `crates/cincel-workspace/src/fonts_tests.rs`: `embedded()` devuelve 10 archivos no vacíos con cabecera TrueType; tras `register_embedded` en un `TestAppContext`, `all_font_names()` contiene las dos familias y `font_id` resuelve `Inter` en 400, 500, 600 y 700 y `JetBrains Mono` en 400 y 700. Si el sistema de texto de los tests no carga fuentes reales, estas comprobaciones pasan al `--smoke-test` (que las registra y falla si no resuelven) y el test se limita a las cabeceras.

---

## 5. Deudas que entran (A.6)

### 5.1 Tope de memoria de la foto en la pestaña de configuración (a)
**Llano.** Una fila más en Configuración → Revisión para elegir cuánta memoria puede usar la foto previa a cada mensaje.

**Técnico.**
- `settings_view.rs`, tabla `ROWS`, después de `review.max_lines`: `key: "review.snapshot_max_total_mb"`, `path: &["review", "snapshot_max_total_mb"]`, título **"Memoria para la foto del proyecto (MB)"**, descripción **"Antes de cada mensaje al agente, Cincel guarda una copia de tus archivos para poder mostrarte y deshacer lo que cambie. Este es el máximo que ocupa esa copia; pasado el tope, los archivos que no entran solo se pueden aceptar enteros."**, control `number(16., 4096., 16., 0)`.
- `cincel-settings/src/settings.rs`: validación de rango 16–4096; fuera de rango → issue "review.snapshot_max_total_mb tiene que estar entre 16 y 4096" y se usa 300. El comentario de `defaults.rs` se reescribe en el mismo lenguaje llano.
- Criterios: la fila aparece al buscar "memoria" o "foto"; cambiarla escribe la clave en `"review"` conservando comentarios; "Restablecer" la quita; el turno siguiente usa el valor nuevo (`SnapshotLimits::from_settings`).
- Tests: `settings_view_tests_e6.rs` (fila visible, escritura, búsqueda, restablecer); unitario de rango en `cincel-settings`.

### 5.2 Cambios binarios del agente: persistencia y deshacer (b)
**Llano.** Si el agente cambia una imagen y cerrás Cincel sin decidir, al volver sigue pendiente. Y `Alt+Shift+U` también recupera un rechazo de un archivo binario, en el orden en que rechazaste las cosas.

**Técnico.**
- Módulo nuevo `crates/cincel-workspace/src/review_binaries.rs` (hijo de `review.rs`, como `review_snapshot.rs`):
  - `binaries.json`: `{ "version": 1, "files": [{ "path", "state": "modified"|"created"|"deleted", "original_hash": sha256|null, "current_hash": sha256|null, "binary": bool, "turn_id", "created_at" }] }`; objetos en `binaries/<sha256>` (bytes originales). Escritura atómica (`.tmp` + `rename`), en el mismo debounce que `schedule_persist`; los objetos no referenciados se borran en cada guardado.
  - Restaurar al abrir el proyecto: por entrada, hash de lo que hay en disco (o ausencia) contra `current_hash`; si coincide, vuelve a `Review.binaries`; si no, se descarta con el mismo aviso que los de texto ("«x» cambió fuera de Cincel; se descartó su revisión"). **Nunca se escribe un archivo al restaurar** (`03-arquitectura.md §6`). Un objeto faltante o corrupto → la entrada queda como "solo aceptar" (sin `original`) con aviso en el log.
- Deshacer unificado (D13):
  - `pub(super) struct RejectLog { entries: Vec<RejectEntry> }`, `enum RejectEntry { Store, Binaries(Vec<BinaryUndo>), Mixed { store: bool, binaries: Vec<BinaryUndo> } }`, tope `cincel_review::UNDO_LIMIT` (32).
  - `ReviewStore::undo_depth(&self) -> usize` (nuevo, `cincel-review`): el workspace compara antes y después de cada rechazo para saber si el store apiló algo.
  - `BinaryUndo { path, change: BinaryChange, rejected: Option<Arc<[u8]>> }`: `rejected` = los bytes que dejó el agente, leídos **antes** de restaurar (`None` si el agente lo había borrado). Si superan `review.max_file_size_kb`, no se guardan, el aviso de rechazo dice "· sin deshacer (archivo demasiado grande)" y la entrada no se apila.
  - `undo_last_reject`: desapila la última entrada; `Store` → `store.undo_last_reject()` (como hoy); `Binaries` → por archivo, vuelve a escribir `rejected` (o borra el archivo si el agente lo había borrado), `on_host_wrote`, reinserta el `BinaryChange`; `Mixed` → las dos. Una entrada `Store` cuyo contenido el store ya descartó por su propio tope se salta.
  - Solo en memoria (igual que el deshacer de texto): no sobrevive a cerrar Cincel.
- El aviso emergente de rechazo de un binario ofrece "Deshacer (Alt+Shift+U)" como los de texto.
- Criterios: agente cambia `logo.png` → cerrar y reabrir Cincel → sigue pendiente en el árbol y el panel; si `logo.png` cambió por fuera mientras Cincel estaba cerrado → se descarta con aviso; rechazar `logo.png` y luego `Alt+Shift+U` → vuelven los bytes del agente y el cambio queda pendiente; rechazar un segmento de texto, después un binario, y `Alt+Shift+U` dos veces → primero vuelve el binario, después el texto; rechazar todo el turno (texto + binario) y un solo `Alt+Shift+U` → vuelven los dos.
- Tests: `binary_review_persist_tests.rs` (guardar/restaurar, descarte por cambio externo, objeto faltante, borrado de huérfanos) y `binary_review_undo_tests.rs` (los cinco casos de arriba, tope de tamaño, archivo creado y borrado), con `FAKE_SHELL_EDITS`; unitario de `undo_depth` en `cincel-review`.

### 5.3 Repaso final del turno fuera del hilo principal (c)
**Llano.** Cuando el agente termina, Cincel sigue comparando la foto con el disco, pero sin trabar la ventana. Si tarda más de un segundo, la barra de estado dice "Revisando los cambios del agente…".

**Técnico.**
- `PhotoState::Ready { turn, snapshot: Arc<ProjectSnapshot> }` y nuevo `PhotoState::Sweeping { turn, snapshot: Arc<ProjectSnapshot>, waiters, host_writes: Vec<PathBuf>, _task: Task<()> }`.
- `end_turn` (y `agent_gone`, y `begin_prompt` al encadenar turnos): si hay foto lista, pasa a `Sweeping` y lanza `snapshot.changes()` en `cx.background_executor()`; **no** cierra el turno. Mientras tanto: los lotes del watcher se siguen adoptando contra la foto (`ready_snapshot` también devuelve la foto en `Sweeping`); lo que Cincel escribe va a `host_writes`.
- Al volver el resultado (`sweep_arrived`): se descartan de la lista las rutas de `host_writes` (son de Cincel) y se refrescan en la foto; se adopta el resto con `adopt_change` en el hilo principal (solo los archivos que difieren); luego lo que hoy hace `end_turn` tras el repaso (`store.end_turn`, `reportable`, `prune`, `refresh`, `schedule_persist`) y se avisa a los `waiters`.
- `begin_prompt` durante `Sweeping`: el prompt espera (mismo mecanismo que `photo_waiter`, renombrado `turn_waiter`: cubre foto y repaso). Cancelar el prompt mientras espera lo descarta, como hoy.
- Aviso: si `Sweeping` dura más de 1 s, la barra de estado muestra "Revisando los cambios del agente…" (misma posición que "N cambios pendientes"); desaparece al terminar.
- Los botones de decidir del turno siguen deshabilitados mientras el turno no terminó (el estado `active_turn` sigue puesto).
- Proyecto inerte (tests): el repaso es síncrono salvo con `set_photo_in_background(true)`, como la foto.
- Medición: escenario `sweep` de `--bench` (M11) en el corpus grande: bloqueo máximo del hilo principal ≤ 8 ms; se publica en `docs/rendimiento.md` junto con la duración total del repaso.
- Criterios: con 20 000 archivos y 200 cambios, la ventana responde durante el repaso (bloqueo ≤ 8 ms); ningún cambio se pierde ni se atribuye mal respecto de hoy (los tests de `snapshot_review_tests.rs` siguen pasando sin cambios); un `Ctrl+S` del usuario durante el repaso no queda como cambio del agente; un prompt enviado durante el repaso sale recién al terminar.
- Tests: `sweep_background_tests.rs` (repaso en fondo con `set_photo_in_background`; `Ctrl+S` durante el repaso; prompt que espera el repaso; cancelar durante la espera; cambio del watcher durante el repaso; `agent_gone` durante el repaso).

### 5.4 Teclas de la barra de búsqueda en el keymap por defecto (d)
**Llano.** `Tab`, `Shift+Tab`, `Enter` y `Ctrl+Enter` de la búsqueda figuran en el mapa de teclas por defecto, como todos los demás atajos.

**Técnico.**
- `cincel-settings/src/defaults.rs`, `DEFAULT_KEYMAP_JSONC`: la sección `Editor && searching` suma `"tab": "editor::search_next_field"` y `"shift-tab": "editor::search_prev_field"`; sección nueva, inmediatamente después, `{"context": "Editor && searching && replacing", "bindings": {"enter": "editor::replace_next", "ctrl-enter": "editor::replace_all"}}`, con un comentario en español que explique la precedencia (D14). Ambas quedan después de la sección `Editor` y de `Editor && review_hunk_under_cursor`.
- Se borran `cincel_editor::search_bar_bindings` (`actions.rs`), `search_bar_bindings`/`keymap_binds` de `cincel-workspace/src/keymap.rs` y su test; el test `default_keymap_has_every_shortcut_of_the_spec` suma las cuatro filas.
- Criterios: con el keymap por defecto, `Tab` en la barra cambia de campo y no inserta un tabulado; `Enter` en el campo de reemplazo reemplaza; `--print-default-keymap` muestra las cuatro teclas; el modal de atajos las lista en "Editor" con el contexto "buscando" / "reemplazando"; con un `keymap.json` del usuario que reasigna `ctrl-enter` en `Editor && searching && replacing`, gana el del usuario.
- Tests: los de `cincel-editor` y `cincel-workspace` sobre la barra siguen pasando con el keymap real instalado; `keymap_search_tests_e6.rs` (los cinco criterios y el caso D14 documentado: usuario con `tab` en `Editor` gana también buscando).

### 5.5 Test automático del buscador con 50 000 rutas (e)
- `crates/cincel-workspace/src/file_finder_perf_tests.rs`: 50 000 rutas sintéticas con forma real (carpetas de 1 a 6 niveles, nombres con acentos, extensiones variadas); para 10 consultas (de 1 a 12 letras, con y sin error de tipeo), `rank(candidates, query, &[], 200)` → mejor de 5 corridas < 50 ms × `CINCEL_PERF_BUDGET_FACTOR` (D15). El mensaje de fallo dice la consulta y el tiempo.
- Además, `TestAppContext`: con 50 000 candidatos el filtrado va al ejecutor de fondo y una tecla nueva descarta el resultado viejo (ya existe la lógica; falta el test con ese tamaño).
- El presupuesto de punta a punta (tecla → pintado) se mide en `--bench finder` (M10), no en el test.

### 5.6 Tests que faltaban (f)
1. **Actualizar Node y cancelar una actualización desde la configuración, de punta a punta** (`settings_update_e2e_tests.rs`): con `ConnectionsFixture`, `index.json` falso con una LTS nueva y descargador en memoria: abrir `Ctrl+,` → Conexiones → aparece "Actualizar a vX" → clic → progreso → "Node actualizado a vX" y `current` apunta a la nueva. Cancelar: descargador lento en memoria → clic en "Cancelar" → vuelve a "Actualizar a vX", sin staging ni `.part`. Lo mismo para el adaptador.
2. **Clic real en los botones de las barras** (`click_tests.rs`, D16): `simulate_click` sobre el centro de `debug_bounds(...)` de: menú (`titlebar-menu`), chat (`titlebar-toggle-chat`), archivos (`titlebar-toggle-files`), atajos de la barra de estado (`statusbar-shortcuts`), conexión de la barra de estado y pendientes de la barra de estado. Cada uno comprueba el efecto visible (menú abierto, panel con foco, modal abierto…). La `×` de la ventana: gpui-kit solo dibuja los controles de ventana con decoraciones del cliente y la ventana de test de GPUI informa decoraciones del servidor, así que el botón no existe en pantalla en los tests; sigue probada por la función exacta que llama su clic (`close_from_title_bar`). Si la plataforma de test permite pedir decoraciones del cliente, se agrega el clic real; si no, queda anotado como límite (§9).
3. **Cancelar una descarga lenta real** (`crates/cincel-connections/tests/slow_download.rs`, `#[ignore]`, fuera del conjunto de cada verificación): servidor HTTP local (`std::net::TcpListener` en un hilo del test) que entrega 1 MB cada 100 ms; `Runtime` con `base_url` apuntando a `http://127.0.0.1:<puerto>/` y el `HttpDownloader` real; cancelar a los 300 ms → el hilo termina en < 500 ms (× factor D15), sin `.part` ni staging, y el servidor ve la conexión cerrada. Se corre explícitamente: `cargo test -p cincel-connections --test slow_download -- --ignored`, y en el CI en un job aparte, semanal y manual (`workflow_dispatch` + `schedule`), no en cada push.

### 5.7 Limpieza opcional de baja prioridad
Llevar `git.added`/`git.modified`/`git.deleted` de `GitGutterColors` a `EditorTheme` (deuda de E5-D). No bloquea el cierre: se hace en E6-F solo si el resto de E6-F terminó; si no, queda anotada como deuda para después de la 1.0.

---

## 6. Instalador liviano (A.2)

### 6.1 Qué se distribuye
- `cincel-1.0.0-x86_64-linux.tar.gz` (+ `.sha256`) con:
  ```
  cincel-1.0.0-x86_64-linux/
    install.sh
    bin/cincel
    share/applications/dev.cincel.Cincel.desktop
    share/icons/hicolor/scalable/apps/dev.cincel.Cincel.svg
    share/icons/hicolor/{16x16,32x32,48x48,64x64,128x128,256x256,512x512}/apps/dev.cincel.Cincel.png
    share/doc/cincel/{LICENSE, CHANGELOG.md, THIRD-PARTY-LICENSES.html, OFL-Inter.txt, OFL-JetBrainsMono.txt}
  ```
- `cincel_1.0.0-1_amd64.deb` (+ `.sha256`) con lo mismo bajo `/usr` (`/usr/bin/cincel`, `/usr/share/...`), y `/usr/share/doc/cincel/copyright` en formato Debian (MIT del código, OFL de las fuentes, referencia a `THIRD-PARTY-LICENSES.html`).
- Los agentes, Node y los adaptadores **no** van en ningún paquete: se descargan al conectar (spec 06 §3), como hoy.

### 6.2 `install.sh`
- POSIX `sh`, sin `sudo`. `./install.sh [--prefix DIR] [--uninstall]`; `DIR` por defecto `$HOME/.local`.
- Instala: `bin/cincel` → `$DIR/bin/cincel` (0755); `.desktop` → `$DIR/share/applications/` con `Exec=` reescrito a la ruta absoluta del binario (el escritorio no siempre tiene `~/.local/bin` en su `PATH`); iconos → `$DIR/share/icons/hicolor/...`; docs → `$DIR/share/doc/cincel/`. Luego, si existen, `update-desktop-database "$DIR/share/applications"` y `gtk-update-icon-cache -f -t "$DIR/share/icons/hicolor"`.
- Comprueba con `ldconfig -p` las bibliotecas de D9; si falta alguna, avisa (no falla) con "Falta libxkbcommon-x11-0: instalala con `sudo apt install libxkbcommon-x11-0`".
- Si `$DIR/bin` no está en `PATH`, avisa cómo agregarlo; si hay otra `cincel` antes en el `PATH` (por ejemplo, la del `.deb`), avisa cuál se va a ejecutar.
- `--uninstall` borra exactamente lo que instaló (lista fija, sin comodines) y **nunca** toca `~/.config/cincel`, `~/.local/share/cincel`, `~/.local/state/cincel` ni `~/.cache/cincel`; lo dice al terminar ("Tu configuración, conexiones y revisiones siguen en …; borralas a mano si querés").
- Textos en español; `--help`.

### 6.3 `.deb` con `cargo-deb`
- `[package.metadata.deb]` en `crates/cincel/Cargo.toml`: `name = "cincel"`, `maintainer` (ver duda en §11), `section = "editors"`, `priority = "optional"`, `depends = "$auto"`, `recommends = "git, xdg-desktop-portal, mesa-vulkan-drivers"`, `extended-description` en inglés, `assets` con las rutas de §6.1, `changelog` generado desde `CHANGELOG.md`, `revision = "1"`.
- Se arma en `ubuntu-22.04` para que `$auto` (vía `dpkg-shlibdeps`) calcule versiones mínimas compatibles con 22.04.

### 6.4 `.desktop` e icono
- `packaging/linux/dev.cincel.Cincel.desktop`: `Name=Cincel`, `GenericName=Code Editor`, `GenericName[es]=Editor de código`, `Comment=Code editor where you review every change an AI agent makes`, `Comment[es]=Editor de código donde revisás cada cambio que hace un agente`, `Exec=cincel %f`, `Icon=dev.cincel.Cincel`, `Terminal=false`, `Type=Application`, `Categories=Development;TextEditor;IDE;`, `Keywords=editor;code;agent;review;`, `StartupWMClass=dev.cincel.Cincel`, `StartupNotify=true`. **Sin `MimeType`**: Cincel no se registra como programa para abrir carpetas, para no disputarle nada al gestor de archivos. Validado con `desktop-file-validate` (en el contenedor).
- Icono: `packaging/icons/dev.cincel.Cincel.svg` (fuente) y los PNG generados una vez con `packaging/icons/render.sh` (`rsvg-convert` dentro de un contenedor `ubuntu:24.04` con `librsvg2-bin`), versionados en `packaging/icons/png/`. Diseño propuesto: cuadrado de esquinas redondeadas en el fondo del tema oscuro, con un cincel en diagonal (mango cálido, virola gris, hoja clara con el filo biselado) y, junto al filo, dos rayitas cortas verde y roja (los colores de la revisión). Legible a 16 px: sin textos, sin detalles finos. Punto de partida (E6-H lo refina):
  ```svg
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
    <rect x="2" y="2" width="60" height="60" rx="14" fill="#21252b"/>
    <g transform="rotate(-45 32 32)">
      <rect x="28" y="5" width="8" height="21" rx="3" fill="#d19a66"/>
      <rect x="27" y="25" width="10" height="4" rx="1" fill="#abb2bf"/>
      <path d="M28.5 29h7v18l-7 6z" fill="#e6e9ef"/>
    </g>
    <rect x="42" y="46" width="12" height="3" rx="1.5" fill="#98c379"/>
    <rect x="42" y="51" width="8" height="3" rx="1.5" fill="#e06c75"/>
  </svg>
  ```

### 6.5 Versión, changelog y avisos de licencias
- `[workspace.package] version = "1.0.0"`; `cincel --version` → `cincel 1.0.0`; `publish = false` en los 13 crates (12 de hoy + `cincel-perf`).
- `CHANGELOG.md` (inglés, *Keep a Changelog*): entrada `1.0.0 - <fecha del tag>` que resume lo que trae la 1.0 por áreas (editor, chat y agentes, revisión, conexiones, productividad, rendimiento, distribución).
- `about.toml` + `about.hbs` (plantilla HTML simple) para `cargo about generate` → `THIRD-PARTY-LICENSES.html` (D19); no se versiona el HTML, se genera en el release.

### 6.6 Construcción local y verificación en contenedores limpios
- `packaging/docker/Dockerfile.build`: `ubuntu:22.04` fijado por *digest*, paquetes de compilación (la lista exacta la fija E6-H compilando desde cero; punto de partida: `build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libwayland-dev libvulkan-dev libfontconfig-dev git curl ca-certificates`), `rustup` instalado en la imagen con el toolchain de `rust-toolchain.toml`, `cargo-deb` y `cargo-about` con versión fija.
- `packaging/build.sh`: dentro de ese contenedor, con `--user $(id -u):$(id -g)`, `CARGO_TARGET_DIR=target/ubuntu-22.04` (dentro del `target/` del repo, ignorado por git) y el registro de cargo montado; arma tarball y `.deb` en `dist/`. Al terminar la verificación, `target/ubuntu-22.04` se borra (con `cargo clean --target-dir`) para no llenar el disco.
- `packaging/verify.sh`: en contenedores **limpios** `ubuntu:22.04` y `ubuntu:24.04`:
  1. `.deb`: `apt-get install -y ./cincel_1.0.0-1_amd64.deb` (resuelve dependencias solas) → `cincel --version` → `desktop-file-validate` → `cincel --smoke-test` bajo `xvfb-run` con `mesa-vulkan-drivers` (lavapipe) y `CINCEL_ALLOW_SOFTWARE_GPU=1` (estos tres paquetes se instalan solo para la prueba) → `apt-get remove cincel` no deja archivos en `/usr`.
  2. Tarball, como usuario común (no root): `./install.sh` → los archivos esperados en `~/.local` → `~/.local/bin/cincel --version` → smoke test como arriba → `./install.sh --uninstall` → no queda ningún archivo instalado y `~/.config/cincel` (creado por el smoke test) sigue ahí.
  3. `ldd` del binario: solo las bibliotecas de D9 (y sus dependencias); el script falla si aparece otra.
  4. `objdump -T` del binario: ninguna versión de símbolo `GLIBC_` mayor que 2.35.
- Smoke test en Wayland: además, el binario del paquete corre `--smoke-test` en el banco (sway, D1).
- Si `docker` no estuviera disponible (no es el caso en la máquina de referencia), `docs/publicacion.md` explica cómo hacer lo mismo con `podman` (mismos comandos) o en una máquina virtual con Ubuntu 22.04.

### 6.7 Criterios de aceptación
- [ ] `packaging/build.sh` produce tarball y `.deb` con la estructura de §6.1.
- [ ] `packaging/verify.sh` pasa completo en `ubuntu:22.04` y `ubuntu:24.04`.
- [ ] El `.desktop` valida; con el tarball instalado en la máquina del autor, Cincel aparece en el lanzador de COSMIC con su icono y la ventana abierta muestra ese mismo icono (lista manual).
- [ ] El binario de los paquetes mide 20–60 MB (M9) o la desviación está documentada.
- [ ] Ningún paquete contiene agentes, Node, credenciales ni rutas personales (`tools/privacy-check.sh` sobre el contenido desempaquetado).

### 6.8 Tests
- `install.sh`: `packaging/tests/install_test.sh` (lo corre `verify.sh`): instalar en un `--prefix` temporal, comprobar la lista exacta de archivos y el `Exec=` absoluto, desinstalar y comprobar que no queda nada; segunda instalación encima de la primera; `--uninstall` sin instalación previa no falla.
- `shellcheck` sobre todos los `.sh` en el CI.

---

## 7. Repositorio público y CI (A.3)

### 7.1 Archivos del repositorio
- `README.md` en **inglés**: qué es, captura principal, rasgo distintivo (revisión en el editor), funciones, instalación (tarball y `.deb`, con los comandos), primer uso, requisitos, cómo compilar, enlace a la documentación, licencia; al final **"En español"**: el mismo contenido resumido y enlace a `docs/usuario/`.
- Capturas en `docs/capturas/` (PNG optimizados, ≤ 300 KB cada una), tomadas por E6-J en el banco (D21): (1) editor con segmentos rojo/verde pendientes, píldora y barra flotante; (2) pestaña de configuración; (3) buscador `Ctrl+P` o modal de atajos. Proyecto de demostración generado (`cincel-perf corpus --demo`), sin nombres reales; la barra de título muestra "demo".
- `CONTRIBUTING.md` (inglés, con una línea en español): cómo compilar, las features de test fijas y por qué, los comandos de §13, estilo (comentarios en inglés, textos de interfaz en español), cómo proponer cambios, el método de specs (`docs/specs/`).
- `CODE_OF_CONDUCT.md`: adopta el Contributor Covenant 2.1 **por referencia** (enlace a la versión oficial, sin copiar el texto), con el canal de contacto "reporte privado de GitHub" (sin correo).
- `SECURITY.md`: cómo reportar una vulnerabilidad (reporte privado de GitHub; sin correo).
- `.github/ISSUE_TEMPLATE/{bug_report.yml, feature_request.yml, config.yml}`: formularios en inglés con etiquetas en español entre paréntesis; el de errores pide versión (`cincel --version`), distribución, escritorio, GPU y las últimas líneas del log (`~/.local/state/cincel/log/`) **recordando borrar datos personales**. `.github/pull_request_template.md`.
- `Cargo.toml`: `repository` y `homepage` = `https://github.com/jotagary25/cincel` (D22), `description`.
- `.gitignore`: sin cambios salvo que E6-H use una carpeta de salida fuera de `target/` (en ese caso se agrega).
- `tools/privacy-check.sh` (D20): sale con 1 si encuentra algo, imprimiendo archivo y línea (o commit) **sin** repetir el valor encontrado completo (lo enmascara).

### 7.2 Workflows de GitHub Actions
- `.github/workflows/ci.yml` (push a `main` y pull requests), `runs-on: ubuntu-22.04`, permisos `contents: read`:
  - Paso común: liberar disco (borrar `/usr/share/dotnet`, `/usr/local/lib/android`, `/opt/ghc`), paquetes de compilación de §6.6, `rustup show` (aplica `rust-toolchain.toml`), `Swatinem/rust-cache`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only` (menos disco; no cambia el comportamiento), `CINCEL_PERF_BUDGET_FACTOR=4`.
  - `fmt`: `cargo fmt --all --check`.
  - `clippy`: `cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings`.
  - `test`: `cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins` y luego `cargo test --workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`. Siempre ese conjunto de features, nunca otro.
  - `deny`: `cargo deny check licenses advisories bans sources`.
  - `shellcheck` sobre `install.sh`, `packaging/**/*.sh`, `tools/**/*.sh`.
- `.github/workflows/slow-tests.yml` (`workflow_dispatch` + semanal): §5.6.3.
- `.github/workflows/release.yml` (tag `v*`), permisos `contents: write` solo en el job de publicación:
  1. `build` en `ubuntu-22.04`: `cargo build --release -p cincel`, `cargo about generate`, arma tarball y `.deb` (mismos scripts que en local, sin Docker: el runner ya es 22.04), `sha256sum`.
  2. `verify`: `packaging/verify.sh` con los artefactos (contenedores `ubuntu:22.04` y `ubuntu:24.04`).
  3. `draft-release`: `gh release create "$TAG" --draft --title "Cincel $VERSION" --notes-file <sección del CHANGELOG>` + los cuatro archivos. Comprueba que el tag coincide con `version` de `Cargo.toml`; si no, falla.
- Acciones de terceros fijadas por hash de commit (D17), con un comentario con la versión legible.

### 7.3 `docs/publicacion.md` (guía para el autor, en español)
Lista numerada con cada paso, qué ver en pantalla y qué hacer si algo sale distinto:
1. Leer el resultado de `tools/privacy-check.sh` que dejó el orquestador (sobre árbol, historia y paquetes).
2. **Decidir la historia de git**: publicarla tal cual o publicar una historia nueva de un solo commit (comandos para las dos opciones). Recomendación: historia nueva si el control encontró algo en commits viejos o si el correo de tus commits es uno que no querés público.
3. Configurar el correo privado de GitHub (`usuario@users.noreply.github.com`) en `git config user.email` para los commits que se publiquen.
4. Crear el repositorio en GitHub: nombre `cincel`, público, **sin** README, licencia ni `.gitignore` iniciales (ya están); por la web o con `gh repo create cincel --public --source . --remote origin`.
5. Completar `jotagary25`: el comando `grep -rn "jotagary25"` lista los lugares; un `sed` los reemplaza todos (o pedírselo a Cincel).
6. `git push -u origin main`.
7. Ajustes del repo: Actions habilitado con permisos de lectura por defecto; regla de protección de `main` (sin *force push*, sin borrado, CI obligatorio para pull requests; el autor puede seguir empujando directo); reporte privado de vulnerabilidades activado; descripción y temas (`editor`, `rust`, `gpui`, `acp`, `ai-agents`).
8. Esperar el CI en verde (qué hacer si falla: abrir el job, copiar el error y pasárselo a Cincel).
9. Primera versión: `git tag -a v1.0.0 -m "Cincel 1.0.0"` + `git push origin v1.0.0` → el workflow deja un **borrador** de release con los cuatro archivos → revisarlo (descargar el `.deb`, probarlo) → "Publish release".
10. Opcional: renombrar la carpeta local del repo a `cincel` (`pendientes-etapa-5.md §A.7`) y cómo actualizar los atajos del sistema que apunten a la carpeta vieja.
11. Qué no hacer: no subir `~/.config/cincel`, no pegar logs sin revisar en un issue.

### 7.4 Criterios de aceptación
- [ ] `tools/privacy-check.sh` sale con 0 sobre el árbol de trabajo y sobre el contenido de los dos paquetes (la historia se informa, la decide el autor).
- [ ] E6-I corre localmente cada paso de `ci.yml` con los mismos comandos (en el contenedor `ubuntu:22.04` de §6.6) y `actionlint` (binario oficial, dentro de un contenedor) valida los tres workflows sin avisos.
- [ ] README, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, plantillas y `docs/publicacion.md` existen y no contienen datos personales.
- [ ] Todos los enlaces relativos del README y de `docs/usuario/` apuntan a archivos que existen (script de comprobación en el CI).

---

## 8. Documentación de usuario (A.4)

### 8.1 Archivos (`docs/usuario/`, español, lenguaje llano, con capturas del banco donde ayuden)
- `README.md`: índice y "Cincel en un minuto".
- `instalacion.md`: tarball (`install.sh`, `--uninstall`, qué queda y qué se borra), `.deb` (doble clic o `sudo apt install ./…`), requisitos (Ubuntu/Pop!_OS 22.04+, GPU con Vulkan; qué hacer sin Vulkan: `CINCEL_ALLOW_SOFTWARE_GPU=1`), actualizar a una versión nueva, desinstalar.
- `primer-arranque.md`: abrir una carpeta (menú, `Ctrl+O`, línea de comandos), las tres zonas (chat, editor, archivos), pestañas y previsualización, guardar, dónde vive la configuración.
- `conexiones.md`: qué es una conexión; conectar Claude, Codex y Antigravity (qué se descarga, cuánto pesa, dónde se guarda); cuentas aisladas (Cincel no usa tus sesiones del sistema y no ve las de otros programas); varias cuentas del mismo proveedor; renombrar, reparar, volver a conectar, desconectar y eliminar; actualizar el adaptador y Node.
- `revision.md`: la regla (todo lo que cambia en la carpeta mientras el agente trabaja es suyo y se revisa); la foto y su tope de memoria; segmentos rojo y verde; aceptar o rechazar un segmento, una línea, un archivo, el turno o todo; archivos creados y borrados (tachados, pestaña de solo lectura); binarios; `Alt+Shift+U`; escribir dentro de un segmento; cerrar con cambios sin decidir (el diálogo); qué pasa al reabrir; qué le cuenta Cincel al agente de lo que rechazaste.
- `atajos.md`: tabla por categoría (las mismas cinco del modal) con tecla y descripción, cómo cambiarlos en `keymap.json` y cómo verlos con `F1`; la nota de D14.
- `ajustes.md`: la pestaña `Ctrl+,` y `settings.json` (conviven; comentarios conservados); cada sección con sus ajustes en lenguaje llano; temas; `--print-default-settings`.
- `problemas.md`: sesión vencida (qué dice el aviso y cómo volver a conectar), sin navegador o el navegador no abre (copiar el enlace), descargas que fallan o quedan a medias (cancelar, reintentar, carpeta de descargas), sin Vulkan, la ventana no abre, dónde está el log y cómo compartirlo sin datos personales, cómo empezar de cero sin perder proyectos.

### 8.2 Criterios y tests
- [ ] Cada flujo de `01-producto.md §4` está explicado en algún archivo.
- [ ] `crates/cincel-workspace/src/user_docs_tests.rs`: cada comando del keymap por defecto y de los widgets tiene su descripción (`shortcuts_modal::description`) presente en `docs/usuario/atajos.md` con su tecla formateada (`format_keystroke`); falla si se agrega un atajo sin documentarlo.
- [ ] Un subagente distinto del que la escribió sigue `instalacion.md`, `primer-arranque.md` y `revision.md` al pie de la letra en el banco (con el agente falso donde haga falta un agente) y anota cada paso que no coincide; se corrigen todos.
- [ ] Sin datos personales (`tools/privacy-check.sh`).

---

## 9. Qué cubre respecto de lo acordado

| Decisión | Cómo la cubre esta spec | Recortes o límites, con motivo |
|---|---|---|
| **A.1** Rendimiento medido y comparado con Zed y un editor de la familia VS Code | §3: 11 métricas con meta y forma de medir; banco externo igual para las tres apps sin instrumentarlas (D1), comprobación cruzada en COSMIC (D2), `--bench` interno (D6), corpus y perfiles reproducibles, tabla en `docs/rendimiento.md`, corrección en E6-G. Todo lo mide el orquestador (§10). | VS Code puro no se mide porque no está instalado; la metodología lo incluye con `VSCODE_BIN`. Las mediciones visuales se hacen en `sway` sin pantalla y no en COSMIC directamente: COSMIC no permite, hasta donde se sabe, capturar ni inyectar teclas sin diálogo, y capturar la sesión real viola la regla de no hacer capturas de pantalla completa; COSMIC entra como comprobación cruzada del arranque (D2). "En frío" es desalojo de los archivos de cada app, no de toda la caché del sistema (sin root, D4). La GPU del banco es la AMD (la NVIDIA no está disponible dentro del contenedor). |
| **A.2** Instalador liviano | §6: tarball + `install.sh` (`~/.local/bin`, `~/.local/share/applications`, `.desktop`, icono) y `.deb` con `cargo-deb`, compilados en `ubuntu-22.04`; `ldd` analizado (D9: nada que vendorizar, Wayland y Vulkan ya por `dlopen`); agentes descargados al conectar; versión 1.0.0 y changelog; verificación en contenedores limpios 22.04 y 24.04 (Docker está disponible; alternativa podman o VM documentada); icono SVG propuesto. | La opción "preparar todos los agentes" desde la configuración, que figuraba en `pendientes-etapa-5.md §A.6`, no entra: A.2 fija que los agentes se descargan al conectar; queda para después de la 1.0. |
| **A.3** Repositorio público guiado | §7: `docs/publicacion.md` paso a paso (crear repo, URL, ramas, protección, releases, historia y correo), README inglés + español con capturas del banco, `CONTRIBUTING`, `CODE_OF_CONDUCT` (incluido), `SECURITY`, plantillas, CI (fmt, clippy `-D warnings`, tests con las features fijas, `cargo deny`) y release que arma tarball y `.deb`; control de datos personales (D20). | La creación del repo, el push, los ajustes de GitHub y la publicación del release los hace el autor (así se acordó); el workflow deja solo un borrador. |
| **A.4** Documentación de usuario | §8: los siete temas pedidos más índice, con test que ata `atajos.md` al keymap y una lectura de comprobación por otro subagente. | — |
| **A.5** Fuentes embebidas | §4: Inter y JetBrains Mono estáticas, registradas al arrancar, respaldo del sistema solo para otras familias, licencia OFL compatible y declarada sin tocar `deny.toml` (D8). | Si el sistema tiene otra versión instalada de la misma familia, `cosmic-text` puede elegirla; se comprueba y se anota (§4.2). |
| **A.6** Deudas | §5.1 (a), §5.2 (b), §5.3 (c), §5.4 (d), §5.5 (e), §5.6 (f); colores de git a `EditorTheme` como limpieza opcional (§5.7). | (f) el clic real sobre la `×` depende de que la plataforma de test permita decoraciones del cliente; si no, sigue probada por su función (§5.6.2). (c) el aviso "Revisando los cambios…" se prueba con el ejecutor en fondo forzado, igual que la foto. §5.7 puede quedar para después si no hay tiempo, sin bloquear el cierre. |
| **A.7** Todo lo mide y prueba el orquestador | §10: cada subetapa tiene subagente; mediciones (E6-C, E6-K), verificación de paquetes (E6-H) y lectura de la documentación (E6-J) las hacen subagentes. Al autor le quedan §7.3 y §12. | — |
| **B** Reglas de construcción | §10.1, escritas tal cual. | — |

Pendientes que **no** entran en la 1.0 y siguen anotados en `pendientes-etapa-5.md §B` y `§C` (ninguno fue parte de lo acordado para esta etapa): pegar imágenes y arrastrar al chat, interfaz de servidores MCP, formularios de `elicitation`, el test que falló una vez en la Etapa 1 (el CI lo hará visible si vuelve), tabulados al siguiente tope y ajuste de línea en píxeles, límites de gpui-kit (código en línea a 11,4 px, tira de título vacía), identidad de Antigravity, tamaño de descarga anticipado, y todo lo de "después de v1".

---

## 10. Plan de subetapas

### 10.1 Reglas fijas de construcción (para todos los subagentes)
1. **Mismo árbol, sin worktrees**: cada worktree duplicaría el directorio `target` (varios GB). Cada subagente trabaja sobre crates o archivos disjuntos de los de su ola.
2. **Solo `Edit`** (nunca reescritura completa) en archivos que otra subetapa de la misma ola también toca (`main.rs`, `Cargo.toml` raíz, `settings_view.rs`, `defaults.rs`, `review.rs`, `workspace.rs`, `keymap.rs`).
3. **Tests nuevos en archivos nuevos** (los nombres de §3–§8).
4. **Features de test fijas**: toda corrida de tests o clippy usa exactamente `--features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support` sobre los crates con GPUI; los crates sin GPUI, sin features. Nunca otra combinación.
5. **`cargo sweep --stamp` antes y `cargo sweep --file` después de cada verificación completa**; los builds en contenedor usan `target/ubuntu-22.04` y se borran al terminar.
6. Cargo en primer plano con timeout; **sin procesos en segundo plano** ni bucles de espera; al terminar, `pgrep -af "sleep|cincel|zed|antigravity|sway"` vacío (los contenedores del banco se lanzan con `--rm` y se comprueba `docker ps` vacío).
7. **Nada de capturas de pantalla completa** ni de la sesión del autor: las únicas capturas salen del escritorio invisible del banco.
8. **Nadie hace commits** ni push; nadie crea el repo público.
9. Nada de credenciales, tokens, rutas personales, nombre del equipo ni correos en el repo; los perfiles de las apps medidas y el corpus viven fuera del repo y se borran.
10. Descargas: solo de fuentes oficiales (releases de Inter y JetBrains Mono, imágenes oficiales de Docker Hub, crates.io, archivos de Ubuntu), con hash comprobado cuando exista.

### 10.2 Subetapas

| Sub | Contenido | Crates / archivos | Depende de | Subagente | Tamaño |
|---|---|---|---|---|---|
| **E6-A** | `cincel --bench` (§3.3): CLI, `bench.rs`, marcas de tiempo, los 8 escenarios de medición y `demo`, `CINCEL_TRACE_TIMINGS`, `bench_tests.rs`. | cincel (`main.rs`), workspace (`bench.rs`, ganchos mínimos) | — | Opus | M |
| **E6-B** | `cincel-perf` (§3.2, §3.4, §3.5): crate, subcomandos, imagen Docker, `run.sh`, perfiles de las apps, comprobación cruzada en COSMIC; tests de §3.10. | `crates/cincel-perf`, `tools/perf/` | — | Opus | L |
| **E6-D** | Fuentes embebidas (§4): descarga verificada, `fonts.rs`, registro en `main.rs`, respaldos, `fonts_tests.rs`. | workspace (`fonts.rs`, `assets/`), cincel (`main.rs`, con `Edit`), chat/editor (respaldos) | — | Sonnet | S |
| **E6-E** | Deudas de revisión: binarios persistidos y deshacer unificado (§5.2), repaso en segundo plano (§5.3), `ReviewStore::undo_depth`. | workspace (`review.rs`, `review_snapshot.rs`, `review_binaries.rs`), review, project (`snapshot.rs` si hace falta `Send`/`Sync`) | — | Opus | L |
| **E6-F** | Deudas menores: fila del tope (§5.1), teclas de búsqueda al keymap (§5.4), test de 50 000 rutas (§5.5), tests faltantes (§5.6), `gpui-kit/test-support` (D16); opcional §5.7. | settings, workspace (`settings_view.rs`, `keymap.rs`, tests), editor (`actions.rs`), connections (tests) | — | Sonnet | M |
| **E6-C** | Primera medición completa (§3.6) de las tres apps con el binario de release de ese momento; borrador de `docs/rendimiento.md` con la lista de metas no cumplidas. | docs | A, B, D | Sonnet | S |
| **E6-G** | Corrección de rendimiento (§3.8) de lo que E6-C marcó, con antes y después; tamaño del binario (D11). | los que corresponda | C, E | Opus | M–L (según C) |
| **E6-H** | Empaquetado (§6): versión 1.0.0, `publish = false`, `strip`, icono y PNG, `.desktop`, `install.sh`, metadatos de `cargo-deb`, `about.toml`, `Dockerfile.build`, `build.sh`, `verify.sh`, `install_test.sh`; verificación en 22.04 y 24.04. | `packaging/`, `crates/cincel/Cargo.toml`, `Cargo.toml` raíz (`Edit`), `CHANGELOG.md` | D | Sonnet | M |
| **E6-I** | Repositorio y CI (§7): README, `CONTRIBUTING`, `CODE_OF_CONDUCT`, `SECURITY`, plantillas, tres workflows, `privacy-check.sh`, comprobación de enlaces, `docs/publicacion.md`; ejecución local de cada paso del CI y `actionlint`. | raíz, `.github/`, `tools/`, docs | H | Sonnet | M |
| **E6-J** | Documentación de usuario (§8) y capturas del banco (D21) para README y manual; `user_docs_tests.rs`; lectura de comprobación por un segundo subagente (Sonnet). | `docs/usuario/`, `docs/capturas/`, workspace (test) | A, B, E, F | Sonnet | M |
| **E6-K** | Medición final con los paquetes de E6-H (el binario que se distribuye) y `docs/rendimiento.md` definitivo. | docs | G, H | Sonnet | S |
| **E6-L** | Integración (orquestador): verificación completa (§13), `privacy-check`, correcciones de otras specs (encabezado), `modulos/*.md`, `docs/etapas/etapa-6.md` con desviaciones y la lista de §12, `pendientes-etapa-5.md` actualizado, README final con las capturas. | docs | todas | orquestador | S |

**Olas sugeridas** (todas en el mismo árbol): **Ola 1**: E6-A, E6-B, E6-D, E6-E, E6-F en paralelo (E6-A y E6-D comparten `main.rs`: solo `Edit` y zonas distintas; E6-E y E6-F comparten `review.rs` solo si F toca el aviso de la barra de estado: no lo toca). **Ola 2**: E6-C y E6-H en paralelo. **Ola 3**: E6-G, E6-I, E6-J. **Ola 4**: E6-K. **Ola 5**: E6-L.

Cada subagente entrega: qué criterios de esta spec cumple (con el test o la medición que lo prueba), cuáles no y por qué (desviación anotada), y la salida de los comandos de §13 para su parte.

---

## 11. Riesgos y dudas

**Dudas para el autor (no bloquean el comienzo; se preguntan en E6-H/E6-I):**
- **Campo `Maintainer` del `.deb`**: Debian pide "nombre <correo>". Propuesta: tu correo privado de GitHub (`usuario@users.noreply.github.com`) o, si preferís no poner ninguno, `Cincel <cincel@example.invalid>` (una dirección reservada que no existe). Hasta que decidas, va la segunda.
- **Historia de git al publicar** (§7.3 paso 2): la decisión es tuya; la guía recomienda.

**Riesgos técnicos:**
- `sway` sin pantalla puede no aceptar 240 Hz o no importar los búferes de alguna app con la GPU AMD del contenedor: E6-B prueba primero con Cincel; alternativas en orden: 144 Hz, renderizador `vulkan` de wlroots, y como último recurso `wf-recorder` + análisis de cuadros con `ffmpeg` (margen mayor, documentado).
- Electron dentro del contenedor puede necesitar `seccomp=unconfined` o `--no-sandbox`: se anota cuál se usó; el efecto en las métricas es despreciable.
- La meta de 400 ms en frío puede depender de la inicialización de Vulkan: §3.8 (se documenta con números y se consulta al autor, nunca se cambia la meta en silencio).
- El CI de GitHub puede quedarse sin disco con las features de test: mitigado con `line-tables-only`, sin incremental y limpieza del runner (§7.2).
- Una Inter o JetBrains Mono del sistema con otra versión puede ganarle a la embebida (§4.2).

---

## 12. Lista de comprobación manual (autor)

Todo lo demás lo midió y probó el orquestador. Para esto usás los paquetes que dejó E6-H en `dist/` (o el borrador del release, si ya publicaste).

1. **Tarball**: descomprimir, `./install.sh`. Buscar "Cincel" en el lanzador de COSMIC: aparece con su icono. Abrirlo: la ventana y el panel muestran el mismo icono. `cincel --version` dice `cincel 1.0.0`.
2. Abrir tu proyecto real: el árbol, las pestañas y el editor se ven con Inter y JetBrains Mono (en el chat, una respuesta con **negrita** se nota más gruesa).
3. Etapa 1: abrir, editar con acentos, buscar y reemplazar, guardar, deshacer; cerrar y reabrir con las mismas pestañas.
4. Etapas 2 y 4: conectar Claude (o usar la conexión que ya tenés), pedirle que explique un archivo y que corra un comando (tarjeta de permiso); repetir con Codex y Antigravity si los usás.
5. Etapa 3 y deudas: pedirle a Claude un cambio en 3 archivos que incluya **una imagen o un archivo binario**; rechazar un segmento, una línea y el binario; `Alt+Shift+U` dos veces (vuelven en orden inverso). Cerrar Cincel con cambios sin decidir, reabrir: siguen ahí, también el binario.
6. Etapa 5: `Ctrl+P`, `Ctrl+,` (en Revisión aparece "Memoria para la foto del proyecto"), `F1` (las teclas de la búsqueda figuran), `Ctrl+L`, git en el margen, el menú y la `×` con un archivo sin guardar.
7. `./install.sh --uninstall`: Cincel desaparece del lanzador y tu configuración sigue en `~/.config/cincel`.
8. **`.deb`**: doble clic (o `sudo apt install ./cincel_1.0.0-1_amd64.deb`), abrir Cincel desde el lanzador y repetir 2 y 5 rápido. Desinstalar con `sudo apt remove cincel`.
9. Leer `docs/rendimiento.md` y `docs/usuario/README.md`; si algo no se entiende, avisar.
10. Seguir `docs/publicacion.md` cuando quieras publicar.

---

## 13. Comandos de verificación

Siempre con el mismo conjunto de features:

```sh
cargo sweep --stamp
cargo fmt --all --check
cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings
cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
cargo test --workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support
cargo deny check licenses advisories bans sources
cargo run -p cincel -- --smoke-test .
cargo run -p cincel --features cincel-workspace/test-support -- --smoke-test .   # control de fugas
cargo build --release -p cincel && target/release/cincel --smoke-test --bench .
cargo test -p cincel-connections --test slow_download -- --ignored          # solo en E6-F y E6-L
cargo test -p cincel-editor -p cincel-chat -p cincel-workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support ranks_fifty_thousand -- --ignored   # medición del buscador, con la máquina tranquila (E6-L)
packaging/build.sh && packaging/verify.sh                                  # E6-H, E6-K, E6-L
tools/perf/run.sh                                                          # E6-C, E6-K
tools/privacy-check.sh                                                     # E6-I, E6-L
shellcheck install.sh packaging/*.sh packaging/**/*.sh tools/*.sh tools/**/*.sh
cargo sweep --file
pgrep -af "sleep|cincel|zed|antigravity|sway"; docker ps                    # vacíos
```

Un subagente verifica solo su parte; el orquestador corre todo:
- Si tocó un crate con GPUI (`cincel-editor`, `cincel-chat`, `cincel-workspace`, `cincel`), selecciona los tres: `cargo test -p cincel-editor -p cincel-chat -p cincel-workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support` (y lo mismo con `clippy`); `cincel` con `cargo test -p cincel`.
- Si tocó solo crates sin GPUI (`cincel-settings`, `cincel-project`, `cincel-review`, `cincel-connections`, `cincel-perf`), `cargo test -p <crate>` sin features.
- El test de 50 000 rutas usa el límite real (`CINCEL_PERF_BUDGET_FACTOR` sin definir) en la máquina de referencia.
