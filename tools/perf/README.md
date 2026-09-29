# Banco de medición externo (`cincel-perf`)

Mide Cincel, Zed y Antigravity IDE (y VS Code, si está instalado) con la misma
vara, sin modificarlos, en un escritorio invisible: `sway` sin pantalla a
240 Hz dentro de un contenedor Docker (`ubuntu:24.04`, fijado por *digest*).
Diseño y métricas: `docs/specs/08-etapa6-cierre-1-0.md` §3.1–§3.6.

Las únicas capturas salen de ese escritorio invisible, que solo contiene la
app medida. La sesión real del usuario nunca se captura.

## Cómo se corre

```sh
cargo build --release -p cincel                       # el binario que se mide
ANTIGRAVITY_BIN=<ruta a antigravity-ide> tools/perf/run.sh smoke   # prueba corta
ANTIGRAVITY_BIN=<ruta a antigravity-ide> tools/perf/run.sh         # medición completa (§3.6)
tools/perf/run.sh cosmic                              # control cruzado en COSMIC (D2)
```

- Las apps entran por variables (`CINCEL_BIN`, `ZED_BIN`, `ANTIGRAVITY_BIN`,
  `VSCODE_BIN`); ninguna ruta queda escrita en el repositorio. Por defecto se
  buscan `zed` y `antigravity-ide`/`antigravity` en el `PATH`.
- `ANTIGRAVITY_BIN` tiene que ser **Antigravity IDE** (el derivado de VS Code).
  "Antigravity 2.0", el gestor de agentes que a veces instala el comando
  `antigravity`, no es un editor: solo muestra una pantalla de inicio de
  sesión. `run.sh` lo detecta y se detiene con un aviso.
- Resultados: `PERF_OUT_DIR` (por defecto una carpeta temporal nueva) con
  `results.jsonl` (una línea JSON por medición), `report.md` (la tabla de
  §3.7), `inner.log`, `container.log` y `logs/` (salida de cada app).
- Corpus y perfiles: `PERF_WORK_DIR` (por defecto temporal, se borra al
  terminar). Ninguna de las dos carpetas puede estar dentro del repositorio.
- Si la carga del sistema pasa de 2, `run.sh` no mide (§3.6); con
  `PERF_IGNORE_LOAD=1` mide igual y la carga queda anotada en `host_env`.

## Qué hace cada pieza

| Pieza | Qué hace |
|---|---|
| `crates/cincel-perf` | Cliente Wayland propio: detecta la ventana (`wlr-foreign-toplevel-management` o `ext-foreign-toplevel-list`), captura cuadros con daño (`wlr-screencopy`, `copy_with_damage`, dos pedidos en vuelo para no perder ninguno) e inyecta teclas y rueda (`zwp_virtual_keyboard_v1`, `zwlr_virtual_pointer_v1`). Subcomandos: `corpus`, `evict`, `launch`, `idle`, `quick-open`, `type`, `scroll`, `key`, `stop`, `probe`, `wait-wayland`, `report`. |
| `Dockerfile` | Imagen con `sway`, `xwayland`, `grim`, `wf-recorder`, `wtype`, Mesa (Vulkan y GL) y las bibliotecas que piden Zed y Electron. |
| `sway.config` | Una salida `HEADLESS-1` 1920x1080 a 240 Hz, sin barra, bordes ni fondo; Xwayland apagado (se miden clientes Wayland nativos). |
| `entry.sh` | Carpeta de ejecución privada y `sway`, con tiempo límite. |
| `inner.sh` | Corre dentro de `sway` y ejecuta el procedimiento de §3.6 por app. |
| `run.sh` | Compila `cincel-perf`, genera el corpus, arma la imagen y lanza el contenedor. |

## Condiciones del contenedor

- `--rm`, `--network none`, `--user $(id -u):$(id -g)` (nada queda de root en
  disco), solo el nodo de render de la GPU no NVIDIA (`--device`), `--shm-size
  1g` (Chromium usa `/dev/shm`). Corpus y apps montados solo lectura.
- `sway --unsupported-gpu`: el módulo NVIDIA del kernel del anfitrión hace que
  sway se niegue a arrancar aunque el contenedor no tenga esa GPU.
