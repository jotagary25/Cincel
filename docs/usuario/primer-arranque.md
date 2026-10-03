# Primer arranque

## Abrir una carpeta

Cincel trabaja siempre sobre una carpeta de proyecto. Podés abrir una de
tres formas:

- Desde el menú (el botón con tres rayitas, arriba a la izquierda) →
  **Abrir carpeta…**.
- Con el atajo `Ctrl+O`.
- Desde la línea de comandos: `cincel ~/mi-proyecto`.

Si abrís Cincel sin indicar ninguna carpeta (`cincel`, sin nada más), vuelve
al último proyecto que tenías abierto, con las mismas pestañas de archivo
que dejaste. La primera vez, o si no hay ningún proyecto anterior, ves una
pantalla de bienvenida con el botón para abrir una carpeta.

El menú también recuerda tus carpetas recientes, en **Carpetas
recientes ▸**.

## Las tres zonas de la ventana

![Editor con la revisión de un cambio](../capturas/revision.png)

- **Chat**, a la izquierda: acá hablás con el agente. Arriba tiene el botón
  **Conectar** para elegir con qué agente trabajar (ver
  [Conexiones](conexiones.md)); además de texto, podés mandarle imágenes (ver
  [El chat](chat.md)).
- **Editor**, en el centro: tus archivos, en pestañas. Acá también se ve la
  revisión de los cambios que hace el agente (ver
  [Revisión de cambios](revision.md)).
- **Archivos**, a la derecha: el árbol de tu proyecto. Un archivo que el
  agente cambió se marca con un contador `+N −M` (líneas agregadas y
  quitadas).

`Ctrl+L` mueve el foco del teclado de una zona a la siguiente, en ese
orden fijo (chat → editor → archivos → chat), sin abrir ni cerrar nada.
`Ctrl+Shift+A` y `Ctrl+Shift+E` muestran, ocultan o enfocan el chat y los
archivos: si el panel está oculto lo abren y te dejan ahí escribiendo; si ya
estabas adentro, lo cierran y te devuelven al editor.

## Pestañas y previsualización

Un solo clic en un archivo del árbol lo abre en **modo previsualización**
(el nombre de la pestaña aparece en cursiva): si abrís otro archivo así, la
misma pestaña se reemplaza, como en un navegador. Doble clic (o empezar a
editar) la **fija**: a partir de ahí queda su propia pestaña y abrir otro
archivo no la reemplaza.

La pestaña muestra un punto cuando el archivo tiene cambios sin guardar.
`Ctrl+S` guarda el archivo activo; `Ctrl+Alt+S` guarda todos los que tengan
cambios pendientes. `Ctrl+W` cierra la pestaña activa.

## Dónde vive la configuración

Todo lo que es tuyo (ajustes, atajos, conexiones, revisiones pendientes,
temas) vive bajo tu carpeta personal, nunca dentro del programa:

| Qué | Dónde |
|---|---|
| `settings.json`, `keymap.json`, temas | `~/.config/cincel/` |
| Conexiones con agentes y sus conversaciones | `~/.local/share/cincel/` |
| Revisiones pendientes, proyectos recientes, log | `~/.local/state/cincel/` |
| Descargas y cachés temporales | `~/.cache/cincel/` |

Para ver o cambiar los ajustes desde la interfaz: `Ctrl+,` (ver
[Ajustes](ajustes.md)).
