# Spec 07: Etapa 5, productividad (buscador, configuración, atajos, git en el margen, foco y menú)

Estado: v1.1 (2026-09-28), definida con el autor.

**v1.1 (2026-09-28):** sin rueda de foco hacia atrás (solo `Ctrl+L`; `select_line` sigue en `Ctrl+Shift+L`); `Ctrl+N` = "Nuevo archivo" (se elimina de `02-visual.md §8` la regla "nunca `Ctrl+Y` ni `Ctrl+N`" y su test); el margen del editor conserva exactamente su ancho y ninguna función de la etapa desplaza el texto; la `×` de la ventana también pregunta por archivos sin guardar.

Versión 1.0 del mismo día: Reemplaza a `docs/etapas/pendientes-etapa-5.md` como fuente de verdad del alcance de la Etapa 5. Lo que ahí figura como rendimiento, instalador, repositorio público, documentación y fuentes pasa a la Etapa 6 (`05-plan-etapas.md`).

Modelo de formato: `06-etapa4-conexiones-y-cincel.md`. Cada ítem tiene primero un **resumen en lenguaje llano** (para el autor) y después el **diseño técnico** (para los subagentes).

Esta spec corrige otras en tres puntos, que se anotan acá porque no se tocan esos archivos en esta etapa (el orquestador los actualiza al cerrar, E5-K):
- `01-producto.md §3` excluía de la v1 el buscador de archivos difuso: ahora entra.
- `02-visual.md §8`: `Ctrl+L` deja de ser "Foco al chat" y pasa a ser "siguiente zona de foco"; `Ctrl+Shift+A` / `Ctrl+Shift+E` cambian de comportamiento (§7); se agregan `Ctrl+P`, `Ctrl+,`, `F1`, `Ctrl+H`, `Ctrl+N`, `Ctrl+Q`. La regla "nunca `Ctrl+Y` ni `Ctrl+N`" ya se quitó de `02-visual.md §8` por pedido del autor.
- `02-visual.md §5`: la barra de git va dentro del gutter actual, sin cambiar su ancho (§6.3).

---

## 1. Qué trae esta etapa (resumen en lenguaje llano)

1. **Buscador rápido de archivos (`Ctrl+P`)**: escribís parte del nombre (aunque sea con letras salteadas o un error de tipeo) y aparece la lista de archivos del proyecto que coinciden, con las letras encontradas resaltadas. Flechas para moverte, `Enter` para abrir, `Esc` para cerrar.
2. **Pantalla de configuración (`Ctrl+,`)**: una pestaña más del editor, como una de archivo, con interruptores y selectores para todo lo que hoy solo se cambia escribiendo en `settings.json`. Lo que cambies ahí se guarda en el mismo archivo (sin borrar tus comentarios), y si editás el archivo a mano la pantalla se actualiza sola. Incluye la gestión de conexiones (renombrar, reparar, volver a conectar, eliminar, actualizar el agente y el Node privado).
3. **Atajos de teclado a la vista**: un botón en la barra de estado (y una entrada en el menú, y `F1`) abre una ventana con todos los atajos agrupados por tema y un buscador. Si cambiaste alguno en tu `keymap.json`, se ve marcado.
4. **Git en el margen**: junto a los números de línea aparecen barritas de color que dicen qué cambiaste respecto del último commit: verde lo añadido, azul lo modificado y una marca roja donde borraste líneas. Van en su propia columna, así que nunca tapan las marcas de los cambios del agente, y el código no se corre ni un píxel: el margen mide lo mismo que hoy.
5. **Chat y archivos con foco**: `Ctrl+Shift+A` (chat) y `Ctrl+Shift+E` (archivos) ya no solo muestran u ocultan: si el panel está oculto lo abren y te dejan escribiendo ahí; si estás dentro, lo cierran y te devuelven al editor; si está abierto pero estás en otro lado, te llevan a él.
6. **Rueda de foco (`Ctrl+L`)**: salta chat → editor → archivos → chat, siempre en ese sentido. Salta los paneles cerrados y nunca abre nada.
7. **Menú**: un botón con tres rayitas a la izquierda de la barra de título, como Zed: abrir carpeta, carpetas recientes, nuevo archivo, guardar, guardar todo, configuración, atajos, conexiones y salir. En la barra de título también aparecen los dos botones para mostrar u ocultar el chat y los archivos.
8. **Cinco arreglos pendientes**:
   - Cuando una sesión de agente vence, el aviso dice el motivo que dio el agente.
   - La búsqueda dentro del archivo (`Ctrl+F`) ahora también **reemplaza** (uno o todos) y encuentra texto en las **líneas rojas** del agente (las líneas originales que el agente quitó y que siguen a la vista hasta que decidas). En esas líneas se puede buscar pero no reemplazar: no son parte del archivo, son la foto de cómo era antes.
   - Cancelar mientras se descarga un agente ("Preparando…") ahora corta la descarga de verdad y borra lo que quedó a medias.
   - Botones "Actualizar adaptador" y "Actualizar Node" en la configuración.
   - Autoguardado "tras N milisegundos sin escribir" (apagado por defecto).

No hay nada que el autor deba decidir antes de empezar; las decisiones que tomó el diseñador están en §2.

---

## 2. Decisiones técnicas de esta spec

| # | Decisión | Motivo |
|---|---|---|
| D1 | `frizbee` **0.13.0** (MIT, MSRV 1.89, sin nightly) para el buscador. | La pidió el autor; MIT está en `deny.toml`; el toolchain del repo es `stable` ≥ 1.90. Da posiciones de coincidencia (`match_list_indices`) para resaltar. |
| D2 | La pestaña de configuración es una vista **propia** (`SettingsView`) hecha con primitivas de gpui-kit (`Switch`, `Select`, `NumberInput`, `Input`, `Button`), **no** el componente `gpui_kit::component::setting::Settings`. | Ese componente trae textos fijos en inglés ("Search...", "Reset All") y gpui-kit 0.6.4 no tiene locale `es`; la sección Conexiones necesita filas a medida de todos modos. |
| D3 | `settings.json` se edita con el **CST de `jsonc-parser` 0.33** (feature `cst`, ya en el árbol de dependencias), que conserva comentarios, orden, comas finales y claves desconocidas. Lectura‑modificación‑escritura en cada cambio, con escritura atómica. | Preserva comentarios sin escribir un editor de JSON propio; releer justo antes de escribir evita pisar una edición a mano. |
| D4 | La pestaña de configuración **no se restaura** al reabrir el proyecto (`layout.json` no cambia). | Su estado *es* `settings.json`; restaurarla obliga a cambiar el formato del layout sin ganancia real. |
| D5 | El botón visible de atajos va en la **barra de estado** (a la derecha, antes del zoom), además de la entrada del menú (que el ítem 7 exige) y `F1`. | La barra de estado ya aloja los controles clicables siempre visibles (conexión, pendientes); un botón en el menú no es "visible". `F1` no choca con nada (`Alt+F*` es lo prohibido). |
| D6 | "Nuevo archivo" (`Ctrl+N`) **crea el archivo en disco** (pide el nombre en un campo flotante, relativo a la carpeta seleccionada en el árbol o a la raíz) y lo abre fijado. | Las pestañas de Cincel siempre tienen ruta; un buffer "sin título" pediría "Guardar como", fuera de alcance. La regla "nunca `Ctrl+Y` ni `Ctrl+N`" se quitó de `02-visual.md §8`; el test que la aplicaba se ajusta (§9.2). |
| D7 | "Conexiones" del menú abre la **pestaña de configuración en la sección Conexiones**. | Es donde se gestionan; el popover del chat es para elegir con cuál trabajar. |
| D8 | Se quita el `Ctrl+L → chat::focus_input` del contexto `Chat` (la acción queda). `editor::select_line` **sigue** en `Ctrl+Shift+L`. | Un binding con contexto gana sobre uno global en GPUI: con ese binding, `Ctrl+L` no avanzaría desde dentro del chat. Revisado: ningún otro binding de `Editor`, `Chat`, de los widgets de Cincel ni de gpui-kit 0.6.4 (`gpui-component`, `gpui-base`) usa `ctrl-l`. |
| D9 | `workspace::focus_chat` **se conserva** sin binding por defecto. | Quien lo tenga en su `keymap.json` no lo pierde. |
| D10 | Git del margen: `git --no-optional-locks diff --no-color --no-ext-diff -U0 HEAD -- <ruta>` por archivo abierto; `git status` también pasa a `--no-optional-locks`. | Sin `--no-optional-locks`, `git status` reescribe `.git/index`, el vigilante de `.git` lo ve y se forma un bucle infinito de refrescos. |
| D11 | Git del margen en una **columna propia de 3 px a la izquierda** de la barra del agente, repartiendo el espacio que ya existe: el gutter mide **exactamente** lo mismo que hoy (§6.3). Los hunks de git se anclan al buffer para seguir las ediciones entre guardados. | "Renderer distinto que no pise": misma forma y distinta posición. El autor no acepta nada que mueva el código (`02-visual.md §6.1`). |
| D12 | Reemplazar dentro de una fila fantasma: **no se reemplaza**, solo se busca. | La fila fantasma es el texto base de la revisión, no el archivo: reemplazar ahí cambiaría lo que restaura "Rechazar" sin tocar el disco, y rompe la regla de solo lectura (`03-arquitectura.md §5`). |
| D13 | Actualizar adaptador/Node **no borra** la versión que está usando un proceso vivo; se poda al próximo arranque. | El adaptador de Claude lanza procesos `node` hijos por `PATH`; borrarle el runtime debajo los rompería. |
| D14 | Cancelar una descarga: `CancelToken` (`Arc<AtomicBool>`) revisado entre lecturas de 64 KiB y antes de cada paso; `npm install` se lanza en su propio grupo de procesos y se mata el grupo. | `ureq` 3 no tiene "abortar"; cortar el bucle de lectura y soltar la respuesta cierra la conexión. |
| D15 | `Salir` y la `×` de la ventana preguntan, con el mismo diálogo, si hay archivos sin guardar. | Principio 5 de `01-producto.md` ("sin sorpresas destructivas"). |
| D16 | Ninguna función de esta etapa desplaza el texto del editor, ni horizontal ni verticalmente, respecto de hoy (§6.3, §10.2, §8.2). | Pedido del autor. |


