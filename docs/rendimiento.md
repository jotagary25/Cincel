# Rendimiento

Estado: **definitivo** (E6-K, 2026-09-29). Medición final de Cincel, Zed y
Antigravity IDE con el banco de `docs/specs/08-etapa6-cierre-1-0.md` §3,
usando el binario **empaquetado** (extraído del tarball que se distribuye,
no `target/release/cincel`). Reemplaza los números de la primera medición
(E6-C) por los finales, con el mismo método; la sección de correcciones de
E6-G (§7) queda tal como la dejó esa subetapa, con su antes y después.

## 1. Resumen

Cincel arranca y responde de forma consistente más rápido que Zed y mucho
más rápido que Antigravity IDE en esta máquina, y usa una fracción de su
memoria (138 MB de RSS del árbol de procesos contra 286 MB de Zed y más de
2 GB de Antigravity IDE). Con el binario que se distribuye, **cumple las
once metas de rendimiento del producto** (`docs/specs/01-producto.md §5`)
que se pueden juzgar con sí/no (M2 es informativa), con dos salvedades ya
conocidas y documentadas, ninguna de las dos un incumplimiento de una meta:

1. **M10** (buscador con 50 000 rutas) se sigue midiendo internamente sobre
   2 009 archivos, no sobre 50 000, por la misma limitación de `run.sh` que
   anotó E6-C (§6); a esa escala menor cumple con margen amplio, y la escala
   real la cubre el test unitario `ranks_fifty_thousand` (fuera del banco).
2. **M11**, en el escenario combinado `all` (con `cinco-mil.rs` abierto en
   una pestaña mientras corre `sweep`), el tramo del vigilante de archivos
   vuelve a superar el presupuesto de 8 ms (10,1 ms) por la causa que ya
   identificó E6-G y dejó pendiente (recargar la pestaña abierta cuesta más
   de lo que cuesta hoy la revisión en sí); **medido con la receta exacta de
   la spec** (`sweep` solo, sobre el corpus grande de 20 000 archivos, sin
   ninguna pestaña abierta) los tres tramos cumplen con margen: 6,1 / 7,4 /
   4,0 ms.

El binario que se distribuye pesa **75,7 MB** (79 410 024 bytes), dentro de
la meta ajustada de M9 (≤ 80 MB, decisión del autor del 2026-09-29:
`01-producto.md §2`, spec 08 §3.1). Los dos paquetes (tarball y `.deb`)
pasan `packaging/verify.sh` completo en `ubuntu:22.04` y `ubuntu:24.04`
(§2 de este documento).

## 2. Paquetes verificados (E6-K)

Reconstruidos desde cero con el código actual (`packaging/build.sh`, dentro
de `ubuntu:22.04`) y verificados con `packaging/verify.sh` en contenedores
limpios `ubuntu:22.04` y `ubuntu:24.04` (instala, corre `--smoke-test`,
desinstala sin dejar nada; `ldd` solo con las bibliotecas de D9; ningún
símbolo `GLIBC_` por encima de 2.35; smoke test en el escritorio invisible
del banco).

| Archivo | Tamaño | SHA-256 |
|---|---|---|
| `cincel-1.0.0-x86_64-linux.tar.gz` | 26 659 100 bytes (25,4 MB) | `48c9f197d05276bb6845260ec2146a367002e4bedd9d60d5680bae8ee9980452` |
| `cincel_1.0.0-1_amd64.deb` | 17 833 504 bytes (17,0 MB) | `770b1e5b1bab8a7d43a3f17c81480129f161cb803c5e88bdbde86fdd41c2d091` |
| `bin/cincel` (extraído del tarball; es el binario que mide §4–§5) | 79 410 024 bytes (75,7 MB) | `2e16bff4db013471923d020e925e83751e6eff08f3be385441363e348fc6741c` |

`verify.sh: todo bien` en ambas distribuciones; el binario del `.deb` y el
del tarball son el mismo (`cargo deb --no-strip` sobre el mismo build).
Único aviso esperado: el umbral que trae `verify.sh` para M9 sigue
codificado en 20–60 MB (el script no se tocó en esta subetapa); el tamaño
real (75,7 MB) está dentro de la meta vigente de ≤ 80 MB, así que el aviso
es una desviación conocida del script, no del binario (ver §8).

## 3. Equipo y versiones

Máquina de referencia (sin datos personales); es la misma máquina y el
mismo sistema operativo de E6-C, sin cambios:

