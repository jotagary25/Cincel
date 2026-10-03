#!/bin/sh
# Tests `install.sh` on its own, against an already-extracted package tree
# (`docs/specs/08-etapa6-cierre-1-0.md` §6.8). `packaging/verify.sh` runs
# this as a non-root user inside a clean container, pointed at the
# extracted tarball; it can also run directly against `dist` after
# `packaging/build.sh` has produced the tarball.
#
# Usage: install_test.sh PACKAGE_ROOT
#   PACKAGE_ROOT: the extracted `cincel-<version>-x86_64-linux/` directory
#                 (must contain `install.sh`).
set -eu

PACKAGE_ROOT=${1:?"uso: install_test.sh PACKAGE_ROOT"}
PACKAGE_ROOT=$(CDPATH='' cd -- "$PACKAGE_ROOT" && pwd)
INSTALL_SH="$PACKAGE_ROOT/install.sh"
PREFIX=$(mktemp -d)
trap 'rm -rf "$PREFIX"' EXIT INT TERM

fail() {
    echo "install_test.sh: FALLÓ: $1" >&2
    exit 1
}

echo "== install_test.sh: prefix temporal $PREFIX =="

# 1. Instalación limpia: la lista exacta de archivos esperados.
"$INSTALL_SH" --prefix "$PREFIX"

expected_files='
bin/cincel
share/applications/dev.cincel.Cincel.desktop
share/metainfo/dev.cincel.Cincel.metainfo.xml
share/icons/hicolor/scalable/apps/dev.cincel.Cincel.svg
share/icons/hicolor/16x16/apps/dev.cincel.Cincel.png
share/icons/hicolor/32x32/apps/dev.cincel.Cincel.png
share/icons/hicolor/48x48/apps/dev.cincel.Cincel.png
share/icons/hicolor/64x64/apps/dev.cincel.Cincel.png
share/icons/hicolor/128x128/apps/dev.cincel.Cincel.png
share/icons/hicolor/256x256/apps/dev.cincel.Cincel.png
share/icons/hicolor/512x512/apps/dev.cincel.Cincel.png
share/doc/cincel/LICENSE
share/doc/cincel/CHANGELOG.md
share/doc/cincel/THIRD-PARTY-LICENSES.html
share/doc/cincel/OFL-Inter.txt
share/doc/cincel/OFL-JetBrainsMono.txt'

echo "$expected_files" | while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    [ -e "$PREFIX/$rel" ] || fail "no se instaló $rel"
done
echo "-- lista de archivos: ok"

# `Exec=` tiene que quedar con la ruta absoluta del binario instalado (§6.2).
grep -q "^Exec=$PREFIX/bin/cincel %f$" \
    "$PREFIX/share/applications/dev.cincel.Cincel.desktop" \
    || fail "Exec= no quedó absoluto"
echo "-- Exec= absoluto: ok"

# 2. Segunda instalación encima de la primera: no debe fallar ni duplicar.
"$INSTALL_SH" --prefix "$PREFIX"
echo "-- segunda instalación sobre la primera: ok"

# 3. `--uninstall` sin instalación previa (prefix aparte) no falla.
EMPTY_PREFIX=$(mktemp -d)
"$INSTALL_SH" --prefix "$EMPTY_PREFIX" --uninstall
rm -rf "$EMPTY_PREFIX"
echo "-- --uninstall sin instalación previa: ok"

# 4. Desinstalar de verdad: no debe quedar nada de lo instalado.
"$INSTALL_SH" --prefix "$PREFIX" --uninstall
echo "$expected_files" | while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    [ ! -e "$PREFIX/$rel" ] || fail "--uninstall dejó $rel"
done
echo "-- desinstalación completa: ok"

echo "install_test.sh: todo bien"
