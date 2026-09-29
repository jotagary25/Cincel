# Revisión de cambios

Esta es la idea central de Cincel: **todo lo que cambia en la carpeta de tu
proyecto mientras el agente trabaja es del agente, y se revisa en el
editor**. No importa cómo lo haya hecho (editando un archivo, corriendo un
script, un comando de terminal): si cambió mientras el turno estaba activo,
aparece para que lo decidas. Lo único que no cuenta como cambio del agente
es lo que el propio Cincel escribe (un `Ctrl+S` tuyo, el autoguardado, un
archivo nuevo que creaste vos, o un rechazo).

## La foto

Antes de mandarle un mensaje al agente, Cincel se guarda una copia interna
de cómo están tus archivos en ese momento (la "foto"). Esa copia es lo que
permite mostrarte después qué cambió y deshacerlo si hace falta. Tiene un
tope de memoria configurable (por defecto 300 MB, ajustable en
`Ctrl+,` → Revisión → **"Memoria para la foto del proyecto"**): un archivo
que no entra en ese tope solo se puede aceptar entero, sin la revisión
línea por línea.

## Segmentos rojo y verde

Cuando el agente termina su turno, cada archivo que tocó se ve en el editor
con lo que borró en **rojo** (líneas de solo lectura, tachadas de hecho:
son la foto de cómo era antes) y lo que agregó en **verde**. El árbol de
archivos y las pestañas marcan con un punto los archivos con cambios
pendientes.

## Aceptar y rechazar

Sobre cada segmento aparece un botón flotante **Aceptar | Rechazar**. Al
pasar el mouse por una línea del segmento aparecen además `+` y `−` para
decidir esa línea sola.

Además, abajo a la derecha del editor hay una barra flotante con:
**"✓ Aceptar todo" / "✗ Rechazar todo"** (deciden el turno completo, no solo
este archivo), flechas para saltar al cambio anterior o siguiente, un
contador ("cambio 1 de 3") y **"Revisar todo"**, que abre el panel de
revisión (`Ctrl+Shift+R`) con la lista de archivos y sus `+N −M`.

Niveles de decisión, de más chico a más grande:

| Qué decidís | Cómo |
|---|---|
| Una línea | `+` / `−` al pasar el mouse, o `Alt+Enter` / `Alt+Backspace` |
| Un segmento | El botón flotante, o `Ctrl+Enter` / `Ctrl+Backspace` |
| Un archivo entero | `Ctrl+Shift+Enter` / `Ctrl+Shift+Backspace` |
| Todo el turno | `Ctrl+Alt+Enter` / `Ctrl+Alt+Backspace`, o los botones de la barra |

Aceptar deja el texto nuevo, sin color. Rechazar restaura el texto original
de ese segmento y guarda. Si tenés activado "Saltar al siguiente cambio al
decidir" (`Ctrl+,` → Revisión), después de decidir el cursor salta solo al
próximo cambio pendiente.

## Archivos creados y borrados

Un archivo que el agente **creó** aparece en el árbol en verde, con `+N`.
Uno que **borró** aparece tachado, con `−N` y el aviso "Borrado por el
agente · pendiente": sigue viéndose en el árbol aunque ya no esté en el
disco, hasta que decidís. Si lo abrís, ves una pestaña de **solo lectura**
con todo su contenido anterior en rojo, como un único segmento: pasá el
mouse por encima y usá sus botones "✓ Aceptar" / "✗ Rechazar" (aceptar
confirma el borrado y lo saca del árbol; rechazar restaura el archivo).
También podés decidirlo desde "Revisar todo", con la ✓ o la ✗ de su fila.

## Archivos binarios

Los archivos binarios (una imagen, un PDF, los `.pyc` de `__pycache__`,
cualquier archivo que no es texto) quedan **fuera de la revisión**: si el
agente los crea, los cambia o los borra, el cambio se aplica sin
preguntarte y no aparece como pendiente. Si abrís uno desde el árbol, la
pestaña dice "Archivo binario: no se muestra".

## Deshacer un rechazo

`Alt+Shift+U` deshace el último rechazo (el más reciente primero). El aviso
que aparece al rechazar te lo recuerda. Esto solo vive en memoria: no
sobrevive a cerrar Cincel.

## Escribir dentro de un segmento pendiente

Podés editar libremente, incluso dentro de un segmento en revisión. Lo que
vos escribas no cuenta como propuesta del agente: el segmento se reacomoda
alrededor de tu texto. `Ctrl+Z` deshace tu última edición como cualquier
otra; para deshacer directamente la propuesta completa del agente,
rechazala.

## Cerrar Cincel con cambios sin decidir

Si intentás cerrar la ventana (la `×`, `Ctrl+Q`, "Abrir carpeta…" o
"Carpetas recientes" con el menú) y hay cambios de agente sin decidir,
Cincel te avisa cuántos ("Hay N cambios de agente sin decidir en M
archivos") con los botones **Aceptar todo**, **Rechazar todo** y
**Cancelar**. Si además hay archivos con cambios tuyos sin guardar, después
te pregunta si querés guardarlos.

## Al reabrir el proyecto

Los cambios pendientes (de texto y binarios) siguen ahí cuando volvés a
abrir Cincel, siempre que el archivo no haya cambiado por fuera mientras
estaba cerrado. Si cambió por otro programa, esa revisión se descarta con
un aviso ("«archivo» cambió fuera de Cincel; se descartó su revisión"): no
se pierde nada del disco, solo la posibilidad de revisarlo segmento por
segmento.

## Lo que Cincel le cuenta al agente

En tu próximo mensaje, Cincel le agrega automáticamente al agente un
resumen de lo que rechazaste o modificaste de su propuesta anterior, para
que tenga en cuenta tu decisión sin que tengas que explicarla vos.