| | |
|---|---|
| CPU | AMD Ryzen 5 5600H with Radeon Graphics (6 núcleos / 12 hilos) |
| GPU usada por el banco | AMD Radeon Vega (Cezanne), nodo `renderD129`, controlador `amdgpu` |
| Otra GPU presente (no usada) | NVIDIA GeForce RTX 3050 Ti Mobile — el banco la descarta automáticamente (D1: necesita su propio kit de contenedores) |
| RAM | ≈13,5 GB (14 152 348 KB totales) |
| Distribución | Pop!_OS 24.04 LTS |
| Kernel | 7.1.5-76070105-generic |
| Compositor real (COSMIC, no usado para medir) | `cosmic-comp` 0.1~1790118748~24.04~0fbd457 |
| Compositor del banco (escritorio invisible) | `sway` 1.9, salida `HEADLESS-1` 1920×1080 @ 240 Hz |
| Imagen del banco | `ubuntu:24.04` fijada por *digest* (`tools/perf/Dockerfile`) |
| Imagen de compilación | `ubuntu:22.04` fijada por *digest* (`packaging/docker/Dockerfile.build`, D9) |
| Carga del sistema al medir | entre 1,0 y 1,6 (1 min) en cada corrida; siempre dentro del límite de `run.sh` (≤ 2), sin necesidad de `PERF_IGNORE_LOAD` |

Versiones medidas:

| App | Versión |
|---|---|
| Cincel | `cincel 1.0.0` (binario extraído de `cincel-1.0.0-x86_64-linux.tar.gz`, 79 410 024 bytes; ver §2) |
| Zed | `Zed 1.20.2` |
| Antigravity IDE | `Antigravity IDE 2.5.5 (VS Code 1.107.0)` |
| VS Code | no instalado en la máquina de referencia; columna omitida |

## 4. Tabla principal

Mediana (p50) y, donde aplica, p90/p95 y máximo. La columna "¿Cincel
cumple?" solo juzga a Cincel contra la meta de `01-producto.md §5`; las
demás apps son la comparación.

| Métrica | Meta | Cincel | Zed | Antigravity | VS Code | ¿Cincel cumple? |
|---|---|---|---|---|---|---|
| M1 Arranque en frío → primer cuadro con contenido | < 400 ms | 303,0 ms · p90 326,4 · máx 329,1 (n=10) | 1293,3 ms · p90 1310,9 · máx 1333,9 (n=10) | 5457,7 ms · p90 5624,8 · máx 5819,8 (n=10) | no instalado | **sí** |
| M1 Arranque en frío → ventana mapeada | informativa | 253,1 ms · p90 275,3 · máx 275,5 | 1234,6 ms · p90 1252,1 · máx 1272,9 | 2386,8 ms · p90 2420,5 · máx 2516,9 | no instalado | — |
| M2 Arranque tibio → primer cuadro con contenido | informativa | 228,9 ms · p90 246,1 · máx 249,2 | 286,2 ms · p90 299,8 · máx 348,1 | 4463,7 ms · p90 4485,6 · máx 4558,5 | no instalado | — |
| M3 Abrir `cinco-mil.rs` (Ctrl+P → cuadro estable) | < 50 ms | 31,7 ms · p95 53,9 (n=5) | 95,2 ms · p95 122,2 | 892,4 ms · p95 967,3 | no instalado | **sí** |
| M4 Tecla → pantalla en `cinco-mil.rs` | < 16 ms (externa: p95 ≤ 20,2 ms) | p50 5,9 · p95 7,1 · máx 7,8 ms | p50 9,6 · p95 10,6 · máx 12,7 ms | p50 24,7 · p95 43,0 · máx 44,5 ms | no instalado | **sí** |
| M5 Abrir `un-mega.rs` (1 MB) | < 200 ms | 116,2 ms · p95 144,5 (n=5) | 98,2 ms · p95 107,3 | 54,7 ms · p95 927,6 | no instalado | **sí** |
| M5 Scroll en `un-mega.rs` | p95 ≤ 16,7 ms, ninguno > 33 ms | p95 8,3 · máx 10,6 ms | p95 10,4 · máx 16,6 ms | p95 23,1 · máx 28,0 ms | no instalado | **sí** |
| M5 Tecla → pantalla en `un-mega.rs` | p95 < 16 ms (externa ≤ 20,2 ms) | p50 6,1 · p95 6,6 · máx 8,0 ms | p50 12,8 · p95 16,3 · máx 17,5 ms | p50 23,2 · p95 39,7 · máx 41,6 ms | no instalado | **sí** |
| M6 Abrir `cincuenta-mil.rs` (50 000 líneas) | < 200 ms | 35,2 ms · p95 186,7 (n=5) | 133,8 ms · p95 138,4 | 19,5 ms · p95 45,7 | no instalado | **sí** |
| M6 Scroll en `cincuenta-mil.rs` | p95 ≤ 16,7 ms, ninguno > 33 ms | p95 9,5 · máx 13,9 ms | p95 16,2 · máx 21,1 ms | p95 21,6 · máx 30,8 ms | no instalado | **sí** |
| M6 Tecla → pantalla en `cincuenta-mil.rs` | p95 < 16 ms (externa ≤ 20,2 ms) | p50 6,3 · p95 7,2 · máx 8,4 ms | p50 14,4 · p95 22,1 · máx 39,8 ms | p50 19,8 · p95 38,4 · máx 44,4 ms | no instalado | **sí** |
| M7 Memoria en reposo (proyecto mediano, un archivo abierto, 30 s) | < 300 MB (RSS del árbol) | RSS árbol 138 MB · PSS 114 MB · RSS principal 138 MB (1 proceso) | RSS árbol 286 MB · PSS 232 MB · RSS principal 250 MB (2 procesos) | RSS árbol 2043 MB · PSS 1260 MB · RSS principal 285 MB (14 procesos) | no instalado | **sí** |
| M8 CPU en reposo (60 s tras los 30 s de M7) | ≤ 0,1 % y 0 cuadros | 0,00 % · 0 cuadros | 0,57 % · 0 cuadros | 6,12 % · 0 cuadros | no instalado | **sí** |
| M9 Tamaño del binario | ≤ 80 MB (meta ajustada por el autor, 2026-09-29) | 75,7 MB (79 410 024 bytes) | — | — | — | **sí** |
| M10 Buscador de archivos | < 50 ms | interna: p95 16,7 ms sobre 2 009 archivos (ver §6, misma salvedad de escala que E6-C) | — | — | — | sí, con la salvedad de §6 |
| M11 Repaso final del turno (proyecto grande, 200 cambios) | sin bloqueos > 8 ms | interna, `sweep` solo sobre el corpus grande (20 000 archivos, receta exacta de la spec): máx. foto 6,1 / vigilante 7,4 / repaso 4,0 ms | — | — | — | **sí** (ver §5 para el escenario combinado `all`) |

