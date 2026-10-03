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
Estado: **cerrada** (2026-09-29). Construcción, verificación y desviaciones en `docs/etapas/etapa-5.md`.
Spec: `07-etapa5-productividad.md` (fuente de verdad; reemplaza a `docs/etapas/pendientes-etapa-5.md`). Buscador rápido de archivos (`Ctrl+P`, `frizbee`); pantalla de configuración como pestaña (`Ctrl+,`) que escribe el mismo `settings.json` conservando comentarios, con la gestión de conexiones y las actualizaciones de adaptador y Node; modal de atajos con buscador (botón en la barra de estado, `F1`); git en el margen del editor (añadido, modificado, eliminado contra HEAD, en su propia columna y sin cambiar el ancho del margen); paneles con foco (`Ctrl+Shift+A` / `Ctrl+Shift+E`) y rueda de foco (`Ctrl+L`); menú en la barra de título (con `Ctrl+N` = nuevo archivo, y salir o cerrar la ventana preguntando por archivos sin guardar) y botones de paneles, sin que nada mueva el texto del editor; y cinco deudas: motivo de `AuthRequired`, buscar y reemplazar con filas fantasma, cancelar descargas de verdad, actualizar adaptador y Node, autoguardado tras una pausa.
**Comprobación manual**: la lista de `07-etapa5-productividad.md` §14.

## Etapa 6: rendimiento, distribución y publicación (cierre = versión 1.0)
Estado: **cerrada** (2026-09-29). Construcción, verificación, decisiones y
desviaciones en `docs/etapas/etapa-6.md`. **Versión 1.0 lista para
publicar**: al autor le quedan crear el repositorio público
(`docs/publicacion.md`) y la lista de comprobación manual
(`docs/etapas/etapa-6.md`, o `08-etapa6-cierre-1-0.md` §12).
Spec: `08-etapa6-cierre-1-0.md` (fuente de verdad del alcance).
- **Rendimiento** medido contra `01-producto.md §5` (arranque en frío, abrir 5 000 líneas, latencia de tecleo, archivos de 1 MB y de 50 000 líneas, memoria y CPU en reposo sin redibujo continuo) y **comparado con Zed y Antigravity** (VS Code cuando esté instalado) con un banco de medición reproducible que no instrumenta las apps ni captura la sesión del autor; mediciones internas con `cincel --bench`; resultados en `docs/rendimiento.md`; corrección de lo que no cumpla.
- **Instalador liviano**: tarball + `install.sh` y `.deb` con `cargo-deb`, compilados en GitHub Actions sobre `ubuntu-22.04`, verificados en contenedores limpios 22.04 y 24.04; icono propio; versión 1.0.0 y changelog; los agentes se siguen descargando al conectar.
- **Repositorio público**: lo hace el autor con la guía `docs/publicacion.md`. README en inglés con sección en español y capturas, `CONTRIBUTING`, `CODE_OF_CONDUCT`, plantillas, CI con fmt, clippy `-D warnings`, tests (siempre con las features `cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`) y `cargo deny`, y release que arma los dos paquetes; control de datos personales.
- **Documentación de usuario** en español (`docs/usuario/`): instalación, primer arranque, conexiones, revisión de cambios, atajos, ajustes, solución de problemas.
- **Fuentes Inter y JetBrains Mono embebidas** en el binario (licencia OFL).
- **Deudas**: tope de la foto en la configuración, binarios persistidos y con deshacer, repaso final en segundo plano, teclas de búsqueda en el keymap por defecto, test del buscador con 50 000 rutas y los tests que faltaban.
**Comprobación manual**: la lista de `08-etapa6-cierre-1-0.md` §12 (instalar desde el tarball y desde el `.deb` y repetir lo esencial de las etapas 1 a 5).

