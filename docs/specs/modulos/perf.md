# Módulo `cincel-perf`

Banco de medición externo, en un binario aparte, sin GPUI, `publish = false`
(no se empaqueta ni se instala). Mide Cincel, Zed y Antigravity IDE (y VS
Code, si está instalado) con la misma vara, sin instrumentarlos, dentro de
un escritorio invisible (`sway` sin pantalla a 240 Hz, en un contenedor
Docker). Spec: `docs/specs/08-etapa6-cierre-1-0.md` §3.2–§3.6; guía de uso
completa: `tools/perf/README.md`.

## Por qué habla Wayland directamente

Ninguna de las tres apps expone un modo de medición propio, y agregar
instrumentación a Zed o Antigravity queda fuera de lo que Cincel puede
tocar. `cincel-perf` es un cliente Wayland (`wayland-client`,
`wayland-protocols[-wlr,-misc]`) que observa el compositor desde afuera:
detecta la ventana de la app medida, pide cuadros con marca de tiempo y
envía teclas y rueda de mouse como si fueran de un dispositivo físico. Así
mide las tres apps exactamente igual, sin código propio dentro de ninguna.

## Subcommandos

| Subcomando | Qué hace |
|---|---|
| `corpus DIR [--demo] [--no-large] [--no-git]` | Genera, con semilla fija, el corpus de medición: `medio/` (2 009 archivos, ≈200 000 líneas, con un repo git de un commit), `grande/` (20 000 archivos, ≈400 MB), `archivos/` (los tres archivos de prueba de M3/M5/M6). `--demo` agrega `demo/`, el proyecto de las capturas del README (D21). |
| `evict RUTA...` | `posix_fadvise(POSIX_FADV_DONTNEED)` recursivo: desaloja de la caché de páginas los archivos de una app para simular un arranque "en frío" sin `sudo` (D4). |
| `launch --name APP [--app-id ID] [--profile DIR] [--toplevel-protocol wlr\|ext\|none] [--settle-ms] [--idle-ms] -- COMANDO...` | Lanza la app, marca `t0` justo antes del `spawn`, detecta "ventana mapeada" y "primer cuadro con contenido", espera `settle`, y mide RSS/PSS del árbol de procesos y CPU/cuadros durante `idle-ms` (M7, M8). |
| `idle --app APP --pid PID` | La parte de reposo de `launch`, por separado (control cruzado sin capturas, D2). |
| `quick-open --file NOMBRE [--repeat] [--close-after]` | Con la app abierta: `Ctrl+P`, escribe el nombre, espera 1 s, `Enter`, mide hasta el primer cuadro estable (M3, apertura de M5/M6). |
| `type [--count 100] [--interval-ms 150] [--key a]` | Tecla y borrado alternados (el archivo no crece); mide tecla → primer cuadro dañado, p50/p95/máx (M4, M5, M6). |
| `scroll [--seconds 3] [--every-ms 8]` | Rueda de mouse continua; intervalo entre cuadros producidos. |
| `key COMBINACIÓN... [--gap-ms 150]` | Inyecta teclas sueltas (`ctrl+p`, `enter`, `text:hola`) sin medir nada; construir secuencias a mano. |
| `stop --pid PID [--profile DIR]` | Termina la app y limpia su perfil. |
| `probe` | Lista qué protocolos Wayland ofrece el compositor actual a un cliente común, sin capturar ni inyectar nada (usado para la comprobación de D1/D2 en COSMIC). |
| `wait-wayland [--timeout-ms]` | Espera a que el compositor del contenedor esté listo antes de lanzar nada. |
| `report ARCHIVO.jsonl...` | Arma la tabla Markdown de la spec §3.7 a partir de una o más corridas de `results.jsonl`. |

Cada medición imprime una línea JSON a la salida estándar; nada se
interpreta ni se agrega salvo con `report`.

## Cómo se corre el banco

```sh
cargo build --release -p cincel                                    # el binario que se mide
ANTIGRAVITY_BIN=<ruta a antigravity-ide> tools/perf/run.sh smoke    # prueba corta
ANTIGRAVITY_BIN=<ruta a antigravity-ide> tools/perf/run.sh          # medición completa
tools/perf/run.sh cosmic                                           # control cruzado en COSMIC (D2)
```

`tools/perf/run.sh` compila `cincel-perf`, genera el corpus, arma la imagen
de `tools/perf/Dockerfile` (`ubuntu:24.04` fijada por *digest*, con `sway`,
`grim`, `wf-recorder`, `wtype`, Mesa) y lanza el contenedor (`--rm`,
`--network none`, `--user $(id -u):$(id -g)`, solo el nodo de render de la
GPU no NVIDIA, corpus y apps montados solo lectura). Las rutas de las apps
entran por variables de entorno (`CINCEL_BIN`, `ZED_BIN`, `ANTIGRAVITY_BIN`,
`VSCODE_BIN`), nunca escritas en el repositorio. Resultados en
`PERF_OUT_DIR` (`results.jsonl`, `report.md`, logs); corpus y perfiles en
`PERF_WORK_DIR`; ninguna de las dos puede estar dentro del repositorio.

## Qué mide

Detalle completo del método (marcas de tiempo, "primer cuadro con
contenido", "cuadro estable", margen de error) en
`docs/specs/08-etapa6-cierre-1-0.md` §3.2 y `tools/perf/README.md`. En
resumen: los tiempos se toman en `CLOCK_MONOTONIC` (misma base para la
inyección y para la marca `ready` que da `sway` con cada cuadro, ±4,2 ms de
margen a 240 Hz); "ventana mapeada" es el primer `toplevel` nuevo con el
`app_id` esperado; "primer cuadro con contenido" exige que al menos el 2 %
de la ventana difiera del color de fondo del primer cuadro **y** que el
cuadro tenga 16 colores distintos o más (evita contar como contenido un
rectángulo liso que Electron pinta mientras acomoda la ventana); "cuadro
estable" es el primero tras el cual no hay más daño durante 100 ms; la
memoria es RSS del proceso principal y RSS/PSS sumados del árbol
(`smaps_rollup`, `launch` se declara *subreaper* para no perder procesos que
se desprenden del padre).

## Tests

Unitarios (`crates/cincel-perf/src/tests.rs` y módulos propios): generación
determinista del corpus (dos corridas → mismos hashes), cálculo de
p50/p95, detección de "primer cuadro con contenido" y "cuadro estable"
sobre imágenes sintéticas, suma de PSS/RSS con un `/proc` falso, armado de
la tabla de `report`.

## Criterios de aceptación
- [ ] `cincel-perf` mide las tres apps (VS Code si `VSCODE_BIN` está) sin
      modificar ninguna, y ninguna captura sale del escritorio invisible.
- [ ] `probe` no captura ni inyecta nada: solo lista protocolos.
- [ ] Dos corridas de `corpus` con la misma semilla producen los mismos
      hashes de archivo.