## 5. Mediciones internas de Cincel (`cincel --bench`)

Binario extraído del tarball (§2), corrido dentro del banco (escritorio
invisible). Dos corridas: `all` sobre el proyecto mediano (2 009 archivos
de texto, ≈ 200 000 líneas, con `--bench-file medio/bench/cinco-mil.rs`,
igual que E6-C) y, aparte, `sweep` solo sobre el proyecto grande (20 000
archivos), que es la receta que pide §3.3 para M11 y que E6-C no había
llegado a correr (§6 de esa medición).

| Escenario | Lo que midió |
|---|---|
| `startup` | 184,7 ms totales (arranque **tibio**: el binario ya estaba en la caché de páginas por los lanzamientos anteriores de la misma corrida). Desglose: proceso→`main` 10,9 ms, `main`→config 2,8 ms, config→ventana 156,9 ms, ventana→primer cuadro 14,0 ms. |
| `open` (`cinco-mil.rs`) | Abrir → primer cuadro: p50 10,6 ms · p95 39,4 ms (10 repeticiones) |
| `typing` (`cinco-mil.rs`) | Tecla → cuadro: p50 4,2 ms · p95 5,7 ms · máx 9,8 ms (200 teclas) |
| `scroll` (`cinco-mil.rs`) | Intervalo entre cuadros: p50 4,2 ms · p95 5,0 ms · máx 7,4 ms (719 eventos, 0 sin cuadro) |
| `idle` (30 s reposo + 60 s ventana) | 0,00 % CPU, 0 cuadros pintados, VmRSS 189,1 MB |
| `finder` (`Ctrl+P`, 5 consultas) | Tecla → resultados: p50 9,6 ms · p95 16,7 ms · máx 24,4 ms, sobre 2 009 candidatos (misma salvedad de escala que E6-C, ver §6) |
| `sweep` (dentro de `all`, 200 archivos cambiados de 2 009, con `cinco-mil.rs` abierto en una pestaña) | Foto 14,2 ms · repaso 22,1 ms; bloqueo máximo del hilo principal: **foto 8,7 ms**, **vigilante 10,1 ms**, repaso propio 4,8 ms — el residuo que ya identificó E6-G (§7.2: recargar la pestaña abierta), no la revisión en sí (0,09 ms de su parte) |
| `sweep` (aparte, corpus grande — 20 000 archivos, 200 cambiados, sin ninguna pestaña abierta; **receta exacta de M11**) | Foto 224,7 ms · repaso 47,9 ms; bloqueo máximo del hilo principal: foto 6,1 ms, vigilante 7,4 ms, repaso propio 4,0 ms — **los tres dentro del presupuesto de 8 ms** |
| `review-1mb` (`un-mega.rs`, 1 MB) | 50 cambios del agente → segmentos visibles en 55,0 ms (meta: < 100 ms, cumple); tecleo con segmentos pendientes p50 7,1 ms · p95 10,9 ms (meta < 16 ms, cumple); 50 segmentos inline |
| `review-1mb` (`cincuenta-mil.rs`, 50 000 líneas) | 50 cambios del agente → 93,4 ms; **0 segmentos inline** y `file_level: true`, como pide M6 |