## Etapa 7: conexiones más limpias, imágenes en el chat y comentarios para el agente (cierre = versión 0.2.0)
Estado: **cerrada** (2026-09-30), con una **ronda 2** de correcciones y mejoras tras la prueba del autor (spec `10-etapa7-ronda2.md`, 2026-10-01) dentro de la misma 0.2.0. Construcción, verificación, desviaciones y lista de comprobación manual en `docs/etapas/etapa-7.md`. **Versión 0.2.0.**
Spec: `09-etapa7-conexiones-imagenes-comentarios.md` (fuente de verdad del alcance).
- **Conexiones**: el menú "Conectar" y el botón del encabezado del chat muestran solo el nombre de la conexión, el tipo de agente con su icono y la insignia de estado; correo, plan y "Usado hace…" quedan solo en Configuración → Conexiones. Los menús desplegables (los dos del encabezado del chat y los demás) se cierran solos con clic fuera, cambio de foco, `Esc` o la ventana sin foco. (Corregido por la ronda 2: el encabezado muestra solo el nombre y una insignia del estado de la conexión, y las filas del menú son de dos líneas.)
- **Imágenes en el chat**: `Ctrl+V`, arrastrar desde el explorador de archivos y botón "Adjuntar"; PNG, JPEG, GIF y WebP, reducidas a 2 000 px de lado, máximo 10 MB; miniaturas con `×`, vista a tamaño completo, bloques `image` de ACP, aviso si el agente no las acepta, guardadas con la conversación.
- **Comentarios sobre segmentos y selecciones**: botón "Comentar" junto a Aceptar y Rechazar, "Comentar selección" (menú contextual y `Ctrl+Shift+M`), caja intercalada entre las líneas sin mover el código, etiquetas en la caja del chat, envío con el próximo mensaje dentro de `<user_review_feedback>`, tarjetas en el mensaje enviado, persistencia con la revisión y contadores.
**Comprobación manual**: la lista de `09-etapa7-conexiones-imagenes-comentarios.md` §11 (incluye Claude, Codex y Antigravity reales atendiendo un comentario y una imagen).
- **Ronda 2** (correcciones y mejoras tras la prueba del autor, dentro de la misma 0.2.0, antes de publicarla): spec `10-etapa7-ronda2.md` (encabezado y menú del chat, imágenes arriba del texto, "Pensando…" en la conversación, seguir el final y flecha "ir al final", bloques de código largos plegados, margen al final del editor, ocho mejoras al escribir y limpieza al guardar); su lista de comprobación manual es la de su §16.

## Etapa 8: terminal integrado (pospuesta)
Decisión del autor (2026-09-30): el terminal integrado tendrá su propia etapa, después de la 7 (ya cerrada). Sin especificar todavía; se escribirá su propia spec antes de construir.

## Después de v1 (ideas ordenadas)
Lo que quedó anotado al cerrar la Etapa 7 (detalle en `docs/etapas/etapa-7.md`
"Después de la 0.2.0"): listar los comentarios pendientes en el panel
"Revisar todo"; recodificar los GIF animados reducidos conservando la
animación (hoy se manda el primer cuadro con aviso); adjuntar una imagen
arrastrándola desde el árbol de Cincel (hoy se hace con "Mencionar en el
chat", porque gpui-kit no ofrece arrastre desde el árbol); y el límite
conocido de que un modelo de Codex sin imágenes responde con error aunque el
adaptador las anuncie.

Lo que quedó anotado al cerrar la Etapa 6 (detalle y motivo en
`docs/etapas/etapa-6.md` "Después de la 1.0"): llevar los colores de git de
`GitGutterColors` a `EditorTheme`; el residuo de ≈10 ms de M11 en el
escenario combinado (recargar una pestaña abierta durante el repaso final
del turno); adoptar el cambio del agente sobre un solo archivo grande (1 MB
o más) sin bloquear el hilo principal; bajar el binario por debajo de los
75,7 MB actuales si hiciera falta; comprobación cruzada del arranque en
COSMIC (`tools/perf/run.sh cosmic`) y medir el buscador de archivos sobre el
corpus grande (50 000 rutas reales, no solo el test unitario).

Ideas de producto, sin fecha: LSP (completado, diagnósticos, ir a
definición) · autenticación de agentes dentro de la app (el terminal
integrado pasó a la Etapa 8; los comentarios sobre segmentos y las imágenes en
el chat se cerraron en la Etapa 7) ·
multi-cursor y plegado · búsqueda en proyecto · paleta de comandos
(`Ctrl+Shift+P`) · OpenCode y agentes con API key o gateway · panel de git ·
ACP v2 (rename/delete/binarios) · macOS.
