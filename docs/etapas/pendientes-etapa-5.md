> **Nota (2026-09-28): la Etapa 5 se cerró.** Su fuente de verdad fue
> `docs/specs/07-etapa5-productividad.md`; su alcance construido, verificación,
> desviaciones y lista de comprobación manual están en
> `docs/etapas/etapa-5.md`. Esta lista queda como registro histórico. Lo
> pendiente vive en dos lugares: el trabajo ya planificado para después de la
> v1.0 (rendimiento, instalador, repositorio público, documentación de
> usuario, fuentes embebidas) está en `docs/specs/05-plan-etapas.md`
> (Etapa 6); las deudas técnicas menores que ninguna de las dos etapas asumió
> todavía siguen abiertas en la sección B de más abajo, con las que la
> Etapa 5 sí resolvió marcadas como tales.
>
> **Nota (2026-09-29): la Etapa 6 se cerró.** Cierra también la versión
> **1.0**. Su fuente de verdad fue `docs/specs/08-etapa6-cierre-1-0.md`; su
> alcance construido, verificación, decisiones y desviaciones están en
> `docs/etapas/etapa-6.md`. Los puntos 5 a 9 de la sección A (rendimiento,
> instalador, repositorio público, documentación de usuario, fuentes
> embebidas) y las deudas de la sección B que le tocaban ya están cerrados,
> marcados como tales más abajo. Lo que queda abierto después de la 1.0
> apunta a `docs/etapas/etapa-6.md` §"Después de la 1.0".

# Pendientes para la Etapa 5 (v1.0) y después

Recopilado el 2026-09-27 al cerrar la Etapa 4, revisando toda la conversación de diseño y las desviaciones anotadas en `docs/etapas/etapa-0.md` a `etapa-4.md`.

## A. Etapa 5: alcance acordado (cierre = versión 1.0)

Los puntos 1 a 4 se construyeron en la Etapa 5 (ver `docs/etapas/etapa-5.md`);
los puntos 5 a 9 pasaron a la Etapa 6 (`docs/specs/05-plan-etapas.md`).

1. ~~**Buscador rápido de archivos** (`Ctrl+P`): búsqueda difusa sobre el árbol con `frizbee`, abre en previsualización; respeta `.gitignore`.~~ **Construido.**
2. ~~**Pantalla de configuración** (`Ctrl+,`): secciones con interruptores y selectores para lo ya construido (tema y modo claro/oscuro, fuentes y tamaños, ajuste de línea, tabulación y espacios, auto-cierre de pares, autoguardado, saltar al siguiente cambio al decidir, rutas sensibles, conexiones: renombrar, reparar, volver a conectar, eliminar). Escribe el mismo `settings.json`; las dos vías conviven.~~ **Construido.**
3. ~~**Botón visible y modal de atajos** con buscador, agrupados por categoría (generales, editor, revisión, chat, conexiones), leídos del keymap efectivo (incluye los del usuario).~~ **Construido.**
4. ~~**Git en el margen del editor**: barras de añadido/modificado/eliminado por línea desde `git diff`, sin pisar los segmentos del agente (renderer distinto, como Zed); en el árbol ya está.~~ **Construido.**
5. ~~**Rendimiento**: medir y publicar arranque en frío, latencia de tecleo, memoria en reposo, CPU en reposo (0% sin redibujo continuo), archivos de 1 MB y 50 000 líneas; corregir lo que no cumpla `01-producto.md §5`.~~ **Construido en la Etapa 6** (`docs/rendimiento.md`, `docs/etapas/etapa-6.md`).
6. ~~**Instalador**: tarball + `install.sh` con `dist` (build en contenedor Ubuntu 22.04, vendorizando `libxkbcommon`/`libxcb*`/`libstdc++`, `dlopen` de wayland-client), `.deb` con `cargo-deb`; decisión tomada: instalador liviano (los agentes se descargan al conectar).~~ **Construido en la Etapa 6** (`packaging/`, `docs/etapas/etapa-6.md`). La opción "preparar todos los agentes" desde configuración no entró en el alcance acordado de la Etapa 6 (`08-etapa6-cierre-1-0.md` §9): queda para después de la 1.0.
7. ~~**Repositorio público**: URL real en `Cargo.toml`, README final con capturas, `CONTRIBUTING`, CI de GitHub Actions (fmt, clippy `-D warnings`, tests, `cargo deny`), y renombrar la carpeta del repo a `cincel` (lo hace el autor).~~ **Construido en la Etapa 6**: todo lo que no requiere la cuenta del autor está listo (`docs/etapas/etapa-6.md`); crear el repositorio, empujar y renombrar la carpeta local siguen siendo del autor (`docs/publicacion.md`).
8. ~~**Documentación de usuario**: instalación, primer arranque, conexiones, revisión de cambios, atajos, ajustes.~~ **Construido en la Etapa 6** (`docs/usuario/`).
9. ~~**Fuentes**: Inter y JetBrains Mono no están en la máquina de referencia y sin ellas la negrita del markdown no cambia de peso y el aspecto difiere de la spec. Propuesta: embeber ambas en el binario (licencia OFL, permitida) para un aspecto idéntico en cualquier equipo.~~ **Construido en la Etapa 6** (`crates/cincel-workspace/src/fonts.rs`, `docs/specs/08-etapa6-cierre-1-0.md` §4).

