# Etapa 1: editor usable

Estado: **completa** (2026-09-18). Fase 1 (los seis crates) y fase 2 (el editor dentro de las pestañas del workspace) están hechas y verificadas; queda la lista de comprobación manual del final de este documento, que solo puede hacer una persona frente a la ventana.

## Fase 1: qué se construyó

| Crate | Contenido | Tests |
|---|---|---|
| `asteroid-text` | `Buffer` completo: rope, anclas (`AnchorMap`), transacciones con `EditSource`, undo/redo (también con rangos), agrupado de 300 ms, eventos, CRLF/BOM preservados, estado guardado, `edit_many`, `set_text_minimal` + módulo `diff::minimal_edits`, snapshots comparables en O(1), solo lectura y carga lossy | 49 |
| `asteroid-syntax` | `LanguageRegistry` con 18 lenguajes (features por defecto, gramáticas de crates.io, sin `[patch]`), `SyntaxState` incremental con `CancelFlag`, `highlights()` para 12 capturas, `HighlightTheme` One Dark, injections a un nivel | 17 |
| `asteroid-project` | `Worktree` + `WorktreeScan` (raíz sincrónica, resto en fondo), `Watcher` (notify, 100 ms, directorios), `BufferStore` (dirty, hash+mtime, conflictos, `apply_agent_write`, `read_for_agent`), `GitStatus` + `GitStatusWatcher` (CLI `git`), `Recents`, `IgnoreRules` | 66 |
| `asteroid-settings` | `Config::load` tolerante con `SettingsIssue`, `Settings`/`Keymap`/`Theme` completos, `ContextExpr` (`&&`, `||`, `!`, paréntesis), keymap por defecto con las 32 filas de `02-visual §8`, dos temas embebidos, `ThemeRegistry`, `SettingsWatcher`, `default_settings_jsonc/keymap_jsonc` | 63 |
| `asteroid-editor` | Elemento editor completo: resaltado incremental en fondo, `WrapMap` (Fenwick), virtualización con caché, movimiento con columna objetivo, sangría heredada y detectada, undo/redo, portapapeles, búsqueda propia con regex, ir a línea, scrollbar que se desvanece, cursor que deja de parpadear, IME con subrayado, mouse completo; API estable (`EditorView::new`, `EditorSettings`, `EditorTheme`, `EditorEvent`, `default_key_bindings`); hunks simulados conservados. **Fuga al cerrar de E0 resuelta** (`WeakInputHandler`) | 86 |
| `asteroid-workspace` + `asteroid` | Tema desde settings proyectado a gpui-kit, tipografía con cadena de respaldo, keymap→`KeyBinding` con recarga en caliente, 20 acciones (`workspace::*` + `editor::save_all`), apertura de proyecto (CLI, diálogo del portal, recientes, estado vacío), árbol virtualizado con colores de git, teclado y menú contextual, pestañas con punto sucio / previsualización / cierre con diálogo / reordenar, breadcrumb, barra de estado, toasts, `layout.json` por proyecto, `--print-default-settings/keymap`, y en la fase 2 el editor en cada pestaña | 49 |

