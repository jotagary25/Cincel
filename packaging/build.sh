#!/bin/sh
# Builds the release binary and the two distributable packages
# (`docs/specs/08-etapa6-cierre-1-0.md` §6.6) inside the pinned
# `ubuntu:22.04` image (`packaging/docker/Dockerfile.build`), so the result
# links against glibc 2.35 (D9) regardless of the host's own glibc.
#
# Output: dist/cincel-1.0.0-x86_64-linux.tar.gz(.sha256) and
# dist/cincel_1.0.0-1_amd64.deb(.sha256).
#
# `CARGO_TARGET_DIR` is `target/ubuntu-22.04` (inside the repo's ignored
# `target/`, D17/§6.6); call `cargo clean --target-dir target/ubuntu-22.04`
# afterwards to reclaim the disk it used (`packaging/verify.sh` does this
# when run after `build.sh`, and so does `packaging/README.md`'s recipe).
set -eu

REPO_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
IMAGE_TAG=cincel-build:ubuntu-22.04
CARGO_TARGET_SUBDIR=target/ubuntu-22.04
DIST_DIR="$REPO_ROOT/dist"
VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)
PKG_NAME="cincel-$VERSION-x86_64-linux"

echo "==> Versión: $VERSION"

echo "==> Preparando la imagen de compilación (ubuntu:22.04, D9)"
docker build -q -t "$IMAGE_TAG" \
    -f "$REPO_ROOT/packaging/docker/Dockerfile.build" \
    "$REPO_ROOT/packaging/docker" >/dev/null

mkdir -p "$REPO_ROOT/$CARGO_TARGET_SUBDIR" "$DIST_DIR"

run_in_container() {
    docker run --rm \
        --user "$(id -u):$(id -g)" \
        -e HOME=/tmp \
        -e CARGO_TARGET_DIR="/repo/$CARGO_TARGET_SUBDIR" \
        -v "$REPO_ROOT:/repo" \
        -w /repo \
        "$IMAGE_TAG" \
        sh -c "$1"
}

# Maintainer del `.deb` (§11, decisión del autor): `CINCEL_MAINTAINER`, si no
# `packaging/maintainer.txt` (ignorado por git, ver `packaging/README.md`),
# si no el valor genérico que ya trae `crates/cincel/Cargo.toml`. El archivo
# real nunca se escribe en el repo: se restaura al terminar.
MAINTAINER="${CINCEL_MAINTAINER:-}"
if [ -z "$MAINTAINER" ] && [ -f "$REPO_ROOT/packaging/maintainer.txt" ]; then
    MAINTAINER=$(head -n1 "$REPO_ROOT/packaging/maintainer.txt")
fi
CINCEL_CARGO_TOML="$REPO_ROOT/crates/cincel/Cargo.toml"
restore_cargo_toml() {
    if [ -f "$CINCEL_CARGO_TOML.orig" ]; then
        mv -f "$CINCEL_CARGO_TOML.orig" "$CINCEL_CARGO_TOML"
    fi
}
trap restore_cargo_toml EXIT INT TERM
if [ -n "$MAINTAINER" ]; then
    cp "$CINCEL_CARGO_TOML" "$CINCEL_CARGO_TOML.orig"
    escaped=$(printf '%s' "$MAINTAINER" | sed 's/[&\]/\\&/g')
    sed -i "s|^maintainer = .*|maintainer = \"$escaped\"|" "$CINCEL_CARGO_TOML"
fi

echo "==> Compilando cincel en release (glibc 2.35)"
run_in_container "cargo build --release --locked -p cincel"

echo "==> Generando THIRD-PARTY-LICENSES.html (cargo-about, D19)"
run_in_container "cargo about generate \
    --config packaging/about.toml \
    --manifest-path crates/cincel/Cargo.toml \
    packaging/about.hbs \
    -o $CARGO_TARGET_SUBDIR/THIRD-PARTY-LICENSES.html"

echo "==> Armando el .deb (cargo-deb)"
run_in_container "cargo deb -p cincel \
    --no-strip \
    -o $CARGO_TARGET_SUBDIR/cincel_${VERSION}-1_amd64.deb"
cp "$REPO_ROOT/$CARGO_TARGET_SUBDIR/cincel_${VERSION}-1_amd64.deb" "$DIST_DIR/"

echo "==> Armando el tarball (§6.1)"
STAGE_DIR="$REPO_ROOT/$CARGO_TARGET_SUBDIR/stage/$PKG_NAME"
rm -rf "$STAGE_DIR"
mkdir -p \
    "$STAGE_DIR/bin" \
    "$STAGE_DIR/share/applications" \
    "$STAGE_DIR/share/icons/hicolor/scalable/apps" \
    "$STAGE_DIR/share/doc/cincel"
for size in 16x16 32x32 48x48 64x64 128x128 256x256 512x512; do
    mkdir -p "$STAGE_DIR/share/icons/hicolor/$size/apps"
    cp "$REPO_ROOT/packaging/icons/png/$size/apps/dev.cincel.Cincel.png" \
        "$STAGE_DIR/share/icons/hicolor/$size/apps/dev.cincel.Cincel.png"
done

install -m 0755 "$REPO_ROOT/packaging/install.sh" "$STAGE_DIR/install.sh"
install -m 0755 "$REPO_ROOT/$CARGO_TARGET_SUBDIR/release/cincel" "$STAGE_DIR/bin/cincel"
install -m 0644 "$REPO_ROOT/packaging/linux/dev.cincel.Cincel.desktop" \
    "$STAGE_DIR/share/applications/dev.cincel.Cincel.desktop"
install -m 0644 "$REPO_ROOT/packaging/icons/dev.cincel.Cincel.svg" \
    "$STAGE_DIR/share/icons/hicolor/scalable/apps/dev.cincel.Cincel.svg"
install -m 0644 "$REPO_ROOT/LICENSE" "$STAGE_DIR/share/doc/cincel/LICENSE"
install -m 0644 "$REPO_ROOT/CHANGELOG.md" "$STAGE_DIR/share/doc/cincel/CHANGELOG.md"
install -m 0644 "$REPO_ROOT/$CARGO_TARGET_SUBDIR/THIRD-PARTY-LICENSES.html" \
    "$STAGE_DIR/share/doc/cincel/THIRD-PARTY-LICENSES.html"
install -m 0644 "$REPO_ROOT/crates/cincel-workspace/assets/fonts/inter/OFL.txt" \
    "$STAGE_DIR/share/doc/cincel/OFL-Inter.txt"
install -m 0644 "$REPO_ROOT/crates/cincel-workspace/assets/fonts/jetbrains-mono/OFL.txt" \
    "$STAGE_DIR/share/doc/cincel/OFL-JetBrainsMono.txt"

(cd "$REPO_ROOT/$CARGO_TARGET_SUBDIR/stage" && tar --numeric-owner --owner=0 --group=0 \
    -czf "$DIST_DIR/$PKG_NAME.tar.gz" "$PKG_NAME")

echo "==> Sumas de verificación"
(cd "$DIST_DIR" && sha256sum "$PKG_NAME.tar.gz" > "$PKG_NAME.tar.gz.sha256")
(cd "$DIST_DIR" && sha256sum "cincel_${VERSION}-1_amd64.deb" > "cincel_${VERSION}-1_amd64.deb.sha256")

echo "==> Listo:"
ls -la "$DIST_DIR"