## 6. Lo que no se pudo medir bien, y por qué

- **M10 (buscador con 50 000 rutas) sigue midiéndose sobre 2 009 archivos,
  no sobre 50 000.** Misma limitación que anotó E6-C: `cincel --bench all`
  comparte una única `RUTA` entre los ocho escenarios, y esa ruta tiene que
  contener los tres archivos de `open`/`typing`/`scroll`, que solo existen
  dentro de `medio/`. El alcance de E6-K pedía repetir `all` y `sweep`
  contra el corpus grande, no `finder`; no se amplió a `finder grande` para
  no salirse de ese alcance. El número que sí tenemos (p95 16,7 ms sobre
  2 009 archivos) cumple la meta con margen amplio, pero no es una prueba a
  la escala de 50 000 rutas; esa escala la cubre el test unitario
  `ranks_fifty_thousand` de `crates/cincel-workspace/src/
  file_finder_perf_tests.rs` (spec §13, lo corre el orquestador en E6-L,
  fuera del banco).
- **M11 en el escenario combinado `all`** (con `cinco-mil.rs` abierto en una
  pestaña) sigue mostrando el residuo de 10,1 ms que documentó E6-G en su
  §9.2 (recargar la pestaña abierta, no la revisión): no es un incumplimiento
  nuevo ni de esta subetapa, y la receta oficial de M11 (`sweep` solo, sobre
  el corpus grande) sí cumple con margen (§4, §5). Sigue anotado para
  después de E6-G/E6-K, sin fecha asignada.
- **Comprobación cruzada en COSMIC (D2)** no se repitió en esta corrida:
  `tools/perf/run.sh cosmic` no se ejecutó (no estaba en el alcance de
  E6-K). Sigue pendiente, igual que en E6-C.
- **VS Code** no está instalado en la máquina de referencia: esa columna
  queda vacía en toda la tabla, como prevé §3.2.

## 7. Metodología (resumen) y margen de error

Banco: contenedor Docker (`ubuntu:24.04` fijado por *digest*) con `sway`
sin pantalla a 240 Hz; `cincel-perf` habla Wayland directamente para
lanzar cada app, detectar su ventana, capturar cuadros con marca de tiempo
e inyectar teclas y rueda del mouse. Cada app corre en un perfil aislado
propio (carpeta de datos nueva, sin extensiones ni servidores de lenguaje,
sin telemetría, cursor sin parpadeo), montada solo lectura dentro del
contenedor; el corpus (`medio/` 2 009 archivos ≈ 200 000 líneas, `grande/`
20 000 archivos, y los tres archivos de prueba) se genera con semilla fija
y no se versiona. Detalle completo: `docs/specs/08-etapa6-cierre-1-0.md`
§3.2–§3.5 y `tools/perf/README.md`.

Margen de error: un cuadro a 240 Hz son 4,17 ms, así que cada muestra tiene
±4,2 ms de error; M1 suma otros ±1 ms por el `spawn`. La meta de tecleo
(M4/M5/M6, 16 ms) se juzga con la medición **interna**; la externa se
acepta con p95 ≤ 20,2 ms (16 ms + un cuadro de espera del compositor). El
banco mide hasta que el compositor tiene el cuadro nuevo, no hasta que la
luz sale del monitor real (ese retardo adicional es igual para las tres
apps).

### Cómo repetir esta medición

El binario que se mide es el **empaquetado**, no `target/release/cincel`:

```sh
packaging/build.sh                              # produce dist/*.tar.gz
tar xzf dist/cincel-1.0.0-x86_64-linux.tar.gz -C /ruta/fuera/del/repo
CINCEL_BIN=/ruta/fuera/del/repo/cincel-1.0.0-x86_64-linux/bin/cincel \
ZED_BIN=/ruta/a/zed \
ANTIGRAVITY_BIN=/ruta/a/antigravity-ide \
tools/perf/run.sh full          # las tres apps; ver tools/perf/README.md
```

`ANTIGRAVITY_BIN` tiene que apuntar al **IDE** derivado de VS Code (por
ejemplo `~/Applications/antigravity-ide/antigravity-ide`), no al comando
`antigravity` del gestor de agentes ("Antigravity 2.0"), que solo muestra
una pantalla de inicio de sesión; `run.sh` lo detecta y se detiene con un
aviso si se confunden. `PERF_WORK_DIR` y `PERF_OUT_DIR` (corpus, perfiles y
resultados) van siempre fuera del repositorio y se pueden reusar entre
corridas separadas por app para no repetir la generación del corpus ni el
armado de la imagen (así se hizo en E6-K: `results.jsonl` acumula las tres
apps y `cincel-perf report` arma la tabla del §4 con todas juntas).
Mediciones internas sueltas del binario empaquetado:

