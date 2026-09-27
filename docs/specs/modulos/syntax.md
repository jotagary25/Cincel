# Módulo `cincel-syntax`

Resaltado de sintaxis con tree-sitter. Sin GPUI.

## Responsabilidades
- `LanguageRegistry`: detecta lenguaje por extensión y por nombre de archivo (`Makefile`, `Dockerfile`), y por primera línea `#!`. Lenguajes v1 (todos como cargo features activadas por defecto, gramáticas de crates.io): Rust, TypeScript, TSX, JavaScript, JSON, TOML, YAML, Markdown, HTML, CSS, Python, Go, Bash, C, C++, Java, SQL, Dockerfile.
- `SyntaxState` por buffer: árbol de tree-sitter, parseo incremental con `InputEdit` derivado de `BufferEvent::Edited`, parseo en ejecutor de fondo con cancelación (`Parser::set_cancellation_flag`) si tarda > 5 ms; el resaltado se recalcula solo en el rango visible pedido.
- `highlights(range) -> Vec<(Range<usize>, HighlightId)>` con las 12 capturas estándar mapeadas a `HighlightId`; el tema resuelve `HighlightId -> color`.
- Injections básicas: bloques de código en Markdown y `<script>`/`<style>` en HTML.
- Sin gramática: texto plano.

## Criterios de aceptación
- [ ] Un archivo Rust de 5 000 líneas parsea completo en < 30 ms en frío; una edición de una letra reparsea en < 2 ms.
- [ ] Depender de `tree-sitter` desde crates.io y compilar sin `[patch]` de `tree-sitter-language`.
- [ ] Tests: para cada lenguaje, un archivo de ejemplo produce al menos una captura de `keyword`, `string` y `comment`.
