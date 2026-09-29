# Atajos de teclado

Esta es la tabla completa de los atajos que trae Cincel por defecto, la
misma que ves apretando `F1` (o el icono de teclado de la barra de estado,
o "Atajos de teclado" en el menú). Ahí además podés buscar por acción o por
tecla, y se marca en amarillo "Personalizado" cualquiera que hayas
cambiado.

Para cambiar un atajo, editá `~/.config/cincel/keymap.json` (el modal tiene
un enlace "Abrir keymap.json" que lo crea si no existe). Cada fila de ese
archivo dice, entre llaves, en qué situación aplica ("contexto") y qué tecla
hace qué acción; podés copiar una sección de acá abajo, cambiarle la tecla y
Cincel la aplica sola, sin reiniciar.

Las teclas se escriben con `Ctrl`, `Alt` y `Shift` antes de la tecla
(`Ctrl+Shift+A`); `Esc`, `Supr`, `RePág`/`AvPág`, `Inicio`/`Fin` y las
flechas `↑ ↓ ← →` son sus propias teclas.

## Generales

| Tecla(s) | Qué hace |
|---|---|
| `Ctrl+O` | Abrir una carpeta como proyecto |
| `Ctrl+N` | Crear un archivo nuevo |
| `Ctrl+Q` | Salir de Cincel |
| `Ctrl+,` | Abrir la configuración |
| `F1` | Ver los atajos de teclado |
| `Ctrl+Shift+A` | Mostrar u ocultar el chat |
| `Ctrl+Shift+E` | Mostrar u ocultar los archivos |
| `Ctrl+L` | Saltar a la próxima zona de foco (chat → editor → archivos → chat) |
| `Ctrl+P` | Buscar archivos |
| `Ctrl+W` | Cerrar la pestaña activa |
| `Ctrl+Shift+V` | Alternar la vista previa de Markdown (con el editor o la pestaña enfocados) |
| `Ctrl+=` | Agrandar la interfaz |
| `Ctrl+-` | Achicar la interfaz |
| `Ctrl+0` | Restablecer el zoom de la interfaz |
| `Enter` (en el árbol de archivos) | Abrir el archivo seleccionado en el árbol |
| `Esc` (en el modal de atajos) | Cerrar el modal de atajos |

## Editor

### Guardar, deshacer y buscar

| Tecla(s) | Qué hace |
|---|---|
| `Ctrl+S` | Guardar el archivo |
| `Ctrl+Alt+S` | Guardar todos los archivos |
| `Ctrl+Z` | Deshacer |
| `Ctrl+Shift+Z`, `Ctrl+Y` | Rehacer |
| `Ctrl+F` | Buscar en el archivo |
| `Ctrl+H` | Buscar y reemplazar en el archivo |
| `F3` | Ir a la coincidencia siguiente |
| `Shift+F3` | Ir a la coincidencia anterior |
| `Ctrl+G` | Ir a una línea |
| `Esc` (en el editor, sin la búsqueda abierta) | Cerrar la búsqueda o el diálogo abierto |
| `Alt+Z` | Alternar el ajuste de línea |

### La barra de búsqueda (`Ctrl+F` / `Ctrl+H`)

Mientras la barra de búsqueda está abierta, estas teclas mueven el foco
**dentro de la barra** en lugar de editar el archivo (nota de diseño más
abajo):

| Tecla(s) | Qué hace |
|---|---|
| `Enter` | Ir a la coincidencia siguiente |
| `Shift+Enter` | Ir a la coincidencia anterior |
| `Esc` | Cerrar la búsqueda |
| `Tab` | Mover el foco al otro campo del buscador |
| `Shift+Tab` | Mover el foco al otro campo del buscador (hacia atrás) |
| `Alt+R` | Alternar expresiones regulares en la búsqueda |
| `Alt+C` | Alternar mayúsculas y minúsculas en la búsqueda |

Y, con el foco en el campo **"Reemplazar…"**:

| Tecla(s) | Qué hace |
|---|---|
| `Enter` | Reemplazar la coincidencia actual |
| `Ctrl+Enter` | Reemplazar todas las coincidencias |

> Nota de diseño: si vos mismo reasignás `Tab`, `Enter` o `Ctrl+Enter` en tu
> propia sección "Editor" de `keymap.json` (sin mencionar "buscando"), esa
> reasignación gana también dentro de la barra de búsqueda — igual que en
> Zed. Es la única excepción a "la barra de búsqueda manda mientras está
> abierta".

### Cursor, selección y edición