Total: ~28 900 líneas de Rust, **353 tests** al cerrar la fase 2. Verificación de la etapa: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --features asteroid-editor/test-support,asteroid-workspace/test-support -- -D warnings`, `cargo test --workspace` con esas mismas features (351/0), `cargo deny check licenses`, `asteroid --smoke-test .` (exit 0), la app corrida 5 s sobre este repo sin panics, y el detector de fugas de GPUI (`cargo run -p asteroid --features asteroid-workspace/test-support -- --smoke-test .`, exit 0 incluso con pestañas restauradas).

## Mediciones (perfil dev, máquina de referencia)
- Parse Rust 5 000 líneas: 18–20 ms en frío; reparseo de una letra: 0,6 ms mediana.
- Árbol de 50 000 archivos: raíz en 1,5 ms, completo en 136 ms. Watcher → árbol: ~60 ms.
- Editor con 50 000 filas y ajuste de línea activado: p50 1,8 ms, p95 2,4 ms por frame; primer frame ~40 ms.

## Fase 2: el editor dentro del workspace

Cada pestaña es un `TabContent::Editor(Entity<EditorView>)` sobre el mismo
`BufferHandle` que abre `BufferStore` (el que también escribe el agente), así
que edición, guardado y recarga pasan por un solo lugar.

- **Tema y ajustes**: `ThemeColors → EditorTheme` (todos los tokens de
  `02-visual.md` §2 más las 12 capturas, en el orden de `HighlightId`) y
  `Settings → EditorSettings` (`soft_wrap`, `tab_size`, `insert_spaces`,
  `show_whitespace`, `ruler` con `0 = sin guía`, `cursor_blink`, cadena de
  fuentes monoespaciadas resuelta contra las instaladas, tamaño y alto de
  línea). Viven en `theme::editor_theme` / `theme::editor_settings`.
- **Recarga en caliente y zoom**: cada `SettingsEvent` y cada `workspace::zoom_*`
  llaman a `CenterPanel::refresh_editor_style`, que hace `set_settings`,
  `set_theme` y `set_language` en todos los editores abiertos.
- **Teclas**: `asteroid_editor::default_key_bindings()` se instala *debajo* del
  keymap del usuario, así que `editor::*` y `review::*` resuelven y cualquier
  línea de `keymap.json` los pisa. El editor declara `Editor` (y
  `Editor && searching`) dentro del contexto `Center`, que a su vez está dentro
  de `Workspace`.
- **Guardado**: `EditorEvent::SaveRequested` → `BufferStore::save` →
  `EditorView::mark_saved`; error como aviso. `editor::save_all` (acción propia
  del workspace, el editor no la define) guarda todas las pestañas sucias, y
  `files.autosave = "on_focus_change"` guarda al perder el foco la ventana o al
  cambiar de pestaña.
- **Cambios externos**: `BufferStore::handle_fs_events` llega como
  `ProjectEvent::BuffersChanged(Vec<BufferChange>)`. `Reloaded` →
  `EditorView::buffer_changed`; `Conflict` → aviso con dos botones («Recargar
  del disco» / «Mantener mi versión») atados a `discard_changes` /
  `keep_my_version`; `Deleted` → la pestaña pasa a llamarse «archivo
  (eliminado)».
- **Archivos que no son UTF-8**: se abren igual, en un `Buffer::from_bytes_lossy`
  en solo lectura fuera del store, con el aviso «Archivo abierto en solo
  lectura: contenido no UTF-8». El límite `review.max_file_size_kb` no impide
  abrir nada: es del review.
- **Barra de estado**: «Ln X, Col Y» desde `EditorEvent::CursorMoved`, con la
  columna en caracteres (no en bytes), más EOL y lenguaje de la pestaña activa.
- **Breadcrumb**: `ruta › símbolo`, con la definición que envuelve al cursor
  (`EditorView::symbol_at_cursor`), refrescada en cada `CursorMoved` y en cada
  `DirtyChanged` — así el nombre aparece igual cuando el reparse termina
  después del movimiento.
- **`layout.json`**: cada pestaña guarda la fila visible
  (`EditorEvent::ScrollChanged`) y el cursor (`cursor: [fila, columna]`). Al
  reabrir se llama `set_scroll_row` y después `set_cursor`, en ese orden: el
  editor solo autodesplaza cuando el cursor cae fuera de la vista, así que la
  vista queda exactamente donde estaba.

## Hallazgos y desviaciones registradas
1. `tree-sitter-highlight` no sirve para parseo incremental ni por rango: `asteroid-syntax` usa `Query`/`QueryCursor` directo con el mismo mapeo de capturas. Markdown y Dockerfile vienen de `arborium-*` (misma gramática, sin segundo runtime C).
2. `Parser::set_cancellation_flag` no existe en tree-sitter 0.27: se usa `progress_callback`.
3. JSONC con `jsonc-parser` (no json5) para que un archivo válido acá lo sea también en Zed/VS Code. Se reportan claves desconocidas como aviso.
4. `SettingsWatcher` producía **un bucle de recargas** porque la máscara de `notify` incluye `IN_OPEN` (leer el archivo genera un evento). **Resuelto en la fase 2**: el watcher descarta `EventKind::Access(_)` y `EventKind::Other` (`asteroid-settings`, con dos tests: leer no avisa, escribir sí), y `asteroid-workspace` mantiene la huella mtime+tamaño como segunda barrera.
5. gpui-kit 0.6.1 no permite suprimir la tira de título del grupo central: queda una franja fina vacía sobre las pestañas.
6. Soft wrap mide en columnas monoespaciadas y los tabs se pintan como `tab_size` espacios (no al siguiente tab stop). Búsqueda no recorre filas fantasma ni tiene reemplazo.
7. Fuentes: en la máquina de referencia no están Inter ni JetBrains Mono; se resuelven respaldos.
8. Deseos de API pendientes para E2/E3 (rendimiento, no bloqueantes): `drain_events_with_snapshots`, `SyntaxState` consultable durante el reparse, `highlights` por filas, `Language::highlight_once`, `line_lens(range)`; `asteroid-project` puede pasar a `asteroid_text::minimal_edits`.
9. Los tres deseos de API que dejó la primera pasada de la fase 2 —
   `symbol_at_cursor`, `set_cursor` y `scroll_row`/`set_scroll_row`, más el
   evento `EditorEvent::ScrollChanged`— los agregó `asteroid-editor` y el
   workspace ya los usa: breadcrumb con símbolo y reapertura exacta (vista y
   cursor). `EditorEvent` dejó de derivar `Eq` por el `f32` de `ScrollChanged`.
10. Incidente: el disco de la máquina de referencia llegó al 100% durante esta etapa (el directorio `target` alcanzó 37 GB por variantes de features del motor gráfico). Se liberó espacio; el perfil de compilación se mantiene sin cambios por decisión del autor.

## Lista de comprobación manual (autor)

Cierra la Etapa 1 de `docs/specs/05-plan-etapas.md`. Todo con el binario de
depuración; cada paso dice qué mirar.

```bash
cd asteroid-editor
cargo build -p asteroid            # una vez
./target/debug/asteroid .          # abre este mismo repo
```

1. **Abrir un proyecto Rust real.** Arrancá sin argumentos
   (`./target/debug/asteroid`): tiene que abrir el último proyecto; borrá
   `~/.local/state/asteroid/recents.json` y volvé a arrancar para ver la
   pantalla de bienvenida con «Abrir carpeta (Ctrl+O)». Con `Ctrl+O` elegí este
   repo: se abre el árbol a la derecha, el chat a la izquierda y la barra de
   estado abajo.
2. **Navegar el árbol.** Con un clic en una carpeta se abre y se cierra; con las
   flechas ↑↓ se recorre, → abre, ← cierra y Enter abre el archivo. Los archivos
   modificados según git salen en amarillo y los nuevos en verde; una carpeta
   con cambios adentro lleva un punto. Con `Ctrl+Shift+E` el árbol se colapsa y
   vuelve.
3. **Abrir y editar con acentos.** Un clic abre en previsualización (título en
   itálica), un doble clic la fija. Escribí `// ñandú áéíóú` en
   `crates/asteroid-workspace/src/center.rs`: aparece el punto sucio en la
   pestaña, «Ln X, Col Y» sigue al cursor y la columna cuenta **caracteres**
   (poné el cursor después de `ñ`: la columna avanza de a uno). Probá el IME o
   `Ctrl+Shift+U` si lo tenés configurado. Movete dentro de una función: el
   breadcrumb muestra `crates › … › center.rs › nombre_de_la_función`.