**Repaso de D16 (qué podría mover el texto y por qué no lo hace):** barras de git: dentro de los 13 px de hoy (§6.3). Reemplazar: misma franja de 28 px que ya ocupa la búsqueda (§10.2). Buscador `Ctrl+P`, modal de atajos, campo "Nuevo archivo", diálogo de salida y menú: flotantes, fuera del flujo. Botón de atajos: en la barra de estado. Botones de paneles y menú: dentro de la barra de título, que no cambia de alto. Pestaña de configuración: es otra pestaña, no toca el editor. Paneles con foco y rueda: solo mueven el foco (abrir o cerrar un panel cambia el ancho del centro como hoy, y con ajuste de línea el texto se reacomoda igual que al arrastrar el borde de un panel).

---

## 3. Buscador rápido de archivos (`Ctrl+P`)

### 3.1 Comportamiento
- `workspace::toggle_file_finder` (`Ctrl+P`, global). Sin proyecto abierto: no hace nada salvo un toast "Abrí una carpeta para buscar archivos". Con el buscador abierto, `Ctrl+P` lo cierra.
- Fuente: todos los **archivos** (no carpetas) de `Project::worktree().entries()`. El `Worktree` ya respeta `.gitignore` y `files.exclude` y excluye `.git`, así que el buscador hereda exactamente lo que muestra el árbol. Si el escaneo no terminó (`is_scan_complete() == false`), se busca sobre lo que hay y se muestra la fila "Buscando archivos…" al pie; al llegar más entradas (`ProjectEvent::TreeChanged`) se vuelve a filtrar.
- Consulta vacía: primero las pestañas abiertas en orden de uso más reciente (la activa al final, como Zed, para que `Ctrl+P Enter` vuelva a la anterior), después los primeros archivos del árbol en su orden; tope 200 filas.
- Consulta no vacía: coincidencia difusa con `frizbee::Matcher::new(consulta, &Config::default())` sobre la **ruta relativa** con `/` como separador. Orden: puntaje descendente (`radix_sort_matches`); empate → ruta más corta → orden alfabético. Tope de 200 resultados.
- Con más de 5 000 archivos el filtrado corre en `cx.background_executor()`; cada tecla cancela el filtrado anterior (se descarta el resultado si la consulta cambió). Presupuesto: < 50 ms desde la tecla hasta pintar con 50 000 rutas en la máquina de referencia (se registra el tiempo en `tracing::debug!`).
- Teclado (contexto `FileFinder`): `↑`/`↓` mueven la selección (con vuelta), `PageUp`/`PageDown` de a 10, `Enter` abre el seleccionado en **pestaña de previsualización** (`CenterPanel::open_file(path, pin: false)`) y cierra, `Esc` cierra sin abrir. Clic en una fila = `Enter` sobre esa fila. Clic fuera del buscador lo cierra.
- Al cerrar sin abrir, el foco vuelve a donde estaba (se guarda el `FocusHandle` previo); al abrir, va al editor de la pestaña.
- El texto escrito se conserva entre aperturas durante la sesión, preseleccionado para reemplazarlo al escribir (como Zed).

### 3.2 Aspecto
- Flotante arriba al centro de la ventana, a 72 px × `ui_scale` del borde superior, ancho `min(560 px, ancho de ventana − 48 px)` × `ui_scale`, alto máximo 420 px × `ui_scale`. `bg.elevated`, radio 6, borde 1 px `border`, sombra de popover. Sin velo oscuro (como la paleta de Zed); una capa transparente capta el clic de afuera.
- Arriba, campo de texto (gpui-kit `Input`, alto 32 px) con placeholder "Buscar archivos por nombre…".
- Filas de 28 px: nombre del archivo 13 px en `text`, y a su derecha la carpeta relativa en 12 px `text.muted` con elipsis al inicio si no entra. Letras coincidentes en `text.accent` y peso 600, tanto en el nombre como en la carpeta. Fila seleccionada con fondo `bg.surface`. Icono de tipo de archivo 14 px (el mismo `file_icon` del árbol).
- Estados vacíos: sin resultados "Ningún archivo coincide con «consulta»"; proyecto sin archivos "Este proyecto no tiene archivos visibles (revisá files.exclude y .gitignore)".
- Todas las medidas se multiplican por `ui_scale`.

### 3.3 Diseño técnico
- Módulo nuevo `crates/cincel-workspace/src/file_finder.rs`: `pub struct FileFinder { query: Entity<InputState>, candidates: Arc<Vec<Arc<str>>>, results: Vec<FinderMatch>, selected: usize, previous_focus: Option<FocusHandle>, filter_task: Option<Task<()>> }`, `pub struct FinderMatch { relative: Arc<str>, positions: Vec<usize> /* offsets de byte, en orden ascendente */ }`, `pub const KEY_CONTEXT: &str = "FileFinder"`.
- Función pura y testeable `pub fn rank(candidates: &[Arc<str>], query: &str, recent: &[Arc<str>], limit: usize) -> Vec<FinderMatch>`. `MatchIndices` de frizbee trae las posiciones en orden inverso: se invierten y se convierten a offsets de byte en límite de `char` (probar con una ruta con "ñ" y "á").
- `Workspace` es dueño de `Option<Entity<FileFinder>>` y lo pinta como capa absoluta, igual que el panel de revisión.
- Acciones: `workspace::toggle_file_finder`; dentro del buscador `file_finder::select_next`, `file_finder::select_prev`, `file_finder::page_down`, `file_finder::page_up`, `file_finder::confirm`, `file_finder::dismiss` (bindings fijos en `built_in_bindings`, contexto `FileFinder`, como los de los modales).
- Dependencia: `frizbee = "0.13"` en `[workspace.dependencies]` y en `cincel-workspace`.

### 3.4 Criterios de aceptación
- [ ] `Ctrl+P` con proyecto abre el buscador con el foco en el campo; sin proyecto, toast y nada más.
- [ ] Con los archivos `src/main.rs`, `src/lib.rs`, `docs/leeme.md`, la consulta `mrs` pone `src/main.rs` primero y resalta `m`, `r`, `s`.
- [ ] Un archivo ignorado por `.gitignore` o por `files.exclude` no aparece nunca.
- [ ] `↓ ↓ Enter` abre el tercer resultado en una pestaña en cursiva (previsualización) y el foco queda en su editor.
- [ ] `Esc` cierra y devuelve el foco al elemento que lo tenía (el chat, si se abrió desde el chat).
- [ ] Consulta sin coincidencias: se ve "Ningún archivo coincide con «…»".
- [ ] Rutas con acentos se resaltan sin cortar caracteres.

### 3.5 Tests
- Unitarios (`file_finder.rs`): orden por puntaje y desempate; tope `limit`; consulta vacía con recientes primero; posiciones en bytes con acentos; consulta con mayúsculas.
- `TestAppContext` (`tests.rs`, patrón `init_test` + proyecto temporal): abrir con `Ctrl+P` (`dispatch_action`), escribir, `SelectNext` + `Confirm` abre previsualización; `.gitignore` respetado; `Esc` restaura el foco previo; `toggle_file_finder` sin proyecto no abre nada.

---

## 4. Pantalla de configuración (`Ctrl+,`)

### 4.1 Comportamiento
- `workspace::open_settings` (`Ctrl+,`, global). Si ya hay una pestaña de configuración, la activa; si no, la crea **fijada** a la derecha de la activa. Funciona también sin proyecto abierto: en ese caso la pantalla vacía cede el área central a la pestaña (y al cerrarla vuelve la pantalla vacía).
- Es una pestaña más: se activa con clic, se cierra con `Ctrl+W`, con la `×` o con clic medio; nunca muestra el punto de "sin guardar" porque cada cambio se guarda al instante.
- Barra de estado con la pestaña activa: a la izquierda "Configuración" (sin `Ln, Col`, codificación ni lenguaje).
- Estructura: columna izquierda de secciones (180 px) y contenido a la derecha con scroll. Arriba del contenido, un buscador "Buscar ajustes…" que filtra por título, descripción y clave JSON (sin distinguir mayúsculas ni acentos); con texto, se muestran todas las secciones que tengan coincidencias y se ocultan las demás; sin resultados: "Ningún ajuste coincide con «…»". Arriba a la derecha, botón "Abrir settings.json" (abre el archivo en una pestaña fijada, creándolo con `{}` si no existe).
- Cada fila: título (13 px, `text`), descripción (12 px, `text.muted`), clave JSON en monoespaciada 11 px `text.muted` (por ejemplo `editor.soft_wrap`), el control a la derecha y, si el valor difiere del predeterminado, un botón "Restablecer" (icono) que **quita la clave** del archivo.

### 4.2 Secciones y controles

