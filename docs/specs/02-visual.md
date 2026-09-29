# Spec 02: Visual e interacción

Estado: v1.0. Describe **cómo se ve y se siente** Cincel. Los valores concretos (colores, tamaños) son los predeterminados; todos son configurables por tema.

**Corrección de la Etapa 5** (`docs/specs/07-etapa5-productividad.md`, 2026-09-28): §5 (git en el margen), §8 (barra de título con menú y botones de paneles, botón de atajos, atajos nuevos y cambiados). Detalle en las secciones correspondientes más abajo.

## 1. Ventana y layout

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ● ● ●  [chat ▾] [tab: main.rs ●] [tab: lib.rs]        [árbol ▾]  Cincel │  barra de título integrada (CSD)
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

Tokens de color (tema oscuro predeterminado "Cincel Dark", inspirado en One Dark de Zed; el tema claro "Cincel Light" sigue al sistema vía el portal `Settings`):

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

Colores de sintaxis: 12 capturas estándar de tree-sitter (`keyword`, `function`, `type`, `string`, `number`, `comment`, `variable`, `property`, `operator`, `punctuation`, `constant`, `attribute`) con la paleta One Dark. El tema es un archivo JSON en `~/.config/cincel/themes/`.

## 3. Tipografía

- UI: `Inter`, respaldo `system-ui`. 13 px.
- Código: `JetBrains Mono`, respaldo `Zed Mono`, `DejaVu Sans Mono`, `monospace`. 14 px, interlineado 1.5. Ligaduras desactivadas por defecto.
- **Desde la Etapa 6 (2026-09-29):** Inter 4.1 y JetBrains Mono 2.304 vienen dentro del programa (`crates/cincel-workspace/assets/fonts/`, licencia OFL) y se registran al arrancar; el aspecto es el mismo en cualquier equipo. Los respaldos solo se usan si el usuario elige en la configuración otra familia que no está instalada.
- Renderizado subpíxel con gamma 1.8 (lo que provee el motor gráfico); ajuste `text.rendering = "subpixel" | "grayscale"`.

## 4. Forma y densidad

- Radio de borde 4 px en botones y tarjetas; 6 px en popovers; 0 en paneles.
- Sin sombras en paneles; sombra suave (`0 2px 8px #0006`) solo en popovers y en la barra flotante.
- Alturas: fila del árbol 24 px, pestaña 32 px, barra de estado 26 px, fila de línea = tamaño de fuente × 1.5.
- Iconos: Lucide, 14 px en UI y 16 px en el árbol.
- Animaciones: solo desvanecidos de 120 ms en popovers y en la aparición de botones flotantes. Nada más se anima.

## 5. El editor

- **Gutter**: números de línea alineados a la derecha, `text.muted`, número actual en `text`, en una columna de al menos 3 dígitos de ancho. A la izquierda de los números, una barra de 3 px por línea con los colores de diff del agente y, en su propia columna de 3 px (Etapa 5), los colores de git contra `HEAD`: `git.added` (`#98c379`) en las líneas que no existen en `HEAD`, `git.modified` (`#61afef`) en las que reemplazan una de `HEAD`, y `git.deleted` (`#e06c75`) como una marca de 3 px × 6 px centrada en el borde entre filas donde se borró texto. Las dos columnas (git y agente) conviven en la misma fila sin pisarse; las filas fantasma no llevan barra de git (no existen en el disco) ni número. El ancho del gutter no depende de la revisión ni de git: mostrar o decidir segmentos, y tener o no cambios de git, nunca corre el código — la columna de git se paga repartiendo el espacio que ya existía antes de los números (`docs/specs/07-etapa5-productividad.md` §6.3, D11), no agrandando el gutter.
- **Línea actual**: fondo `bg.elevated` al 60% (sin borde).
- **Cursor**: barra de 2 px, parpadeo 500 ms que se detiene tras 5 s sin escribir.
- **Selección**: `selection`, esquinas rectas.
- **Coincidencias de búsqueda**: fondo `#e5c07b` al 30%; la actual al 55%.
- **Espacios en blanco**: ocultos por defecto; ajuste para mostrarlos.
- **Ajuste de línea**: activado por defecto (`editor.soft_wrap: true`), al ancho que le deja el layout al editor: redimensionar un dock o la ventana vuelve a ajustar el texto. `Alt+Z` lo alterna en la pestaña actual; sin ajuste, las líneas largas se desplazan a lo ancho dentro del editor y nunca se dibujan fuera de él (debajo de un dock). Guía vertical opcional en la columna 100.
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
                                                               editor, en la primera fila del
                                                               segmento que le deja lugar
