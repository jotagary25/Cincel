#!/bin/sh
# Instalador de Cincel para el paquete comprimido (`cincel-1.0.0-x86_64-linux.tar.gz`,
# `docs/specs/08-etapa6-cierre-1-0.md` §6.1-§6.2). POSIX sh, sin `sudo`: copia
# los archivos a una carpeta de usuario (por defecto `~/.local`) y agrega el
# icono y la entrada de menú. `--uninstall` borra exactamente lo que instaló.
#
# Uso:
#   ./install.sh [--prefix DIR] [--uninstall] [--help]
set -eu

PREFIX="$HOME/.local"
UNINSTALL=0

usage() {
    cat <<'EOF'
Uso: ./install.sh [--prefix DIR] [--uninstall] [--help]

Instala Cincel en tu carpeta personal, sin pedir contraseña.

Opciones:
  --prefix DIR   carpeta base de la instalación (por defecto: ~/.local)
  --uninstall    borra exactamente lo que este script instaló
  -h, --help     muestra esta ayuda

Lo que instala (bajo DIR):
  bin/cincel
  share/applications/dev.cincel.Cincel.desktop
  share/icons/hicolor/.../dev.cincel.Cincel.{svg,png}
  share/doc/cincel/...

`--uninstall` nunca toca tu configuración, tus conexiones ni tus revisiones
(`~/.config/cincel`, `~/.local/share/cincel`, `~/.local/state/cincel`,
`~/.cache/cincel`): esos quedan donde estaban.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || { echo "install.sh: --prefix necesita un valor" >&2; exit 64; }
            PREFIX=$2
            shift 2
            ;;
        --prefix=*)
            PREFIX=${1#--prefix=}
            shift
            ;;
        --uninstall)
            UNINSTALL=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "install.sh: opción desconocida: $1" >&2
            usage >&2
            exit 64
            ;;
    esac
done

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

ICON_SIZES="16x16 32x32 48x48 64x64 128x128 256x256 512x512"

# Lista fija de lo que este script instala, relativa a DIR (sin comodines:
# `--uninstall` borra exactamente esto y nada más).
list_installed_files() {
    echo "bin/cincel"
    echo "share/applications/dev.cincel.Cincel.desktop"
    echo "share/icons/hicolor/scalable/apps/dev.cincel.Cincel.svg"
    for size in $ICON_SIZES; do
        echo "share/icons/hicolor/$size/apps/dev.cincel.Cincel.png"
    done
    for doc in "$SCRIPT_DIR"/share/doc/cincel/*; do
        [ -f "$doc" ] || continue
        echo "share/doc/cincel/$(basename "$doc")"
    done
}

do_install() {
    mkdir -p "$PREFIX/bin" "$PREFIX/share/applications" \
        "$PREFIX/share/icons/hicolor/scalable/apps" "$PREFIX/share/doc/cincel"
    for size in $ICON_SIZES; do
        mkdir -p "$PREFIX/share/icons/hicolor/$size/apps"
    done

    install -m 0755 "$SCRIPT_DIR/bin/cincel" "$PREFIX/bin/cincel"

    # El escritorio no siempre tiene ~/.local/bin en su PATH: `Exec=` va con
    # la ruta absoluta del binario instalado (§6.2).
    sed "s|^Exec=cincel|Exec=$PREFIX/bin/cincel|" \
        "$SCRIPT_DIR/share/applications/dev.cincel.Cincel.desktop" \
        > "$PREFIX/share/applications/dev.cincel.Cincel.desktop"

    install -m 0644 \
        "$SCRIPT_DIR/share/icons/hicolor/scalable/apps/dev.cincel.Cincel.svg" \
        "$PREFIX/share/icons/hicolor/scalable/apps/dev.cincel.Cincel.svg"
    for size in $ICON_SIZES; do
        install -m 0644 \
            "$SCRIPT_DIR/share/icons/hicolor/$size/apps/dev.cincel.Cincel.png" \
            "$PREFIX/share/icons/hicolor/$size/apps/dev.cincel.Cincel.png"
    done

    for doc in "$SCRIPT_DIR"/share/doc/cincel/*; do
        [ -f "$doc" ] || continue
        install -m 0644 "$doc" "$PREFIX/share/doc/cincel/$(basename "$doc")"
    done

    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
    fi

    check_libraries
    check_path

    echo "Cincel se instaló en $PREFIX/bin/cincel."
}

# D9: Wayland y Vulkan se cargan en tiempo de ejecución (dlopen) y no
# aparecen acá; solo se comprueban las bibliotecas que el binario enlaza.
check_libraries() {
    command -v ldconfig >/dev/null 2>&1 || return 0
    ldconfig_out=$(ldconfig -p 2>/dev/null || true)
    for entry in \
        "libc.so.6:libc6" \
        "libgcc_s.so.1:libgcc-s1" \
        "libxcb.so.1:libxcb1" \
        "libxkbcommon.so.0:libxkbcommon0" \
        "libxkbcommon-x11.so.0:libxkbcommon-x11-0"
    do
        lib=${entry%%:*}
        pkg=${entry##*:}
        if ! echo "$ldconfig_out" | grep -q "$lib"; then
            echo "Falta $pkg: instalala con \`sudo apt install $pkg\`" >&2
        fi
    done
}

check_path() {
    case ":$PATH:" in
        *":$PREFIX/bin:"*) ;;
        *)
            echo "$PREFIX/bin no está en tu PATH. Agregá esta línea a tu ~/.bashrc (o ~/.zshrc):"
            echo "  export PATH=\"$PREFIX/bin:\$PATH\""
            ;;
    esac
    other=$(command -v cincel 2>/dev/null || true)
    if [ -n "$other" ] && [ "$other" != "$PREFIX/bin/cincel" ]; then
        echo "Aviso: tu PATH ejecuta $other antes que $PREFIX/bin/cincel."
    fi
}

do_uninstall() {
    removed=0
    while IFS= read -r rel; do
        f="$PREFIX/$rel"
        if [ -e "$f" ]; then
            rm -f "$f"
            removed=$((removed + 1))
        fi
    done <<EOF
$(list_installed_files)
EOF

    # Directorios que este instalador pudo haber creado; se borran solo si
    # quedaron vacíos (nunca con -f ni recursivo sobre nada más).
    for dir in \
        "$PREFIX/share/doc/cincel" \
        "$PREFIX/share/icons/hicolor/scalable/apps"
    do
        rmdir "$dir" 2>/dev/null || true
    done
    for size in $ICON_SIZES; do
        rmdir "$PREFIX/share/icons/hicolor/$size/apps" 2>/dev/null || true
    done

    if [ "$removed" -eq 0 ]; then
        echo "No había nada instalado en $PREFIX."
    else
        echo "Se desinstaló Cincel de $PREFIX ($removed archivos)."
    fi
    echo "Tu configuración, conexiones y revisiones siguen en ~/.config/cincel, ~/.local/share/cincel, ~/.local/state/cincel y ~/.cache/cincel; borralas a mano si querés."
}

if [ "$UNINSTALL" -eq 1 ]; then
    do_uninstall
else
    do_install
fi