| Sección | Ajuste (clave) | Control | Notas |
|---|---|---|---|
| Apariencia | `theme.mode` | Selector: "Seguir al sistema" / "Oscuro" / "Claro" | |
| | `theme.dark`, `theme.light` | Selector con los nombres del `ThemeRegistry` | |
| | `text_rendering` | Selector: "Subpíxel" / "Escala de grises" | |
| | `window.decorations` | Selector: "Integrada (Cincel)" / "Del escritorio" | Aviso bajo la fila: "Se aplica al reiniciar Cincel". |
| Fuentes | `ui_font_family`, `buffer_font_family` | Selector con las familias instaladas (`window.text_system().all_font_names()`, ordenadas), más la actual aunque no esté instalada (marcada "no instalada") | |
| | `ui_font_size`, `buffer_font_size` | Número 6–72, paso 1 | |
| | `buffer_line_height` | Número 1,0–4,0, paso 0,1 | |
| Editor | `editor.soft_wrap` | Interruptor "Ajustar líneas largas" | |
| | `editor.tab_size` | Número 1–16 | |
| | `editor.insert_spaces` | Interruptor "Insertar espacios al tabular" | |
| | `editor.auto_close_pairs` | Interruptor "Cerrar pares automáticamente" | |
| | `editor.show_whitespace`, `editor.cursor_blink` | Interruptores | |
| | `editor.ruler` | Número 0–400 ("0 oculta la guía") | |
| Archivos | `files.autosave` | Selector: "Apagado" / "Al perder el foco" / "Tras una pausa" | |
| | `files.autosave_delay_ms` | Número 100–60 000, paso 100 | Solo habilitado con "Tras una pausa". |
| | `files.exclude` | Lista editable de patrones (fila por patrón con "Quitar", campo "Agregar patrón" + `Enter`) | |
| Revisión | `review.jump_to_next_on_decide` | Interruptor "Saltar al siguiente cambio al decidir" | |
| | `review.sensitive_paths` | Lista editable, como `files.exclude` | Descripción: "Rutas que siempre piden permiso antes de que el agente las toque". |
| | `review.max_file_size_kb`, `review.max_lines` | Números (1–2048 y 1–50 000) | Descripción: "No se puede superar el límite fijo de 2 MB / 50 000 líneas". |
| Conexiones | ver §4.4 | | |

`connections.mcp_servers` y `connections.runtime.node_version` no tienen control propio: una nota al pie de Conexiones dice "Los servidores MCP y la versión de Node se configuran en settings.json" con el botón "Abrir settings.json".

### 4.3 Escritura de `settings.json` (las dos vías conviven)
- Nuevo módulo `crates/cincel-settings/src/edit.rs`:
  ```rust
  pub enum SettingsEdit { Set { path: &'static [&'static str], value: serde_json::Value }, Remove { path: &'static [&'static str] } }
  pub fn apply_edit(text: &str, edit: &SettingsEdit) -> Result<String, EditError>;   // puro
  pub fn write_edit(paths: &Paths, edit: &SettingsEdit) -> Result<(), EditError>;    // lee, aplica, escribe atómico
  pub enum EditError { Syntax { line: usize, column: usize, message: String }, Io(std::io::Error), NotAnObject }
  ```
  Con `jsonc_parser::cst::CstRootNode::parse(texto, &opciones)` (las mismas opciones estrictas que `jsonc::parse`): `object_value_or_set()`, luego `object_value_or_set(segmento)` por cada segmento intermedio, y `get(clave)` → `set_value(...)` o `append(clave, ...)`; `Remove` usa `remove()` y deja el objeto padre aunque quede vacío. `serde_json::Value` → `CstInputValue` con una conversión propia (números con su representación de `serde_json`).
- Archivo inexistente o vacío: se parte de `"{\n  // Ajustes de Cincel. `cincel --print-default-settings` muestra todos con su explicación.\n}\n"`.
- Escritura atómica: `settings.json.tmp` en el mismo directorio → `fsync` → `rename`; se conservan los permisos del original.
- Justo después de escribir, el workspace llama a `crate::settings::reload(SettingsEvent::SettingsChanged, cx)` y aplica el resultado **sin** el toast "Configuración recargada" (lo extrae a `Workspace::apply_reloaded(reloaded, notify: bool)`, que usan tanto el vigilante como la pestaña). El evento del vigilante que llega después tiene la misma huella (`Fingerprint`) y se ignora.
- Si el archivo tiene un error de sintaxis, la pestaña no escribe: banner ámbar "settings.json tiene un error en la línea N, columna M: … Corregilo para poder cambiar ajustes desde acá." con "Abrir settings.json"; los controles se ven deshabilitados con los valores en vigor.
- Editar a mano sigue igual: el vigilante recarga y la pestaña, que lee siempre de `AppSettings` al pintar, muestra los valores nuevos (observa el global con `cx.observe_global::<AppSettings>`).
- Comentarios, orden de claves, comas finales, claves desconocidas y la sección vieja `agents` se conservan byte a byte fuera del valor tocado.

### 4.4 Sección Conexiones
- Lista de conexiones (`Connections::list_with_status`): monograma del proveedor, etiqueta, identidad en gris, "Usado hace …", insignia (Conectada / Sesión vencida / No disponible con el motivo en tooltip). Acciones por fila: "Renombrar" (campo en línea; `Enter` guarda, `Esc` cancela; vacío no se acepta: "El nombre no puede quedar vacío"), "Volver a conectar" (solo vencidas), "Reparar" (solo no disponibles), "Eliminar…" (abre el F3 existente del `ConnectionsModal` ya posicionado en esa conexión). Botón "Conectar nuevo agente…" arriba (F2).
- Subsección "Agentes instalados": una fila por `AgentKind` instalado con la versión ("Claude · adaptador 0.8.2"). Al abrir la sección se consulta el registry ACP en segundo plano (una vez por sesión, más el botón "Buscar actualizaciones"); si `Adapters::update_available` devuelve versión: botón "Actualizar a X.Y.Z". Durante la actualización, barra de progreso y "Cancelar" (§9.3). Al terminar: "Actualizado a X.Y.Z" y, si hay una conexión activa de ese agente, "· se usará al volver a conectar".
- Fila "Node privado": versión instalada o "No instalado (se descarga al conectar Claude o Codex)"; si `Runtime::update_available` devuelve versión: "Actualizar a vX.Y.Z" con la misma mecánica (§9.4).
- Sin red: "No se pudo consultar el catálogo de agentes: sin conexión" en `text.muted`, sin botones de actualizar.
- La vista nunca toca `Connections` directamente: emite `SettingsViewEvent::{Rename { id, label }, Reconnect { id }, Repair { id }, Delete { id }, NewConnection, UpdateAdapter(AgentKind), UpdateNode, CheckUpdates, CancelUpdate}` y `Agents` decide (misma regla que el chat en la Etapa 4).

### 4.5 Diseño técnico de la pestaña
- `center.rs`: la pestaña deja de ser siempre de archivo. `pub enum CenterItem { File(Tab), Settings(Entity<SettingsView>) }`; `CenterPanel.items: Vec<CenterItem>`. Se conservan con el mismo significado `tabs()` → iterador de las `Tab` de archivo, `active_tab() -> Option<&Tab>` (la de archivo activa; `None` si está activa la de configuración) y se agregan `active_item()`, `is_settings_active()`, `open_settings(section: Option<SettingsSection>)`. Guardado, autoguardado, sucio, layout (`to_layout` omite la de configuración), revisión y vista previa de Markdown solo recorren las de archivo. `close_active`, `move_tab`, `activate` y el clic medio funcionan con las dos.
- `focus_active` enfoca el `FocusHandle` de la vista de configuración cuando es la activa, y el de la vista previa de Markdown cuando la pestaña está en vista previa (hoy enfoca el editor oculto).
- Módulo nuevo `crates/cincel-workspace/src/settings_view.rs`: `pub struct SettingsView`, `pub enum SettingsSection { Appearance, Fonts, Editor, Files, Review, Connections }`, contexto de teclas `SettingsPage`. Tab tiene título "Configuración" con icono `IconName::Settings`.
- Los controles de gpui-kit usados no pueden mostrar texto en inglés: se les pasan siempre placeholder y etiquetas propias (test que recorre el texto visible de la vista y falla si encuentra "Search", "Reset", "No results" o "Select").

### 4.6 Criterios de aceptación
- [ ] `Ctrl+,` abre la pestaña "Configuración"; un segundo `Ctrl+,` no crea otra; `Ctrl+W` la cierra.
- [ ] Apagar "Ajustar líneas largas" cambia el editor abierto en el acto y escribe `"soft_wrap": false` dentro de `"editor"` conservando todos los comentarios del archivo (comparación byte a byte del resto).
- [ ] "Restablecer" quita la clave del archivo y la fila vuelve al valor por defecto.
- [ ] Editar `settings.json` a mano (`ui_font_size: 16`) actualiza el campo de la pestaña sin reabrirla.
- [ ] Con un error de sintaxis en el archivo, la pestaña muestra el banner y no escribe nada.
- [ ] Cambiar un ajuste desde la pestaña no muestra el toast "Configuración recargada".
- [ ] Renombrar una conexión desde la pestaña cambia la etiqueta en el popover del chat y en `connections.json`.
- [ ] Con el registry falso publicando una versión nueva del adaptador, aparece "Actualizar a X.Y.Z" y al usarlo queda instalada.
- [ ] La pestaña de configuración no aparece en `layout.json`.

### 4.7 Tests
- Unitarios `cincel-settings::edit`: poner una clave anidada nueva en un archivo con comentarios (el resto intacto); reemplazar un valor existente; quitar una clave; archivo vacío/inexistente; error de sintaxis → `EditError::Syntax` con línea; conservar `agents` y claves desconocidas; comas finales; arrays (`files.exclude`); número decimal (`buffer_line_height: 1.6`); ida y vuelta `apply_edit` + `Settings::parse` sin issues.
- `TestAppContext`: abrir/activar/cerrar la pestaña; cambiar un interruptor escribe el archivo (con `Paths::under(tmp)` vía `isolate_state`) y aplica el ajuste; recarga externa refleja el valor; banner de sintaxis; `to_layout` sin la pestaña; `close_active` con la pestaña activa; eventos de Conexiones llegan a `Agents` (con `ConnectionsFixture`); ningún texto en inglés visible.

---

## 5. Atajos de teclado: botón y modal

