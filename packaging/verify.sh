#!/bin/sh
# Verifies the two packages from `packaging/build.sh` in clean containers
# (`docs/specs/08-etapa6-cierre-1-0.md` §6.6): the `.deb` and the tarball,
# on both `ubuntu:22.04` and `ubuntu:24.04`; `ldd` and the glibc symbol
# version of the binary; and a real Wayland smoke test in the measurement
# bench (D1). Every container is `--rm` and freshly pulled by digest.
#
# Usage: packaging/verify.sh [DIST_DIR]
#   DIST_DIR: where build.sh left the packages (default: dist)
set -eu

REPO_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
DIST_DIR=${1:-"$REPO_ROOT/dist"}
# Docker only bind-mounts absolute paths: accept a relative one (the release
# workflow passes `dist`) by resolving it here.
DIST_DIR=$(cd "$DIST_DIR" && pwd)
VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)
PKG_NAME="cincel-$VERSION-x86_64-linux"
TARBALL="$DIST_DIR/$PKG_NAME.tar.gz"
DEB="$DIST_DIR/cincel_${VERSION}-1_amd64.deb"

UBUNTU_2204="ubuntu:22.04@sha256:b8b6ee6aa931ecd9d0d952abc34dc0e5f7c6a30c6bb71b079fe399fde0329c02"
UBUNTU_2404="ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3"

fail() {
    echo "verify.sh: FALLÓ: $1" >&2
    exit 1
}

[ -f "$TARBALL" ] || fail "no existe $TARBALL (corré packaging/build.sh primero)"
[ -f "$DEB" ] || fail "no existe $DEB (corré packaging/build.sh primero)"

echo "== Tamaño del binario (M9, meta 20-60 MB) =="
BIN_SIZE=$(tar -xOzf "$TARBALL" "$PKG_NAME/bin/cincel" | wc -c)
BIN_MB=$((BIN_SIZE / 1024 / 1024))
echo "bin/cincel: ${BIN_MB} MB"
if [ "$BIN_MB" -lt 20 ] || [ "$BIN_MB" -gt 60 ]; then
    echo "AVISO: fuera de la meta 20-60 MB (desviación a documentar, D11)." >&2
fi

echo "== ldd del binario (D9) =="
BIN_TMP=$(mktemp)
trap 'rm -f "$BIN_TMP"' EXIT
tar -xOzf "$TARBALL" "$PKG_NAME/bin/cincel" > "$BIN_TMP"
chmod +x "$BIN_TMP"
LDD_OUT=$(ldd "$BIN_TMP" 2>&1) || fail "ldd no pudo leer el binario:
$LDD_OUT"
echo "$LDD_OUT"
# Solo las bibliotecas de D9 (Wayland y Vulkan se cargan por dlopen y no
# aparecen acá) y sus propias dependencias de X.
ALLOWED_RE='^(linux-vdso\.so|libc\.so|libm\.so|libgcc_s\.so|libxcb\.so|libxkbcommon\.so|libxkbcommon-x11\.so|libXau\.so|libXdmcp\.so|libxcb-xkb\.so|libbsd\.so|libmd\.so|/lib64/ld-linux-x86-64\.so)'
BAD_LIBS=$(echo "$LDD_OUT" | awk '{print $1}' | grep -Ev "$ALLOWED_RE" || true)
[ -z "$BAD_LIBS" ] || fail "biblioteca fuera de la lista permitida (D9):
$BAD_LIBS"
echo "-- ldd: solo bibliotecas permitidas"

echo "== objdump -T: ninguna versión GLIBC_ mayor que 2.35 =="
if command -v objdump >/dev/null 2>&1; then
    BAD=$(objdump -T "$BIN_TMP" 2>/dev/null | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -Vu | awk -F_ '{
        split($2, v, ".");
        if (v[1] > 2 || (v[1] == 2 && v[2] > 35)) print
    }')
    [ -z "$BAD" ] || fail "símbolos GLIBC_ por encima de 2.35: $BAD"
    echo "-- objdump: ninguna versión de GLIBC_ mayor que 2.35"
