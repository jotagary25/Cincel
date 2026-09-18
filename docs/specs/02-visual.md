# Spec 02: Visual e interacción

Estado: v1.0. Describe **cómo se ve y se siente** Asteroid. Los valores concretos (colores, tamaños) son los predeterminados; todos son configurables por tema.

## 1. Ventana y layout

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ● ● ●  [chat ▾] [tab: main.rs ●] [tab: lib.rs]        [árbol ▾]  Asteroid │  barra de título integrada (CSD)
├────────────────┬──────────────────────────────────────────┬──────────────┤
│  CHAT          │  src › main.rs › fn main                 │  ÁRBOL       │
│  ┌──────────┐  │  1  use std::io;                         │ ▾ src        │
│  │Claude ▾ │  │  2                                       │   main.rs +3 −1
│  └──────────┘  │  3 - let x = 1;          (rojo, fantasma)│   lib.rs     │
│  transcript    │  3 + let x = 2;          (verde)  [✓|✗]  │ ▸ tests      │
│  ...           │  4  println!("{x}");                     │  Cargo.toml  │
│  ┌──────────┐  │                                          │              │
│  │ mensaje  │  │                    ┌──────────────────┐  │              │
│  └──────────┘  │                    │ ✓ Ctrl+↵ ✗ Ctrl+⌫│  │              │
│ modo·modelo·esf│                    │ ↑Alt+K ↓Alt+J 3/7│  │              │
├────────────────┴──────────────────────────────────────────┴──────────────┤
│ main.rs  Ln 3, Col 5  UTF-8  Rust        Claude · listo     7 pendientes │  barra de estado
└──────────────────────────────────────────────────────────────────────────┘
```

- **Barra de título integrada** (decoraciones del lado del cliente, estilo Zed). Botones de ventana a la izquierda o derecha según el ajuste del sistema; override por ajuste `window.decorations = "client" | "server"`.
- **Chat** a la izquierda, ancho inicial 380 px, mínimo 280, redimensionable, colapsable con `Ctrl+Shift+A` o el botón de la barra de título.
- **Editor** al centro con pestañas arriba y breadcrumb debajo (ruta y símbolo actual cuando haya árbol sintáctico).
- **Árbol** a la derecha, ancho inicial 240 px, colapsable con `Ctrl+Shift+E`.
- **Barra de estado** de 26 px: izquierda posición/codificación/lenguaje; derecha agente activo y estado, cambios pendientes (clic abre el panel de revisión).
- Todo el layout se guarda al cerrar y se restaura al abrir.

## 2. Tema

Tokens de color (tema oscuro predeterminado "Asteroid Dark", inspirado en One Dark de Zed; el tema claro "Asteroid Light" sigue al sistema vía el portal `Settings`):

| Token | Oscuro | Uso |
|---|---|---|
| `bg.app` | `#1e2127` | fondo de paneles |
| `bg.editor` | `#282c34` | fondo del editor |
| `bg.surface` | `#21252b` | tarjetas, inputs |
| `bg.elevated` | `#2c313a` | menús, popovers, barra flotante |
| `border` | `#181a1f` | bordes de paneles |
| `border.focus` | `#528bff` | foco |
| `text` | `#abb2bf` | texto principal |
| `text.muted` | `#5c6370` | secundario, números de línea |
| `text.accent` | `#61afef` | enlaces, activos |
| `cursor` | `#528bff` | caret |
| `selection` | `#3e4451` | selección |
| `diff.deleted.bg` | `#e06c75` al 18% | fondo de línea eliminada |
| `diff.deleted.word` | `#e06c75` al 38% | palabra eliminada |
| `diff.added.bg` | `#98c379` al 18% | fondo de línea añadida |
| `diff.added.word` | `#98c379` al 38% | palabra añadida |
| `diff.gutter.deleted` / `.added` / `.modified` | `#e06c75` / `#98c379` / `#e5c07b` | barras del gutter |
| `status.error` / `.warning` / `.ok` | `#e06c75` / `#e5c07b` / `#98c379` | estados |

Colores de sintaxis: 12 capturas estándar de tree-sitter (`keyword`, `function`, `type`, `string`, `number`, `comment`, `variable`, `property`, `operator`, `punctuation`, `constant`, `attribute`) con la paleta One Dark. El tema es un archivo JSON en `~/.config/asteroid/themes/`.

## 3. Tipografía

- UI: `Inter`, respaldo `system-ui`. 13 px.
- Código: `JetBrains Mono`, respaldo `Zed Mono`, `DejaVu Sans Mono`, `monospace`. 14 px, interlineado 1.5. Ligaduras desactivadas por defecto.
- Renderizado subpíxel con gamma 1.8 (lo que provee el motor gráfico); ajuste `text.rendering = "subpixel" | "grayscale"`.

## 4. Forma y densidad