### 5.1 Comportamiento
- `workspace::show_shortcuts` (`F1`, global), botón en la barra de estado (icono `IconName::Keyboard` 14 px, tooltip "Atajos de teclado (F1)") y entrada "Atajos de teclado" del menú.
- Modal centrado, ancho `min(640 px, ventana − 48 px)`, alto máximo 70 % de la ventana, con el estilo de los modales de conexiones (`bg.elevated`, radio 6, borde 1 px, scrim 40 %). Título "Atajos de teclado", buscador "Buscar por acción o tecla…" con el foco al abrir, y la lista agrupada con encabezados: **Generales**, **Editor**, **Revisión**, **Chat**, **Conexiones**. Cada fila: descripción en español (13 px `text`), comando en monoespaciada 11 px `text.muted`, contexto legible ("en el editor", "con un segmento bajo el cursor", "buscando", "en el chat"…) y las teclas como chips (`Kbd` de gpui-kit con el formato `Ctrl+Shift+A`). Una acción con varias teclas muestra todas.
- Anulaciones del usuario: la fila lleva la insignia "Personalizado" (`text.accent`) y, en gris, "antes: Ctrl+X" o "reemplaza a «descripción del comando anterior»". Un `null` del usuario muestra la fila del predeterminado atenuada con "Desactivado en tu keymap.json".
- Buscar filtra por descripción, nombre del comando y teclas (escribir "ctrl+p" o "ctrl-p" encuentra el buscador), sin distinguir mayúsculas ni acentos. Sin resultados: "Ningún atajo coincide con «…»". Al pie: "Los atajos se cambian en keymap.json" + botón "Abrir keymap.json" (lo abre en una pestaña, creándolo con `[]` si no existe).
- `Esc` o clic fuera cierra y devuelve el foco. Recargar `keymap.json` con el modal abierto lo actualiza.

### 5.2 Diseño técnico
- `cincel-settings::keymap`: `KeymapSection` gana `origin: KeymapOrigin { Default, User }` (no se serializa; `Keymap::parse` pone `Default`, `Keymap::load_from` marca las del usuario con `User`). Nuevo `pub fn effective_bindings(&self) -> Vec<EffectiveBinding>` con `EffectiveBinding { keystroke: Keystroke, context: ContextExpr, command: Option<String>, origin: KeymapOrigin, replaces: Option<String> /* comando del nivel inferior con la misma tecla y contexto */ }`: una fila por (tecla, contexto) con la de nivel más alto ganando.
- Además de `keymap.json`, el modal incluye los bindings de los widgets (`cincel_editor::default_key_bindings`, `cincel_chat::default_key_bindings`, `built_in_bindings` del workspace), leídos de los `gpui::KeyBinding` (`keystrokes()`, `action().name()`, `predicate()`), con origen "predeterminado"; un binding del keymap con misma tecla y contexto los reemplaza. Se excluyen los bindings internos de gpui-kit (listas, inputs).
- Módulo nuevo `crates/cincel-workspace/src/shortcuts.rs`: `pub enum ShortcutCategory { General, Editor, Review, Chat, Connections }`, `pub fn category(command: &str) -> ShortcutCategory` (tabla explícita: `review::*` y `workspace::{next_change, prev_change, next_file_with_changes, accept_turn, reject_turn, undo_last_reject, open_review_panel}` → Revisión; `chat::*`, `workspace::{toggle_chat, focus_chat, mention_in_chat}` → Chat; `workspace::open_connections` → Conexiones; `editor::*` → Editor; el resto → Generales), `pub fn description(command: &str) -> Option<&'static str>` (tabla en español; sin descripción se muestra el nombre del comando), `pub fn format_keystroke(&Keystroke) -> String` (`ctrl-shift-a` → "Ctrl+Shift+A"; `enter` → "Enter", `escape` → "Esc", `backspace` → "Backspace", `delete` → "Supr", flechas → "↑ ↓ ← →", `pageup`/`pagedown` → "RePág"/"AvPág"), `pub fn context_label(&ContextExpr) -> String`.
- `workspace::open_connections` (nueva, sin binding por defecto): la usa el menú (§8) y la categoría Conexiones.
- Vista `ShortcutsModal` con contexto `ShortcutsModal` y acciones `shortcuts::dismiss` (`Esc`).

### 5.3 Criterios de aceptación
- [ ] `F1` y el botón de la barra de estado abren el modal con el foco en el buscador.
- [ ] Todos los comandos del keymap por defecto y de los widgets tienen descripción en español.
- [ ] Con `keymap.json` = `[{"bindings":{"ctrl-shift-a":"workspace::toggle_tree"}}]`, la fila de `Ctrl+Shift+A` dice "Personalizado" y "reemplaza a «Mostrar u ocultar el chat»".
- [ ] Con `{"context":"Editor","bindings":{"ctrl-g":null}}`, "Ir a la línea" aparece atenuada con "Desactivado en tu keymap.json".
- [ ] Buscar "ctrl+p" deja solo "Buscar archivos".

### 5.4 Tests
- Unitarios: `effective_bindings` (orden de capas, `replaces`, `null`); `format_keystroke` (tabla); `category`; test que recorre `Keymap::default()` + los bindings de widgets y exige `description(..).is_some()` para todos.
- `TestAppContext`: abrir con `ShowShortcuts`, filtrar, recarga de keymap refresca, `Esc` devuelve el foco.

---

## 6. Git en el margen del editor

### 6.1 Comportamiento
- Para cada archivo abierto en una pestaña que pertenece a un repositorio git y está **seguido** por git: barras de 3 px en la columna de git del gutter:
  - **Añadido** (líneas que no existen en HEAD): `git.added` (`#98c379`).
  - **Modificado**: `git.modified` (`#61afef`).
  - **Eliminado** (líneas de HEAD que ya no están): una marca roja `git.deleted` (`#e06c75`) de 3 px × 6 px (× `ui_scale`), centrada en el borde entre las dos filas donde estaban, y una barra de 1 px de alto a lo ancho de la columna; si el borrado es al principio del archivo, en el borde superior de la fila 1.
- Archivos no seguidos (nuevos sin `git add`), fuera de un repositorio, binarios, o sin `git` en el sistema: sin barras y sin error.
- Se actualiza: al abrir la pestaña, al guardar el archivo (desde Cincel o si cambia en disco), y cuando cambian el índice o HEAD (commit, `git add`, `checkout`, `stash`… hechos en la terminal). Entre guardados, las barras siguen al texto (si escribís líneas arriba, bajan con él); lo que escribiste y aún no guardaste no se refleja hasta guardar.
- Las filas fantasma (líneas rojas del agente) no llevan barra de git. La barra de git y la del agente pueden convivir en la misma fila, cada una en su columna.
- Sin hover, sin ventana del hunk, sin "revertir" (fuera de alcance).

### 6.2 Git en `cincel-project`
- `git.rs`:
  - `GitStatus::collect` pasa a `git --no-optional-locks -C <raíz> status --porcelain=v2 -z --untracked-files=all` (D10).
  - `pub fn git_dir(root) -> Option<PathBuf>` con `git rev-parse --absolute-git-dir` (sirve para worktrees, donde `.git` es un archivo).
  - `pub struct LineDiff { hunks: Vec<GitHunk> }`, `pub struct GitHunk { pub kind: GitHunkKind, pub new_rows: Range<u32> /* vacío para Deleted: posición */, pub old_len: u32 }`, `pub enum GitHunkKind { Added, Modified, Deleted }`.
  - `pub fn diff_against_head(repo_root: &Path, path: &Path) -> Option<LineDiff>`: `git --no-optional-locks -C <repo> diff --no-color --no-ext-diff --no-renames -U0 HEAD -- <ruta relativa>`, parseando solo las cabeceras `@@ -a[,b] +c[,d] @@` (b = 0 → Added; d = 0 → Deleted en la fila c; si no → Modified). Un archivo en el índice pero no en HEAD da todo Added. Salida binaria (`Binary files … differ`) o código ≠ 0 → `None`.
  - `GitStatus` expone `is_tracked(path)`.
- `GitDirWatcher` (nuevo, en `watcher.rs` o `git.rs`): `notify` sobre `<gitdir>` (no recursivo) y `<gitdir>/refs/heads` (recursivo), con debounce de 300 ms. Solo cuentan eventos de creación, modificación, borrado y renombre (no de apertura) de `HEAD`, `index`, `packed-refs`, `ORIG_HEAD`, `MERGE_HEAD` y `refs/heads/**`; se ignoran `*.lock`. Emite `GitDirEvent::Changed`.
- En `crates/cincel-workspace/src/project.rs`: `Project` arranca el `GitDirWatcher` junto al `GitStatusWatcher` (mismo `ProjectOptions::watch_git`); ante `Changed`: `git_watcher.trigger()` y recálculo de los diffs de los archivos abiertos. Nuevo evento `ProjectEvent::GitDiffChanged(PathBuf)`. Los diffs corren en el ejecutor de fondo, uno por archivo, descartando resultados viejos (contador de versión por ruta).

### 6.3 Pintado en `cincel-editor`
- `EditorView::set_git_diff(Option<Vec<GitGutterHunk>>, cx)` con `GitGutterHunk { kind, rows: Range<u32> }` en filas del buffer. El editor convierte cada rango a anclas del buffer (`cincel-text::anchor`) al recibirlo, para que sigan las ediciones hasta el siguiente cálculo. El editor no conoce git: recibe filas.
- Geometría del gutter (`element.rs`). Hoy: `GUTTER_PADDING_LEFT 4` + `GUTTER_BAR_WIDTH 3` + `GUTTER_BAR_GAP 6` + números + `GUTTER_TEXT_GAP 12`. Desde esta etapa, repartiendo los mismos 13 px antes de los números:

  | Tramo | Hoy | Etapa 5 |
  |---|---|---|
  | `GUTTER_PADDING_LEFT` | 4 | **2** |
  | `GIT_BAR_WIDTH` (nueva) | — | **3** |
  | `GIT_BAR_GAP` (nueva) | — | **2** |
  | `GUTTER_BAR_WIDTH` (barra del agente) | 3 | 3 |
  | `GUTTER_BAR_GAP` | 6 | **3** |
  | Suma antes de los números | 13 | 13 |
  | Números y `GUTTER_TEXT_GAP 12` (con el borde de 2 px del hunk centrado ahí) | igual | igual |

  `gutter_width` y el origen del texto quedan **idénticos** a los de hoy, con y sin cambios de git, con y sin revisión. La barra del agente se corre 2 px a la izquierda y queda 3 px más cerca de los números; el borde del hunk no se mueve. Las medidas siguen la misma regla de escala que hoy (el gutter usa píxeles lógicos fijos y la fuente del código, que ya sigue el zoom).