- Electron (Antigravity IDE, VS Code) corre con `--no-sandbox`: su sandbox
  necesita un ayudante *setuid* o espacios de nombres de usuario, que un
  usuario sin privilegios dentro del contenedor no tiene. No hace falta
  `seccomp=unconfined`.

## Perfiles aislados (§3.5)

Cada corrida crea un perfil nuevo (con `HOME` y carpetas XDG propias) y lo
borra al terminar; nunca se lee ni se escribe la configuración real.

- **Cincel**: `CINCEL_CONFIG_DIR` y XDG propios; `editor.cursor_blink: false`.
- **Zed**: `--user-data-dir`; sin servidores de lenguaje, sin actualización,
  sin telemetría, sin IA (`disable_ai`), sin parpadeo. Además
  `session.trust_all_worktrees: true`: sin eso Zed abre el diálogo de "modo
  restringido" y se come las teclas (equivale a `--disable-workspace-trust`).
- **Antigravity IDE / VS Code**: las opciones de §3.5 más
  `workbench.startupEditor: none`. Para Antigravity IDE se escribe además la
  clave `antigravityOnboarding` en `User/globalStorage/state.vscdb` (una base
  SQLite mínima que genera `cincel-perf`): sin ella el perfil nuevo muestra la
  bienvenida con inicio de sesión de Google en lugar del editor. Una vez con
  contenido se cierra su panel de agente (`Ctrl+Alt+B`): sin red muestra un
  indicador animado sin fin que ensuciaría todas las mediciones; es el
  equivalente de apagar la IA en Zed.

## Cómo se mide (resumen de §3.2)

- Tiempos en `CLOCK_MONOTONIC`: `t0` justo antes de lanzar, marca de cada
  inyección justo antes del `flush`, y la marca `ready` de cada cuadro que da
  sway (misma base; si no coincidiera, se usa la hora de recepción y la línea
  lo dice con `"clock": "client"`).
- **Ventana mapeada**: primer `done` de una ventana nueva con el `app_id`
  esperado (`dev.cincel.Cincel`, `dev.zed.Zed`; para Electron, la primera
  ventana nueva).
- **Primer cuadro con contenido**: se toma el color del escritorio vacío antes
  de lanzar; los cuadros que todavía lo muestran se saltean. El fondo es el
  color dominante del primer cuadro que muestra la ventana, y hay contenido
  cuando al menos el 2 % de la ventana difiere de ese fondo **y** el cuadro
  tiene 16 colores distintos o más (texto e iconos con antialias). La segunda
  condición es un agregado a la spec: evita contar como contenido un
  rectángulo liso (Electron muestra uno mientras acomoda la ventana).
- **Cuadro estable** (apertura de archivo): el primero tras el cual no hay
  daño durante 100 ms.
- **Tecla → pantalla**: tecla y borrado alternados; antes de cada tecla se
  espera 50 ms sin cuadros; latencia hasta el primer cuadro dañado.
- **Memoria**: RSS del proceso principal y RSS/PSS sumados del árbol
  (`smaps_rollup`); `launch` se declara *subreaper* para no perder procesos
  que se desprenden del padre.
- Margen: ±4,2 ms por muestra a 240 Hz; la meta de tecleo se juzga con la
  medición interna y la externa se acepta con p95 ≤ 20,2 ms.

## COSMIC (D1 y D2)

Comprobado con `cincel-perf probe` en una sesión COSMIC real (solo lista los
protocolos que ofrece el compositor, no captura nada): `cosmic-comp` ofrece a
un cliente común `ext_foreign_toplevel_list_v1`,
`ext_image_copy_capture_manager_v1` y `zwp_virtual_keyboard_manager_v1`; no
ofrece `zwlr_screencopy_manager_v1`, `zwlr_foreign_toplevel_manager_v1` ni
`zwlr_virtual_pointer_manager_v1`. Es decir, a diferencia de lo que suponía
D1, COSMIC sí permite capturar e inyectar teclas sin diálogo; el banco igual
no lo usa, porque eso capturaría la sesión real.

`run.sh cosmic` arranca un `cosmic-comp` anidado (una ventana en el
escritorio, con carpetas de configuración y estado propias para no tocar las
del usuario) y mide solo "ventana mapeada" con `--toplevel-protocol ext`.
