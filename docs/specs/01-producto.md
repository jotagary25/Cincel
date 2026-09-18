# Spec 01: Producto

Estado: v1.0 (2026-09-17). Fuente de verdad sobre **qué** hace Asteroid. Las decisiones de fondo están en `docs/00-sintesis-y-decisiones.md`; la investigación en `docs/research/`.

## 1. Qué es

Asteroid es un editor de código de escritorio para Linux, minimalista y liviano, con un chat integrado que conecta agentes de programación (Claude Code, Codex, Gemini CLI, OpenCode y cualquier otro compatible con ACP) usando las suscripciones que el usuario ya tiene. Su rasgo distintivo: **todo cambio que hace un agente se muestra dentro del editor, segmento por segmento, y el usuario acepta o rechaza cada segmento o cada línea**. Los cambios nunca se revisan en el chat.

Público objetivo: el propio autor y desarrolladores con perfil similar. Plataforma: Linux, Wayland primero (X11 como respaldo). Máquina de referencia: Pop!_OS 24.04 con escritorio COSMIC.

## 2. Principios

1. **Liviano y rápido.** Arranque bajo 400 ms en frío; binario entre 20 y 60 MB; redibujar solo cuando algo cambia.
2. **Aspecto y sensación de Zed.** Densidad compacta, pocos adornos, tipografía cuidada.
3. **El editor es el lugar de la verdad.** El chat narra; el editor muestra.
4. **El agente escribe primero, el usuario decide después.** Los cambios llegan al disco de inmediato para que el agente pueda compilar y probar; cada segmento queda pendiente de aprobación y es reversible de forma quirúrgica.
5. **Sin sorpresas destructivas.** Nada se borra sin que el usuario lo pida; "rechazar" siempre tiene un deshacer; los cambios pendientes sobreviven a cerrar la app.
6. **Nada de API keys.** Los agentes se autentican con la sesión de sus propios CLIs.

## 3. Alcance de la versión 1

### Incluido
- Abrir una carpeta (argumento de línea de comandos o diálogo del sistema).
- Árbol de archivos con estado de "modificado por el agente" y contadores `+N −M`.
- Pestañas de archivos; editor con: colores de sintaxis (tree-sitter) para ~15 lenguajes, edición con cursor único, selección con teclado y mouse, deshacer/rehacer, buscar en el archivo, guardar, indentación básica, números de línea, ajuste de línea opcional, entrada de texto internacional (acentos, dead keys, IME).
- Chat con agentes ACP: elegir agente, sesión nueva, streaming de respuestas en markdown, llamadas a herramientas visibles y plegables, pedidos de permiso, plan del agente, selector de modo/modelo/esfuerzo, comandos slash del agente, cancelar.
- Revisión de cambios en el editor: segmentos rojo/verde, aceptar/rechazar por segmento, por línea, por archivo y por turno; navegación entre cambios; barra flotante; panel de revisión con la lista de archivos; deshacer el último rechazo; persistencia entre sesiones; informe de rechazos al agente en el siguiente mensaje.
- Ajustes en JSON (con comentarios) y mapa de teclas configurable; tema oscuro y claro.

### Excluido de v1 (candidatos para v2)
- Autocompletado y diagnósticos LSP.
- Terminal integrado (el login de agentes se delega al terminal del sistema).
- Panel de git, commits, blame. Solo se lee el estado de git para colorear el árbol.
- Multi-cursor, plegado de código, minimapa.
- Búsqueda en todo el proyecto, paleta de comandos, buscador de archivos difuso.
- Extensiones o plugins.
- Comentarios sobre segmentos para dialogar con el agente (estilo Antigravity).
- Soporte para macOS y Windows.

## 4. Flujos de usuario

### F1. Abrir un proyecto
1. `asteroid ~/proyecto` o `asteroid` y luego "Abrir carpeta" (diálogo del portal del sistema).
2. Aparece el árbol a la derecha, el chat a la izquierda y un editor vacío al centro con la pista "Abrí un archivo o hablale al agente".
3. Al reabrir la app, vuelve al último proyecto con las mismas pestañas.

### F2. Editar un archivo
1. Clic en el árbol abre una pestaña. Doble clic la fija; un solo clic la abre en modo previsualización (como Zed).
2. El título de la pestaña muestra un punto cuando hay cambios sin guardar. `Ctrl+S` guarda.
3. `Ctrl+F` abre la búsqueda en el archivo, con coincidencias resaltadas y `Enter`/`Shift+Enter` para navegar.

### F3. Hablar con un agente
1. Arriba del chat, selector de agente. La lista viene del registro oficial de ACP más lo que el usuario agregue en ajustes. Los agentes cuyo ejecutable no está instalado aparecen atenuados con la pista para instalarlos.
2. Al elegir un agente, Asteroid lo lanza. Si el agente pide autenticación, muestra una tarjeta con el comando a ejecutar en un terminal y un botón "Ya me autentiqué" que reintenta.
3. El usuario escribe y envía con `Enter` (`Shift+Enter` inserta salto de línea). Puede mencionar archivos con `@` y arrastrar archivos desde el árbol.
4. La respuesta se muestra en streaming. Cada herramienta que usa el agente aparece como una tarjeta plegada con título, estado y, al desplegarla, el detalle. Las tarjetas de edición muestran el archivo y `+N −M` y al hacer clic llevan al primer segmento pendiente de ese archivo en el editor.
5. Si el agente pide permiso, aparece una tarjeta con los botones que ofrece el agente (por ejemplo "Permitir", "Permitir siempre", "Rechazar"). El chat queda a la espera.
6. `Esc` o el botón de detener cancelan el turno actual.
7. Abajo del chat: selector de modo, modelo y esfuerzo, solo con las opciones que el agente anuncie.

