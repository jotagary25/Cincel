# Módulo `cincel-project`

Proyecto abierto: árbol, watcher, buffers y git. Sin GPUI.

## Responsabilidades
- `Worktree`: recorre la carpeta con `ignore::WalkBuilder` (respeta `.gitignore`, oculta `.git`, `target`, `node_modules` según ajustes), construye un árbol ordenado (carpetas primero, luego archivos, orden natural). Actualización incremental por eventos del watcher. Expone `entries()`, `children(dir)`, `find(path)`.
- `Watcher`: `notify` con debounce de 100 ms, observando **directorios** no archivos, excluyendo lo ignorado. Eventos: `Created`, `Modified`, `Removed`, `Renamed`. Detecta saves atómicos (temp + rename).
- `BufferStore`: abre buffers bajo demanda (`open(path) -> Entity/Arc<Buffer>`), uno por ruta; sabe si está sucio, su mtime y hash al cargar/guardar; `save(path)`; `reload_from_disk(path)` si el archivo cambió por fuera y el buffer no está sucio; si está sucio y cambió por fuera, marca conflicto y avisa. `apply_agent_write(path, content, turn_id)`: calcula ediciones mínimas contra el snapshot que el agente leyó, las aplica con `EditSource::Agent`, guarda. `read_for_agent(path, line, limit)` desde memoria.
- `GitStatus`: ejecuta `git --no-optional-locks status --porcelain=v2 -z --untracked-files=all` en el ejecutor de fondo al abrir y en cada lote de eventos del watcher (debounce 500 ms); expone estado por ruta (`Untracked`, `Modified`, `Added`, `Deleted`, `Conflicted`, `Ignored`) y `is_tracked(path)`. Si no hay `git` o no es repo, vacío sin error. `--no-optional-locks` es necesario desde la Etapa 5: sin él, `git status` reescribe `.git/index`, el vigilante de `.git` lo ve y se arma un bucle infinito de refrescos.
- Recientes: lista de últimos 10 proyectos en el estado XDG.

## Etapa 5: git en el margen del editor

Detalle, verificación y desviaciones en `docs/etapas/etapa-5.md`. Spec:
`docs/specs/07-etapa5-productividad.md` §6.2.

- **`diff_against_head(repo_root, path) -> Option<LineDiff>`**: corre `git
  --no-optional-locks -C <repo> diff --no-color --no-ext-diff --no-renames
  -U0 HEAD -- <ruta>` y parsea solo las cabeceras `@@ -a[,b] +c[,d] @@` en
  `LineDiff { hunks: Vec<GitHunk> }`, `GitHunk { kind: GitHunkKind, new_rows,
  old_len }` (`GitHunkKind::{Added, Modified, Deleted}`). Un archivo en el
  índice pero no en `HEAD` da todo `Added`; salida binaria o código ≠ 0 dan `None`.
- **`GitDirWatcher`**: `notify` sobre el directorio git (no recursivo) y
  `refs/heads` (recursivo), debounce de 300 ms, contando solo creación,
  modificación, borrado y renombre (no apertura) de `HEAD`, `index`,
  `packed-refs`, `ORIG_HEAD`, `MERGE_HEAD` y `refs/heads/**`, ignorando
  `*.lock`. Emite `GitDirEvent::Changed`; no emite al correr el propio `git
  --no-optional-locks status`.
- `git_dir(root) -> Option<PathBuf>` con `git rev-parse --absolute-git-dir`
  (sirve para worktrees, donde `.git` es un archivo).

## Criterios de aceptación
- [ ] Abrir un proyecto con 50 000 archivos (con `node_modules` ignorado) tarda < 500 ms hasta tener el árbol raíz y termina en fondo.
- [ ] Crear/borrar/renombrar un archivo desde otra app se refleja en el árbol en < 300 ms.
- [ ] Guardar desde Cincel no dispara una recarga del propio buffer (ignorar eventos propios por hash).
- [ ] Escribir un archivo desde fuera mientras está abierto y limpio lo recarga; si está sucio, marca conflicto.