```
- Las filas fantasma (eliminadas) van **encima** de las añadidas del mismo segmento, como en un diff unificado.
- El segmento tiene un borde izquierdo de 2 px en el gutter (`diff.gutter.modified` si tiene ambas, `.added` o `.deleted` si solo una).
- El **pill** aparece siempre en el segmento que contiene el cursor o el mouse; en los demás segmentos aparece solo al pasar el mouse. Ancho fijo 190 px, alto 24 px, alineado al borde derecho del área de texto (12 px antes de la barra de scroll). Es una capa encima del texto: no ocupa filas ni corre nada.
- **Dónde se pone el pill**: se prueban, en orden, la primera fila del segmento, las siguientes filas del segmento y la fila justo encima del segmento (en un segmento de pura eliminación, las filas fantasma son las candidatas); gana la primera cuyo texto termina al menos 12 px antes del borde izquierdo del pill. Si ninguna deja lugar, se pinta un **pill compacto** (dos botones de 24 × 24 px, `✓` y `✗`, 52 px en total; con el agente escribiendo, una tercera celda con el spinner) en la primera fila del segmento, al 70 % de opacidad. Los clics siguen a donde se pintó.
- **Colores del pill**: habilitado, `✓` en `status.ok`, `✗` en `status.error` y las palabras en `text`, a opacidad plena; la mitad bajo el mouse lleva fondo `bg.surface`. Deshabilitado (el agente está escribiendo): todo en `text.muted` y el spinner.
- Al pasar el mouse por una línea concreta del segmento, aparecen **sobre la columna de números** de esa línea dos iconos: `+` (aceptar esta línea) y `−` (rechazar esta línea), con 2 px entre ellos, alineados al borde derecho de la columna; el número de esa línea se oculta mientras tanto (las filas fantasma no tienen). Miden 14 px si los dos entran en la columna (mínimo 3 dígitos) y 11 px si no (con la fuente de 14 px, tres dígitos miden ≈ 25 px). Las filas fantasma también los tienen: aceptar una fila fantasma confirma su eliminación; rechazarla la restaura. `Alt+Enter` / `Alt+Backspace` hacen lo mismo con la línea del cursor.
- Los segmentos de un turno anterior aún pendientes se ven igual; el pill muestra un pequeño reloj con tooltip "Turno anterior".

### 6.2 Barra flotante del editor
- Abajo a la derecha del editor, 24 px de margen, `bg.elevated`, radio 6, sombra.
- Contenido: `✓ Aceptar  Ctrl+↵` · `✗ Rechazar  Ctrl+⌫` · `↑ Alt+K` `↓ Alt+J` · `3/7 cambios` · botón `Revisar todo`. `Aceptar` y `Rechazar` actúan sobre el segmento del cursor (o el actual) y se leen habilitados: `✓` en `status.ok`, `✗` en `status.error`, palabra en `text`; el botón bajo el mouse lleva fondo `bg.surface`.
- Mientras el agente escribe: spinner y texto "El agente está editando…", botones deshabilitados.
- Cuando no quedan cambios en el archivo: la barra muestra "Sin cambios pendientes en este archivo · 4 en otros archivos → siguiente archivo (Alt+L)" durante 3 s y desaparece.
- Se oculta si el editor pierde el foco de ventana.

### 6.3 Panel de revisión (`Ctrl+Shift+R`)
Popover anclado a la barra de estado o panel lateral en el chat (ajuste). Lista de archivos del turno: icono, ruta relativa, `+N −M`, estado (pendiente / decidido). Botones arriba: `Aceptar todo (Ctrl+Alt+↵)` `Rechazar todo (Ctrl+Alt+⌫)`. Clic en un archivo lo abre y salta al primer segmento pendiente. Si hay archivos creados o borrados, se listan con etiqueta `nuevo` / `eliminado`.

### 6.4 Árbol de archivos
Archivo con cambios pendientes: nombre en `#e5c07b`, sufijo `+N −M` en `text.muted` tamaño 11. Archivo nuevo del agente: `#98c379`. Carpeta con hijos pendientes: punto de 6 px a la derecha. Raíz: contador total.