### F4. Revisar los cambios de un agente
1. Mientras el agente escribe, los archivos tocados se marcan en el árbol y en las pestañas con un indicador de actividad. Los botones de aceptar/rechazar están deshabilitados hasta que termina el turno.
2. Al terminar, en cada archivo tocado los segmentos cambiados se ven en el editor: líneas eliminadas en rojo (intercaladas, solo lectura) y líneas añadidas en verde. Si el archivo no estaba abierto, sigue cerrado pero aparece en el árbol y en el panel de revisión.
3. Sobre cada segmento, un botón flotante `Aceptar | Rechazar`. Al pasar el mouse sobre una línea del segmento aparecen `+` y `−` para decidir solo esa línea.
4. Abajo a la derecha del editor, una barra flotante: `Aceptar Ctrl+Enter · Rechazar Ctrl+Backspace · ↑ Alt+K ↓ Alt+J · 3/7 cambios`.
5. Aceptar quita el color y deja el texto nuevo. Rechazar restaura el texto original de ese segmento y guarda. Ambos avanzan al siguiente cambio si el ajuste "saltar al siguiente" está activo (por defecto sí).
6. `Ctrl+Shift+Enter` / `Ctrl+Shift+Backspace` decide todo el archivo. `Ctrl+Alt+Enter` / `Ctrl+Alt+Backspace` decide todo el turno.
7. `Ctrl+Shift+R` abre el panel de revisión: lista de archivos con `+N −M`, botones "Aceptar todo" y "Rechazar todo", y clic para saltar al archivo.
8. `Alt+Shift+U` deshace el último rechazo (con un aviso emergente al rechazar que lo recuerda).
9. El usuario puede escribir dentro de un segmento pendiente. Lo que escriba no cuenta como cambio del agente y el segmento se reacomoda.
10. `Ctrl+Z` deshace la edición del agente como un solo paso (el segmento desaparece porque el texto vuelve al original). `Ctrl+Z` no deshace un "aceptar".
11. Al cerrar y reabrir Asteroid, los segmentos pendientes siguen ahí, siempre que el archivo no haya cambiado por fuera. Si cambió por fuera, se descarta la revisión de ese archivo y se avisa.
12. En el siguiente mensaje al agente, Asteroid adjunta un resumen de lo que el usuario rechazó o modificó de su propuesta.

### F5. Autonomía del agente
Ajuste con tres valores, elegible por sesión desde el chat:
- **Revisar después** (predeterminado): el agente escribe sin pedir permiso; todo queda pendiente de aprobación en el editor.
- **Pedir antes**: cada edición pide permiso antes de escribirse, mostrando el diff propuesto en el chat. Además queda pendiente en el editor.
- **Aplicar siempre**: sin pedir permiso y aceptando automáticamente. Útil para tareas mecánicas.
Independientemente del valor, una lista de patrones de archivos sensibles (por defecto `.env*`, `**/.git/**`, `Cargo.lock`, `package-lock.json`) pide siempre permiso antes de escribirse.

## 5. Requisitos no funcionales

| Requisito | Meta |
|---|---|
| Arranque en frío hasta ventana visible | < 400 ms en la máquina de referencia |
| Abrir archivo de 5 000 líneas | < 50 ms hasta primer pintado |
| Escribir una letra | < 16 ms hasta repintar |
| Archivo de 1 MB | edición fluida; revisión inline activa |
| Archivo > 2 MB o > 50 000 líneas | edición fluida; revisión solo por archivo (sin segmentos inline) |
| Memoria en reposo con proyecto mediano | < 300 MB |
| Uso de CPU en reposo | 0% (sin redibujo continuo) |
| Sin GPU Vulkan | arranca con OpenGL; sin GPU alguna, arranca con renderizado por software si se pide con `ASTEROID_ALLOW_SOFTWARE_GPU=1` |

## 6. Compatibilidad con agentes (v1)

| Agente | Cómo se lanza | Autenticación |
|---|---|---|
| Claude Code | `npx -y @agentclientprotocol/claude-agent-acp@<versión del registro>` | sesión de `claude` en la máquina |
| Codex | `npx -y @agentclientprotocol/codex-acp@<versión>` | sesión de `codex` (ChatGPT) |
| Gemini CLI | `npx -y @google/gemini-cli@<versión> --acp` | sesión de `gemini` |
| OpenCode | `opencode acp` | la propia de OpenCode |
| Cualquier otro del registro ACP | según el registro | según anuncie |

Requisitos del sistema: Node ≥ 22 para los agentes distribuidos por npm. Asteroid detecta si falta y lo indica.

## 7. Fuera de alcance explícito
- Asteroid no llama a ninguna API de modelos directamente ni guarda credenciales.
- Asteroid no envía telemetría.