- Radio de borde 4 px en botones y tarjetas; 6 px en popovers; 0 en paneles.
- Sin sombras en paneles; sombra suave (`0 2px 8px #0006`) solo en popovers y en la barra flotante.
- Alturas: fila del árbol 24 px, pestaña 32 px, barra de estado 26 px, fila de línea = tamaño de fuente × 1.5.
- Iconos: Lucide, 14 px en UI y 16 px en el árbol.
- Animaciones: solo desvanecidos de 120 ms en popovers y en la aparición de botones flotantes. Nada más se anima.

## 5. El editor

- **Gutter**: números de línea alineados a la derecha, `text.muted`, número actual en `text`. A la izquierda de los números, una barra de 3 px por línea con los colores de diff del agente (v1) y de git (v2). Las filas fantasma no llevan número.
- **Línea actual**: fondo `bg.elevated` al 60% (sin borde).
- **Cursor**: barra de 2 px, parpadeo 500 ms que se detiene tras 5 s sin escribir.
- **Selección**: `selection`, esquinas rectas.
- **Coincidencias de búsqueda**: fondo `#e5c07b` al 30%; la actual al 55%.
- **Espacios en blanco**: ocultos por defecto; ajuste para mostrarlos.
- **Ajuste de línea**: desactivado por defecto; con guía vertical opcional en la columna 100.
- **Scroll**: barra fina de 8 px que aparece al mover el mouse y se desvanece; scroll suave en trackpad; con rueda, 3 líneas por paso.

## 6. La revisión de cambios (detalle visual)

### 6.1 Segmento pendiente
```
 12 │   fn suma(a: i32, b: i32) -> i32 {
    │ - ▏  a - b                                   ← fila fantasma: fondo diff.deleted.bg,
    │ - ▏  ^^^                                       texto con sintaxis al 70% de opacidad,
 13 │ + ▏  a + b                                     palabra "−" en diff.deleted.word
 14 │   }                                          ← fila real: fondo diff.added.bg,
                                                     palabra "+" en diff.added.word
                                        ┌───────────────────┐
                                        │ ✓ Aceptar  ✗ Rechazar │  pill flotante, bg.elevated,
                                        └───────────────────┘  alineado al borde derecho del
                                                               editor, en la primera fila del hunk
```
- Las filas fantasma (eliminadas) van **encima** de las añadidas del mismo segmento, como en un diff unificado.
- El segmento tiene un borde izquierdo de 2 px en el gutter (`diff.gutter.modified` si tiene ambas, `.added` o `.deleted` si solo una).
- El **pill** aparece siempre en el segmento que contiene el cursor o el mouse; en los demás segmentos aparece solo al pasar el mouse. Ancho fijo 190 px, alto 24 px.
- Al pasar el mouse por una línea concreta del segmento, aparecen en el gutter de esa línea dos iconos de 14 px: `+` (aceptar esta línea) y `−` (rechazar esta línea). Las filas fantasma también los tienen: aceptar una fila fantasma confirma su eliminación; rechazarla la restaura.
- Los segmentos de un turno anterior aún pendientes se ven igual; el pill muestra un pequeño reloj con tooltip "Turno anterior".

### 6.2 Barra flotante del editor
- Abajo a la derecha del editor, 24 px de margen, `bg.elevated`, radio 6, sombra.
- Contenido: `✓ Aceptar  Ctrl+↵` · `✗ Rechazar  Ctrl+⌫` · `↑ Alt+K` `↓ Alt+J` · `3/7 cambios` · botón `Revisar todo`.
- Mientras el agente escribe: spinner y texto "El agente está editando…", botones deshabilitados.
- Cuando no quedan cambios en el archivo: la barra muestra "Sin cambios pendientes en este archivo · 4 en otros archivos → siguiente archivo (Alt+L)" durante 3 s y desaparece.
- Se oculta si el editor pierde el foco de ventana.

### 6.3 Panel de revisión (`Ctrl+Shift+R`)
Popover anclado a la barra de estado o panel lateral en el chat (ajuste). Lista de archivos del turno: icono, ruta relativa, `+N −M`, estado (pendiente / decidido). Botones arriba: `Aceptar todo (Ctrl+Alt+↵)` `Rechazar todo (Ctrl+Alt+⌫)`. Clic en un archivo lo abre y salta al primer segmento pendiente. Si hay archivos creados o borrados, se listan con etiqueta `nuevo` / `eliminado`.

### 6.4 Árbol de archivos
Archivo con cambios pendientes: nombre en `#e5c07b`, sufijo `+N −M` en `text.muted` tamaño 11. Archivo nuevo del agente: `#98c379`. Carpeta con hijos pendientes: punto de 6 px a la derecha. Raíz: contador total.

### 6.5 Avisos emergentes
Abajo al centro del editor, 4 s, `bg.elevated`. Al rechazar: "Segmento rechazado · Deshacer (Alt+Shift+U)". Al descartar una revisión porque el archivo cambió por fuera: "La revisión de X se descartó: el archivo cambió fuera de Asteroid".

