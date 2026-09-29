#!/bin/sh
# Container entry point: a private runtime folder, then sway (which runs
# inner.sh and exits when inner.sh asks it to). PERF_TIMEOUT bounds the whole
# run from inside, so nothing outlives a stuck measurement.
set -eu
XDG_RUNTIME_DIR=$(mktemp -d /tmp/xdg.XXXXXX)
chmod 700 "$XDG_RUNTIME_DIR"
export XDG_RUNTIME_DIR
HOME=$(mktemp -d /tmp/home.XXXXXX)
export HOME
export WLR_BACKENDS=headless
export WLR_HEADLESS_OUTPUTS=1
export WLR_LIBINPUT_NO_DEVICES=1
export WLR_RENDERER="${WLR_RENDERER:-gles2}"
unset DISPLAY
# The host's NVIDIA kernel module makes sway refuse to start; the container
# only has the other GPU's render node, so the check does not apply.
exec timeout --kill-after=10 "${PERF_TIMEOUT:-3600}" sway --unsupported-gpu -c /bench/tools/sway.config
