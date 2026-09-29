# Spec 05: Plan de etapas y método de trabajo

## Método (desarrollo guiado por especificaciones)
1. Las specs de `docs/specs/` son el contrato. Un subagente recibe una spec de módulo (o una parte), construye contra sus criterios de aceptación, con tests, y entrega un resumen de qué cumple y qué no.
2. El orquestador integra, ejecuta `cargo fmt --check && cargo clippy -D warnings && cargo test && cargo deny check licenses`, prueba en la máquina de referencia y corrige o devuelve al subagente.
3. Cada etapa termina con una **lista de comprobación manual** para el autor. Si pasa, el autor hace el commit. Los subagentes y el orquestador no hacen commits.
4. Toda desviación de una spec se anota en la spec (sección "Desviaciones") antes de cerrar la etapa; las specs se versionan con el código.
5. Nada de credenciales, tokens ni rutas de sesiones de agentes en el repo. Los ajustes del autor viven en `~/.config/cincel/`, fuera del repo.

## Etapa 0: cimientos y experimentos de riesgo
Objetivo: comprobar que las tres apuestas técnicas funcionan en la máquina de referencia antes de construir en serio.
- Workspace Cargo con todos los crates (vacíos o mínimos), `rust-toolchain.toml`, `deny.toml`, `README.md`.
- **E0-a Ventana**: `cincel` abre una ventana GPUI con el dock de tres paneles vacíos, tema oscuro, barra de estado, en Wayland/COSMIC. Sin editor aún.
- **E0-b Agente**: `cincel-acp` con el ejemplo `chat` que lanza `claude-agent-acp` y `codex-acp` vía `npx`, negocia, crea sesión, envía un prompt y muestra la respuesta en streaming por consola; agente falso para tests.
- **E0-c Fila fantasma**: `cincel-editor` renderiza un archivo de texto con colores de sintaxis y una fila fantasma roja de solo lectura intercalada entre dos líneas reales (hunk simulado), con scroll y cursor.

**Comprobación manual**: (1) `cargo run` abre la ventana, se redimensiona y cierra bien en COSMIC; (2) el ejemplo `chat` responde usando tus sesiones de Claude y de Codex sin pedir claves; (3) la ventana de prueba del editor muestra la fila roja, el cursor no puede escribir en ella pero sí seleccionarla y copiarla.
**Si E0-c falla** tras dos intentos: se evalúa el plan B (vista de revisión de solo lectura) antes de seguir.

## Etapa 1: editor usable
`cincel-text`, `cincel-syntax`, `cincel-project`, `cincel-settings`, `cincel-editor` (sin revisión), `cincel-workspace` (árbol, pestañas, barra de estado, atajos, tema, layout persistido).
**Comprobación manual**: abrir un proyecto Rust real; navegar el árbol; abrir, editar (con acentos), buscar, guardar, deshacer; cerrar y reabrir con las mismas pestañas; cambiar el tamaño de fuente en `settings.json` en caliente.

## Etapa 2: chat y agentes
`cincel-acp` completo, `cincel-chat`, integración en el workspace (menciones, permisos, autonomía, autenticación, cancelar, selectores de modo/modelo).
**Comprobación manual**: pedirle a Claude que explique un archivo; pedirle que ejecute `ls`; ver la tarjeta de permiso y responder; cambiar de modelo; cancelar a mitad; repetir con Codex y Antigravity.

## Etapa 3: revisión de cambios
`cincel-review` completo, `DiffTransformMap` y `BlockMap` en el editor, pill, `+`/`−` por línea, barra flotante, panel de revisión, árbol con `+N −M`, persistencia, rebase de ediciones del usuario, informe al agente, diálogo de buffer sucio.
**Comprobación manual**: pedirle a Claude un cambio que toque 3 archivos; ver los segmentos en el editor; aceptar uno, rechazar otro, rechazar una sola línea de un tercero; escribir a mano dentro de un segmento pendiente; cerrar y reabrir Cincel con pendientes; deshacer un rechazo; en el siguiente mensaje comprobar que el agente sabe qué rechazaste.

## Etapa 4: conexiones de agentes y renombrado a Cincel
Spec: `06-etapa4-conexiones-y-cincel.md`. Conexiones aisladas por suscripción (Claude, Codex, Google Antigravity) con perfiles propios, Node privado, adaptadores y binarios descargados al conectar; renombrado a Cincel con migración de carpetas; log a archivo.
**Comprobación manual**: la lista de `06-etapa4-conexiones-y-cincel.md` §11.

## Etapa 5: productividad
Spec: `07-etapa5-productividad.md` (fuente de verdad; reemplaza a `docs/etapas/pendientes-etapa-5.md`). Buscador rápido de archivos (`Ctrl+P`, `frizbee`); pantalla de configuración como pestaña (`Ctrl+,`) que escribe el mismo `settings.json` conservando comentarios, con la gestión de conexiones y las actualizaciones de adaptador y Node; modal de atajos con buscador (botón en la barra de estado, `F1`); git en el margen del editor (añadido, modificado, eliminado contra HEAD, en su propia columna y sin cambiar el ancho del margen); paneles con foco (`Ctrl+Shift+A` / `Ctrl+Shift+E`) y rueda de foco (`Ctrl+L`); menú en la barra de título (con `Ctrl+N` = nuevo archivo, y salir o cerrar la ventana preguntando por archivos sin guardar) y botones de paneles, sin que nada mueva el texto del editor; y cinco deudas: motivo de `AuthRequired`, buscar y reemplazar con filas fantasma, cancelar descargas de verdad, actualizar adaptador y Node, autoguardado tras una pausa.
**Comprobación manual**: la lista de `07-etapa5-productividad.md` §14.

## Etapa 6: rendimiento, distribución y publicación (cierre = versión 1.0)
- **Rendimiento** medido contra `01-producto.md §5` (arranque en frío, abrir 5 000 líneas, latencia de tecleo, archivos de 1 MB y de 50 000 líneas, memoria y CPU en reposo sin redibujo continuo) y corrección de lo que no cumpla.
- **Instalador liviano**: tarball + `install.sh` y `.deb` con `cargo-deb`, compilados en GitHub Actions sobre `ubuntu-22.04`; los agentes se siguen descargando al conectar.
- **Repositorio público**: lo hace el autor; Cincel solo lo guía. README en inglés con sección en español, `CONTRIBUTING`, CI con fmt, clippy `-D warnings`, tests (siempre con las features `cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`) y `cargo deny`.
- **Documentación de usuario** en español: instalación, primer arranque, conexiones, revisión de cambios, atajos, ajustes.
- **Fuentes Inter y JetBrains Mono embebidas** en el binario (licencia OFL).
**Comprobación manual**: instalar desde el tarball y desde el `.deb` en una máquina limpia (o contenedor) y repetir las comprobaciones de las etapas 1 a 5.

## Después de v1 (ideas ordenadas)
LSP (completado, diagnósticos, ir a definición) · terminal integrado y autenticación de agentes dentro de la app · comentarios sobre segmentos para dialogar con el agente · multi-cursor y plegado · búsqueda en proyecto · paleta de comandos (`Ctrl+Shift+P`) · OpenCode y agentes con API key o gateway · panel de git · ACP v2 (rename/delete/binarios) · macOS.
