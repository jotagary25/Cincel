# Manual de Cincel

Cincel es un editor de código para Linux. Su idea central: cuando le pedís
algo a un agente de programación (Claude Code, Codex o Google Antigravity),
todo lo que cambia en tus archivos se ve en el propio editor, línea por
línea, en rojo y verde — y vos decidís qué queda y qué no. Nada se acepta
solo.

## Cincel en un minuto

1. Abrís una carpeta con tu proyecto.
2. Le hablás a un agente desde el chat, a la izquierda.
3. Mientras el agente escribe, tus archivos van cambiando de verdad en el
   disco (así puede compilar y probar lo que hace).
4. Cuando termina, cada cambio aparece en el editor como un segmento: lo que
   borró en rojo, lo que agregó en verde.
5. Aceptás o rechazás cada segmento (o cada línea, o el archivo entero, o
   todo el turno de una vez). Rechazar siempre se puede deshacer. También
   podés dejar un comentario sobre unas líneas para que el agente lo lea en
   tu próximo mensaje.

Capturas de referencia (tema oscuro, un proyecto de demostración):

![Revisión de cambios](../capturas/revision.png)
![Configuración](../capturas/configuracion.png)
![Buscador de archivos](../capturas/buscador.png)

## Índice

- [Instalación](instalacion.md) — tarball, `.deb`, requisitos, actualizar y
  desinstalar.
- [Primer arranque](primer-arranque.md) — abrir una carpeta, las tres zonas
  de la ventana, pestañas, guardar.
- [Conexiones](conexiones.md) — conectar Claude, Codex y Antigravity;
  la lista de conexiones; cuentas aisladas; renombrar, reparar, actualizar y
  desconectar.
- [El chat](chat.md) — el encabezado y el estado de la conexión, escribir
  mensajes, lo que hace el agente mientras trabaja, leer sin que el chat te
  mueva (y la flecha para volver al final), bloques de código largos
  plegados, adjuntar imágenes (pegar, arrastrar, botón del clip), verlas en
  grande y qué pasa si el agente no las acepta.
- [Escribir en el editor](editor.md) — marcar dónde más aparece una palabra,
  copiar y cortar la línea entera, la sangría, desplazarte más allá del
  final del archivo y la limpieza al guardar.
- [Revisión de cambios](revision.md) — la regla, los segmentos, aceptar y
  rechazar, comentarios para el agente, archivos creados y borrados,
  deshacer.
- [Atajos de teclado](atajos.md) — la tabla completa, por categoría.
- [Ajustes](ajustes.md) — la pestaña de configuración y `settings.json`.
- [Solución de problemas](problemas.md) — sesión vencida, sin navegador,
  descargas que fallan, sin GPU, el log.

Cincel es de uso personal y no manda ningún dato tuyo a ningún servidor
propio: no hay telemetría. Los agentes se conectan con tu propia
suscripción, como si abrieras su programa de línea de comandos vos mismo.