- Capa de pintado propia (`paint_git_gutter`), después del fondo de fila y antes de la barra del agente; no se pinta en filas `RowKind::Phantom`; con ajuste de línea, la barra cubre todas las filas visuales de la línea.
- Tema: tokens nuevos `git.added`, `git.modified`, `git.deleted` en `cincel-settings::Theme` y en los temas incluidos (claro: `#50a14f`, `#4078f2`, `#e45649`); un tema de usuario sin esas claves usa los valores del tema incluido de su apariencia (sin issue). `EditorTheme` gana `git_added`, `git_modified`, `git_deleted`.
- El compositor del chat (`EditorChrome::Minimal`) no tiene gutter: nada cambia.

### 6.4 Criterios de aceptación
- [ ] En un repositorio de prueba: modificar la línea 3, agregar dos líneas tras la 10 y borrar la 20, guardar → azul en 3, verde en 11–12, marca roja entre 18 y 19 (numeración nueva).
- [ ] `git commit -am` desde fuera hace desaparecer las barras en menos de 1 s sin tocar el archivo.
- [ ] Escribir tres líneas arriba de un cambio sin guardar desplaza sus barras tres filas.
- [ ] Con un segmento pendiente del agente en la misma fila, se ven las dos barras en sus columnas.
- [ ] Un archivo sin seguimiento y una carpeta sin git no muestran barras ni avisos.
- [ ] `gutter_metrics` y el origen del texto dan exactamente el mismo valor que antes de esta etapa (mismo archivo, misma fuente, mismo zoom), con y sin hunks de git y con y sin revisión.
- [ ] Abrir el proyecto y dejarlo 30 s quieto no dispara ningún `git status` repetido (el log no crece: sin bucle por `index`).

### 6.5 Tests
- Unitarios `cincel-project`: parseo de cabeceras `@@` (añadido, borrado, modificado, varias; `-0,0`); `diff_against_head` en un repo temporal (se salta si no hay `git`, como los tests existentes); archivo no seguido → `None`; `GitDirWatcher` emite al hacer commit y **no** emite al correr `git --no-optional-locks status`.
- Unitarios `cincel-editor`: test de regresión que fija `gutter_width` = 13 px + ancho de números + 12 px (el valor de hoy) y comprueba que `set_git_diff` y un `ReviewView` con hunks no lo cambian ni cambian el origen del texto; las anclas desplazan las barras tras insertar líneas; sin barras en filas fantasma.
- `TestAppContext` (`cincel-workspace`): abrir un archivo de un repo temporal con cambios muestra `set_git_diff` con los hunks esperados (consultando el estado del editor en test); guardar recalcula.

---

## 7. Paneles con foco (`Ctrl+Shift+A` y `Ctrl+Shift+E`)

### 7.1 Regla exacta
`workspace::toggle_chat` (`Ctrl+Shift+A`) y `workspace::toggle_tree` (`Ctrl+Shift+E`), y también los botones de la barra de título (§8.3):
1. Panel oculto → abrirlo y **enfocarlo**.
2. Panel visible y el foco **dentro** del panel → cerrarlo y devolver el foco al área central (§7.3).
3. Panel visible y el foco en otro lado → **enfocarlo** (sin cerrar nada).

"Foco dentro del panel": en el chat, cualquier elemento dentro del árbol del `ChatPanel`: el compositor, la lista de mensajes, la lista de conversaciones, el popover "Conectar", el menú contextual y los popovers `@` y `/`. En archivos, el árbol o cualquier parte del `FilesPanel` (incluido su menú contextual). Se calcula con `focus_handle.contains_focused(window, cx)` del panel; los popovers se pintan dentro del árbol de elementos del panel, así que cuentan (hay test que lo comprueba).

"Enfocar": en el chat, el compositor (`ChatPanel::focus_input`); en archivos, el elemento seleccionado del árbol, o el primero si no hay selección (`TreeState::set_selected_index(Some(0))` + `TreeState::focus`); con el árbol vacío o sin proyecto, el propio `FilesPanel`.

Con un modal abierto (§7.4) las dos acciones no hacen nada.

### 7.2 Diseño técnico
- `Workspace::on_toggle_chat` / `on_toggle_tree` → `fn toggle_focus(&mut self, zone: FocusZone, window, cx)`.
- `pub enum FocusZone { Chat, Center, Files }` (`crates/cincel-workspace/src/focus.rs`), con `Workspace::focus_zone(window, cx) -> Option<FocusZone>` (`None`: foco en la barra de título, la de estado o la raíz), `Workspace::focus(zone, window, cx)` y `Workspace::is_zone_visible(zone, cx)` (`Center` siempre visible).
- `ChatPanel::contains_focus(&self, window: &Window, cx: &App) -> bool` y `FilesPanel::contains_focus(...)`, `FilesPanel::focus_selected_or_first(window, cx)`.

### 7.3 Destino al cerrar
`CenterPanel::focus_active`: el editor de la pestaña activa; la vista previa de Markdown si la pestaña está en vista previa; la vista de configuración si es la activa; sin pestañas, el propio `CenterPanel` (y sin proyecto, el `Workspace`, para que los atajos globales sigan vivos).

### 7.4 Qué cuenta como "modal abierto"
`Workspace::is_modal_open(cx)`: el `ConnectionsModal` abierto, el diálogo "¿Guardar cambios?" de una pestaña, el buscador de archivos, el modal de atajos, el campo "Nuevo archivo", el diálogo de salida con cambios sin guardar, el panel de revisión y el menú de la barra de título desplegado.

### 7.5 Criterios de aceptación
- [ ] Chat oculto + `Ctrl+Shift+A` → visible y el cursor en el compositor.
- [ ] Escribiendo en el compositor + `Ctrl+Shift+A` → chat oculto y el cursor en el editor activo.
- [ ] Con la lista de conversaciones abierta + `Ctrl+Shift+A` → chat oculto (el popover cuenta como dentro).
- [ ] Chat visible, cursor en el editor + `Ctrl+Shift+A` → foco en el compositor, el chat sigue visible.
- [ ] Árbol oculto + `Ctrl+Shift+E` → visible, con el foco en el archivo seleccionado (o el primero), y las flechas lo mueven.
- [ ] Con el modal de conexiones abierto, ninguno de los dos atajos hace nada.

### 7.6 Tests (`TestAppContext`)
Los seis criterios, uno por test, más: sin pestañas el foco vuelve al `CenterPanel`; con la pestaña de configuración activa vuelve a la vista de configuración.

---

## 8. Barra de título: menú y botones de paneles

### 8.1 Aspecto
De izquierda a derecha, dentro de `TitleBar::new()`: botón de menú (`IconName::Menu`, 16 px, botón fantasma de 28 × 28 px), botón de chat (`IconName::PanelLeft` si está oculto / `PanelLeftClose` si visible, tooltip "Mostrar u ocultar el chat (Ctrl+Shift+A)"), el título actual (nombre del proyecto, `text.sm`, peso 500), un espaciador, y el botón de archivos (`PanelRight` / `PanelRightClose`, tooltip "Mostrar u ocultar los archivos (Ctrl+Shift+E)") justo antes de los controles de ventana que pone gpui-kit. Sin proyecto, los dos botones de paneles no se muestran. Los iconos en `text.muted`, `text` al pasar el mouse; el del panel visible en `text`. Todo × `ui_scale` vía rems. Con `window.decorations = "server"` la barra se sigue dibujando igual (sin los controles de ventana).

### 8.2 Menú
`gpui_kit::component::menu::PopupMenu` desplegado desde el botón con `DropdownMenu::dropdown_menu` (anclado arriba a la izquierda), con `action_context` = el `FocusHandle` del editor de la pestaña activa si la hay, o el del `Workspace` (así los atajos que muestra el menú son los del contexto real y las acciones llegan a quien corresponde):

| Ítem | Acción | Atajo mostrado | Habilitado |
|---|---|---|---|
| Abrir carpeta… | `workspace::open_folder` | Ctrl+O | siempre |
| Carpetas recientes ▸ | submenú con `Recents::projects()` (hasta 10; etiqueta = ruta con `~` para el home); clic abre esa carpeta reemplazando la actual; al final, separador y "Borrar la lista" | — | vacío: un ítem deshabilitado "No hay carpetas recientes" |
| Nuevo archivo… | `workspace::new_file` | Ctrl+N | con proyecto |
| (separador) | | | |
| Guardar | `editor::save` | Ctrl+S | con una pestaña de archivo activa |
| Guardar todo | `editor::save_all` | Ctrl+Alt+S | con proyecto |
| (separador) | | | |
| Configuración | `workspace::open_settings` | Ctrl+, | siempre |
| Atajos de teclado | `workspace::show_shortcuts` | F1 | siempre |
| Conexiones | `workspace::open_connections` | — | siempre (abre la configuración en Conexiones, D7) |
| (separador) | | | |
| Salir | `workspace::quit` | Ctrl+Q | siempre |