```sh
CINCEL_BIN=/ruta/fuera/del/repo/cincel-1.0.0-x86_64-linux/bin/cincel
"$CINCEL_BIN" --bench all medio --bench-file medio/bench/cinco-mil.rs
"$CINCEL_BIN" --bench sweep grande     # M11 con la receta exacta de la spec
```

## 8. Nota de esta corrida (E6-K)

Subetapa E6-K (§10.2), posterior a E6-G (corrección) y E6-H (empaquetado).
Pasos, en orden:

1. **Reconstrucción de los paquetes** con el código actual:
   `packaging/build.sh` (compiló desde cero dentro de `ubuntu:22.04`, sin
   caché previa de `target/ubuntu-22.04`: ≈16 min) y
   `packaging/verify.sh` (≈6 min, `ubuntu:22.04` y `ubuntu:24.04`, más el
   smoke test en el escritorio invisible). Nombres, tamaños y `sha256` en
   §2. `target/ubuntu-22.04` (2,4 GB) se borró al terminar toda la
   subetapa (`rm -rf`, como documentó E6-H).
2. **Medición externa completa** (`tools/perf/run.sh full`) con el binario
   extraído del tarball como `CINCEL_BIN`, en tres llamadas sucesivas (una
   por app: Cincel ≈5 min, Zed ≈4 min, Antigravity IDE ≈8 min), reusando el
   mismo `PERF_WORK_DIR` (corpus) y `PERF_OUT_DIR` (resultados) entre las
   tres para no regenerarlos; la corrida de Cincel incluye `cincel --bench
   all` (§5) contra el corpus mediano, igual que E6-C.
3. **Medición adicional de M11 con la receta exacta de la spec**: una
   cuarta llamada a `tools/perf/run.sh full` (solo Cincel, `PERF_REPS=1`
   para no repetir innecesariamente los diez arranques fríos/tibios ya
   medidos en el paso 2) con `PERF_BENCH_ARGS="sweep grande"`, sobre el
   proyecto de 20 000 archivos (≈3 min). Es la medición que E6-C había
   dejado pendiente (§6 de esa corrida) por no ampliar su propio alcance.
4. **Este documento**, con los números finales.

Duración total de la medición (sin contar la escritura de este documento):
≈43 minutos. Ninguna corrida falló (`inner.status` en `0` las cuatro
veces; sin líneas `error` en ningún `results.jsonl`). No hizo falta ningún
arreglo en `tools/perf/` ni en `packaging/`: los scripts corrieron sin
cambios. Los perfiles se borraron al terminar cada llamada a `run.sh` (lo
hace el propio script); el corpus, los resultados y el binario extraído
del tarball vivieron en una carpeta de trabajo fuera del repositorio y se
borraron al cerrar esta subetapa. `docker ps` y `pgrep -af
"cargo|rustc|cincel|zed|antigravity|sway"` vacíos al terminar.

**Desviación anotada:** por indicación explícita para esta subetapa no se
corrió `cargo sweep` (regla general de §10.1.5 de la spec), para no gastar
tiempo en un paso de limpieza de caché que no aporta a la medición ni a la
verificación de paquetes; no afecta ningún resultado de este documento.

## 9. Correcciones (E6-G)

**Resumen llano.** El repaso del turno ya no traba la ventana: los 200
cambios de un turno se revisan en tramos cortos en vez de todos juntos.
En reposo Cincel ya no se despierta solo (antes lo hacían unas 90 veces por
segundo tres vigilantes de archivos y el parpadeo del cursor aunque
estuviera quieto). El tamaño del binario no se pudo bajar a 60 MB sin
quitar funciones o perder velocidad: queda para que decida el autor (el
2026-09-29 el autor ajustó la meta a ≤ 80 MB, que el binario final sí
cumple: §1, §2).

**Cómo se midió.** `cincel --bench` (binario de release) en la sesión real
de COSMIC (pantalla de 60 Hz), no en el escritorio invisible del banco: los
números de tecleo y apertura quedan atados al refresco de 60 Hz y el
arranque incluye la GPU y el compositor reales, así que solo sirven para
comparar antes y después entre sí, no con la tabla de §4. Corpus de
`cincel-perf corpus` (`medio/` 2 009 archivos, `grande/` 20 000). "Antes" es
el binario de E6-C; "después", el de E6-G. La medición oficial la repite
E6-K con los paquetes (§2, §4, §5).

### 9.1 Tabla antes/después