### 6.5 Avisos emergentes
Abajo al centro del editor, 4 s, `bg.elevated`. Al rechazar: "Segmento rechazado · Deshacer (Alt+Shift+U)". Al descartar una revisión porque el archivo cambió por fuera: "La revisión de X se descartó: el archivo cambió fuera de Cincel".

## 7. El chat

- **Cabecera**: selector de agente (icono + nombre + estado: `listo`, `pensando…`, `esperando permiso`, `desconectado`), botón de sesión nueva, historial de sesiones (lista simple por fecha).
- **Transcript**: mensajes del usuario en burbuja contra el borde derecho, `text.accent` al 14 % sobre `bg.app`, borde de 1 px en `text.accent` al 35 %, radio 8, ancho máximo 85 %, con su texto en markdown renderizado igual que las respuestas (listas, negrita, código en línea, bloques con el estilo de bloque de código); un párrafo con menciones se arma con texto y chips en línea; respuestas del agente sin fondo ni burbuja, markdown renderizado (encabezados de 15 px como mucho, listas, tablas, código sobre `bg.editor` con borde de 1 px y cabecera con lenguaje y "Copiar", enlaces). Sin etiquetas "Vos" ni nombre del agente: la diferencia es solo visual. Escala: cuerpo 13 px, código 12,5 px, filas de herramienta / plan / ayudas 12 px, etiquetas y fechas 11 px; todas (y alturas, radios y el composer) se multiplican por el zoom de la interfaz (`Ctrl+=` / `Ctrl+-` / `Ctrl+0`), igual que el árbol, las pestañas, el breadcrumb y la barra de estado. La selección de texto en el chat (respuestas, burbujas, bloques de código, composer) es `text.accent` al 35 %, en ambos temas; la del editor de código no cambia. El texto llega en streaming y se agrega sin saltos.
- **Pensamiento del agente**: plegado por defecto en una línea "Pensando… (12 s)"; desplegable.
- **Tarjeta de herramienta**: una fila de 28 px con icono según el tipo (leer, editar, ejecutar, buscar, pensar, otro), título del agente, estado (spinner, ✓, ✗) y a la derecha `+N −M` si es edición. Clic: despliega detalle (comando y salida para ejecución; diff resumido para edición con botón "Ver en el editor" que salta al primer segmento pendiente).
- **Plan**: lista de tareas con casillas de estado, plegable.
- **Pedido de permiso**: tarjeta destacada con borde `border.focus`, título, detalle plegado, y los botones que el agente ofrece, en el orden que los envía; el primero es el predeterminado (`Enter`). `Esc` equivale a rechazar.
- **Autenticación**: tarjeta con el comando a copiar (`claude auth login`, etc.), botón "Copiar", botón "Abrir terminal" (lanza el terminal predeterminado con el comando) y "Reintentar".
- **Input**: un editor de Cincel configurado para Markdown, dentro de una caja de 1 px en `border` (`border.focus` con foco), sin gutter ni números, con ajuste de línea, que crece de 1 a 8 líneas y después hace scroll. `Enter` envía (o confirma el `@` / `/` abierto, o responde el permiso pendiente) y `Shift+Enter` inserta un salto de línea. Resalta la estructura con los tokens de sintaxis y ningún color nuevo: encabezados y marcadores de lista, cita y énfasis en `keyword`; código en línea en `string`; líneas de fence, reglas y URL de enlaces en `comment`; texto de enlaces y menciones `@archivo` en `function`. Cada fila de un bloque cercado (fences incluidos) lleva fondo `bg.editor` de ancho completo y fuente monoespaciada, con el resaltado del lenguaje si el info string nombra uno conocido; el resto usa la fuente de la interfaz. Placeholder "Escribí un mensaje para Claude…" (lo que hacen `@` y `/` va en la línea de ayuda de abajo). Botón enviar / detener dentro del borde derecho. Debajo, fila de selectores compactos: los que anuncie el agente (modo, modelo, esfuerzo), como en la captura de referencia. Cincel no agrega un selector de autonomía propio (`01-producto.md` §F5).
- **Menciones**: `@` abre una lista de archivos del proyecto filtrada al escribir; el archivo elegido se inserta como chip.
- **Comandos slash**: `/` abre la lista que anunció el agente, con su descripción.