- Abrir una carpeta (del diálogo o de recientes) usa `Workspace::open_project` como hoy: reemplaza la actual en la misma ventana. Una carpeta reciente que ya no existe: toast "La carpeta «…» ya no existe" y se quita de la lista (`Recents::remove` + `save`).
- **Nuevo archivo** (`workspace::new_file`, `Ctrl+N` global): campo **flotante** arriba al centro del área central, con el mismo estilo y posición que el buscador de archivos (§3.2), para no empujar el editor (D16): "Nuevo archivo en <carpeta>/" + campo con el nombre (se aceptan subcarpetas `a/b.rs`, que se crean). `Enter` crea el archivo vacío y lo abre fijado con el foco en su editor; `Esc` cancela. Carpeta base: la carpeta seleccionada en el árbol, o la del archivo seleccionado, o la raíz. Errores en la misma barra, en `status.error`: "Ya existe «nombre»", "El nombre no puede estar vacío", "No se pudo crear: <motivo>"; una ruta que sale del proyecto (`..`, absoluta) se rechaza con "El archivo tiene que quedar dentro del proyecto".
- **Salir** (`workspace::quit`, `Ctrl+Q` global) y la **`×` de la ventana** siguen el mismo camino, `Workspace::request_quit(window, cx)`: sin archivos sin guardar, se guardan ventana, layout, revisión y conversación y se cierra (`cx.quit()`). Con archivos sin guardar: diálogo con el estilo del "¿Guardar cambios?" — "Hay N archivos con cambios sin guardar" y botones "Guardar todo y salir" (`Enter`), "Salir sin guardar" y "Cancelar" (`Esc`). "Guardar todo y salir" solo cierra si todos se guardaron; si alguno falla, el diálogo se queda con el error.
- La `×`: `window.on_window_should_close` (en `Workspace::open_window`) devuelve `false` cuando hay archivos sin guardar y abre el diálogo (vía `request_quit`); devuelve `true` cuando no los hay o cuando el diálogo ya confirmó (bandera `quit_confirmed`). El guardado del estado de la ventana que hoy hace ese callback se mantiene. `TitleBar::on_close_window` no se usa: el callback del sistema cubre la `×` de la barra integrada y la del escritorio.

### 8.3 Botones de paneles
Clic = exactamente las acciones `workspace::toggle_chat` / `workspace::toggle_tree` con la regla de §7.1 (con el mouse, el foco suele estar fuera del panel, así que un clic con el panel visible y el foco en el editor lo enfoca; un segundo clic, ya con el foco dentro, lo cierra). El icono refleja visible/oculto en cada render.

### 8.4 Criterios de aceptación
- [ ] El botón de menú abre el menú con los ítems y atajos de la tabla; `Esc` lo cierra.
- [ ] "Carpetas recientes" lista las recientes; elegir una la abre en la misma ventana.
- [ ] "Nuevo archivo…" con `docs/nota.md` crea la carpeta y el archivo y lo abre fijado; repetir dice "Ya existe «docs/nota.md»".
- [ ] "Guardar" está deshabilitado sin pestaña de archivo activa.
- [ ] `Ctrl+Q` con un archivo sucio muestra el diálogo; "Cancelar" no cierra.
- [ ] La `×` de la ventana con un archivo sucio muestra el mismo diálogo y la ventana sigue abierta; sin archivos sucios cierra directamente como hoy.
- [ ] `Ctrl+N` abre el campo "Nuevo archivo".
- [ ] Los botones de chat y archivos siguen la regla de §7.1 y su icono cambia.

### 8.5 Tests
- Unitarios: validación del nombre de "Nuevo archivo" (vacío, `..`, absoluto, subcarpetas); etiqueta de reciente con `~`.
- `TestAppContext`: `NewFile` + confirmar crea y abre; `Quit` con sucio abre el diálogo y `Cancelar` lo cierra (el test no llama a `cx.quit()`: se inyecta un `quit_hook` en test); la decisión de la `×` se prueba sobre `Workspace::should_close(cx) -> bool` (la función que llama `on_window_should_close`): `false` + diálogo abierto con sucio, `true` sin sucio y tras confirmar; recientes inexistentes se podan; `toggle_chat` desde el botón (`simulate_click` sobre `debug_selector("titlebar-toggle-chat")`).

---

## 9. Rueda de foco (`Ctrl+L`)

### 9.1 Regla
- `workspace::focus_next_zone` (`Ctrl+L`) recorre **Chat → Centro → Archivos → Chat**, solo en ese sentido (no hay rueda hacia atrás).
- Solo zonas visibles: con el chat oculto, Centro → Archivos → Centro; con los dos ocultos, no hace nada (ya está en el centro) salvo enfocar el centro si el foco estaba fuera de toda zona.
- Nunca abre ni cierra paneles. Con un modal abierto (§7.4) no hace nada. La pestaña de configuración y la vista previa de Markdown cuentan como Centro.
- Desde fuera de toda zona (`focus_zone() == None`): `Ctrl+L` va al chat si está visible, si no al centro.
- Enfocar cada zona = lo mismo que §7.1 ("Enfocar") y §7.3 para el centro.

### 9.2 Cambios de keymap (en `DEFAULT_KEYMAP_JSONC` y en los bindings de widgets)
- Global: quitar `"ctrl-l": "workspace::focus_chat"`; agregar `"ctrl-l": "workspace::focus_next_zone"` y `"ctrl-n": "workspace::new_file"`. `workspace::focus_chat` sigue registrada (D9).
- `cincel_chat::default_key_bindings`: quitar `ctrl-l → chat::FocusInput` (la acción sigue).
- `cincel_editor::default_key_bindings`: sin cambios (`ctrl-shift-l` sigue siendo `SelectLine`).
- Test `default_keymap_avoids_the_forbidden_keys` (`cincel-settings/src/defaults.rs`): se elimina la comprobación de `ctrl-y`/`ctrl-n` (la regla ya no existe en `02-visual.md §8`); comentario nuevo: `// 02-visual.md §8: ni Super, ni Alt+F* (los toma COSMIC/GNOME).` Los vetos de `Super` y `Alt+F*` siguen para todo el keymap.

### 9.3 Criterios de aceptación
- [ ] Con los tres visibles y el foco en el editor: `Ctrl+L` → árbol, `Ctrl+L` → chat, `Ctrl+L` → editor.
- [ ] Con el chat oculto: `Ctrl+L` desde el editor → árbol → editor; el chat sigue oculto.
- [ ] Con el foco en el compositor del chat, `Ctrl+L` va al editor (no se queda en el chat).
- [ ] `Ctrl+Shift+L` en el editor sigue seleccionando la línea.
- [ ] Con el buscador de archivos abierto, `Ctrl+L` no mueve el foco.

### 9.4 Tests
Unitario: `next_zone(current, visibles)` como función pura con tabla completa (3 zonas y `None` × todas las visibilidades). `TestAppContext`: los seis criterios. El test `default_keymap_has_every_shortcut_of_the_spec` se actualiza con las filas nuevas (`ctrl-l`, `ctrl-p`, `ctrl-,`, `f1`, `ctrl-n`, `ctrl-q`, `ctrl-h`).

---

## 10. Deudas técnicas incluidas

### 10.1 `AuthRequired` con el motivo del agente
- `cincel-acp/src/protocol.rs`: `AgentEvent::AuthRequired { methods: Vec<AuthMethod>, message: Option<String> }`. En `connection.rs`, los tres `Err(error) if error.code == ErrorCode::AuthRequired` pasan `message: (!error.message.trim().is_empty()).then(|| error.message.clone())` (y si el error trae `data` con un campo de texto `message`/`reason`, se prefiere ese; se documenta en el código).
- Workspace (`agents.rs`): el motivo se **redacta** (`cincel_log::redact` + recorte de query de URLs, como las líneas de login) y se recorta a 240 caracteres; se registra en el log junto al tail de stderr que ya se registra, y se guarda en el banner.
- `cincel-chat`: `ConnectionBanner::Expired { id, label, reason: Option<String> }`; el banner muestra debajo de "La sesión de «X» venció", en 12 px `text.muted`, "El agente dijo: «motivo»". Sin motivo, igual que hoy.
- Criterios: con el agente falso respondiendo `auth_required` con mensaje "token revoked", el banner muestra "El agente dijo: «token revoked»"; con un mensaje que contiene `https://x/?code=abc`, se muestra sin la query. Tests: `cincel-acp` (agente falso, variable de entorno para el mensaje), `cincel-chat` (render del banner con y sin motivo), `cincel-workspace` (redacción).

### 10.2 Buscar y reemplazar en el editor, con filas fantasma
**Comportamiento**
- `Ctrl+F` abre la barra de búsqueda como hoy. `Ctrl+H` (`editor::find_replace`) la abre con una segunda fila "Reemplazar" y el foco en el campo de búsqueda (o en el de reemplazo si ya estaba abierta).
- `Tab` / `Shift+Tab` alternan entre los dos campos (contexto `Editor && searching`). Escribir, `Backspace` y `Ctrl+V` (pegar) actúan sobre el campo activo, no sobre el archivo.
- En el campo de búsqueda: `Enter` / `Shift+Enter` siguiente / anterior. En el de reemplazo (contexto `Editor && searching && replacing`): `Enter` reemplaza la coincidencia actual y pasa a la siguiente; `Ctrl+Enter` reemplaza todas. Botones en la fila: "Reemplazar" y "Reemplazar todo".
- Con regex activa, el reemplazo admite `$1`, `${nombre}` (sintaxis de `regex::Regex::replace`).
- "Reemplazar todo" es **una sola transacción** del buffer (un `Ctrl+Z` la deshace entera), con `EditSource::User`, así que la revisión la trata como edición del usuario (rebase) y no acepta ni rechaza nada.
- **Filas fantasma**: la búsqueda también recorre el texto de las filas fantasma (las líneas originales que el agente quitó, en rojo). Sus coincidencias se resaltan igual, cuentan en el contador y se visitan con `Enter`/`Shift+Enter` en orden de pantalla. **No se reemplazan** (D12): "Reemplazar" sobre una coincidencia fantasma no cambia nada y salta a la siguiente coincidencia real, con el aviso en la barra "Esa coincidencia está en una línea que el agente quitó: no se puede reemplazar"; "Reemplazar todo" reemplaza solo las reales y el contador dice, por ejemplo, "5 reemplazadas · 2 en líneas quitadas por el agente sin tocar".
- Contador: "3/12" como hoy; si hay coincidencias fantasma, "3/12 (2 en líneas quitadas)".