| Meta | Medición | Antes | Después | ¿Cumple? |
|---|---|---|---|---|
| M11 | `sweep medio`: bloqueo máx. foto / vigilante / repaso | 7,1 / **31,6** / 8,7 ms | 1,8–5,8 / 6,0–6,9 / 4,5–5,8 ms (3 corridas) | **sí** |
| M11 | `sweep grande` (20 000 archivos): foto / vigilante / repaso | 6,1 / **41,3** / 12,9 ms | 5,0–6,5 / 5,7–6,2 / 5,1–6,6 ms (3 corridas) | **sí** |
| M11 | `all` (tramo `sweep`, con `cinco-mil.rs` abierto en una pestaña): foto / vigilante / repaso | 7,1 / **31,6** / 8,7 ms | 5,8–7,2 / **10,5–13,3** / 4,9–7,9 ms | no, en el tramo del vigilante (ver 9.2) |
| M11 | tramo más largo que el repaso le toma al hilo principal (`watch_adoption.max_slice_ms`, `sweep_max_slice_ms`) | un solo tramo de 26–36 ms | 3,1–3,3 ms | sí |
| M8 | `idle` (60 s tras 30 s): CPU / cuadros | 0,267 % (en `all`), 0,217–0,483 % (suelto) / 0 | 0,000–0,017 % / 0 | **sí** |
| M8 | despertares del proceso en reposo | ≈ 93 por segundo | ≈ 2 por segundo | — |
| M9 | tamaño del binario | 74,9 MB (78 505 968 bytes) | 74,9 MB (78 545 200 bytes) | **no** (ver 9.4) |
| M1 | `startup` total (sesión real) | 2 977,6 ms | 1 726,5 ms | igual o mejor (el tramo config→ventana es de la GPU/compositor y varía entre corridas) |
| M3 | `open` `cinco-mil.rs` p50 / p95 | 12,0 / 58,6 ms | 12,0 / 39,3 ms | no empeoró |
| M4 | `typing` `cinco-mil.rs` p50 / p95 / máx. | 16,3 / 19,2 / 25,2 ms | 16,3 / 18,9 / 22,0 ms | no empeoró (atado a 60 Hz en la sesión real) |

### 9.2 M11: el tramo del vigilante de archivos

**Causa.** Con `CINCEL_TRACE_TIMINGS=1` y tramos nuevos (`watch_batch`,
`review_watch_batch`, `tree_rebuild`) se vio que el vigilante de
`cincel-project` entregaba bien (un lote de 200 eventos le cuesta al
proyecto 0,3–0,5 ms) y que lo caro era la revisión: `Review::on_files_changed`
adoptaba los 200 archivos de golpe (leer, comparar con la foto, calcular el
diff) en un solo tramo de 17–36 ms. Otros dos aportes: el propio banco
escribía los 200 archivos desde el hilo principal (12–17 ms que no son del
editor: el agente escribe desde su propio proceso) y, al cerrar el repaso,
soltar la foto del proyecto grande (271 MB de copias) le costaba 5 ms al
hilo principal.

**Qué se cambió.**
- `review_snapshot.rs`: cola del vigilante (`WatchQueue`). Un lote de más de
  `WATCH_INLINE_PATHS` (4) rutas se encola tal como llega, sin mirarlo, y se
  expande y adopta en tramos de a lo sumo `WATCH_SLICE` (3 ms), como el
  repaso; un lote chico (un guardado, una edición) se adopta en el acto, como
  antes. El repaso final adopta primero lo que siga en la cola, así que el
  turno no se cierra antes. Si Cincel escribe un archivo cuyo cambio del
  agente sigue en la cola, ese cambio se adopta primero (contra la foto),
  para que no se pierda en la base.
- `SWEEP_SLICE` bajó de 4 a 3 ms: GPUI pinta inmediatamente después de una
  actualización que pide cuadro (3–5 ms con un archivo grande abierto), y un
  tramo de 4 ms más ese cuadro llegaba a 8,2 ms.
- La foto del turno se suelta en el ejecutor de fondo (`photo::release`).
- `bench.rs`: `sweep` escribe los cambios del "agente" en el ejecutor de
  fondo, y su salida agrega `watch_adoption` y `sweep_max_slice_ms`.

**Lo que no llega (en `all`).** En `all`, el tramo `sweep` corre con
`cinco-mil.rs` abierto en una pestaña, y está entre los 200 archivos que
cambia el "agente". El bloqueo de 10,5–13,3 ms del tramo del vigilante ya no
es de la revisión (su parte en ese momento es 0,09 ms): son el proyecto
recargando la pestaña abierta (1,5–2,3 ms), el editor reconstruyendo su
vista del archivo de 5 000 líneas (≈ 1,4 ms) y el cuadro que GPUI pinta
enseguida (4–5,5 ms de dibujo más ≈ 1 ms de presentación), todo seguido en
el hilo principal. Bajarlo pide sacar del hilo principal la recarga de los
archivos abiertos o abaratar el primer cuadro después de una recarga
(`cincel-project`/`cincel-editor`); queda anotado para después de E6-G.
**E6-K confirma que este residuo sigue presente** (§5, §6) con el binario
empaquetado: 8,7 / 10,1 / 4,8 ms; la receta exacta de M11 (`sweep` solo,
sin pestañas abiertas, sobre el corpus grande) sí cumple con margen (§4).

