# Solución de problemas

## Sesión vencida

Si una conexión deja de funcionar porque el token venció, la contraseña de
esa cuenta cambió o algo similar, la fila de esa conexión muestra la
insignia **"Sesión vencida"** y el chat muestra un banner ("La sesión de
«…» venció") con el botón **Volver a conectar**. Tocalo y repetí el inicio
de sesión: conservás el nombre de la conexión y todas sus conversaciones
anteriores. Ver [Conexiones](conexiones.md).

## «Nombre» no acepta imágenes

Al adjuntar una imagen al chat (con `Ctrl+V`, arrastrándola o con el botón del
clip) ves el aviso **"«Nombre» no acepta imágenes"**, con el nombre de tu
conexión. Significa que ese agente le dijo a Cincel que no sabe leer
imágenes, así que Cincel no la adjunta. Qué hacer:

- Probá con otra conexión (**Conectar** arriba del chat): hoy Claude, Codex y
  Antigravity las aceptan.
- Si ya habías adjuntado imágenes y cambiaste de conexión, ves arriba de la
  caja "«Nombre» no acepta imágenes: quitá las imágenes para enviar". Quitalas
  con la `×` de cada miniatura (o cambiá de nuevo a una conexión que las
  acepte) y podés enviar.
- Si el agente sí las acepta pero el modelo elegido no (pasa con algunos
  modelos de Codex), el agente contesta con un error en el chat: elegí otro
  modelo en el selector de abajo de la caja.

Ver [El chat](chat.md#imágenes).

## Conectá un agente para adjuntar imágenes

Si intentás adjuntar una imagen sin ninguna conexión activa (todavía no
conectaste un agente, o la conexión se está iniciando), ves el aviso
**"Conectá un agente para adjuntar imágenes"** y no se adjunta nada. Tocá
**Conectar** arriba del chat y elegí o creá una conexión (ver
[Conexiones](conexiones.md)); cuando esté activa, volvé a intentarlo.

## No tengo navegador, o no se abre solo

Al conectar un agente o renovar una sesión, Cincel te muestra el enlace de
inicio de sesión en texto monoespaciado con dos botones: **Copiar** y
**Abrir en el navegador**. Si este último no hace nada (por ejemplo, no
tenés ningún navegador configurado como predeterminado), usá **Copiar** y
pegá el enlace vos mismo en el navegador que quieras, en cualquier equipo.
Cuando apruebes el acceso ahí, Cincel se entera solo (por eso no hace falta
que sea el mismo equipo, salvo con Claude, que además te pide pegar de
vuelta un código).

## Una descarga falla o queda a medias

Conectar Claude o Codex por primera vez descarga un Node privado y un
adaptador; Antigravity descarga su propio programa. Si la descarga falla
(sin red, por ejemplo), Cincel te lo dice con un mensaje claro y el botón
**Reintentar**. Si querés cortarla vos, el botón **Cancelar** corta la
descarga de verdad y borra lo que había quedado a medio bajar: no deja
archivos sueltos en `~/.cache/cincel/downloads/`. Para reintentar una
conexión rota más adelante, usá **Reparar** en la lista de conexiones (ver
[Conexiones](conexiones.md)).

## Sin GPU o sin Vulkan

Cincel necesita una GPU con Vulkan para arrancar con buen rendimiento. Si
tu equipo no tiene una, Cincel no abre la ventana y te lo explica en la
terminal. Para forzarlo igual, con un rasterizador por software (mucho más
lento, pero funciona):

```sh
CINCEL_ALLOW_SOFTWARE_GPU=1 cincel
```

## La ventana no abre

- Comprobá la versión: `cincel --version`. Si no imprime nada o da un
  error de biblioteca faltante, revisá `instalacion.md`: `install.sh` te
  avisa qué paquete del sistema instalar si falta alguno
  (`libxkbcommon-x11-0`, por ejemplo).
- Probá `cincel --smoke-test`: abre una ventana, dibuja un cuadro y sale
  con código 0 si todo está bien; si falla, el motivo queda en la
  terminal.
- Si sospechás de un ajuste roto en `settings.json` o `keymap.json`, movelos
  un momento a otro nombre y volvé a abrir Cincel: si arranca así, el
  problema está en ese archivo (revisá el mensaje de error, que dice la
  línea).

## Dónde está el log, y cómo compartirlo sin datos personales

Cincel escribe un registro en `~/.local/state/cincel/log/cincel.log` (se
rota todos los días, se guardan los últimos 7). Por defecto anota lo
esencial (`info`); para más detalle, arrancalo con más nivel:

```sh
RUST_LOG=debug cincel
```

El log **nunca** incluye tokens, enlaces de inicio de sesión ni códigos de
autenticación: esos se filtran antes de escribirse. Aun así, antes de
pegarlo en un reporte de error revisá que no queden rutas con tu nombre de
usuario u otro dato que no quieras compartir, y recortá solo las últimas
líneas relevantes al problema.

## Empezar de cero sin perder tus proyectos

Tus carpetas de proyecto están en el disco, donde las creaste vos: no
dependen en nada de Cincel, así que nunca se pierden por tocar la
configuración del programa. Si querés que Cincel arranque como recién
instalado:

- Para solo los ajustes y los atajos: borrá o renombrá
  `~/.config/cincel/settings.json` y `~/.config/cincel/keymap.json` (o
  usá el botón "Restablecer" de cada ajuste en `Ctrl+,`).
- Para además olvidar tus conexiones con agentes: borrá
  `~/.local/share/cincel/connections/` (vas a tener que volver a conectar
  cada agente).
- Para además olvidar tus revisiones pendientes y la lista de proyectos
  recientes: borrá `~/.local/state/cincel/`.

Ninguna de estas carpetas contiene tus archivos de proyecto: son solo la
configuración y los datos internos de Cincel.