| Tecla(s) | Qué hace |
|---|---|
| `←` / `→` | Mover el cursor un carácter a la izquierda / Mover el cursor un carácter a la derecha |
| `↑` / `↓` | Mover el cursor una fila arriba / Mover el cursor una fila abajo |
| `Ctrl+←` / `Ctrl+→` | Mover el cursor a la palabra anterior / Mover el cursor a la palabra siguiente |
| `Inicio` / `Fin` | Ir al inicio de la línea / Ir al final de la línea |
| `RePág` / `AvPág` | Subir una pantalla / Bajar una pantalla |
| `Ctrl+Inicio` / `Ctrl+Fin` | Ir al inicio del documento / Ir al final del documento |
| `Shift+←` / `Shift+→` | Extender la selección un carácter a la izquierda / Extender la selección un carácter a la derecha |
| `Shift+↑` / `Shift+↓` | Extender la selección una fila arriba / Extender la selección una fila abajo |
| `Ctrl+Shift+←` / `Ctrl+Shift+→` | Extender la selección a la palabra anterior / Extender la selección a la palabra siguiente |
| `Shift+Inicio` / `Shift+Fin` | Extender la selección al inicio de la línea / Extender la selección al final de la línea |
| `Shift+RePág` / `Shift+AvPág` | Extender la selección una pantalla arriba / Extender la selección una pantalla abajo |
| `Ctrl+Shift+Inicio` / `Ctrl+Shift+Fin` | Extender la selección al inicio del documento / Extender la selección al final del documento |
| `Ctrl+A` | Seleccionar todo |
| `Ctrl+Shift+L` | Seleccionar la línea del cursor |
| `Ctrl+D` | Seleccionar la palabra bajo el cursor o la siguiente coincidencia |
| `Backspace` | Borrar el carácter anterior |
| `Supr` | Borrar el carácter siguiente |
| `Ctrl+Backspace` | Borrar la palabra anterior |
| `Ctrl+Supr` | Borrar la palabra siguiente |
| `Enter`, `Ctrl+Enter` (sin segmento bajo el cursor) | Insertar un salto de línea |
| `Tab` (sin la búsqueda abierta) | Indentar o insertar una tabulación |
| `Shift+Tab` (ídem) | Quitar un nivel de indentación |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copiar la selección / Cortar la selección / Pegar el portapapeles |
| `Alt+↑` | Mover las líneas seleccionadas hacia arriba |
| `Alt+↓` | Mover las líneas seleccionadas hacia abajo |
| `Alt+Shift+↑` | Duplicar las líneas seleccionadas arriba |
| `Alt+Shift+↓` | Duplicar las líneas seleccionadas debajo |
| `Ctrl+Shift+K` | Borrar las líneas seleccionadas |
| `Ctrl+/` | Comentar o descomentar las líneas seleccionadas |
| `Ctrl+J` | Unir la línea con la siguiente |
| `Ctrl+Shift+\`, `Ctrl+|` | Ir al paréntesis o llave que corresponde |

## Revisión

| Tecla(s) | Qué hace |
|---|---|
| `Ctrl+Enter` (con un segmento bajo el cursor) | Aceptar el segmento bajo el cursor |
| `Ctrl+Backspace` (ídem) | Rechazar el segmento bajo el cursor |
| `Alt+Enter` (ídem) | Aceptar la línea bajo el cursor |
| `Alt+Backspace` (ídem) | Rechazar la línea bajo el cursor |
| `Ctrl+Shift+Enter` | Aceptar todos los cambios del archivo |
| `Ctrl+Shift+Backspace` | Rechazar todos los cambios del archivo |
| `Ctrl+Alt+Enter` | Aceptar todos los cambios del último turno |
| `Ctrl+Alt+Backspace` | Rechazar todos los cambios del último turno |
| `Alt+J`, `F7` | Ir al próximo cambio pendiente (en toda la ventana) |
| `Alt+K`, `Shift+F7` | Ir al cambio pendiente anterior (en toda la ventana) |
| `Alt+J`, `F7` (con el editor enfocado) | Ir al segmento pendiente siguiente, dentro del archivo |
| `Alt+K`, `Shift+F7` (ídem) | Ir al segmento pendiente anterior, dentro del archivo |
| `Alt+L` | Ir al próximo archivo con cambios pendientes |
| `Ctrl+Shift+R` | Abrir o cerrar el panel de revisión |
| `Alt+Shift+U` | Deshacer el último rechazo |

## Chat

| Tecla(s) | Qué hace |
|---|---|
| `Enter` | Enviar el mensaje |
| `Shift+Enter` | Insertar un salto de línea en el mensaje |
| `Esc` | Cancelar el turno en curso |
| `Ctrl+Shift+N` | Iniciar una sesión nueva |
| `↑` (con un menú emergente abierto: `@`, `/`, selectores) | Mover la selección hacia arriba en el menú emergente |
| `↓` (ídem) | Mover la selección hacia abajo en el menú emergente |
| `Tab` (ídem) | Confirmar la fila seleccionada del menú emergente |
| `Enter` (con un permiso pendiente) | Aceptar el permiso pendiente con la primera opción |
| `Esc` (ídem) | Rechazar el permiso pendiente |

## Buscador de archivos (`Ctrl+P`)

| Tecla(s) | Qué hace |
|---|---|
| `↓` | Mover la selección hacia abajo |
| `↑` | Mover la selección hacia arriba |
| `AvPág` | Bajar diez filas |
| `RePág` | Subir diez filas |
| `Enter` | Abrir el archivo seleccionado |
| `Esc` | Cerrar el buscador de archivos |

## Conexiones

Conectar, cambiar o administrar conexiones no tiene atajo de teclado propio
por ahora: se hace desde el botón **Conectar** del chat o desde
`Ctrl+,` → **Conexiones** (ver [Conexiones](conexiones.md)).