**Diseño técnico** (`cincel-editor`)
- `search.rs`: `pub enum MatchLocation { Buffer(Range<usize>), Phantom { hunk_ix: usize, line_ix: usize, range: Range<usize> /* bytes dentro de la línea */ } }`; `SearchState.matches: Vec<MatchLocation>` en orden de pantalla (fantasmas de un hunk antes de la fila real donde se insertan). Las fantasmas se buscan línea por línea (una coincidencia no cruza filas fantasma), con el mismo `compile`/`find_matches`. `MAX_MATCHES` cuenta las dos.
- `pub replacement: String`, `pub field: SearchField { Find, Replace }`, `pub replace_open: bool`; `pub fn replacement_for(&self, matched: &str) -> String` (literal o expansión regex).
- `view.rs`: acciones nuevas `editor::find_replace`, `editor::replace_next`, `editor::replace_all`, `editor::search_next_field` (`Tab`), `editor::search_prev_field` (`Shift+Tab`); `on_paste` con la barra abierta pega en el campo activo. `key_context` agrega ` replacing` cuando la fila de reemplazo está abierta y es el campo activo. La coincidencia actual fantasma se selecciona en coordenadas de display (el cursor ya puede estar en filas fantasma).
- Bindings por defecto: `ctrl-h` → `editor::find_replace` (contexto `Editor`); en `Editor && searching`: `tab`, `shift-tab`; en `Editor && searching && replacing` (sección después de las de revisión y búsqueda, así gana): `enter` → `editor::replace_next`, `ctrl-enter` → `editor::replace_all`.
- **Sin desplazar el texto más que hoy (D16).** Hoy la barra de búsqueda **empuja**: `EditorView::render` la pone como hijo de la columna flex *encima* del cuerpo, con 28 px de alto, así que el texto baja 28 px mientras está abierta (lo mismo el prompt "Ir a la línea"). Eso se conserva tal cual, y el reemplazo **no agrega una fila**: con la barra en modo reemplazo, la **misma franja de 28 px** se divide en dos campos lado a lado — "Buscar…" (flex 1) y "Reemplazar…" (flex 1) — seguidos de los conmutadores `.*` y `Aa`, el contador y los botones "Reemplazar" y "Reemplazar todo" (con cursor de mano); si el ancho no alcanza, los botones pasan a iconos (`IconName::Replace`/`ReplaceAll` del catálogo, con tooltip) y el texto "Esc cierra" se oculta. El campo activo lleva borde `border.focus`. Abrir, cerrar o alternar el reemplazo nunca cambia la altura de la barra; el aviso de coincidencia fantasma y el contador de reemplazos ocupan el lugar del contador.

**Criterios de aceptación**
- [ ] `Ctrl+H`, buscar `foo`, reemplazo `bar`, `Enter` en el reemplazo cambia solo la coincidencia actual; `Ctrl+Enter` todas; un `Ctrl+Z` las devuelve todas.
- [ ] Regex `(\w+)@` con reemplazo `$1 en` funciona.
- [ ] Con un segmento del agente que quitó la línea `let viejo = 1;`, buscar `viejo` encuentra esa fila roja, la resalta y el contador la incluye; "Reemplazar todo" no la cambia y lo informa.
- [ ] Reemplazar dentro de un segmento pendiente deja el segmento pendiente (se reacomoda como cualquier edición del usuario).
- [ ] `Tab` en la barra no inserta un tabulado en el archivo.
- [ ] Con la barra abierta en modo búsqueda y en modo reemplazo, la primera fila de texto queda a la misma altura (28 px bajo el borde, como hoy con la búsqueda sola).

**Tests**: unitarios de `search.rs` (orden de pantalla con fantasmas, reemplazo literal y regex, conteo); `review_tests.rs` (búsqueda en fantasmas, "Reemplazar todo" no toca fantasmas, rebase tras reemplazar dentro de un hunk); `editing_tests.rs` (una transacción, `Tab` y pegar en la barra); `tests.rs` (la altura de la barra es 28 px en los dos modos y el origen vertical del texto no cambia al alternar el reemplazo).

### 10.3 Cancelar de verdad una descarga en "Preparando…"
- `cincel-connections`: `pub struct CancelToken(Arc<AtomicBool>)` con `new`, `cancel`, `is_cancelled`, `Clone`. `ConnectionsError::Cancelled` (mensaje "Cancelado").
- Firmas: `Connections::prepare(registry, kind, progress, &CancelToken)`, `Runtime::ensure(progress, &CancelToken)`, `Adapters::ensure(.., &CancelToken)`, `install`, `install_binary`, `PackageInstaller::install(.., &CancelToken)`. Los tests existentes pasan `&CancelToken::new()`.
- Bucles de descarga (`runtime.rs` y `adapters.rs`, `download`): leen en bloques de 64 KiB y revisan el token antes de cada lectura, entre reintentos (la pausa de reintento se parte en tramos de 50 ms) y antes de verificar/descomprimir. Al cancelar: se suelta la respuesta (cierra la conexión), se borra el `.part` y el directorio de staging, y se devuelve `Cancelled`. La descompresión revisa el token cada entrada del archivo.
- `NpmInstaller`: el proceso se lanza en su propio grupo (`rustix`, como los agentes); un hilo espera el proceso y otro revisa el token cada 100 ms; al cancelar mata el grupo, espera su fin y borra el staging.
- `connection_modal.rs`: "Cancelar" (y `Esc` confirmado) en "Preparando…" llama a `token.cancel()`; el modal vuelve a "Elegí un agente" de inmediato, sin esperar al hilo, y el resultado tardío del hilo se descarta (ya pasa hoy). Una segunda preparación del mismo agente mientras el hilo viejo termina espera su fin (candado por `agent_id`) para que no compitan por el mismo `.part`.
- Criterios: con un descargador de prueba que entrega 1 MB cada 100 ms, cancelar a los 300 ms deja el hilo terminado en < 500 ms, sin `.part` ni staging y sin instalación; con un npm falso que duerme, cancelar mata el proceso (no queda vivo el pid). Tests en `tests/engine.rs` para los tres caminos (Node, npm, binario) y en `cincel-workspace` para el modal.
- Se retira la desviación de la Etapa 4 "Cerrar el modal durante «Preparando…» no corta una descarga en curso" de `modulos/workspace.md` (E5-K).

### 10.4 Actualizar adaptador y Node (política LTS)
- **Adaptador**: `Adapters::update(registry, kind, node: Option<&NodePaths>, progress, &CancelToken) -> Result<AdapterInstall>`: instala la versión del registry (npm o binaria) en staging y hace `commit`, que ahora recibe `keep: Option<&str>` para **no** borrar la versión en uso por una conexión activa (D13); `Adapters::prune_unused(in_use: &[(&str, &str)])` borra las versiones que no son `current` ni están en uso, y lo llama el workspace al arrancar (antes de lanzar nada, así que nada está en uso) y al detener una conexión.
- **Node**: `Runtime::update_available() -> Result<Option<String>>` (red; resuelve contra `index.json` según `connections.runtime.node_version`: `"lts"` → la LTS más nueva con mayor ≥ 22; `"22"` (un mayor) → la más nueva de esa línea; una versión completa `"22.11.1"` → nunca hay actualización, está fijada). `Runtime::update(progress, &CancelToken) -> Result<NodePaths>`: descarga, verifica, instala y mueve `current`; la versión anterior se conserva hasta `Runtime::prune_old()` en el próximo arranque. Las conexiones npm nuevas usan el Node nuevo; una activa sigue con el suyo hasta volver a conectar.
- Interfaz: §4.4. Mensajes: "Actualizando el adaptador de Claude… 45 %", "Actualizado a 0.9.0", "No se pudo actualizar: <motivo>" con "Reintentar"; Node: "Actualizando Node a v24.2.0…", "Node actualizado a v24.2.0".
- Criterios: registry falso con versión nueva → botón → versión nueva en `current`, la vieja sigue en disco si había conexión activa y desaparece tras `prune_unused`; `index.json` falso con una LTS nueva → "Actualizar a vX"; con `node_version` fijado a versión completa no aparece nunca. Tests en `tests/engine.rs` (descargador en memoria) y `cincel-workspace` (fila y eventos).

### 10.5 Autoguardado tras una pausa
- `cincel-settings`: `Autosave::AfterDelay` (`"after_delay"`), `FilesSettings.autosave_delay_ms: u64` (por defecto 1000; fuera de 100–60 000 → issue "files.autosave_delay_ms tiene que estar entre 100 y 60000" y se usa 1000). El documento por defecto lo explica: `// "off", "on_focus_change" o "after_delay" (guarda tras autosave_delay_ms sin escribir).` El valor por defecto de `autosave` sigue siendo `"off"`.
- `center.rs`: por cada pestaña de archivo, al recibir un cambio del buffer hecho por el usuario (no por el agente ni por una recarga), se reprograma una tarea `cx.spawn` con `Timer::after(delay)`; si al vencer el buffer sigue sucio, se guarda ese archivo (`save_path`). Guardar a mano o cerrar la pestaña cancela la tarea. Rige la misma regla de hoy: con un turno del agente en curso, los archivos en revisión no se autoguardan (se reintenta al terminar el turno). Guardar no acepta ni rechaza nada: es el guardado normal.
- `on_focus_change` sigue igual y no se combina: el modo es uno u otro.
- Criterios y tests (`TestAppContext` con `cx.executor().advance_clock`): escribir, avanzar 999 ms → sigue sucio; 1 ms más → guardado; escribir de nuevo reinicia la cuenta; con turno activo sobre ese archivo no guarda; `"after_delay"` con `autosave_delay_ms: 50` → issue y 1000; ida y vuelta serde.