**Tests.** `crates/cincel-workspace/src/watch_slice_tests.rs`: la ráfaga se
encola y se adopta en tramos (con presupuesto 0, un archivo por tramo);
ningún tramo pasa de 8 ms; el repaso adopta lo que quedó en la cola; una
escritura de Cincel adopta primero el cambio encolado. Los de
`snapshot_review_tests.rs` y `sweep_background_tests.rs` pasan sin cambios.

**Hallazgo aparte (no es M11).** En `review-1mb`, adoptar el cambio del
agente sobre `un-mega.rs` (1 MB) le toma 31 ms al hilo principal, y sobre
`cincuenta-mil.rs` 57–74 ms: es un solo archivo, así que no se puede partir
en tramos; habría que calcular ese diff en el ejecutor de fondo. M5 y M6
siguen cumpliendo (segmentos visibles en 64 ms y 98 ms; con el binario
empaquetado, 55,0 ms y 93,4 ms respectivamente, §5).

### 9.3 M8: CPU en reposo

**Causa.** Contando los despertares por hilo (`/proc/<pid>/task/*/status`)
y las llamadas al sistema (`strace -f -tt` en los 40 s de reposo):
- tres hilos `notify-rs debouncer loop` de `notify-debouncer-full` (el
  vigilante del proyecto, el de `.git` y el de la configuración) dormían
  25, 25 y 75 ms en un bucle **sin fin**, pasara algo o no: ≈ 93 despertares
  por segundo y tres cuartos de la CPU en reposo;
- la tarea de parpadeo del cursor de cada editor seguía despertando al hilo
  principal cada 500 ms aunque el cursor ya no parpadeara (se queda fijo a
  los 5 s sin escribir).

**Qué se cambió.**
- Crate nuevo `crates/cincel-watch` (`Debouncer`): vigila con `notify` y
  arma los lotes con un hilo que se bloquea en su canal mientras no pasa
  nada; el primer evento abre una ventana del largo pedido y todo lo que
  llega en ella sale junto (la ventana no se corre, así que un chorro
  continuo de eventos igual sale cada ventana). Empareja los renombres que
  inotify ya informa de a pares y descarta eventos repetidos. Lo usan los
  tres vigilantes; `notify-debouncer-full` ya no se usa en ningún crate.
- `cincel-editor` (`view.rs`): la tarea de parpadeo termina cuando el cursor
  se queda fijo (5 s sin escribir, o `cursor_blink` apagado) y la vuelve a
  lanzar la próxima tecla. El reloj del parpadeo pasa a ser el del ejecutor
  (el real fuera de los tests), para poder probarlo.

**Lo que sigue despertando en reposo** (lista cerrada): el hilo
`blocking-1` del crate `blocking` (de las dependencias de GPUI y el portal
del escritorio), que espera trabajo con un tope de 500 ms (≈ 2 por
segundo, sin costo medible); el autoguardado de la conversación del chat
cada 30 s; y, en `--bench idle`, el propio temporizador del banco. **E6-K
mide 0,00 % de CPU en reposo con el binario empaquetado** (§4, §5): dentro
de la meta de ≤ 0,1 %.

**Tests.** `crates/cincel-watch/src/tests.rs` (`nothing_wakes_up_at_rest`:
el hilo de lotes no se despierta en 400 ms sin eventos, ni después de una
ráfaga; lotes, renombres emparejados, repetidos) y
`crates/cincel-editor/src/idle_timer_tests.rs` (la tarea de parpadeo termina
en reposo, la tecla la relanza, y con el parpadeo apagado no queda
ninguna).

### 9.4 M9: tamaño del binario

**Qué pesa** (`size -A` y `cargo bloat --release -p cincel --crates`, sobre
78,5 MB de archivo): `.text` 41,0 MB, `.rodata` 20,7 MB (de los cuales 13,0 MB
son las tablas de las 18 gramáticas de tree-sitter y ≈ 3,5 MB las fuentes
embebidas), `.symtab` + `.strtab` 8,6 MB (los nombres de funciones que D11
decidió conservar), `.eh_frame` + `.gcc_except_table` 4,5 MB (desenrollado
de pánicos), `.rela.dyn` 2,0 MB. En `.text` los que más pesan son `gpui`
4,3 MB, la biblioteca estándar 4,1 MB, código C (gramáticas, `ring`) 4,4 MB,
`zvariant` + `zbus` 3,4 MB (portal del escritorio, llavero y
accesibilidad), `naga` 1,3 MB y `cincel-workspace` 1,6 MB.

