# Ajustes

Cincel tiene dos formas de cambiar sus ajustes, que conviven:

- La **pestaña de configuración** (`Ctrl+,`, o "Configuración" en el menú):
  interruptores y selectores agrupados en Apariencia, Fuentes, Editor,
  Archivos, Revisión y Conexiones. Tiene un buscador arriba ("Buscar
  ajustes…") y, junto a cada ajuste, su clave técnica en gris chico (por
  ejemplo `theme.mode`) por si preferís tocarla a mano.
- El archivo **`settings.json`** (`~/.config/cincel/settings.json`), en
  JSON con comentarios. Podés abrirlo con el botón "Abrir settings.json" de
  la propia pestaña.

Lo que cambiés desde la pestaña se escribe en `settings.json` sin borrar
tus comentarios ni el resto de tus ajustes; y si editás el archivo a mano
con otro programa, la pestaña se actualiza sola, sin que haga falta
reiniciar Cincel. El botón "Restablecer" de un ajuste (la flechita junto al
control) le devuelve su valor de fábrica quitando esa clave del archivo.

`cincel --print-default-settings` imprime en la terminal el
`settings.json` completo con el que arranca Cincel de fábrica, comentado;
es útil como referencia o para empezar el tuyo de cero
(`cincel --print-default-settings > ~/.config/cincel/settings.json`).

## Apariencia

- `theme.mode`: seguir al escritorio o fijar el modo oscuro o el claro.
- `theme.dark`: el tema que se usa en modo oscuro.
- `theme.light`: el tema que se usa en modo claro.
- `text_rendering`: cómo se dibujan los bordes de las letras (con
  suavizado de subpíxel o en escala de grises).
- `window.decorations`: quién dibuja la barra de título de la ventana, la
  propia de Cincel o la de tu escritorio (se aplica al reiniciar Cincel).

## Fuentes

- `ui_font_family`: la familia tipográfica de menús, paneles y chat (por
  defecto, Inter, incluida dentro del programa).
- `ui_font_size`: su tamaño, en píxeles, de 6 a 72.
- `buffer_font_family`: la familia tipográfica del código (por defecto,
  JetBrains Mono, también incluida dentro del programa).
- `buffer_font_size`: su tamaño, en píxeles, de 6 a 72.
- `buffer_line_height`: el interlineado del editor, como múltiplo del
  tamaño de letra, de 1,0 a 4,0.

Si elegís una familia que no está instalada en tu sistema, Cincel usa Inter
o JetBrains Mono como respaldo (nunca una fuente al azar); las dos siempre
aparecen en el selector aunque no estén instaladas, porque vienen adentro
del programa.

## Editor

- `editor.soft_wrap`: ajustar al ancho del editor las líneas que no
  entran, en lugar de desplazar a lo ancho (`Alt+Z` lo alterna en la
  pestaña actual).
- `editor.tab_size`: columnas de cada tabulación, de 1 a 16.
- `editor.insert_spaces`: que `Tab` escriba espacios en lugar de un
  carácter de tabulación.
- `editor.show_whitespace`: dibujar los espacios y las tabulaciones
  (`Ctrl+Alt+W` lo alterna en la pestaña actual).
- `editor.ruler`: la columna de la guía vertical; `0` la oculta.
- `editor.cursor_blink`: que el cursor parpadee mientras escribís.
- `editor.auto_close_pairs`: agregar el cierre automáticamente al abrir un
  paréntesis, un corchete, una llave o unas comillas.

## Archivos

- `files.exclude`: patrones que no se muestran en el árbol ni en el
  buscador de archivos, además de lo que ya ignore tu `.gitignore`.
- `files.autosave`: cuándo se guardan los archivos sin que lo pidas
  (nunca, al cambiar de foco, o tras una pausa sin escribir).
- `files.autosave_delay_ms`: con "tras una pausa", cuántos milisegundos sin
  escribir esperar antes de guardar (de 100 a 60 000).
- `files.trim_trailing_whitespace_on_save`: al guardar, quitar los espacios
  y las tabulaciones que sobran al final de cada línea. No toca los archivos
  Markdown (`.md` y `.markdown`, donde dos espacios al final son un salto de
  línea) ni las líneas de un cambio del agente que todavía no decidiste, y
  no hace nada mientras el agente está escribiendo ese archivo. Viene
  encendido; un `Ctrl+Z` después de guardar devuelve los espacios.
- `files.ensure_final_newline_on_save`: al guardar, terminar el archivo con
  un salto de línea si no lo tiene (los archivos vacíos quedan vacíos). Viene
  encendido.

Las dos limpiezas al guardar se explican con ejemplos en
[Escribir en el editor](editor.md#limpiar-el-archivo-al-guardar).

## Revisión

- `review.jump_to_next_on_decide`: después de aceptar o rechazar un
  cambio, saltar solo al próximo cambio pendiente.
- `review.sensitive_paths`: rutas que siempre piden tu confirmación antes
  de que el agente las toque, sea cual sea el permiso que use.
- `review.max_file_size_kb` y `review.max_lines`: por encima de estos
  tamaños un archivo se revisa entero, no segmento por segmento (el límite
  de fábrica, 2 MB o 50 000 líneas, no se puede superar).
- `review.snapshot_max_total_mb`: antes de cada mensaje al agente, Cincel
  guarda una copia de tus archivos para poder mostrarte y deshacer lo que
  cambie; este es el máximo que ocupa esa copia (de 16 a 4096 MB) — pasado
  el tope, los archivos que no entran solo se pueden aceptar enteros.

## Conexiones

- `connections.default_label`: la conexión preseleccionada en el popover
  "Conectar"; sin ninguna (el valor de fábrica), no se preselecciona
  ninguna.
- `connections.runtime.node_version`: la versión del Node privado que usan
  los adaptadores de Claude y Codex (`"lts"` o una versión mayor exacta,
  por ejemplo `"22"`).
- `connections.mcp_servers`: servidores MCP propios que se ofrecen a cada
  sesión de agente (una lista vacía por defecto); ver el comentario del
  propio `settings.json` para el formato de cada entrada.

Esta sección no tiene fila propia en la pestaña de ajustes todavía: se
administra desde `Ctrl+,` → **Conexiones**, que es una pantalla dedicada
(ver [Conexiones](conexiones.md)), no una fila de interruptor.