## 7. El chat

- **Cabecera**: selector de agente (icono + nombre + estado: `listo`, `pensando…`, `esperando permiso`, `desconectado`), botón de sesión nueva, historial de sesiones (lista simple por fecha).
- **Transcript**: mensajes del usuario con fondo `bg.surface` y radio 6; respuestas del agente sin fondo, markdown renderizado (encabezados, listas, tablas, código con resaltado y botón copiar, enlaces). El texto llega en streaming y se agrega sin saltos.
- **Pensamiento del agente**: plegado por defecto en una línea "Pensando… (12 s)"; desplegable.
- **Tarjeta de herramienta**: una fila de 28 px con icono según el tipo (leer, editar, ejecutar, buscar, pensar, otro), título del agente, estado (spinner, ✓, ✗) y a la derecha `+N −M` si es edición. Clic: despliega detalle (comando y salida para ejecución; diff resumido para edición con botón "Ver en el editor" que salta al primer segmento pendiente).
- **Plan**: lista de tareas con casillas de estado, plegable.
- **Pedido de permiso**: tarjeta destacada con borde `border.focus`, título, detalle plegado, y los botones que el agente ofrece, en el orden que los envía; el primero es el predeterminado (`Enter`). `Esc` equivale a rechazar.
- **Autenticación**: tarjeta con el comando a copiar (`claude auth login`, etc.), botón "Copiar", botón "Abrir terminal" (lanza el terminal predeterminado con el comando) y "Reintentar".
- **Input**: área de texto que crece hasta 8 líneas, placeholder "Preguntale a Claude… (@ para archivos, / para comandos)". Botón enviar / detener. Debajo, fila de selectores compactos: autonomía (`Revisar después ▾`), y los que anuncie el agente (modo, modelo, esfuerzo), como en la captura de referencia.
- **Menciones**: `@` abre una lista de archivos del proyecto filtrada al escribir; el archivo elegido se inserta como chip.
- **Comandos slash**: `/` abre la lista que anunció el agente, con su descripción.

## 8. Atajos de teclado (predeterminados, Linux)

| Acción | Tecla | Contexto |
|---|---|---|
| Abrir carpeta | `Ctrl+O` | global |
| Guardar / guardar todo | `Ctrl+S` / `Ctrl+Alt+S` | editor |
| Cerrar pestaña | `Ctrl+W` | editor |
| Buscar en archivo | `Ctrl+F` | editor |
| Deshacer / rehacer | `Ctrl+Z` / `Ctrl+Shift+Z` | editor |
| Ir a línea | `Ctrl+G` | editor |
| Mostrar/ocultar chat | `Ctrl+Shift+A` | global |
| Mostrar/ocultar árbol | `Ctrl+Shift+E` | global |
| Foco al chat | `Ctrl+L` | global |
| Enviar mensaje / salto de línea | `Enter` / `Shift+Enter` | chat |
| Cancelar turno | `Esc` (con el chat enfocado) | chat |
| Aceptar segmento o línea bajo el cursor | `Ctrl+Enter` | editor con segmento pendiente bajo el cursor; si no, salto de línea normal |
| Rechazar segmento o línea bajo el cursor | `Ctrl+Backspace` | idem; si no, borrar palabra |
| Aceptar / rechazar archivo | `Ctrl+Shift+Enter` / `Ctrl+Shift+Backspace` | archivo con pendientes |
| Aceptar / rechazar turno | `Ctrl+Alt+Enter` / `Ctrl+Alt+Backspace` | global con pendientes |
| Siguiente / anterior cambio | `Alt+J` / `Alt+K` (también `F7` / `Shift+F7`) | global |
| Siguiente archivo con cambios | `Alt+L` | global |
| Panel de revisión | `Ctrl+Shift+R` | global |
| Deshacer último rechazo | `Alt+Shift+U` | global |
| Zoom UI | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | global |

Reglas: nunca `Ctrl+Y` ni `Ctrl+N` para aceptar/rechazar; no usar `Super`, `Ctrl+Alt+flechas`, `Alt+F*` (los toma COSMIC/GNOME). Los atajos con contexto hacen fallthrough a la acción normal cuando el contexto no aplica. Todo atajo es un comando con nombre (`review::accept_hunk`, etc.) reasignable en `keymap.json`.

## 9. Estados vacíos y errores
- Sin proyecto: pantalla central con logo, "Abrir carpeta (Ctrl+O)" y proyectos recientes.
- Sin agentes instalados: el chat muestra qué instalar (`npm i -g` no hace falta: `npx` los baja) y verifica Node.
- Agente caído: tarjeta roja en el chat con las últimas líneas de su salida de error y botón "Reiniciar".
- Sin GPU: aviso al arrancar de que se está usando OpenGL o software.
