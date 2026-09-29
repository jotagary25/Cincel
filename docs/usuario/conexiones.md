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
la conexión aparece en la lista con la insignia **"Sesión vencida"** y el
chat muestra un aviso con el botón **Volver a conectar**, que repite el
inicio de sesión sin perder el nombre ni el historial.

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
