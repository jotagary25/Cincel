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

## Etapa 4: pulido y distribución
Git en el árbol y en el gutter, buscador de archivos (`frizbee`), paleta de comandos, empaquetado (`dist` → tarball + `install.sh`; `cargo-deb`), rendimiento (perfil de arranque y de tecleo), documentación de usuario.
**Comprobación manual**: instalar desde el tarball en una máquina limpia (o contenedor) y repetir las comprobaciones de las etapas 1 a 3.

## Después de v1 (ideas ordenadas)
LSP (completado, diagnósticos, ir a definición) · terminal integrado y autenticación de agentes dentro de la app · comentarios sobre segmentos para dialogar con el agente · multi-cursor y plegado · búsqueda en proyecto · panel de git · ACP v2 (rename/delete/binarios) · macOS.
