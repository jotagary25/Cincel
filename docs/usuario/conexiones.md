# Conexiones

Una **conexión** es una cuenta de agente configurada en Cincel: por
ejemplo "Claude · personal" o "Codex · trabajo". Podés tener varias, incluso
del mismo proveedor.

## Aislamiento

Cada conexión vive en su propia carpeta, separada del resto del sistema:
Cincel **no** usa las sesiones que ya tenías abiertas en una terminal, ni
lee `~/.claude`, `~/.codex` ni `~/.gemini`, ni un Node que ya tengas
instalado. Todo lo que un agente necesita para funcionar (su sesión, su
propio Node privado si lo necesita, sus credenciales) queda dentro de esa
carpeta de la conexión. Conectar un agente en Cincel no afecta a las
sesiones de esa cuenta en otros programas, y viceversa.

## Conectar un agente por primera vez

1. En el chat, arriba, tocá **Conectar** y después **Conectar nuevo
   agente…**.
2. Elegí el agente: **Claude**, **Codex** o **Google Antigravity**. Si
   hace falta descargar algo (el adaptador y, para Claude y Codex, un Node
   privado de unos 50 MB; para Antigravity, su propio programa de unos
   333 MB) vas a ver el progreso de la descarga.
3. Cincel te muestra un enlace para iniciar sesión con tu cuenta de ese
   proveedor. Tocá **Abrir en el navegador** (o **Copiar** si preferís
   pegarlo vos) y aprobá el acceso ahí. Con Claude además te va a pedir que
   pegues el código que te da el navegador; con Codex y con Antigravity no
   hace falta: Cincel se entera solo de que aprobaste.
4. Cuando termina, ponele un nombre a la conexión (por ejemplo
   "Claude · personal") y tocá **Guardar**.

A partir de ahí, esa conexión queda en la lista y podés elegirla desde
**Conectar** cuando quieras. Solo una conexión está activa a la vez en el
chat: si elegís otra, la anterior se detiene (no se borra nada).

## La lista de conexiones y el botón del chat

El menú que se abre con **Conectar** (arriba en el chat) muestra una fila
por conexión, en **dos líneas**:

- a la izquierda, el **icono** del proveedor;
- en el medio, arriba el **nombre** que le diste a la conexión y abajo, en
  gris y más chico, **qué agente es** (Claude, Codex o Antigravity). El tipo
  se muestra siempre, también cuando el nombre es justo ese tipo: una
  conexión que se llama "Claude" y es de Claude dice "Claude" arriba y
  "Claude" abajo;
- a la derecha, la **etiqueta de estado**: **Conectada**, **Sesión vencida**
  o **No disponible** (si pasás el mouse por encima, un globo explica el
  motivo). A las que tienen la sesión vencida les aparece al lado **Volver a
  conectar**, y a las no disponibles, **Reparar**.

Las filas no muestran el correo de la cuenta, ni el plan, ni cuándo la
usaste por última vez.

El botón de arriba del chat (el que reemplaza a **Conectar** cuando ya hay
una conexión activa) es más simple: muestra el icono, el **nombre completo**
de la conexión y, a su lado, **una sola etiqueta** con el estado de la
conexión: **conectada**, **desconectada**, **sesión vencida**, **no
disponible** o **autenticación requerida**. Esa etiqueta habla de la
conexión, no de lo que hace el agente, así que no cambia mientras trabaja:
lo que está haciendo se ve abajo, en la conversación (ver
[El chat](chat.md#arriba-del-chat)). El botón no repite el tipo de agente;
ese dato está en el menú. La lista de **Eliminar conexión…** muestra icono,
nombre y tipo en una sola línea.

El **correo de la cuenta**, el **plan** y el **"Usado hace…"** ya no se ven
en el menú ni en el botón: están en `Ctrl+,` → **Conexiones**, en la fila de
cada conexión, junto a su nombre y sus acciones. (El aviso "Listo: conectado
como …" que ves justo al terminar de conectar sí muestra la cuenta: es el
mensaje de ese momento, no una lista.)

### Los menús se cierran solos

El menú de conexiones, el historial de conversaciones (el reloj), el selector
de modelo y modo del agente, y los menús de `@` y `/` del chat se cierran
solos cuando:

- hacés **clic en cualquier otro lado** (el editor, el árbol, el texto de la
  conversación…),
- pasás a otra zona con el teclado (por ejemplo con `Ctrl+L`),
- apretás **`Esc`**, o
- la ventana de Cincel deja de estar en primer plano (cambiás a otro
  programa).

Un clic sobre el mismo botón que abrió el menú lo cierra (y no lo vuelve a
abrir). Elegir una fila de la lista sigue funcionando como siempre. Lo mismo
vale para el menú de la barra de título, el del árbol de archivos y el del
clic derecho en el editor.

## Administrar tus conexiones

Desde la pestaña de ajustes (`Ctrl+,` → **Conexiones**) o desde el menú
contextual de cada fila en el popover **Conectar**:

- **Renombrar**: le cambiás la etiqueta, sin afectar la sesión ni las
  conversaciones.
- **Reparar**: si falta algo que la conexión necesita (por ejemplo,
  borraste sin querer el Node privado o el adaptador), esto lo vuelve a
  descargar sin tocar tu sesión iniciada.
- **Volver a conectar**: repite el inicio de sesión sobre el mismo perfil,
  conservando la etiqueta y las conversaciones (ver más abajo, "Sesión
  vencida").
- **Eliminar…**: pide confirmación y después cierra la sesión en el
  agente, borra sus credenciales de tu equipo y borra sus conversaciones.
  No se puede deshacer. Con Antigravity, además hay que revocar el acceso
  desde tu propia cuenta de Google (Cincel te lo recuerda).

## Sesión vencida

Si el token de una conexión venció o la contraseña de esa cuenta cambió,
la conexión aparece en la lista con la etiqueta **"Sesión vencida"**, la
etiqueta de arriba del chat dice **sesión vencida** y el chat muestra un
aviso con el botón **Volver a conectar**, que repite el inicio de sesión
sin perder el nombre ni el historial.

## Actualizar el adaptador y Node

Cada conexión de Claude o Codex usa un adaptador (el programa que traduce
entre Cincel y el agente) y un Node privado. Cuando hay una versión nueva
disponible, la fila de esa conexión en **Conexiones** muestra un botón
**"Actualizar a…"** con el número de versión; mientras se descarga podés
tocar **Cancelar** para cortarla sin dejar archivos a medio bajar. Lo mismo
existe para el Node privado.

## Varias cuentas del mismo proveedor

No hay límite: podés tener, por ejemplo, "Claude · personal" y
"Claude · trabajo" con cuentas distintas, cada una en su propia carpeta
aislada. Cambiar entre ellas es elegir la fila correspondiente en
**Conectar**.
