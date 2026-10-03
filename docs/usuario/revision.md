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

Sobre cada segmento aparece un botón flotante con tres partes:
**Aceptar | Rechazar | Comentar**. Las dos primeras deciden; la tercera te
deja escribirle una nota al agente sin decidir nada (ver
[Comentarios](#comentarios-para-el-agente)). Si la ventana es angosta, los
tres botones se ven más chicos, solo con su icono. Al pasar el mouse por una
línea del segmento aparecen además `+` y `−` para decidir esa línea sola.

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
confirma el borrado y lo saca del árbol; rechazar restaura el archivo). Ahí
también podés dejar un comentario sobre el archivo borrado (ver
[Comentarios](#comentarios-para-el-agente)).
También podés decidirlo desde "Revisar todo", con la ✓ o la ✗ de su fila.

## Archivos binarios

Los archivos binarios (una imagen, un PDF, los `.pyc` de `__pycache__`,
cualquier archivo que no es texto) quedan **fuera de la revisión**: si el
agente los crea, los cambia o los borra, el cambio se aplica sin
preguntarte y no aparece como pendiente. Si abrís uno desde el árbol, la
pestaña dice "Archivo binario: no se muestra". Tampoco se pueden comentar
(ver [Comentarios](#comentarios-para-el-agente)).

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

## Comentarios para el agente

Un **comentario** es una nota tuya, escrita sobre unas líneas de código, para
que el agente la lea en tu próximo mensaje. Sirve para decirle "esto hacelo
con un diccionario" justo donde lo querés, sin tener que explicar en el chat
de qué archivo y de qué líneas hablás. Comentar **no es decidir**: no acepta
ni rechaza nada. Podés comentar un segmento y además aceptarlo, rechazarlo,
decidirlo línea por línea o dejarlo sin decidir.

### Cómo escribir un comentario

Hay dos caminos, y los dos abren la misma cajita:

- **Sobre un cambio del agente**: el tercer botón del segmento,
  **"Comentar"**. Si ese segmento ya tenía un comentario hecho desde su
  botón, lo abre para editarlo. "Comentar" también funciona mientras el
  agente sigue escribiendo.
- **Sobre cualquier línea**, aunque el agente no la haya tocado: seleccioná
  las líneas y apretá `Ctrl+Shift+M`, o hacé clic derecho y elegí
  **"Comentar selección"**. Sin nada seleccionado, el menú dice **"Comentar
  línea"** y comenta la línea donde está el cursor. (Con el cursor dentro de
  un segmento pendiente y nada seleccionado, `Ctrl+Shift+M` comenta ese
  segmento, igual que el botón.)

El menú del clic derecho trae también **Cortar**, **Copiar** y **Pegar**. Si
hacés clic derecho fuera de lo que tenías seleccionado, el cursor pasa
primero a ese lugar.

La cajita aparece **debajo de las líneas** comentadas, intercalada entre
las del código (como las líneas rojas de lo que el agente quitó). No corre
el texto hacia los costados ni cambia el margen de los números de línea.
Arriba dice "Comentario para el agente" con el archivo y las líneas
(por ejemplo `calc.py:24-28`). Escribí tu nota (en varias líneas si
querés: `Enter` hace un salto de línea) y:

- **`Ctrl+Enter`** (o el botón **Guardar**) la guarda.
- **`Esc`** (o el botón **Cancelar**) descarta lo que escribiste. Si estabas
  editando un comentario que ya existía, queda como estaba.

Guardar una nota vacía es lo mismo que cancelar (y, si estabas editando,
borra el comentario). El máximo es de 8 000 caracteres; pasados los 7 000, la
ayuda de la cajita te dice cuántos te quedan.

Mientras escribís en la cajita, `Ctrl+Enter` guarda la nota y **no** acepta
el segmento, aunque el segmento esté justo ahí.

### Cómo se ve un comentario guardado

- En el margen, junto a la primera línea comentada, queda un iconito de
  mensaje (al pasar el mouse sobre él ves el comienzo del texto).
- La cajita queda **plegada**: una sola fila con el icono y tu nota en una
  línea, cortada con "…" si es larga. Un clic en el texto o en el iconito
  del margen la despliega y muestra la nota entera; otro clic la vuelve a
  plegar.
- **Editar** y **Borrar** aparecen a la derecha de la cajita cuando pasás el
  mouse por encima (o cuando el cursor del teclado está en esas líneas).

El comentario sigue a sus líneas: si escribís arriba, abajo o adentro, se
mueve con ellas. Si esas líneas desaparecen (las borraste, o un rechazo las
reemplazó), el comentario queda sobre la línea más cercana.

### Qué se ve en el chat antes de enviar

Cada comentario sin enviar aparece también en la caja del chat, arriba del
texto, como una etiqueta con su nombre de archivo y líneas (por ejemplo
`calc.py:24-28`) y una `×`. Ordenadas por archivo y línea. Podés:

- tocar la etiqueta para abrir el archivo en esa línea;
- tocar su `×` para borrar el comentario (desaparece también del margen);
- o apretar **Borrar** en la cajita del margen (desaparece la etiqueta).

No hay un botón aparte para enviarlos: **viajan con el próximo mensaje que
mandes** (de texto o con imágenes). Un mensaje vacío no se envía aunque haya
comentarios esperando.

La barra de estado de abajo cuenta los comentarios junto con los cambios
("3 cambios pendientes · 2 comentarios", o solo "2 comentarios" si no hay
cambios), y en el árbol cada archivo comentado muestra el icono y la
cantidad después de su `+N −M`.

### Qué recibe el agente

Solo lo que vos escribiste, **una sola vez**:

- Cada comentario con el archivo, las líneas, **cómo quedó ese cambio**
  (sin decidir, aceptado, rechazado, mitad aceptado y mitad rechazado, o "sin
  cambios del agente" si comentaste líneas que él no tocó), el código tal
  como está ahora y tu nota.
- Junto con eso, lo de siempre: las diferencias de lo que rechazaste o
  editaste a mano (ver más abajo).
- Nunca le llega lo que aceptaste o dejaste sin decidir por sí solo: sin
  comentario, no se entera.

Si el código comentado es muy largo (más de 120 líneas), el agente recibe una
parte y se le avisa cuántas líneas faltan. Si todavía no guardaste el archivo,
se le aclara que ese es el texto del editor.

Ejemplo. Pedís un cambio en `calc.py` y el agente le saca el descuento a
`total`. Rechazás ese segmento y le escribís: "No cambies la firma: agregá
el descuento como una función aparte". Además, en `util.py` seleccionás la
línea `TIMEOUT = 3`, elegís "Comentar selección" y escribís: "Esto debería
salir de la configuración". Escribís "atendé mis comentarios" y enviás. El
agente recibe, en ese orden:

1. Lo que rechazaste en `calc.py` (como siempre).
2. Los dos comentarios: el de `calc.py` con las líneas 24 a 25, estado
   "rechazado" y el código ya restaurado, y el de `util.py` con la línea 7,
   estado "sin cambios del agente" y la línea tal como está.

### Qué pasa al comentar y decidir

| Si... | El agente recibe |
|---|---|
| Comentás y aceptás | Solo tu nota (marcada como aceptada), sin las diferencias |
| Comentás y rechazás | Las diferencias del rechazo y tu nota (marcada como rechazada, con el código ya restaurado) |
| Comentás y no decidís | Tu nota, marcada como "sin decidir", con el código del agente |
| Comentás y editás a mano esas líneas | Las diferencias de tu edición y tu nota con el código **ya editado** |
| Comentás líneas que el agente no tocó | Tu nota, marcada "sin cambios del agente" |
| Decidís línea por línea | Tu nota con el estado que resulta: aceptado, rechazado o mitad y mitad (mientras queden líneas del rango sin decidir, sigue "sin decidir") |

El estado se calcula **al enviar**, con lo último que hayas decidido: podés
comentar, y después "Aceptar todo" o "Rechazar todo", y el comentario sigue
en el margen con el estado nuevo hasta que lo mandes.

Si el agente responde reescribiendo esas mismas líneas, el segmento se
actualiza contra el original, como cuando le pedís cualquier otro cambio.

### Después de enviar

- Los comentarios enviados **desaparecen** del margen, de la caja del chat,
  del contador y de lo que Cincel guarda al cerrar: **no se repiten** en el mensaje siguiente.
- Tu mensaje en el chat muestra una **tarjeta por comentario**: archivo,
  líneas, estado, el código (hasta 6 líneas; un clic muestra el resto) y tu
  nota. Tocar el encabezado abre el archivo en esa línea.
- Si el mensaje no llega a salir (por ejemplo lo cancelás mientras Cincel
  prepara la foto), los comentarios **vuelven** al margen y a la caja del
  chat, y la tarjeta dice "No se envió: el comentario volvió al margen".
  Si salió, no vuelven, aunque el agente falle después.
- Cambiar de conexión o empezar una conversación nueva **no** toca los
  comentarios sin enviar: son de tu proyecto, no de la conexión, y viajan con
  el próximo mensaje a la conexión que esté activa.

### Cerrar y volver a abrir

Los comentarios sin enviar **se guardan al cerrar Cincel** y vuelven al abrir
la carpeta, en las mismas líneas. Si el archivo cambió por fuera mientras
tanto, Cincel busca el código comentado y lo vuelve a ubicar; si no lo
encuentra, el comentario queda en la línea más cercana. Si el archivo ya no
existe, el comentario se descarta con un aviso ("El comentario sobre «x» se
descartó: el archivo ya no existe").

### Lo que no se puede comentar

- **Archivos binarios** (imágenes, PDF, etc.): no tienen texto que mostrar
  ("Archivo binario: no se muestra"), así que no tienen botón, menú ni
  atajo para comentar. Si un archivo de texto comentado pasa a ser binario
  durante un turno del agente, el comentario se descarta con el aviso "El
  comentario sobre «x» se descartó: el archivo ya no es de texto".
- La caja de texto del chat (ahí `Ctrl+Shift+M` no hace nada).

## Cerrar Cincel con cambios sin decidir

Si intentás cerrar la ventana (la `×`, `Ctrl+Q`, "Abrir carpeta…" o
"Carpetas recientes" con el menú) y hay cambios de agente sin decidir,
Cincel te avisa cuántos ("Hay N cambios de agente sin decidir en M
archivos") con los botones **Aceptar todo**, **Rechazar todo** y
**Cancelar**. Si además tenés comentarios sin enviar, el aviso suma una línea
("También hay 2 comentarios sin enviar: se guardan y vuelven al abrir la
carpeta."); ningún botón los borra. Si solo tenés comentarios (ningún cambio
pendiente), el aviso no aparece: se guardan igual. Si además hay archivos con
cambios tuyos sin guardar, después te pregunta si querés guardarlos.

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
que tenga en cuenta tu decisión sin que tengas que explicarla vos. Si
escribiste [comentarios](#comentarios-para-el-agente), van en ese mismo
aviso, después del resumen. Lo que aceptaste, o dejaste sin decidir, no se le
cuenta por sí solo, y nada se manda dos veces.