## B. Deudas técnicas anotadas en las etapas (candidatas para la 5 o inmediatamente después)
- ~~`AgentEvent::AuthRequired` no trae el `message` del agente: el motivo real solo llega por el tail de stderr (hallazgo del caso Gemini). Agregar `message: Option<String>` en `cincel-acp`.~~ **Resuelto en la Etapa 5** (`docs/specs/07-etapa5-productividad.md` §10.1, `docs/specs/modulos/acp.md`).
- Un test falló una sola vez en la Etapa 1 y no se repitió en seis corridas; nunca se identificó. Vigilar en CI con reintento y registro del nombre. (Distinto del test intermitente de conexiones que sí se corrigió en la Etapa 5, ver `docs/etapas/etapa-5.md`.)
- ~~Búsqueda en el editor: no recorre filas fantasma y no tiene reemplazar.~~ **Resuelto en la Etapa 5** (`docs/specs/07-etapa5-productividad.md` §10.2, `docs/specs/modulos/editor.md`).
- Ajuste de línea medido en columnas monoespaciadas; los tabs se pintan como `tab_size` espacios, no al siguiente tab stop; sin columna objetivo en píxeles en prosa (compositor).
- Código inline en respuestas del agente a ~11,4 px por límite de gpui-kit 0.6.1; tira de título vacía sobre las pestañas (gpui-kit no permite suprimirla); revisar al subir de versión de gpui-kit.
- ~~Autoguardado solo "al perder foco"; falta "tras N segundos".~~ **Resuelto en la Etapa 5** (`docs/specs/07-etapa5-productividad.md` §10.5, `files.autosave = "after_delay"`).
- Chat: pegar imágenes no soportado y sin aviso; arrastrar del árbol al chat no existe (solo "Mencionar en el chat" por menú contextual) por falta de fuente de arrastre en gpui-kit.
- Conexiones: ~~cancelar durante "Preparando…" no corta la descarga en curso (termina y se descarta);~~ **resuelto en la Etapa 5** (`docs/specs/07-etapa5-productividad.md` §10.3, `CancelToken`); "sin navegador" se detecta por heurística; identidad de Antigravity no disponible (su token no trae email); Claude y Codex no informan tamaño de descarga por adelantado; ~~"Actualizar adaptador" cuando el registry publica versión nueva está previsto pero sin botón visible;~~ **resuelto en la Etapa 5** (§10.4, botón "Actualizar a X.Y.Z" en la pestaña de configuración); ~~Node privado: política de actualización de versión LTS.~~ **resuelta en la Etapa 5** (`Runtime::update_available`/`update`).
- Conversaciones: al eliminar una, deshacer de 5 s; las de conexiones anteriores quedan en "(conexión anterior)" solo lectura; al abrir un proyecto se arranca con conversación vacía (decisión, revisable).
- `settings.connections.mcp_servers` se pasa al agente pero no tiene interfaz ni prueba con un servidor MCP real.
- Formularios de `elicitation` (`form`): se responden pero la interfaz de formulario en el chat es mínima; verificar con un caso real (OAuth de MCP en Claude).
- Antigravity: 333 MB/1 GB, sin suma de verificación publicada, bug conocido en contenedores (seccomp) no aplica a escritorio; sus registros van al `tmp/` del perfil (verificar en uso real que no vuelva a escribir en `/tmp`).
- `cargo` desde fuera del repo no aplica el toolchain (documentar `cargo +stable` o usar `rust-toolchain` en CI).
- El directorio `target` crece hasta llenar el disco por las variantes de features; CI y desarrollo deben usar siempre el mismo conjunto (`cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`).

## C. Decidido para después de la v1.0
- OpenCode y agentes con API key o gateway (métodos `gemini-api-key`, `agent-platform`, `api-key`, gateways).
- Paleta de comandos (`Ctrl+Shift+P`).
- Autocompletado y diagnósticos LSP; terminal integrado (y login dentro de la app en terminal real); comentarios sobre segmentos para dialogarlos con el agente; multi-cursor y plegado; minimapa; búsqueda en todo el proyecto; panel de git (commits, blame); texto enriquecido tipo claude.ai en el compositor; ACP v2 cuando salga de borrador (diffs con rename/delete/binarios, terminal del agente, prompts con steering); macOS y Windows.