**Palancas revisadas.**
- `lto = "fat"` y `codegen-units = 1`: ya estaban en `[profile.release]`.
- `strip = true`: bajaría 8,6 MB (a ≈ 69,9 MB), sin costo en velocidad, pero
  las trazas de un pánico perderían los nombres de funciones (la línea y el
  archivo del pánico sí se conservan). No alcanzaba la meta original y
  contradice D11: no se aplicó; quedó como propuesta para el autor.
- `panic = "abort"`: descartado. Las tareas de `tokio` que hablan con los
  agentes (ACP) dependen del desenrollado (`tokio` las aísla con
  `catch_unwind`): con `abort`, un pánico en una conexión cerraría todo el
  editor con los archivos sin guardar. Ahorraría ≈ 4,5 MB de tablas y algo de
  código.
- Features de dependencias: las 18 gramáticas se usan todas (son lenguajes
  del producto); los formatos de imagen (`exr`, `tiff`, `webp`…) los activa
  `gpui-pre` mismo y las features de Cargo solo se suman, no se pueden
  quitar desde Cincel; `zbus` viene de `rfd`/`ashpd`, `oo7` y `accesskit`.
- Duplicados (`cargo tree -d`): `skrifa`/`read-fonts`, `png`, `sha2`,
  `hashbrown`, `notify` 7… vienen todos de árboles de terceros (gpui-kit y
  gpui-pre piden versiones distintas); no se resuelven con features.
- `opt-level = "s"`/`"z"`: descartado por la regla de no ceder rendimiento.

**Resultado de E6-G.** 78 545 200 bytes (74,9 MB), igual que antes más
39 KB de código nuevo: no alcanzaba la meta original de 20–60 MB sin quitar
funciones o perder velocidad. **Decisión del autor (2026-09-29):** la meta
de M9 se ajusta a ≤ 80 MB (`01-producto.md §2`, spec 08 §3.1), en vez de
aceptar `strip`/`panic = "abort"` o quitar funciones. **El binario final
que empaqueta E6-H y mide E6-K pesa 79 410 024 bytes (75,7 MB)**: cumple la
meta ajustada (§1, §2, §4).

### 9.5 Fuente de la interfaz

El registro del arranque decía que `.SystemUIFont` se resolvía a
"DejaVu Sans" (en esta máquina, "Ubuntu"): es la sonda de gpui-kit al
iniciarse, antes de que Cincel aplique su tema. `theme.rs` ya nombraba la
familia de la interfaz en el tema de gpui-kit; ahora la cadena empieza por
`fonts::UI_FAMILY`/`fonts::BUFFER_FAMILY`, nombra también la familia
monoespaciada de gpui-kit (antes quedaba la que eligiera la sonda) y lo deja
escrito en el registro: `tipografía de la interfaz ui=Inter mono=…` (la
monoespaciada es la de `buffer_font_family` si está instalada; por defecto
JetBrains Mono). Verificado con `cincel --smoke-test`.

## 10. Resultado final: cumplimiento de M1 a M11

Juzgado sobre el binario que se distribuye (§2), con la medición de §4–§5.

| Meta | ¿Cumple? | Número |
|---|---|---|
| M1 (arranque en frío → primer cuadro) | **sí** | 303,0 ms (meta < 400 ms) |
| M2 (arranque tibio) | informativa | 228,9 ms |
| M3 (abrir 5 000 líneas) | **sí** | 31,7 ms (meta < 50 ms) |
| M4 (tecla → pantalla, 5 000 líneas) | **sí** | p50 5,9 ms (meta < 16 ms) |
| M5 (archivo de 1 MB: apertura, scroll, tecleo) | **sí** | apertura 116,2 ms; scroll p95 8,3 ms; tecleo p50 6,1 ms |
| M6 (archivo de 50 000 líneas) | **sí** | apertura 35,2 ms; scroll p95 9,5 ms; tecleo p50 6,3 ms |
| M7 (memoria en reposo) | **sí** | 138 MB de RSS del árbol (meta < 300 MB) |
| M8 (CPU en reposo) | **sí** | 0,00 % y 0 cuadros (meta ≤ 0,1 %) |
| M9 (tamaño del binario) | **sí** | 75,7 MB (meta ajustada ≤ 80 MB) |
| M10 (buscador con 50 000 rutas) | sí, con salvedad de escala (§6) | p95 16,7 ms sobre 2 009 archivos (meta < 50 ms) |
| M11 (repaso final del turno, proyecto grande) | **sí** | máx. 7,4 ms (meta ≤ 8 ms; receta exacta de la spec, `sweep` sobre 20 000 archivos) |

Las dos únicas salvedades (M10 a menor escala que la nominal, y el residuo
ya conocido de M11 en el escenario combinado `all`) están documentadas en
§6, con la causa y por qué no se ampliaron en esta subetapa.