else
    echo "AVISO: objdump no está instalado en el host; se omite (desviación)." >&2
fi

verify_deb_in() {
    image=$1
    echo "== .deb en $image =="
    docker run --rm \
        -v "$DEB:/pkg/cincel.deb:ro" \
        "$image" \
        sh -c '
            set -eu
            export DEBIAN_FRONTEND=noninteractive
            apt-get update >/dev/null
            apt-get install -y /pkg/cincel.deb >/dev/null
            cincel --version
            apt-get install -y --no-install-recommends desktop-file-utils >/dev/null
            desktop-file-validate /usr/share/applications/dev.cincel.Cincel.desktop
            apt-get install -y --no-install-recommends appstream >/dev/null
            appstreamcli validate --no-net /usr/share/metainfo/dev.cincel.Cincel.metainfo.xml
            apt-get install -y --no-install-recommends xvfb mesa-vulkan-drivers ca-certificates fonts-dejavu-core >/dev/null
            CINCEL_ALLOW_SOFTWARE_GPU=1 xvfb-run -a cincel --smoke-test /tmp
            apt-get remove -y cincel >/dev/null
            leftover=$(find /usr -iname "*cincel*" 2>/dev/null || true)
            if [ -n "$leftover" ]; then
                echo "quedaron archivos tras desinstalar:" >&2
                echo "$leftover" >&2
                exit 1
            fi
            echo "-- .deb: instala, corre, desinstala sin dejar nada"
        '
}

verify_tarball_in() {
    image=$1
    echo "== tarball en $image (usuario común) =="
    docker run --rm \
        -v "$TARBALL:/pkg/tarball.tar.gz:ro" \
        -v "$REPO_ROOT/packaging/tests/install_test.sh:/pkg/install_test.sh:ro" \
        "$image" \
        sh -c '
            set -eu
            export DEBIAN_FRONTEND=noninteractive
            apt-get update >/dev/null
            # The tarball assumes a real desktop already has these (D9); a
            # bare container does not, so they go in here to stand in for
            # that desktop, same as the deb Depends field.
            apt-get install -y --no-install-recommends \
                xvfb mesa-vulkan-drivers ca-certificates fonts-dejavu-core \
                libxkbcommon0 libxkbcommon-x11-0 libxcb1 >/dev/null
            useradd -m tester
            cp /pkg/tarball.tar.gz /pkg/install_test.sh /home/tester/
            su tester -c "
                set -eu
                cd /home/tester
                tar xzf tarball.tar.gz
                sh install_test.sh '"$PKG_NAME"'
                ./'"$PKG_NAME"'/install.sh
                ~/.local/bin/cincel --version
                CINCEL_ALLOW_SOFTWARE_GPU=1 xvfb-run -a ~/.local/bin/cincel --smoke-test /tmp
                test -d ~/.config/cincel
                ./'"$PKG_NAME"'/install.sh --uninstall
                test ! -e ~/.local/bin/cincel
                test -d ~/.config/cincel
            "
            echo "-- tarball: instala, corre, desinstala sin borrar la config"
        '
}

for img in "$UBUNTU_2204" "$UBUNTU_2404"; do
    verify_deb_in "$img"
    verify_tarball_in "$img"
done

echo "== Smoke test en Wayland (banco sway, D1) =="
STAGE=$(mktemp -d)
tar -xzf "$TARBALL" -C "$STAGE"
if [ -x "$REPO_ROOT/target/release/cincel-perf" ] \
    && docker image inspect cincel-perf:ubuntu-24.04 >/dev/null 2>&1
then
    CINCEL_BIN="$STAGE/$PKG_NAME/bin/cincel" PERF_IGNORE_LOAD=1 \
        "$REPO_ROOT/tools/perf/run.sh" smoke \
        || fail "el binario empaquetado no pasó el smoke test de Wayland"
    echo "-- smoke test en sway: ok"
else
    echo "AVISO: cincel-perf o su imagen no están listos; se omite (desviación anotada)." >&2
fi
rm -rf "$STAGE"

echo "verify.sh: todo bien"
