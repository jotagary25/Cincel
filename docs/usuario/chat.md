# El chat: mensajes, imágenes y comentarios

El chat está a la izquierda de la ventana. Ahí le escribís al agente con el
que estés conectado (ver [Conexiones](conexiones.md)): sin una conexión
activa el chat te lo dice ("No hay ningún agente conectado") y no hay a
quién escribirle todavía.

## Arriba del chat

Con una conexión activa, la franja de arriba del chat muestra, de izquierda
a derecha:

- el **botón de la conexión**: el icono del agente y el **nombre completo**
  que le pusiste a esa conexión (por ejemplo "Claude personal"). Tocarlo
  abre el menú de [Conexiones](conexiones.md). El nombre usa todo el lugar
  libre; solo si el panel está tan angosto que no entra, se corta con "…" y
  el nombre entero aparece al dejar el mouse encima;
- **una sola etiqueta de color** con el estado de la conexión (no de lo que
  hace el agente):

  | Etiqueta | Qué quiere decir |
  |---|---|
  | **conectada** (verde) | el agente está en marcha y listo para responderte |
  | **desconectada** (rojo) | el proceso del agente se cayó o se detuvo |
  | **sesión vencida** (amarillo) | hay que iniciar sesión de nuevo (ver [Sesión vencida](conexiones.md#sesión-vencida)) |
  | **no disponible** (rojo) | falta algo que la conexión necesita; al dejar el mouse encima, un globo explica el motivo |
  | **autenticación requerida** (amarillo) | el agente pide que te autentiques |

  Si se dan varias a la vez, se muestra la más urgente, en este orden:
  sesión vencida, no disponible, autenticación requerida, desconectada y,
  al final, conectada;
- los botones de **Nueva conversación** y de **Historial** (el reloj).

La etiqueta **no cambia mientras el agente trabaja**: durante todo un turno
sigue diciendo "conectada". Lo que el agente está haciendo en cada momento
se ve abajo, en la conversación (ver
[Mientras el agente trabaja](#mientras-el-agente-trabaja)). Arriba tampoco
aparece el tipo de agente (Claude, Codex o Antigravity): ese dato está en
el menú de conexiones.

Sin ninguna conexión activa, en lugar de todo esto ves el botón
**Conectar** y ninguna etiqueta.

## Escribir un mensaje

- **`Enter`** envía el mensaje; **`Shift+Enter`** hace un salto de línea.
- Si escribís **`@`**, se abre una lista de archivos de tu proyecto para
  mencionar uno. Con **`/`** se abre la lista de comandos que ofrece el
  agente.
- **`Esc`** cancela lo que el agente esté haciendo en ese momento.
- **`Ctrl+Shift+N`** empieza una conversación nueva; el reloj de arriba abre
  el historial de conversaciones.

Además del texto, en la misma caja podés adjuntar **imágenes** (más abajo)
y, sin escribirlos en el chat, llevar **comentarios** sobre tu código (ver
[Comentarios para el agente](revision.md#comentarios-para-el-agente)): los
comentarios pendientes aparecen como etiquetas arriba del texto y viajan con
tu próximo mensaje.

## Mientras el agente trabaja

Desde que enviás un mensaje hasta que el agente termina, **abajo de todo en
la conversación** hay una fila con una ruedita girando que dice qué está
haciendo:

| Texto | Cuándo |
|---|---|
| **Pensando…** | el agente razona, o todavía no mandó ninguna señal |
| **Trabajando…** | está usando una herramienta (editar, leer, ejecutar, buscar) |
| **Escribiendo…** | te está llegando la respuesta |
| **Esperando permiso…** | pidió un permiso y espera tu decisión |

El texto cambia en el momento en que cambia lo que hace el agente: por
ejemplo, después de una herramienta vuelve a "Pensando…" si razona de
nuevo, y pasa a "Escribiendo…" cuando empieza la respuesta. Cuando el
agente ya muestra su propio bloque de razonamiento ("Pensando… (5 s)") como
último elemento, no se repite la fila. Al terminar o cancelar el turno la
fila desaparece, y no se guarda con la conversación.

## Leer la conversación sin que se mueva

- Si estás **al final**, el chat va bajando solo a medida que llega la
  respuesta.
- Si **subís a leer** algo anterior (con la rueda o arrastrando la barra),
  el chat **se queda donde lo dejaste**: no te mueve cuando llegan más
  mensajes, ni cuando el agente usa herramientas, ni cuando pide un permiso,
  ni cuando la respuesta termina.
- Mientras estás arriba aparece, flotando sobre la conversación, un **botón
  redondo con una flecha hacia abajo** ("Ir al final"). Tocarlo te lleva al
  final y el chat vuelve a seguir la respuesta. No aparece si la
  conversación es tan corta que no hay nada para desplazar.
- El chat **vuelve a seguir** también cuando bajás vos hasta el final, y
  cuando **enviás un mensaje** (así ves lo que mandaste).
- Abrir una conversación del historial o empezar una nueva te deja al
  final.

## Bloques de código largos

Un bloque de código de **más de 20 líneas** en la respuesta del agente (o en
un mensaje tuyo) no ocupa toda la pantalla: se muestra **plegado**, con sus
primeras 12 líneas, un difuminado abajo y un botón **"Ver más (N líneas)"**,
donde N es la cantidad de líneas que están ocultas. Un bloque de 20 líneas
o menos se ve entero.

- Tocá **"Ver más"** para desplegarlo entero; al pie del bloque pasa a decir
  **"Ver menos"**, que lo vuelve a plegar.
- Cada bloque se pliega y se despliega por separado. Cincel **recuerda
  cuáles desplegaste mientras la conversación esté abierta**; si la cerrás y
  la volvés a abrir desde el historial, vuelven a verse plegados.
- Mientras el bloque va llegando, se pliega apenas pasa las 20 líneas, y el
  número del botón se va actualizando.
- **"Copiar"** (en la cabecera del bloque) copia **siempre el bloque
  entero**, esté plegado o desplegado.
- Se pliegan los bloques que empiezan al margen izquierdo de la respuesta.
  Un bloque que viene metido adentro de una lista o de una cita se muestra
  siempre entero, igual que el código en línea y las salidas de las
  herramientas.
- Si arrastrás para seleccionar un texto que pasa del texto corriente a un
  bloque largo (o al revés), la selección se corta en el borde del bloque:
  seleccioná cada parte por separado.

## Imágenes

Podés mandarle al agente una captura de pantalla o una foto (por ejemplo,
"así se ve el error" o "quiero que quede como esto"). Sirven archivos
**PNG, JPEG, GIF y WebP**; Cincel reconoce el tipo mirando el contenido, no
la extensión del nombre.

### Tres formas de adjuntar

1. **Pegar** con **`Ctrl+V`** en la caja de texto. Si lo que copiaste es una
   imagen (por ejemplo una captura de pantalla), se adjunta y no se pega
   texto. Si copiaste archivos en el explorador de archivos del sistema, se
   adjuntan los que sean imágenes. Si copiaste solo texto, se pega el texto
   como siempre.
2. **Arrastrar** uno o varios archivos desde el explorador de archivos del
   sistema hasta la caja de texto. Mientras los tenés encima, el borde de la
   caja se marca con otro color para avisarte que ahí se sueltan.
3. El botón del **clip** ("Adjuntar imagen"), abajo a la izquierda dentro de
   la caja: abre la ventana para elegir archivos (podés elegir varios a la
   vez).

Si en el árbol de archivos de Cincel hacés clic derecho sobre una imagen y
elegís **"Mencionar en el chat"**, la imagen también se adjunta (en vez de
escribir `@archivo`). Con cualquier otro tipo de archivo, ese mismo menú
sigue insertando la mención `@archivo` como siempre.

> **Pegar archivos desde el explorador en Linux con Wayland**: los archivos
> que copiás en el explorador (con `Ctrl+C`) y pegás en el chat llegan y se
> adjuntan igual que los que arrastrás. Lo que depende de tu escritorio es el
> arrastrar y soltar entre ventanas: si en el tuyo no funciona, usá `Ctrl+V`
> o el botón del clip.

### Cómo se ven antes de enviar

Cada imagen adjunta aparece como una **miniatura** cuadrada dentro de la
caja, arriba del texto, con una **`×`** en la esquina para quitarla (no se
envía). Pasando el mouse por la miniatura ves su nombre, medidas y peso.
Mientras Cincel prepara una imagen, su miniatura muestra un indicador de
carga y la ayuda de la caja dice "Preparando 1 imagen…" (o "Preparando N
imágenes…"); `Enter` espera a que terminen antes de enviar.

Un mensaje puede tener solo imágenes, sin texto. Las imágenes se mandan en
el orden en que las adjuntaste (en el mensaje ya enviado se ven arriba del
texto).

### Límites

- **Hasta 10 imágenes por mensaje.** Si intentás pasar de ahí, Cincel
  adjunta las que entran y te avisa "Se pueden adjuntar hasta 10 imágenes
  por mensaje".
- **Tamaño**: si el lado más largo de una imagen pasa de **2 000
  píxeles**, Cincel la **achica** antes de mandarla (sin deformarla). Una
  imagen que ya es más chica se manda tal cual, con los bytes originales.
  Si aun achicada pesa más de **10 MB**, no se adjunta y te avisa con su
  peso ("«foto.png» pesa 14,2 MB aun reducida; el máximo es 10 MB").
- Un **GIF animado** que haya que achicar pierde la animación: se manda su
  primer cuadro y Cincel te avisa ("La animación de «x» se pierde al
  reducirla").
- Si un archivo no es una imagen de esos cuatro tipos, o no se puede leer,
  el aviso lo dice con su nombre ("«x» no es una imagen PNG, JPEG, GIF o
  WebP", "«x» no se pudo leer: …") y el resto de lo que adjuntaste sigue ahí.

### Ver una imagen en grande

En tu mensaje ya enviado, las imágenes se ven en chico **arriba del texto**
(y las tarjetas de comentario, debajo); un mensaje sin texto muestra solo
las miniaturas. Un
**clic** sobre una la abre en grande, centrada sobre un fondo oscuro que
cubre la ventana, a su tamaño real si entra o ajustada a la ventana si no,
con una línea abajo que dice nombre, medidas y peso ("foto.png · 1920 × 1080 ·
1,2 MB"). **`Esc`** la cierra; también la cierran un clic fuera de la
imagen o la `×` de arriba a la derecha. El teclado vuelve a donde estaba.

### Se guardan con la conversación

Las imágenes que enviaste se guardan junto con la conversación: cuando la
volvés a abrir desde el historial, siguen ahí. Si por algún motivo falta el
archivo de una imagen, en su lugar se ve "Imagen no disponible". Borrar una
conversación borra también sus imágenes.

### Si el agente no acepta imágenes

No todos los agentes ni todos los modelos entienden imágenes. Hoy Claude,
Codex y Antigravity las aceptan, pero Cincel lo comprueba cada vez:

- Si no hay ninguna conexión activa, al intentar adjuntar ves "Conectá un
  agente para adjuntar imágenes".
- Si el agente conectado no las acepta, ves "«Nombre» no acepta imágenes"
  (con el nombre de tu conexión) y no se adjunta nada, por ninguna de las
  tres vías.
- Si ya tenías imágenes en la caja y cambiás a una conexión que no las
  acepta, las miniaturas se quedan, arriba de la caja aparece "«Nombre» no
  acepta imágenes: quitá las imágenes para enviar" y `Enter` no envía hasta
  que las quites con la `×`.
- Puede pasar que el agente diga que acepta imágenes y aun así el modelo que
  elegiste no pueda leerlas (le pasa a algunos modelos de Codex). Entonces
  el agente responde con un error y Cincel lo muestra en el chat como
  cualquier otro error: probá con otro modelo desde el selector de abajo de
  la caja.

Ver también [Solución de problemas](problemas.md).
