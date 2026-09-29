#!/bin/sh
# Renders the PNG sizes of the app icon from the SVG source
# (`docs/specs/08-etapa6-cierre-1-0.md` §6.4). Runs `rsvg-convert` inside a
# disposable `ubuntu:24.04` container so the host needs nothing installed;
# the output is versioned under `packaging/icons/png/` (it is small and
# regenerating it needs Docker, which is not always at hand).
#
# Usage: packaging/icons/render.sh
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
SVG="$SCRIPT_DIR/dev.cincel.Cincel.svg"
OUT_DIR="$SCRIPT_DIR/png"
IMAGE="ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3"

if [ ! -f "$SVG" ]; then
    echo "render.sh: no se encontró $SVG" >&2
    exit 1
fi

for size in 16 32 48 64 128 256 512; do
    mkdir -p "$OUT_DIR/${size}x${size}/apps"
done

# The container runs as root (apt-get needs it); the output directory is
# chowned back to the caller's uid/gid at the end so nothing root-owned is
# left on the host.
docker run --rm \
    -v "$SCRIPT_DIR:/work:ro" \
    -v "$OUT_DIR:/out" \
    -w /work \
    -e HOST_UID="$(id -u)" \
    -e HOST_GID="$(id -g)" \
    "$IMAGE" \
    sh -c '
        set -eu
        apt-get update >/dev/null
        apt-get install -y --no-install-recommends librsvg2-bin >/dev/null
        for size in 16 32 48 64 128 256 512; do
            rsvg-convert -w "$size" -h "$size" dev.cincel.Cincel.svg \
                -o "/out/${size}x${size}/apps/dev.cincel.Cincel.png"
        done
        chown -R "$HOST_UID:$HOST_GID" /out
    '

echo "Iconos generados en $OUT_DIR"
