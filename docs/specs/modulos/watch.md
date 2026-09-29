# Módulo `cincel-watch`

Vigilancia de archivos en lotes, sin despertares en reposo. Sin GPUI. Spec:
`docs/specs/08-etapa6-cierre-1-0.md` §9.3 (E6-G), `docs/rendimiento.md` §9.3.

## Por qué existe

Antes de la Etapa 6, los tres vigilantes de Cincel (el proyecto, `.git` y la
configuración) usaban `notify-debouncer-full`. Esa biblioteca arma lotes con
un hilo que se despierta cada cuarto de su ventana **para siempre**, pase
algo o no: con tres vigilantes eso eran ≈93 despertares por segundo y tres
cuartos de la CPU de Cincel en reposo (medido con `strace -f -tt` y contando
despertares por hilo en `/proc/<pid>/task/*/status`, `docs/rendimiento.md`
§9.3), muy por encima de la meta M8 (CPU en reposo ≤ 0,1 %).

`cincel-watch` resuelve lo mismo con un hilo que se **bloquea en su canal**
mientras no pasa nada: no hay temporizador corriendo en el vacío, así que en
reposo no hay ningún despertar atribuible a este crate.

## API

- `Debouncer::new(name, window, handler)`: arranca un hilo con ese nombre
  que llama a `handler` con cada lote no vacío. El primer evento de una
  ráfaga abre una ventana de `window`; todo lo que llega dentro de ella sale
  junto cuando la ventana se cierra (la ventana no se corre: un chorro
  continuo de eventos sigue saliendo cada `window`, no nunca).
- `Debouncer::watch(path, mode)` / `unwatch(path)`: delegan en el
  `RecommendedWatcher` de `notify` de abajo.
- `Debouncer::wakeups()`: cuántas ventanas abrió el hilo de lotes desde que
  arrancó — en reposo no se mueve; lo usa el test `nothing_wakes_up_at_rest`.
- `Batch { events: Vec<notify::Event>, errors: Vec<notify::Error> }`: lo que
  entregó una ventana.

Dentro de un lote:
- un renombre que el backend ya emparejó (`RenameMode::Both`, que inotify
  informa junto a sus dos mitades con el mismo `tracker`) descarta las
  mitades sueltas del mismo `tracker`;
- un evento idéntico al anterior (mismo `kind`, mismos `paths`) se descarta.

Los errores del backend viajan en el mismo lote, en una lista aparte, y
nunca detienen el hilo.

Soltar el `Debouncer` detiene la vigilancia; el hilo de lotes termina solo
cuando entrega la última ventana pendiente (el `watcher` se suelta primero:
su callback es dueño del emisor del canal, así que el hilo ve el canal
cerrarse).

## Quién lo usa

Los tres vigilantes de Cincel construyen su propio `Debouncer` en vez de un
`notify-debouncer-full`, que ya no se usa en ningún crate:
- `cincel-project::watcher::Watcher` (el proyecto, `modulos/project.md`),
  ventana de 100 ms, observando el directorio raíz de forma recursiva.
- El vigilante de `.git` (`cincel-project::git::GitDirWatcher`), ventana de
  300 ms.
- `cincel-settings::watcher::SettingsWatcher` (`settings.json`,
  `keymap.json`, `themes/`), ventana de 100 ms.

Cada uno sigue traduciendo los eventos crudos de `notify` a su propio tipo
(`FsEvent`, `GitDirEvent`, `SettingsEvent`); `cincel-watch` no sabe nada de
proyectos, git ni configuración — solo agrupa y depura eventos de `notify`.

## Tests

`crates/cincel-watch/src/tests.rs`: `nothing_wakes_up_at_rest` (el hilo de
lotes no se despierta en 400 ms sin eventos, ni después de entregar una
ráfaga), armado de lotes, emparejamiento de renombres, descarte de eventos
repetidos, errores del backend en su propia lista.

## Criterios de aceptación
- [ ] Con tres vigilantes activos y el proyecto quieto, `wakeups()` de cada
      uno no avanza en 5 s.
- [ ] Un lote de más de un evento en la misma ventana llega junto, en el
      orden en que ocurrieron.
- [ ] Soltar el `Debouncer` deja de observar sin bloquear el hilo que lo
      suelta.