## 8. Atajos de teclado (predeterminados, Linux)

**Barra de título y menú (Etapa 5).** A la izquierda de la barra de título, un
botón con tres rayitas abre un menú (`gpui_kit::component::menu::PopupMenu`)
con: Abrir carpeta…, Carpetas recientes ▸, Nuevo archivo…, Guardar, Guardar
todo, Configuración, Atajos de teclado, Conexiones y Salir — cada ítem
muestra el atajo de la tabla de abajo cuando tiene uno. A su lado, dos
botones muestran u ocultan el chat y los archivos (misma acción que
`Ctrl+Shift+A`/`Ctrl+Shift+E`, con la regla de foco de más abajo). La barra
de estado suma un botón de atajos de teclado (icono de teclado, a la derecha,
antes del zoom) que abre el mismo modal que `F1`. Ninguno de estos elementos
cambia el alto de la barra de título ni de la de estado, ni corre el editor
(`docs/specs/07-etapa5-productividad.md` D16).

| Acción | Tecla | Contexto |
|---|---|---|
| Abrir carpeta | `Ctrl+O` | global |
| Nuevo archivo | `Ctrl+N` | global, con proyecto abierto |
| Guardar / guardar todo | `Ctrl+S` / `Ctrl+Alt+S` | editor |
| Cerrar pestaña | `Ctrl+W` | editor |
| Buscar en archivo | `Ctrl+F` | editor |
| Buscar y reemplazar en archivo | `Ctrl+H` | editor |
| Deshacer / rehacer | `Ctrl+Z` / `Ctrl+Shift+Z` | editor |
| Ir a línea | `Ctrl+G` | editor |
| Vista previa de Markdown | `Ctrl+Shift+V` | editor y área de pestañas, con Markdown activo |
| Buscador rápido de archivos | `Ctrl+P` | global, con proyecto abierto |
| Configuración | `Ctrl+,` | global |
| Atajos de teclado | `F1` | global |
| Salir | `Ctrl+Q` | global |
| Mostrar/ocultar chat (abre y enfoca / enfoca / cierra, ver regla abajo) | `Ctrl+Shift+A` | global |
| Mostrar/ocultar árbol (misma regla) | `Ctrl+Shift+E` | global |
| Siguiente zona de foco: chat → editor → archivos → chat | `Ctrl+L` | global |
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

**Regla de `Ctrl+Shift+A` / `Ctrl+Shift+E` (Etapa 5, `07-etapa5-productividad.md`
§7):** panel oculto → se abre y el foco va a él (el compositor del chat, o el
archivo seleccionado del árbol); panel visible con el foco dentro de él →
se cierra y el foco vuelve al editor (o al área de pestañas); panel visible
con el foco en otro lado → se enfoca sin cerrar nada. `Ctrl+L` recorre las
zonas visibles en ese orden fijo, sin abrir ni cerrar paneles y sin rueda
hacia atrás; `workspace::focus_chat` (el atajo de "foco al chat" de antes de
esta etapa) sigue existiendo como comando, sin tecla asignada por defecto.
Con un modal abierto (conexiones, buscador de archivos, atajos, nuevo
archivo, diálogo de guardar, panel de revisión o el menú de la barra de
título desplegado), estos tres atajos no hacen nada.

Reglas: no usar `Super`, `Ctrl+Alt+flechas`, `Alt+F*` (los toma COSMIC/GNOME). Los atajos con contexto hacen fallthrough a la acción normal cuando el contexto no aplica. Todo atajo es un comando con nombre (`review::accept_hunk`, etc.) reasignable en `keymap.json`. Los bindings por defecto exactos, ya verificados contra `crates/cincel-settings/src/defaults.rs`, están además en `docs/specs/07-etapa5-productividad.md` §11.1.

## 9. Estados vacíos y errores
- Sin proyecto: pantalla central con logo, "Abrir carpeta (Ctrl+O)" y proyectos recientes.
- Sin agentes instalados: el chat muestra qué instalar (`npm i -g` no hace falta: `npx` los baja) y verifica Node.
- Agente caído: tarjeta roja en el chat con las últimas líneas de su salida de error y botón "Reiniciar".
- Sin GPU: aviso al arrancar de que se está usando OpenGL o software.
