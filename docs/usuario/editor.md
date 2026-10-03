# Escribir en el editor

Esta página junta las pequeñas ayudas que el editor de Cincel te da mientras
escribís código: marcar dónde más aparece una palabra, copiar y cortar una
línea sin seleccionarla, ordenar la sangría, moverte más allá del final del
archivo y limpiar el archivo al guardar. Valen en el editor de las
pestañas; la caja donde escribís en el [chat](chat.md) y la cajita de un
comentario son campos de texto más simples y no las tienen. Las teclas
están todas en [Atajos de teclado](atajos.md).

## Dónde más aparece esta palabra

Si dejás el cursor sobre una palabra (adentro o justo al final de ella),
Cincel marca con un **fondo suave**, igual al de la selección pero más
tenue, las **otras veces que esa palabra aparece** en lo que estás viendo.
Cuenta como palabra lo que tiene letras, números o guion bajo, y se marca
solo la palabra completa y con las mismas mayúsculas: con el cursor en
`total`, no se marca `total_b` ni `Total`.

También funciona con una selección: si seleccionás un trozo de **una sola
línea** (por ejemplo `a + `), se marcan las demás veces que ese mismo trozo
aparece. Una selección de varias líneas, vacía o solo de espacios no marca
nada.

- Las marcas aparecen **un instante después** de que dejás de mover el
  cursor o de escribir, y desaparecen apenas te movés otra vez.
- Solo se marcan las apariciones que están a la vista, y la que tenés debajo
  del cursor (o la selección misma) no se marca.
- **No se muestran con la búsqueda abierta** (`Ctrl+F` con algo escrito):
  ahí mandan las coincidencias de la búsqueda, en amarillo.

## Copiar y cortar la línea entera

Con **nada seleccionado**:

- **`Ctrl+C`** copia la línea donde está el cursor, entera y con su salto de
  línea (también la última del archivo, aunque no termine con salto). El
  cursor no se mueve.
- **`Ctrl+X`** hace lo mismo y además **borra la línea**. Un `Ctrl+Z` la
  devuelve. En una línea roja del agente (de solo lectura) copia, pero no
  borra nada.

Al pegar con **`Ctrl+V`**, una línea copiada así **entra como una línea
nueva arriba de la línea donde está el cursor**, no en medio del texto, y se
pega tal cual, sin cambiarle la sangría. El cursor queda en la misma
columna del mismo texto, que ahora está una línea más abajo. Si en cambio
tenés algo seleccionado, `Ctrl+V` pega reemplazando la selección, como
siempre.

Si copiás o cortás **con** una selección, Cincel se olvida de que antes
habías copiado una línea: ese `Ctrl+V` pega un trozo común. El menú del clic
derecho ("Cortar", "Copiar", "Pegar") hace lo mismo que las teclas.

## La sangría

La sangría es la cantidad de espacios (o tabulaciones) que van al comienzo de
una línea. Cincel usa la sangría que tiene el archivo y, si no se nota
ninguna, la de `editor.tab_size` y `editor.insert_spaces` (ver
[Ajustes](ajustes.md)). Tres ayudas:

- **Cerrar `}`, `]` o `)` en una línea que solo tiene sangría**: Cincel le
  **quita un nivel** a esa sangría y después escribe el carácter, para que
  el cierre quede alineado con su apertura. Es lo mismo que haría
  `Shift+Tab`. Por ejemplo, después de `fn a() {` y `Enter` el cursor queda
  con 4 espacios; al escribir `}` quedan 0 y la llave se alinea con `fn`. Si
  la línea ya tiene código antes del cursor (`    foo}`), no toca la
  sangría. Si justo después del cursor está el mismo cierre que Cincel puso
  solo al abrir el par, escribirlo salta por encima, como siempre. Un solo
  `Ctrl+Z` deshace las dos cosas.
- **`Enter` en una línea que solo tiene sangría**: la línea que queda atrás
  **queda vacía**, sin espacios sueltos, y la línea nueva arranca con la
  sangría que había antes del cursor (si el cursor estaba en medio de la
  sangría, los espacios que quedaban después se descartan). Con código en la
  línea, `Enter` sigue como siempre.
