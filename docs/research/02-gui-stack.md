# Informe 02: Stack GUI en Rust para un editor de código nativo en Linux (2026)

Fecha: 2026-09-17. Subagente Opus; versiones verificadas contra crates.io y manifiestos del repo de Zed.

## 1. Resumen ejecutivo

**GPUI ya no es "el framework privado de Zed"**: existe un ecosistema publicado y activo (`gpui-pre` + `gpui-kit`, ex `gpui-component`) que incluye un componente Editor de código con tree-sitter, LSP, folding, decorations, diagnostics y multi-cursor, más componentes de chat (`Message`/`Bubble`) y Markdown incremental. Cubre casi literalmente el milestone 1.

**GPUI eliminó `blade` y reimplementó el renderer de Linux sobre `wgpu`** ([PR #46758](https://github.com/zed-industries/zed/pull/46758), 2026-02-13), reduciendo el riesgo GPU/NVIDIA/Wayland.

## 2. Frameworks GUI: estado actual

| Framework | Última versión (fecha) | Licencia | Editor de código usable | Wayland | Veredicto |
|---|---|---|---|---|---|
| gpui (crates.io) | 0.2.2 (2025-10-22) estancado | Apache-2.0 | — | Bueno, con aristas | Usar vía `gpui-pre` |
| **gpui-pre** | 0.3.5 (2026-09-14), snapshots semanales | Apache-2.0 | — | igual | Vía práctica |
| **gpui-kit** (ex gpui-component) | 0.6.1 (2026-09-09), 14.4k ★ | Apache-2.0 | **Editor completo** | CSD con bugs | **Opción A** |
| iced | 0.14.0 (2025-12-07) | MIT | `text_editor` básico | Bueno | Opción C |
| egui/eframe | 0.36.2 (2026-09-08) | MIT/Apache | inadecuado | OK | Descartar |
| Slint | 1.18.0 (2026-09-16) | GPL-3.0 / royalty-free / comercial | sin editor | Bueno | No es su caso |
| Xilem/Masonry | 0.4.0 (2025-10-29) | Apache-2.0 | experimental | Parcial | Muy temprano |
| floem | 0.2.0 (2024-11-14) | MIT | `floem-editor-core` | Regular | Riesgo alto |
| Makepad | 1.0.0 (2025-05-13) | MIT/Apache | propio | Débil | No |
| Dioxus | 0.7.10 / 0.8.0-alpha | MIT/Apache | Blitz inmaduro | webview | No |
| Tauri | 2.11.5 (2026-09-15) | Apache/MIT | CodeMirror 6 (excelente) | WebKitGTK problemático | Opción B |
| Freya | 0.4.3 / 0.5.0-rc.6 | MIT | básico | — | Nicho |

### Notas críticas

**GPUI.** `gpui` 0.2.2 congelado desde octubre 2025 (incluso `main` declara `version = "0.2.2"`). El camino real es **`gpui-pre`** 0.3.5: snapshots semanales automatizados del repo de Zed publicados por huacnlee (mantenedor de gpui-kit), con auditoría de licencias en CI. Partido en `gpui-pre`, `gpui-pre-macros`, `gpui-pre-platform`, `gpui-pre-linux`. Riesgo: dependencia de un tercero y pre-releases con breaking changes.

Problemas abiertos de Wayland en GPUI:
- [#14473](https://github.com/zed-industries/zed/issues/14473) sin kinetic scrolling en Wayland (+159 👍)
- [#53522](https://github.com/zed-industries/zed/issues/53522) lentitud en GNOME 50 / Wayland
- [#47004](https://github.com/zed-industries/zed/issues/47004) copy/cut roto en Wayland
- [#60964](https://github.com/zed-industries/zed/issues/60964) dead keys y compose no funcionan
- [#58775](https://github.com/zed-industries/zed/issues/58775) ~45s de arranque con Intel iGPU
- gpui-kit [#3035](https://github.com/longbridge/gpui-kit/issues/3035) sin decoraciones de ventana en Wayland

IME vía `zwp_text_input_v3` implementado; fractional scaling funciona con blur al redimensionar ([#25195](https://github.com/zed-industries/zed/issues/25195), [#33464](https://github.com/zed-industries/zed/issues/33464)). **GPUI no integra AccessKit** (accesibilidad nula).

**egui**: `TextEdit` sin culling ([#3086](https://github.com/emilk/egui/issues/3086)); 10 MB → ~2 GB RAM ([#2799](https://github.com/emilk/egui/issues/2799)). Descartar.

**floem**: 0.2.0 de noviembre 2024; último push 2026-06-21. `lapce-core` no publicado en crates.io.

**Tauri**: CodeMirror 6 es el mejor widget de edición disponible (`MergeView` unificado, `Decoration.widget` para bloques inline). Pero WebKitGTK en Linux: `Error 71` en Wayland+NVIDIA, ventana gris en X11, workarounds (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, `__NV_DISABLE_EXPLICIT_SYNC=1`) con coste (~14% CPU con DMA-BUF vs ~53% en copia). No es "Rust nativo".

## 3. Building blocks

**Text buffers:** `ropey` 1.6.1 estable / 2.0.0-beta.1; `crop` 0.4.3 (MIT, más rápido). `rope`/`text` de Zed son GPL-3.0. Recomendación: ropey.

**Syntax highlighting:** `tree-sitter` 0.27.0 (2026-08-30), MIT. **ABI 15 default** (rompe runtimes viejos, cf. Zed [#24632](https://github.com/zed-industries/zed/issues/24632)). Estrategias: Zed = WASM como extensiones; Helix = `.so` en runtime; gpui-kit = **crates estáticos por cargo feature** (30+ gramáticas). Para milestone 1: estático por features.

**Text shaping:** `cosmic-text` 0.19.0; alternativa `parley` 0.11.1 + `swash` + `fontique`; `glyphon` 0.12.0. Con GPUI ya resuelto (`ZED_FONTS_GAMMA` default 1.8).

**LSP client:** `lsp-types` estancado en 0.97.0 (2024-06). **`async-lsp` 0.2.4** (2026-04-24) simétrico cliente/servidor, mejor opción. `tower-lsp` es server-side.

**Infra:** `notify` 8.2.0 (+ `notify-debouncer-full`; ojo límites inotify). `nucleo` 0.5.0 (MPL-2.0) para fuzzy (`fuzzy-matcher` muerto). `gix` 0.87.1 para git. `ignore` 0.4.33 para walk + gitignore. `alacritty_terminal` 0.26.0 (Apache-2.0) para terminal, milestone 2.

**Chat panel:** `pulldown-cmark` 0.13.4 + `mdstream` 0.3.0 (streaming-first, separa bloques committed vs pending). gpui-kit 0.6 trae parsing incremental de Markdown.

## 4. Cores reutilizables y licencias

| Crate de Zed | Licencia | publish |
|---|---|---|
| `crates/gpui` | Apache-2.0 | true |
| `crates/editor` | GPL-3.0-or-later | workspace |
| `crates/language` | GPL-3.0-or-later | workspace |
| `crates/text` | GPL-3.0-or-later | workspace |
| `crates/rope` | GPL-3.0-or-later | workspace |
| `crates/buffer_diff` | GPL-3.0-or-later | workspace |
| `crates/sum_tree` | Apache-2.0 | false |

Reutilizar `editor`/`multi_buffer`/`language` obliga a GPL-3.0 y el acoplamiento con el workspace de Zed es brutal.

**Trampa legal activa**: [issue #55470](https://github.com/zed-industries/zed/issues/55470) reporta cadena `gpui → sum_tree → ztracing (GPL-3.0)`. En la práctica no-op fuera de Zed, pero **`cargo deny check licenses` obligatorio en CI**.

`helix-core` (MPL-2.0) no publicado (placeholder 0.0.0). `lapce-core` tampoco.

## 5. Diff e inline review

**Crates:** `imara-diff` 0.2.0 (Apache-2.0), hasta 30× más rápido que `similar`, algoritmo Histogram, lo usan `gix` y Zed. `similar` 3.2.0 para word-level dentro de línea. `diffy` 0.5.2 para patches unificados. `dissimilar` 1.0.11.

**Modelo de Zed (a copiar):**
- `BufferDiff` = buffer actual + `base_text_buffer` + secondary diff opcional.
- `DiffHunk` con rango en buffer actual y rango en texto base.
- `DiffHunkStatus` (added/modified/deleted) + `DiffHunkSecondaryStatus`.
- Trait `DiffOperations` (`stage`, `unstage`) pluggable: ahí se enchufa accept/reject del agente.
- **Render de líneas borradas**: `DisplayMap` + `BlockMap` inserta bloques de altura N entre líneas reales; las líneas borradas son bloques con fondo rojo sobre las añadidas en verde ([PR #22994](https://github.com/zed-industries/zed/pull/22994)).
- `agent.single_file_review: true` habilita revisión inline dentro del archivo.

**Requisito duro del widget**: soportar **block decorations / phantom lines**. CodeMirror 6: nativo. gpui-kit Editor: tiene `TextDecorationCollection` pero no documenta block decorations → probablemente implementar. iced/egui: no.

**ACP**: el `diff` de ACP es `{path, oldText, newText}` y el permiso es por tool call. **El accept/reject por hunk no existe en el protocolo; se computa del lado cliente** con `imara-diff`.

## 6. Distribución en Linux y GPU

**GPU.** Zed requiere Vulkan (`NoSupportedDeviceFound` si falta). Con wgpu hay fallback GL/GLES. No confiar en lavapipe (no conformante, inusable). Variables: `ZED_DEVICE_ID`, `DRI_PRIME=1`, `MESA_VK_DEVICE_SELECT=list`, `ZED_FONTS_GAMMA`. wgpu vs blade: ~30 MB extra VRAM.

**Packaging.**
- Tarball + `install.sh` (como Zed): lo más simple, recomendado para empezar. glibc ≥ 2.31.
- `.deb` con `cargo-deb` 3.8.0.
- Flatpak: doloroso para un editor (sandbox escape para LSPs/shells; roturas [#53238](https://github.com/zed-industries/zed/issues/53238), [#53129](https://github.com/zed-industries/zed/issues/53129), [#63717](https://github.com/zed-industries/zed/issues/63717)). Diferir.
- AppImage: Ubuntu 24.04 renombró `libfuse2` → `libfuse2t64`. No recomendado.
- `cargo-dist` vivo (push 2026-09-17) para workflows + instaladores.

**Tamaño y arranque.** Zed arranca en 0.4–0.6 s en Linux. Binario GPUI con LTO + strip + ~10 gramáticas: decenas de MB.

**Runtime deps Wayland**: `libxkbcommon`, `wayland-client`, `libdecor`, `xdg-desktop-portal` (`rfd` 0.17.2 o `ashpd` 0.13.13 para diálogos).

## 7. Recomendación final

### Opción A — GPUI (`gpui-pre`) + `gpui-kit` 0.6.1
Único stack donde el milestone 1 casi ya existe: Editor con tree-sitter + LSP + folding + decorations + diagnostics + multi-cursor + búsqueda, `Message`/`Bubble` para chat, Markdown incremental, dock layouts, DataTable. 14.4k ★, push diario, Apache-2.0, producción en Longbridge Pro. Feel de Zed.
Riesgos: 1) `gpui-pre` depende de un tercero; 2) breaking changes, pinear exacto; 3) auditoría de licencias (#55470); 4) Wayland: sin kinetic scrolling, dead keys rotos, CSD con bugs; 5) sin AccessKit; 6) **block decorations para el diff inline probablemente hay que construirlas** (mayor incertidumbre técnica).

### Opción B — Tauri 2.11 + CodeMirror 6
Único camino donde el widget y el diff inline ya están resueltos (MergeView, block widgets). Un dev entrega el milestone 1 mucho más rápido. Riesgos: no es Rust nativo ni Zed-like; WebKitGTK en Linux. Para Ubuntu/Pop!_OS + GNOME + Intel/AMD es bastante estable.

### Opción C — iced 0.14
Sólido, Wayland decente, COSMIC lo respalda. `text_editor` no es editor de código: se construiría desde cero.

### Opción D — floem + editor de Lapce
floem 0.2.0 de noviembre 2024. Riesgo de quedar varado.

### Camino pragmático
1. **Spike de 2-3 días con Opción A**: `gpui-pre 0.3.5` + `gpui-kit 0.6.1`, abrir archivo, tree-sitter Rust, y **probar si se pueden insertar block decorations** en el Editor. Ese spike decide todo.
2. Si falla → Opción B (Tauri + CM6).
3. ACP (`agent-client-protocol` 2.1.0) desde el día 1.
4. Diff con `imara-diff` 0.2.0, hunks del lado cliente.
5. Distribución: tarball + `install.sh`, luego `.deb`. Flatpak al final o nunca.
6. `cargo deny check licenses` en CI desde el primer commit.

**Fuentes:** [crates.io](https://crates.io) · [zed-industries/zed](https://github.com/zed-industries/zed) · [PR #46758 blade→wgpu](https://github.com/zed-industries/zed/pull/46758) · [issue #55470](https://github.com/zed-industries/zed/issues/55470) · [longbridge/gpui-kit](https://github.com/longbridge/gpui-kit) · [gpui-kit.com/component/editor](https://gpui-kit.com/component/editor) · [zed.dev/docs/linux](https://zed.dev/docs/linux) · [PR #22994](https://github.com/zed-industries/zed/pull/22994) · [ACP tool calls](https://agentclientprotocol.com/protocol/tool-calls) · [Tauri Linux graphics](https://v2.tauri.app/develop/debug/linux-graphics/) · [imara-diff](https://github.com/pascalkuthe/imara-diff)

---

# Adenda 02-b: correcciones verificadas (distribución, GPU, escritorio)

## Corrección 1: el target NO es GNOME, es COSMIC
Verificado en la máquina del usuario: Pop!_OS 24.04 LTS, `XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP=COSMIC`, glibc 2.39, portales `xdg-desktop-portal-cosmic` + `-gtk`, inotify `max_user_instances=1024`, `max_user_watches=118729`.
- COSMIC no impone la política CSD-only de mutter.
- **COSMIC está escrito en Rust sobre `iced`/`libcosmic`** ([libcosmic](https://github.com/pop-os/libcosmic), MPL-2.0). Sube a iced en el ranking: único stack que se integra nativamente con el escritorio del target.
- Prior art: **`cosmic-edit`** (GPL-3.0) editor multi-tab con tree-sitter + syntect sobre `cosmic-text`. Referencia, no copiable a producto propietario.
- Issue relevante: [cosmic-epoch#2215](https://github.com/pop-os/cosmic-epoch/issues/2215) (sesiones COSMIC exponiendo solo llvmpipe a clientes Vulkan).
- GPU del usuario: híbrido AMD Cezanne (RADV, Mesa 26.1.6, Vulkan 1.4) + NVIDIA RTX 3050 Ti (580.173.02). **Híbrido AMD+NVIDIA en Wayland es donde viven la mayoría de los bugs GPU reportados.** Probar explícitamente.

## Corrección 2: GPUI ya no falla con `NoSupportedDeviceFound`
Verificado con `strings` sobre Zed 1.20.2 instalado: 0 ocurrencias de `blade_graphics`, 219 de `wgpu_hal` (vulkan, gles), 0 de `NoSupportedDeviceFound`. Errores actuales: `No GPU adapters found`, `Currently you are using a software emulated GPU`, `Unsupported GPU`. `ZED_ALLOW_EMULATED_GPU` sigue vigente.
- Piso real: wgpu 30 acepta Vulkan 1.1+ o GLES 3.0+/GL 3.3+. La doc de Zed está desactualizada.
- **wgpu NO hace fallback automático Vulkan→GL** ([gfx-rs/wgpu#972](https://github.com/gfx-rs/wgpu/issues/972)). Hay que implementar la cascada: Vulkan → `Backends::GL` → software detrás de env var.
- Fallbacks por stack: iced = `tiny-skia` CPU automático (el mejor); egui = glow/OpenGL; GPUI = cascada manual; Vello = `vello_cpu` 0.2.0.
- [zed#57002](https://github.com/zed-industries/zed/issues/57002): redibujo continuo hace GPUI inusable en VMs. **Regla: redibujar por evento, no a 60 Hz; detectar `is_software_emulated`.**

## Corrección 3: tamaños de binario medidos
| Artefacto | Tamaño |
|---|---|
| Zed 1.20.2 tarball | 128.5 MB |
| Zed binario desempaquetado | 331.2 MB (strip solo ahorra 2%) |
| Lapce 0.4.6 tarball / .deb | 23.5 MB / 16.6 MB |
| Helix 25.07.1 tarball | 16.6 MB |
| app iced + wgpu | ~25 MB |
| app iced solo tiny-skia | ~4 MB, 60 ms arranque |
Los 331 MB de Zed son payload (gramáticas, wasmtime, temas, fuentes). **Objetivo realista Asteroid: 20–60 MB.** Cold start: 150–350 ms con wgpu (init GPU + shaders domina).

## Corrección 4: Flatpak de Zed no es oficial; `dist` sigue vivo
- Flathub `dev.zed.Zed` marcado Unverified; usa `--talk-name=org.freedesktop.Flatpak` (sale del sandbox al arrancar). Petición oficial cerrada 2026-02-05.
- **`dist`** (ex cargo-dist) v0.33.0 (2026-09-11); crates.io atrasado en 0.32.0, instalar con su shell installer. No genera .deb/.rpm/AppImage.
| Tool | Versión | Fecha |
|---|---|---|
| `dist` | 0.33.0 | 2026-09-11 |
| `cargo-deb` | 3.8.0 | 2026-09-01 |
| `release-plz` | 0.3.168 | 2026-09-17 |
| `cargo-packager` | 0.11.8 | 2025-11-27 (estancado) |
- AppImage: usar **static-PIE runtime 1.0.3** (sin `libfuse.so.2`).

## Detalles Wayland (verificados sobre el binario de Zed)
- **winit no usa `libdecor`**: dibuja CSD propias con frame Adwaita de SCTK. No depender de libdecor; exponer override (Zed: `ZED_WINDOW_DECORATIONS`).
- Zed hace **`dlopen` de `libwayland-client.so.0`** (feature `dlopen` de `wayland-client`) para correr también en X11-only. Copiar.
- Zed vendoriza `libxkbcommon`, `libxcb*`, `libX11-xcb`, `libbsd`, `libstdc++.so.6` con rpath.
- `xdg-desktop-portal` es la dependencia runtime no obvia: `rfd` 0.17.2 vía `ashpd` + `zbus`. Portales: `FileChooser`, `OpenURI`, `Secret`, `Settings` (dark mode), `Inhibit`.
- Zed pide inotify `max_user_instances ≥ 128`, `max_user_watches ≥ 8000`.

## Ranking revisado
1. **GPUI (`gpui-pre` 0.3.5) + `gpui-kit` 0.6.1**: único con Editor tree-sitter+LSP+folding+decorations hecho. Riesgo GPU bajó con wgpu. Nuevo: cascada Vulkan→GL y redibujo por evento a mano; sin AccessKit; sin integración COSMIC.
2. **iced 0.14** (sube): el escritorio del target está construido sobre iced; fallback tiny-skia gratis; `libcosmic` (MPL-2.0) reutilizable para portales. Riesgo grande sin cambios: `text_editor` no es editor de código; diff inline desde cero.
3. **Tauri 2.11 + CodeMirror 6**: mejor widget de edición/diff; WebKitGTK+NVIDIA roto; en híbrido AMD+NVIDIA hace falta `__NV_DISABLE_EXPLICIT_SYNC=1`.

## Plan de distribución
1. Tarball + `install.sh` con `dist`, compilado en contenedor Ubuntu 22.04 (glibc 2.35); vendorizar `libxkbcommon`/`libxcb*`/`libstdc++`; `dlopen` de wayland-client.
2. `.deb` con `cargo-deb`, uno por release de distro (matrix de Lapce como plantilla).
3. `release-plz` para versionado/changelog.
4. Saltar Snap. Flatpak al final o nunca. AppImage solo con static runtime.
5. No exigir Vulkan: wgpu con `VULKAN` + `GL`, cascada manual, software adapters detrás de env var.
6. `cargo deny check licenses` en CI desde el commit 1.

---

# Adenda 02-c: building blocks, correcciones verificadas en código

## Renderizado de fuentes en Linux (hallazgo importante)
- **`cosmic-text` + `glyphon` son grayscale-only**: `cosmic-text/src/swash.rs` hardcodea `Format::Alpha` y `Content::SubpixelMask => log::warn!("TODO: SubpixelMask")`; `glyphon` solo tiene atlas `R8Unorm`/`Rgba8Unorm`; la cobertura se usa como alpha sin corrección gamma → el clásico texto fino "no nativo" de apps Rust en Linux. PR #532 de cosmic-text abierto sin mergear (2026-08-26).
- **Zed lo resolvió**: PR #45423 (2026-01-06) implementa subpixel estilo ClearType en Linux con dual-source blending (`gpui_wgpu/src/shaders_subpixel.wgsl`), setting `text_rendering_mode`, tunables `ZED_FONTS_GAMMA` (1.8) y `ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST`. **Argumento fuerte a favor de GPUI frente a iced/cosmic-text.**
- **fontconfig**: `fontdb` (cosmic-text) usa `fontconfig-parser` puro Rust que ignora `rgba`, `hintstyle`, `antialias`, `lcdfilter`. `fontique` 0.11.1 (parley) usa libfontconfig real (`FcFontSort`, fallback por script e idioma). Pero `parley::PlainEditor` está respaldado por un `String` (issue #524): usar parley solo para layout, nunca como buffer.
- **`rustybuzz` está archivado** (2026-07-26); reemplazo `harfrust` 0.13.3 (cosmic-text migró en 0.15).

## Correcciones de crates
| Tema | Corrección |
|---|---|
| Fuzzy | `nucleo` sin releases desde 2024-04 y MPL-2.0. **`frizbee` 0.13.0** (MIT, 2026-08-13, repo activo): ~4× más rápido, ~20× en Unicode; lo usan blink.cmp, atuin, skim, television. Preferir. |
| Git | **`gix` no implementa push**; blame es plumbing. **Zed shellea al binario `git`** (`git not found on $PATH, can't push`): hereda credential helpers, SSH agent, hooks. Usar gix para lectura, `git` CLI para escritura. |
| LSP types | `lsp-types` 0.97 estancado; sucesor **`gen-lsp-types` 0.11.0** (MIT, metamodelo LSP 3.18). `async-lsp` 0.2.4 pinea lsp-types 0.95: elegir uno. Ningún editor grande usa un framework crate para el cliente LSP. |
| Terminal | `alacritty_terminal` 0.26 es liviano (17 deps directas, sin async ni GUI, API estable). Puede entrar antes de M2. |
| Vello | `vello_hybrid` se renombra `vello_gpu`; el vello compute pasó a `research/`; `vello_cpu` es el maduro. |

## Bugs sin release en ropes
- **`crop` 0.4.3**: data race en `tiny_arc.rs` (CoW `is_unique()` con load `Relaxed`), arreglado en commit `291ebca` (2026-08-23) sin release. Pinear git rev posterior.
- **`ropey` 1.6.1**: `Ord`/`cmp` roto (issue #120), fix en master 2026-09-09 sin release. No meter ropes en `BTreeSet`.
- **Helix sigue en `ropey` 1.6.1**; cambió regex → **`regex-cursor` 0.1.5** (búsqueda sobre haystacks discontiguos) y tree-sitter → **`tree-house` 0.4.0** (MPL-2.0, arregla injections). No hay `regex-cursor` para crop: argumento para ropey.
- `lapce-xi-rope` 0.4.0 (Apache-2.0) es el único rope en crates.io con capa Delta/OT (`Delta`, `Subset`, `Engine`).

## tree-sitter
- ABI 15 desde 0.25; MIN_COMPATIBLE 13, así que 0.27 carga gramáticas 13/14/15. El fallo real son **dos copias de `tree-sitter-language`** en el grafo (git vs crates.io): error *expected `LanguageFn`, found `LanguageFn`*. Zed lo parchea con `[patch.crates-io] tree-sitter-language = { git = … }`. **Regla: tree-sitter siempre desde crates.io.** `tree-sitter-language` 0.1.8 subió MSRV a 1.90.
- **`arborium` 2.18.2** (MIT/Apache): ~70 gramáticas permisivas en una dependencia, 32 temas, host wasm; alimenta docs.rs. Vendoriza un fork de tree-sitter.
- Carga de gramáticas: Zed = C→WASM (wasi-sdk) bajo wasmtime; Helix = git + `cc` + dlopen `.so` (303 gramáticas, con workspace trust #15177); Lapce = `.so` prebuilds sin chequear ABI.

## File watching
`notify` 9.0.0-rc.5 arregla que inotify **abandone silenciosamente subtrees** al llegar al límite de watches (8.2.0 no da error). Mitigación: observar solo lo que no está en `.gitignore` (`ignore::WalkBuilder`), observar directorios y no archivos (saves atómicos llegan como `MOVED_TO`).

## Stack de building blocks revisado
| Capa | Elección |
|---|---|
| Rope | `ropey` (lo que usa gpui-kit y Helix); vigilar release del fix #120 |
| Undo/anchors | Propio (transacciones + anclas `(offset, Assoc)`) |
| Search sobre rope | `regex-cursor` 0.1.5 |
| Parsing | `tree-sitter` 0.27 de crates.io; evaluar `arborium` para cobertura amplia |
| Highlighting | `tree-sitter-highlight` 0.27 o `tree-house` 0.4 (MPL-2.0) |
| Texto | lo que trae GPUI (subpixel + gamma) |
| LSP | el cliente de gpui-kit; `gen-lsp-types` si se extiende |
| Watching | `notify` 9 rc + `notify-debouncer-full`, scoped con `ignore` |
| Fuzzy | `frizbee` 0.13 (MIT) |
| Diff | `imara-diff` 0.2 + `similar` 3.2 (word-level) |
| Git | `gix` lectura; `git` CLI escritura |
| Terminal | `alacritty_terminal` 0.26 |
| Búsqueda en proyecto | `ignore` + `grep-*` |

---

# Adenda 02-d: frameworks, cuarta pasada (Freya, matiz del BlockMap, correcciones)

## Freya 0.5.0-rc.6 tiene un code editor de primera clase
`crates/freya-code-editor/`: `ropey` + `tree-sitter 0.26`, bring-your-own-grammar (`EditorLanguage::new(LANGUAGE, HIGHLIGHTS_QUERY)`), reparse incremental (`InputEditExt`, `RopeTextProvider`), virtualizado (`VirtualScrollView`), `freya-edit` con undo/redo real, **AccessKit real** (Orca funciona en Wayland). Cadencia semanal.
Faltantes: sin multi-cursor, **IME roto en el CodeEditor** ([#2248](https://github.com/marc2332/freya/issues/2248)), sin primary selection, sin LSP, Skia = builds largos y binarios gordos. **Riesgo: bus factor 1** (marc2332, 2323 commits; también mantiene solo el fork `freya-skia-safe`), cero respaldo corporativo, su app insignia parada desde 2026-06.

## Matiz sobre el BlockMap
| Arquitectura del editor | ¿Insertar filas fantasma? |
|---|---|
| Virtual list de líneas (Freya `VirtualScrollView`, Makepad `inlays.rs`) | Fácil: se controla la lista de items |
| Buffer shapeado con display map (gpui-kit, iced `text_editor`, egui `TextEdit`) | Requiere una capa BlockMap en el pipeline |
gpui-kit sí virtualiza el layout de líneas visibles (`calculate_visible_range` + `layout_lines`), así que agregar la capa es de dificultad media, no imposible. **Referencias permisivas para portar**: `floem/src/views/editor/phantom_text.rs` y `visual_line.rs` (MIT) y `makepad/code_editor/src/{inlays.rs,diff.rs}` (MIT/Apache).

## Correcciones a frameworks
- **iced**: accesibilidad cero ([#552](https://github.com/iced-rs/iced/issues/552) desde 2020); clipboard roto en master/0.15 sobre GNOME ([#3418](https://github.com/iced-rs/iced/issues/3418)); 0.14.0 es seguro; bus factor 1; 9 meses sin release.
- **libcosmic**: 1.0.0, git-only, **sí tiene a11y** vía `iced_accessibility`. `cosmic-edit` v1.9.0 se autodescribe "pre-alpha incompleto".
- **egui**: 0.34+ usa Skrifa + vello_cpu, 0.35 harfrust (ligaduras reales); AccessKit por defecto con AT-SPI; pero `TextEdit` sigue teselando el documento entero cada frame; usar `glow`, no wgpu ([#5269](https://github.com/emilk/egui/issues/5269)).
- **Slint**: inusable pasando 5K líneas ([#10087](https://github.com/slint-ui/slint/issues/10087)); freezes solo en Wayland ([#10912](https://github.com/slint-ui/slint/issues/10912)); cláusula §3 de la licencia royalty-free prohíbe exponer APIs de Slint (plugins → licencia paga).
- **Xilem**: ambos maintainers core a Canva; 17 commits desde julio 2026.
- **floem**: cero commits desde 2026-06-21; `main` impublicable (6 deps git); winit forkeado en 0.29.
- **Tauri**: blur de texto tras resize con fractional scaling en Wayland ([wry#1727](https://github.com/tauri-apps/wry/issues/1727), sin respuesta); migración GTK4 lleva 9 meses; escape: Tauri 3 alpha + `tauri-runtime-cef` (+170 MB).
- **Makepad**: multi-cursor real, folding, inlays, `diff.rs`, primary selection; pero lexer Rust hardcodeado, cero a11y, IME roto con fcitx, historia git destruida por squash-dumps.

## Transversales
- AccessKit 0.25 no soporta rich text: el código se expone como texto plano multilínea en todos los frameworks.
- PRIMARY selection (paste con botón central) falta en casi todos; solo Makepad la tiene.
- `parley::PlainEditor`: un solo estilo, `String`, sin undo ni multi-cursor.
- winit 0.31 lleva ~10 meses en beta.
- Dioxus Labs adquirida por Cognition (2026-09-10).
- **`gpui-unofficial` 1.20.2** publicado hoy: una GitHub Action republica cada tag de release de Zed con versión espejo. Alternativa a `gpui-pre` 0.3.5 con mejor trazabilidad (versión = versión de Zed).

## Ranking final del stream
1. Freya 0.5 (camino más corto a un editor, diff inline natural, a11y; riesgo de mantenedor único).
2. GPUI + gpui-kit (mejor producto terminado, look Zed; BlockMap propio; sin a11y).
3. egui 0.36 con `glow` + widget virtualizado propio.
4. libcosmic (integración con el escritorio; git-only).
Descartados: Xilem, floem, Slint, Makepad, Blitz. Tauri solo aceptando el blur de Wayland o CEF.

**Consejo transversal:** mantener el core del editor (rope, tree-sitter, modelo de diff, LSP) en crates agnósticos del framework; la capa UI queda reemplazable.

---

# Adenda 02-e: crates de diff, corrección

- **`similar` 3.2.0** (2026-08-17, Apache-2.0, 198M descargas) ya tiene `Algorithm::Histogram`, `Hunt`, `RawMyers`, refinamiento inline, `WhitespaceMode`, `merge::TextMerge` (three-way diff3), `no_std`. El benchmark "imara-diff 30× más rápido" es obsoleto (medía contra Myers plano).
- **`imara-diff` estancado en 0.2.0 desde junio 2025**; gitoxide lo forkeó como **`gix-imara-diff` 0.2.5** con releases activas en 2026.
- `patiencediff` es GPL-2.0+: descartar.
- Zed hace `Diff::compute(Algorithm::Histogram, &input)` + **`postprocess_lines(&input)`**, y el postprocess es requisito de corrección: sin él, diffs del mismo buffer contra bases distintas anclan el mismo cambio en filas diferentes. **Aplicar siempre el postprocess (slider/indent heuristic).**
- Recomendación revisada: `similar` 3.2 (Histogram + word-level en un solo crate, mantenido) o `gix-imara-diff`; en ambos casos con postprocess de hunks.
