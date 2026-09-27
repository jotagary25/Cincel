# Cincel

Editor de código minimalista para Linux, escrito en Rust con el motor gráfico de Zed (GPUI), con chat integrado para agentes de programación (Claude Code, Codex, Gemini CLI, OpenCode y cualquier agente compatible con [ACP](https://agentclientprotocol.com)) usando tus propias suscripciones.

Lo que lo distingue: **cada cambio que hace un agente se muestra dentro del editor, segmento por segmento, y vos aceptás o rechazás cada segmento o cada línea.** Nunca revisás cambios en el chat.

Estado: en construcción. Ver [`docs/specs/05-plan-etapas.md`](docs/specs/05-plan-etapas.md).

Repositorio público: pendiente.

## Documentación
- [`docs/00-sintesis-y-decisiones.md`](docs/00-sintesis-y-decisiones.md): por qué está hecho así.
- [`docs/specs/`](docs/specs/): qué hace, cómo se ve, cómo está construido, y el plan.
- [`docs/research/`](docs/research/): la investigación previa (ACP, frameworks Rust, UX de revisión de cambios, análisis de Zed).

## Requisitos
- Linux con Wayland o X11; GPU con Vulkan o OpenGL.
- Para compilar: Rust estable (lo instala `rustup` según `rust-toolchain.toml`) y el paquete `libxkbcommon-x11-dev` (Debian/Ubuntu).
- Una suscripción de Claude, ChatGPT (Codex) o Google (Antigravity). Cincel descarga por su cuenta los agentes oficiales y el entorno que necesitan, en su propia carpeta; no hace falta tener Node ni los CLIs instalados, ni sesiones iniciadas en el sistema.

## Licencia
MIT. Ver [`LICENSE`](LICENSE).