- **`Backspace` dentro de la sangría** borra **un nivel entero** en vez de un
  espacio: con sangría de 4, de 8 espacios pasás a 4, de 6 a 4 y de 1 a 0. Si
  antes del cursor hay una tabulación, o tabulaciones mezcladas con espacios,
  o código, borra un carácter como siempre.

## Más allá de la última línea

En el editor podés seguir desplazando hasta que la **última línea quede a
media pantalla**, como en otros editores. No se agrega nada al archivo: es
solo espacio vacío al final, y los números de línea y el texto no cambian.
Sirve, por ejemplo, para que la barra flotante de Aceptar y Rechazar de la
revisión no tape las últimas líneas. Un archivo corto que entra entero en
pantalla también se puede subir hasta dejar su última línea a media
pantalla. Un clic en ese espacio vacío pone el cursor al final de la última
línea.

`Ctrl+Fin` y las flechas solo traen el cursor a la vista, como siempre; no
bajan hasta el fondo de ese espacio.

## Desplazar sin mover el cursor y ver los espacios

- **`Ctrl+↑`** y **`Ctrl+↓`** desplazan el texto **una línea** hacia arriba
  o hacia abajo **sin mover el cursor** ni la selección. Si el cursor queda
  fuera de lo que se ve, se queda ahí hasta que escribas o te muevas;
  entonces la vista vuelve a él.
- **`Ctrl+Alt+W`** muestra u oculta los **espacios en blanco** (puntos para
  los espacios y flechas para las tabulaciones) **solo en la pestaña
  actual**, igual que `Alt+Z` hace con el ajuste de línea. Lo que queda para
  siempre se elige en `Ctrl+,` → Editor → "Mostrar espacios en blanco"
  (`editor.show_whitespace`).

## Limpiar el archivo al guardar

Cada vez que se guarda un archivo, Cincel puede hacer dos limpiezas, como
hace Zed:

1. **Quitar los espacios y las tabulaciones que sobran al final de cada
   línea.**
2. **Terminar el archivo con un salto de línea** si no lo tiene (un archivo
   vacío queda vacío, y no se quitan las líneas en blanco del final).

Las dos **vienen encendidas**. Valen para todas las formas de guardar:
`Ctrl+S`, `Ctrl+Alt+S` (guardar todo), el autoguardado y "Guardar" al
cerrar. Se apagan por separado en `Ctrl+,` → **Archivos**:

- "Quitar espacios al final de las líneas al guardar"
  (`files.trim_trailing_whitespace_on_save`),
- "Terminar el archivo con un salto de línea al guardar"
  (`files.ensure_final_newline_on_save`).

Con las dos apagadas, el archivo se guarda exactamente como lo ves.

La limpieza es **una sola edición tuya**: si no te gustó, un **`Ctrl+Z`** la
deshace (y el archivo queda como modificado otra vez). Si no había nada para
limpiar, no se hace nada. El cursor y la selección siguen a su texto, y los
comentarios siguen a sus líneas. Un archivo con saltos de línea de Windows
sigue con saltos de Windows.

Cincel **no limpia** en estos casos:

- **Archivos Markdown** (`.md` y `.markdown`): no se les quitan los espacios
  del final, porque dos espacios al final de una línea son un salto de línea
  en Markdown. El salto de línea final sí se agrega.
- **Las líneas de un cambio del agente que todavía no decidiste**: se
  guardan como están, así que lo que estás revisando no se altera. Las demás
  líneas del archivo sí se limpian.
- **Un archivo en el que el agente está trabajando** en ese momento (el turno
  sigue en curso): el guardado escribe el texto tal cual, sin limpiar.

> **En un archivo en revisión**: si limpiás un archivo que tiene cambios del
> agente, las líneas que ya decidiste (aceptadas, rechazadas o que el agente
> no tocó) sí se limpian, y eso **cuenta como una edición tuya**, igual que
> si hubieras borrado esos espacios a mano. Por eso viaja al agente, como
> parte de lo que editaste a mano, en tu próximo mensaje (ver
> [Revisión de cambios](revision.md#qué-recibe-el-agente)). Es correcto, porque el archivo
> cambió, pero puede sorprender si el agente te comenta unos espacios que
> no escribió.