---

## 11. Resumen de acciones, contextos y claves nuevas

### 11.1 Acciones
| Acción | Atajo por defecto | Contexto |
|---|---|---|
| `workspace::toggle_file_finder` | `ctrl-p` | global |
| `workspace::open_settings` | `ctrl-,` | global |
| `workspace::show_shortcuts` | `f1` | global |
| `workspace::focus_next_zone` | `ctrl-l` | global |
| `workspace::quit` | `ctrl-q` | global |
| `workspace::new_file` | `ctrl-n` | global |
| `workspace::open_connections` | — | — |
| `workspace::focus_chat` (se conserva) | — (antes `ctrl-l`) | — |
| `workspace::toggle_chat` / `toggle_tree` (regla nueva) | `ctrl-shift-a` / `ctrl-shift-e` | global |
| `editor::find_replace` | `ctrl-h` | `Editor` |
| `editor::search_next_field` / `search_prev_field` | `tab` / `shift-tab` | `Editor && searching` |
| `editor::replace_next` / `replace_all` | `enter` / `ctrl-enter` | `Editor && searching && replacing` |
| `file_finder::{select_next, select_prev, page_down, page_up, confirm, dismiss}` | `down`, `up`, `pagedown`, `pageup`, `enter`, `escape` | `FileFinder` |
| `shortcuts::dismiss` | `escape` | `ShortcutsModal` |

`chat::focus_input` pierde `ctrl-l` en el contexto `Chat`.

### 11.2 Contextos nuevos
`FileFinder`, `ShortcutsModal`, `SettingsPage`, `NewFilePrompt`, `QuitDialog`, y el modificador `replacing` del editor.

### 11.3 `settings.json`
`files.autosave`: `"off" | "on_focus_change" | "after_delay"`; `files.autosave_delay_ms`: número (1000). Temas: `git.added`, `git.modified`, `git.deleted`. Nada más cambia.

---

## 12. Plan de subetapas

Regla fija: **toda** corrida de tests, en cualquier subetapa, usa exactamente `--features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support`; nunca otra combinación (ni "sin features"), para no multiplicar el directorio `target`. Cargo en primer plano con timeout; sin procesos en segundo plano. Nadie hace commits.

| Sub | Contenido | Crates | Depende de | Subagente | Tamaño |
|---|---|---|---|---|---|
| **E5-A** | Foco: `FocusZone`, `contains_focus`, regla de §7, rueda de §9, cambios de keymap de §9.2 (incluidos `chat::focus_input`, `ctrl-n` y el ajuste del test de teclas prohibidas), declarar **todas** las acciones `workspace::*` nuevas de §11.1 (las que aún no tengan manejador quedan registradas y se completan en su subetapa) y sus bindings por defecto; `is_modal_open` con lo que exista. | workspace, settings (defaults), chat, editor (`actions.rs`) | — | Opus | M |
| **E5-B** | `cincel-settings`: `edit.rs` (CST, atómico), `Autosave::AfterDelay` + `autosave_delay_ms`, `KeymapOrigin` + `effective_bindings`, tokens `git.*` del tema. Feature `cst` de `jsonc-parser`. | settings | — | Sonnet | M |
| **E5-C** | Conexiones: `CancelToken` y cancelación real (Node, npm, binario), `Adapters::update`/`prune_unused`, `Runtime::update_available`/`update`/`prune_old`; `AuthRequired.message` en `cincel-acp`. | connections, acp | — | Opus | M |
| **E5-D** | Git en el margen: `diff_against_head`, `--no-optional-locks`, `GitDirWatcher`, `ProjectEvent::GitDiffChanged`, `set_git_diff` y pintado con anclas en el editor, con el gutter del mismo ancho que hoy. | project, editor, workspace (`project.rs`) | E5-B (tokens) | Opus | L |
| **E5-E** | Buscar y reemplazar con filas fantasma (§10.2). | editor | E5-A (keymap) | Opus | M |
| **E5-F** | Buscador `Ctrl+P` (§3). | workspace | E5-A | Sonnet | M |
| **E5-G** | Pestaña de configuración (§4): `CenterItem`, `SettingsView`, secciones, escritura, recarga, sección Conexiones con actualizaciones y cancelar. | workspace | E5-A, E5-B, E5-C | Opus | L |
| **E5-H** | Modal de atajos y botón de la barra de estado (§5). | workspace | E5-A, E5-B | Sonnet | S |
| **E5-I** | Barra de título: menú, recientes, "Nuevo archivo", "Salir" y la `×` de la ventana con el mismo diálogo, botones de paneles (§8). | workspace | E5-A, E5-G (abre configuración), E5-H | Sonnet | M |
| **E5-J** | Autoguardado tras pausa en `center.rs`, banner con motivo de `AuthRequired` en chat/workspace, cableado del cancelar en el modal de conexiones. | workspace, chat | E5-B, E5-C, E5-G (`CenterItem`) | Sonnet | S |
| **E5-K** | Integración (orquestador): verificación completa, actualización de `01-producto.md §3`, `02-visual.md §5 y §8`, `modulos/{workspace,editor,chat,settings,project,connections,acp}.md` (y retirar las desviaciones resueltas), `docs/etapas/etapa-5.md` con desviaciones y la lista de comprobación de §14. | docs | todas | orquestador | S |

Olas sugeridas (cada subagente en su worktree; el orquestador integra en orden): **Ola 1**: E5-A, E5-B, E5-C en paralelo (crates o archivos disjuntos; E5-A y E5-B tocan `defaults.rs` en zonas distintas: A la sección de keymap, B el documento de settings). **Ola 2**: E5-D, E5-E, E5-F, E5-H. **Ola 3**: E5-G. **Ola 4**: E5-I, E5-J. **Ola 5**: E5-K.

Cada subagente entrega: qué criterios de esta spec cumple (con el test que lo prueba), cuáles no y por qué (desviación anotada), y la salida de los comandos de §15 para su crate.

---

## 13. Riesgos y decisiones abiertas

No hay decisiones abiertas.

Riesgos técnicos conocidos, ya mitigados en el diseño: bucle de `git status` sobre `.git/index` (D10, test en §6.5); textos en inglés de gpui-kit (D2, test en §4.7); precedencia de un binding con contexto sobre uno global (D8, criterio de §9.3 desde el chat); desplazamiento del texto (D16, tests en §6.5 y §10.2).

## 14. Lista de comprobación manual (autor)

1. `Ctrl+P`, escribir parte de un nombre con una letra de más: aparece el archivo con las letras resaltadas; `Enter` lo abre en cursiva.
2. `Ctrl+,`: cambiar el tema a claro, la fuente del editor a 16 y apagar el ajuste de línea; abrir `settings.json` desde el botón y ver que tus comentarios siguen ahí. Editar a mano `ui_font_size` y ver el cambio en la pestaña. `Ctrl+W` la cierra.
3. En Configuración → Conexiones: renombrar una conexión; si aparece "Actualizar", actualizar el adaptador.
4. Conectar un agente nuevo que haya que descargar y cancelar a mitad de "Preparando…": el modal vuelve enseguida y en `~/.cache/cincel/downloads/` no queda el `.part`.
5. `F1` (o el botón del teclado en la barra de estado): buscar "chat"; si tenés un `keymap.json`, ver tus atajos marcados como "Personalizado".
6. En un archivo de un repositorio: cambiar una línea, agregar otras, borrar una y guardar: azul, verde y la marca roja. Hacer `git commit -am` en la terminal y ver que desaparecen.
7. `Ctrl+Shift+A` con el chat oculto (se abre y escribís ahí), otra vez (se cierra y volvés al editor); lo mismo con `Ctrl+Shift+E`. Probar los dos botones de la barra de título.
8. `Ctrl+L` varias veces: chat → editor → archivos; con el chat oculto, lo salta. `Ctrl+Shift+L` en el editor sigue seleccionando la línea.
9. El menú de la barra de título: abrir una carpeta reciente, "Nuevo archivo…" (y `Ctrl+N`), "Salir" con un archivo sin guardar; cerrar con la `×` con un archivo sin guardar (pregunta lo mismo).
9b. Con una revisión pendiente y cambios de git en el mismo archivo, abrir y cerrar la búsqueda y el reemplazo: el código no se mueve de costado y la barra no crece.
10. `Ctrl+H` en un archivo con un cambio del agente pendiente: buscar una palabra que esté en una línea roja (la encuentra) y "Reemplazar todo" (no la toca y lo dice).
11. Poner `"files": { "autosave": "after_delay" }`, escribir y esperar un segundo: el punto de "sin guardar" desaparece.

---

## 15. Comandos de verificación

Siempre con el mismo conjunto de features:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support -- -D warnings
cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
cargo test --workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support
cargo deny check licenses
cargo run -p cincel -- --smoke-test .
```

Un subagente verifica solo su parte; el orquestador corre el workspace completo:
- Si tocó un crate con GPUI (`cincel-editor`, `cincel-chat`, `cincel-workspace`), selecciona **los tres** para que las tres features sean válidas y el conjunto sea siempre el mismo: `cargo test -p cincel-editor -p cincel-chat -p cincel-workspace --features cincel-editor/test-support,cincel-workspace/test-support,cincel-chat/test-support` (y lo mismo con `clippy`).
- Si tocó solo crates sin GPUI (`cincel-settings`, `cincel-project`, `cincel-connections`, `cincel-acp`), `cargo test -p <crate>` sin features: esos crates no tienen features de test ni dependen de GPUI, así que no generan variantes en `target`.

Se deja de usar el `cargo check` sin features de la Etapa 4 sobre los crates con GPUI.