4. **Buscar.** `Ctrl+F`, escribí `fn `, `F3` y `Shift+F3` recorren; `Esc` cierra.
   `Ctrl+G` salta a una línea.
5. **Guardar y deshacer.** `Ctrl+S` guarda: el punto desaparece y
   `git diff --stat` muestra el archivo tocado. `Ctrl+Z` deshace y `Ctrl+Shift+Z`
   rehace; el punto vuelve a aparecer y desaparecer según corresponda. Dejá el
   archivo como estaba (`git checkout -- <archivo>`) y fijate que el editor
   recarga solo el contenido del disco.
6. **Cerrar y reabrir con las mismas pestañas.** Abrí tres archivos, dejá el
   cursor en el medio de uno, cerrá la ventana y volvé a abrir el proyecto:
   vuelven las mismas pestañas, en el mismo orden, con la misma activa, el
   cursor donde lo dejaste y la vista en el mismo lugar. `Ctrl+W` sobre un
   archivo con cambios pregunta
   «¿Guardar cambios?» con Guardar / Descartar / Cancelar.
7. **Volver a donde estabas.** Abrí un archivo largo, bajá hasta que la vista
   haya hecho scroll y dejá el cursor dentro de una función. Cerrá Asteroid y
   volvé a abrirlo: la pestaña vuelve con la vista en el mismo lugar y el
   breadcrumb muestra esa función.
8. **Cambiar el tamaño de fuente en caliente.** Con la ventana abierta:
   ```bash
   mkdir -p ~/.config/asteroid
   printf '{ "buffer_font_size": 20, "ui_font_size": 16 }\n' > ~/.config/asteroid/settings.json
   ```
   El código crece sin reiniciar y aparece el aviso «Configuración recargada»
   (una sola vez: si parpadea sin parar, volvió el hallazgo 4). Probá también
   `{ "theme": { "mode": "light" } }` y `Ctrl+=` / `Ctrl+-` / `Ctrl+0` para el
   zoom. Borrá el archivo cuando termines.
9. **Cambios externos.** Con un archivo abierto y **sin** editarlo, tocalo desde
   otra terminal (`echo "// desde afuera" >> <archivo>`): el editor lo recarga
   solo. Ahora editalo en Asteroid sin guardar y volvé a tocarlo desde afuera:
   aparece el aviso con «Recargar del disco» / «Mantener mi versión». Borralo
   desde afuera: la pestaña pasa a «archivo (eliminado)».
10. **Un archivo que no es texto.** *Opcional.* Asteroid no está pensado para
   binarios, pero si abrís por error una imagen o un ejecutable no debe
   romperse: se abre en solo lectura con un aviso y no deja escribir (probalo
   con `printf "\xff\xfe" > /tmp/raro.bin` y abriendo `/tmp`).
