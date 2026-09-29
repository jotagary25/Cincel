#!/usr/bin/env bash
# External measurement bench of Cincel (spec 08 §3.2 and §3.6).
#
#   tools/perf/run.sh [full|smoke|cosmic]
#
#   full    every measurement of §3.6 for each app (about 15 min per app)
#   smoke   one cold start per app plus one short quick-open/type/scroll
#   cosmic  cross-check in a nested cosmic-comp (D2): only "window mapped"
#
# The apps come in through variables, never written here:
#   CINCEL_BIN       default: <repo>/target/release/cincel
#   ZED_BIN          default: `command -v zed`
#   ANTIGRAVITY_BIN  default: `command -v antigravity-ide` or `antigravity`
#                    (it must be the VS Code-based IDE, not "Antigravity 2.0")
#   VSCODE_BIN       optional (the `code` executable)
# Other variables:
#   PERF_APPS        subset to measure (default: every app found)
#   PERF_WORK_DIR    corpus and profiles (default: a new temporary folder,
#                    deleted at the end); never inside the repository
#   PERF_OUT_DIR     results (default: a new temporary folder, kept)
#   PERF_REPS        cold/warm starts per app in full mode (default 10)
#   PERF_BENCH_ARGS  arguments of the internal `cincel --bench` step (full),
#                    relative to a writable copy of the corpus
#   PERF_RENDER_NODE GPU render node (default: first non-NVIDIA GPU)
#   PERF_ELECTRON_FLAGS  extra flags for Electron apps (default --no-sandbox)
#   PERF_IGNORE_LOAD=1   run even if the load average is above 2
#
# Nothing of the user's own configuration is read or written: every app gets
# a fresh profile (HOME and XDG folders included) that is deleted after each
# run. The only screen captures come from the invisible desktop of the
# container.
set -euo pipefail

MODE=${1:-full}
case $MODE in full | smoke | cosmic) ;; *)
    echo "uso: $0 [full|smoke|cosmic]" >&2
    exit 2
    ;;
esac

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
IMAGE=cincel-perf:ubuntu-24.04
read -r -a CARGO_CMD <<<"${PERF_CARGO:-cargo +stable}"

die() {
    echo "run.sh: $*" >&2
    exit 1
}

inside_repo() {
    case $(realpath -m "$1")/ in "$REPO"/*) return 0 ;; esac
    return 1
}

# ------------------------------------------------------------------ apps

declare -A APP_DIR APP_EXE APP_FILE
find_app() {
    local name=$1 bin=$2
    [ -n "$bin" ] || return 1
    local real
    real=$(realpath -e "$bin" 2>/dev/null) || return 1
    local dir base parent
    dir=$(dirname "$real")
    base=$(basename "$real")
    parent=$(dirname "$dir")
    case $name in
        cincel)
            APP_FILE[$name]=$real
            APP_EXE[$name]=/apps/cincel/cincel
            return 0
            ;;
        zed)
            # The `zed` command is a small CLI; the editor is libexec/zed-editor.
            if [ -x "$parent/libexec/zed-editor" ]; then
                APP_DIR[$name]=$parent
                APP_EXE[$name]=/apps/zed/libexec/zed-editor
                return 0
            fi
            ;;
    esac
    # Electron apps: `bin/<name>` is a launcher script next to the binary.
    if [ "$(basename "$dir")" = bin ] && head -c2 "$real" | grep -q '#!'; then
        if [ -x "$parent/$base" ]; then
            APP_DIR[$name]=$parent
            APP_EXE[$name]=/apps/$name/$base
            return 0
        fi
        # VS Code .deb: /usr/share/code/bin/code -> /usr/share/code/code
        if [ -x "$parent/$(basename "$parent")" ]; then
            APP_DIR[$name]=$parent
            APP_EXE[$name]=/apps/$name/$(basename "$parent")
            return 0
        fi
    fi
    APP_DIR[$name]=$dir
    APP_EXE[$name]=/apps/$name/$base
}

CINCEL_BIN=${CINCEL_BIN:-$REPO/target/release/cincel}
ZED_BIN=${ZED_BIN:-$(command -v zed || true)}
ANTIGRAVITY_BIN=${ANTIGRAVITY_BIN:-$(command -v antigravity-ide || command -v antigravity || true)}
VSCODE_BIN=${VSCODE_BIN:-}

FOUND=()
find_app cincel "$CINCEL_BIN" && FOUND+=(cincel) ||
    echo "run.sh: no está $CINCEL_BIN; construilo con: cargo build --release -p cincel" >&2
find_app zed "$ZED_BIN" && FOUND+=(zed) || echo "run.sh: Zed no encontrado (ZED_BIN)" >&2
find_app antigravity "$ANTIGRAVITY_BIN" && FOUND+=(antigravity) ||
    echo "run.sh: Antigravity no encontrado (ANTIGRAVITY_BIN)" >&2
find_app vscode "$VSCODE_BIN" && FOUND+=(vscode) ||
    echo "run.sh: VS Code no instalado en la máquina de referencia; se omite" >&2

read -r -a APPS <<<"${PERF_APPS:-${FOUND[*]}}"
for app in "${APPS[@]}"; do
    [ -n "${APP_EXE[$app]:-}" ] || die "la app $app no está disponible"
    case $app in
        antigravity | vscode)
            # The comparison is with the VS Code-based editor. "Antigravity
            # 2.0" (the agent manager, often what `antigravity` runs) is not
            # an editor: it only shows a sign-in screen.
            [ -f "${APP_DIR[$app]}/resources/app/product.json" ] ||
                die "$app no es un editor derivado de VS Code (falta resources/app/product.json); apuntá ${app^^}_BIN al ejecutable del IDE (por ejemplo antigravity-ide)"
            ;;
    esac
done
[ "${#APPS[@]}" -gt 0 ] || die "no hay apps para medir"

# ------------------------------------------------------------------ folders

CREATED_WORK=0
if [ -z "${PERF_WORK_DIR:-}" ]; then
    PERF_WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/cincel-perf-work.XXXXXX")
    CREATED_WORK=1
fi
PERF_OUT_DIR=${PERF_OUT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/cincel-perf-out.XXXXXX")}
inside_repo "$PERF_WORK_DIR" && die "PERF_WORK_DIR no puede estar dentro del repositorio"
inside_repo "$PERF_OUT_DIR" && die "PERF_OUT_DIR no puede estar dentro del repositorio"
mkdir -p "$PERF_WORK_DIR" "$PERF_OUT_DIR"
PROFILES=$PERF_WORK_DIR/perfiles
CORPUS=$PERF_WORK_DIR/corpus
rm -rf "$PROFILES"
mkdir -p "$PROFILES"

CONTAINER=cincel-perf-$$
NESTED_PID=
cleanup() {
    if docker ps -q --filter "name=^${CONTAINER}$" | grep -q .; then
        docker kill "$CONTAINER" >/dev/null 2>&1 || true
    fi
    if [ -n "$NESTED_PID" ] && kill -0 "$NESTED_PID" 2>/dev/null; then
        kill "$NESTED_PID" 2>/dev/null || true
        wait "$NESTED_PID" 2>/dev/null || true
    fi
    rm -rf "$PROFILES"
    if [ "$CREATED_WORK" = 1 ]; then
        rm -rf "$PERF_WORK_DIR"
    fi
}
trap cleanup EXIT

# ------------------------------------------------------------------ checks

load=$(cut -d' ' -f1 /proc/loadavg)
if awk -v l="$load" 'BEGIN { exit !(l > 2) }' && [ "${PERF_IGNORE_LOAD:-0}" != 1 ]; then
    die "la carga del sistema es $load (> 2): esperá o repetí más tarde (PERF_IGNORE_LOAD=1 para forzar)"
fi

echo "run.sh: compilando cincel-perf" >&2
(cd "$REPO" && timeout 900 "${CARGO_CMD[@]}" build --release -p cincel-perf >&2)
PERF=$REPO/target/release/cincel-perf

if [ ! -d "$CORPUS/medio" ]; then
    echo "run.sh: generando el corpus en $CORPUS" >&2
    corpus_flags=()
    [ "$MODE" = full ] || corpus_flags+=(--no-large)
    "$PERF" corpus "$CORPUS" "${corpus_flags[@]}" >"$PERF_OUT_DIR/corpus.jsonl"
fi

# ------------------------------------------------------------------ environment

gpu_node() {
    if [ -n "${PERF_RENDER_NODE:-}" ]; then
        echo "$PERF_RENDER_NODE"
        return
    fi
    local node
    for node in /sys/class/drm/renderD*; do
        # NVIDIA (0x10de) needs its own container toolkit: skipped.
        if [ "$(cat "$node/device/vendor")" != 0x10de ]; then
            echo "/dev/dri/$(basename "$node")"
            return
        fi
    done
}

zed_version() {
    # Only "Zed <version>": the full line also names the install path.
    if [ -n "${APP_DIR[zed]:-}" ]; then
        "${APP_DIR[zed]}/bin/zed" --version 2>/dev/null | awk 'NR == 1 {print $1, $2}'
    fi
}

write_env() {
    local node=$1 driver
    driver=$(sed -n 's/^DRIVER=//p' "/sys/class/drm/$(basename "$node")/device/uevent" 2>/dev/null)
    jq -nc \
        --arg mode "$MODE" \
        --arg cpu "$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo | head -n1)" \
        --arg cores "$(nproc)" \
        --arg mem_total "$(awk '/^MemTotal/ {print $2}' /proc/meminfo)" \
        --arg mem_free "$(awk '/^MemAvailable/ {print $2}' /proc/meminfo)" \
        --arg gpu "$(basename "$node") ($driver)" \
        --arg kernel "$(uname -r)" \
        --arg cosmic "$(dpkg-query -W -f '${Version}' cosmic-comp 2>/dev/null || true)" \
        --arg zed "$(zed_version)" \
        --arg load "$(cat /proc/loadavg)" \
        '{kind: "host_env", mode: $mode, cpu: $cpu, cores: $cores,
          mem_total_kb: $mem_total, mem_available_kb: $mem_free, gpu: $gpu,
          kernel: $kernel, cosmic_comp: $cosmic, zed: $zed, loadavg: $load}' \
        >>"$PERF_OUT_DIR/results.jsonl"
}

# ------------------------------------------------------------------ cosmic

run_cosmic() {
    # D2: a nested cosmic-comp window on the author's desktop; only "window
    # mapped" is measured (ext-foreign-toplevel-list), nothing is captured.
    command -v cosmic-comp >/dev/null || {
        echo '{"kind":"cosmic","error":"cosmic-comp no está instalado"}' >>"$PERF_OUT_DIR/results.jsonl"
        return
    }
    [ -n "${WAYLAND_DISPLAY:-}" ] || die "el modo cosmic necesita una sesión Wayland"
    local parent_display=$WAYLAND_DISPLAY
    case $parent_display in /*) ;; *) parent_display=$XDG_RUNTIME_DIR/$parent_display ;; esac
    local nested=$PERF_WORK_DIR/cosmic
    rm -rf "$nested"
    mkdir -p "$nested/run" "$nested/home" "$nested/config" "$nested/state" "$nested/data" "$nested/cache"
    chmod 700 "$nested/run"
    # The nested compositor gets its own runtime, config and state folders so
    # it never reads or writes the author's COSMIC settings.
    env -u DISPLAY WAYLAND_DISPLAY="$parent_display" XDG_RUNTIME_DIR="$nested/run" \
        HOME="$nested/home" XDG_CONFIG_HOME="$nested/config" XDG_STATE_HOME="$nested/state" \
        XDG_DATA_HOME="$nested/data" XDG_CACHE_HOME="$nested/cache" \
        timeout 900 cosmic-comp --no-xwayland >"$PERF_OUT_DIR/cosmic-comp.log" 2>&1 &
    NESTED_PID=$!
    local socket=
    for candidate in wayland-1 wayland-0 wayland-2; do
        if XDG_RUNTIME_DIR="$nested/run" WAYLAND_DISPLAY=$candidate \
            timeout 20 "$PERF" wait-wayland --timeout-ms 15000 >/dev/null 2>&1; then
            socket=$candidate
            break
        fi
    done
    if [ -z "$socket" ]; then
        echo '{"kind":"cosmic","error":"cosmic-comp anidado no arrancó"}' >>"$PERF_OUT_DIR/results.jsonl"
        return
    fi
    local probe
    probe=$(XDG_RUNTIME_DIR="$nested/run" WAYLAND_DISPLAY=$socket "$PERF" probe)
    jq -c '{kind: "cosmic_probe", bench_protocols}' <<<"$probe" >>"$PERF_OUT_DIR/results.jsonl"
    if [ "$(jq -r '.bench_protocols.ext_foreign_toplevel_list_v1' <<<"$probe")" = null ]; then
        echo '{"kind":"cosmic","error":"cosmic-comp no ofrece ext_foreign_toplevel_list_v1 a clientes comunes"}' \
            >>"$PERF_OUT_DIR/results.jsonl"
        return
    fi
    local reps=${PERF_REPS:-10}
    [ "$MODE" = cosmic ] && [ -n "${PERF_COSMIC_REPS:-}" ] && reps=$PERF_COSMIC_REPS
    for app in "${APPS[@]}"; do
        local cmd=()
        if [ "$app" = cincel ]; then
            cmd=("${APP_FILE[cincel]}")
        else
            cmd=("${APP_DIR[$app]}${APP_EXE[$app]#/apps/"$app"}")
        fi
        case $app in antigravity | vscode) read -r -a extra <<<"${PERF_ELECTRON_FLAGS:---no-sandbox}" && cmd+=("${extra[@]}") ;; esac
        for i in $(seq 1 "$reps"); do
            "$PERF" evict "$(dirname "${cmd[0]}")" >/dev/null
            XDG_RUNTIME_DIR="$nested/run" WAYLAND_DISPLAY=$socket \
                "$PERF" launch --name "$app" --tag cosmic --toplevel-protocol ext \
                --profile "$PROFILES/$app-cosmic-$i" --settle-ms 0 --idle-ms 0 \
                --app-log "$PERF_OUT_DIR/cosmic-$app-$i.log" -- "${cmd[@]}" "$CORPUS/medio" \
                >>"$PERF_OUT_DIR/results.jsonl" || true
        done
    done
    kill "$NESTED_PID" 2>/dev/null || true
    wait "$NESTED_PID" 2>/dev/null || true
    NESTED_PID=
}

# ------------------------------------------------------------------ bench

run_bench() {
    local node
    node=$(gpu_node)
    if [ -z "$node" ] || [ ! -e "$node" ]; then
        die "no hay una GPU usable en /dev/dri"
    fi
    write_env "$node"

    echo "run.sh: armando la imagen $IMAGE" >&2
    timeout 1800 docker build -q -t "$IMAGE" "$REPO/tools/perf" >/dev/null

    local mounts=(
        -v "$CORPUS:/corpus:ro"
        -v "$PROFILES:/perfiles"
        -v "$PERF_OUT_DIR:/out"
        -v "$REPO/tools/perf:/bench/tools:ro"
        -v "$PERF:/bench/cincel-perf:ro"
    )
    local envs=(
        -e PERF_MODE="$MODE"
        -e PERF_APPS="${APPS[*]}"
        -e PERF_REPS="${PERF_REPS:-10}"
        -e PERF_BENCH_ARGS="${PERF_BENCH_ARGS:-all medio --bench-file medio/bench/cinco-mil.rs}"
        -e PERF_TIMEOUT="${PERF_TIMEOUT:-5400}"
        -e WLR_RENDER_DRM_DEVICE="$node"
    )
    for app in "${APPS[@]}"; do
        if [ "$app" = cincel ]; then
            mounts+=(-v "${APP_FILE[cincel]}:/apps/cincel/cincel:ro")
        else
            mounts+=(-v "${APP_DIR[$app]}:/apps/$app:ro")
        fi
        envs+=(-e "PERF_CMD_$app=${APP_EXE[$app]}")
        case $app in
            antigravity | vscode)
                # Chromium's sandbox needs a setuid helper or user namespaces,
                # neither available to an unprivileged container user.
                envs+=(-e "PERF_ARGS_$app=${PERF_ELECTRON_FLAGS:---no-sandbox}")
                ;;
        esac
        case $app in
            antigravity)
                # Its agent side panel opens by itself and, without network,
                # animates a "No internet" spinner forever; it is closed like
                # Zed's AI features are turned off (D5).
                envs+=(-e "PERF_SETUP_$app=${PERF_ANTIGRAVITY_SETUP:-ctrl+alt+b}")
                ;;
        esac
    done
    rm -f "$PERF_OUT_DIR/inner.status"
    echo "run.sh: midiendo ${APPS[*]} ($MODE) en el escritorio invisible" >&2
    local code=0
    timeout --kill-after=30 "$((${PERF_TIMEOUT:-5400} + 120))" docker run --rm \
        --name "$CONTAINER" \
        --network none \
        --user "$(id -u):$(id -g)" \
        --group-add "$(stat -c %g "$node")" \
        --device "$node" \
        --shm-size 1g \
        "${mounts[@]}" "${envs[@]}" \
        "$IMAGE" >"$PERF_OUT_DIR/container.log" 2>&1 || code=$?
    if [ "$code" -ne 0 ]; then
        echo "run.sh: el contenedor terminó con código $code (ver container.log)" >&2
    fi
    [ "$(cat "$PERF_OUT_DIR/inner.status" 2>/dev/null)" = 0 ] ||
        echo "run.sh: alguna medición falló (ver inner.log y logs/)" >&2
}

case $MODE in
    cosmic)
        write_env "$(gpu_node)"
        run_cosmic
        ;;
    *) run_bench ;;
esac

"$PERF" report "$PERF_OUT_DIR/results.jsonl" >"$PERF_OUT_DIR/report.md"
echo "run.sh: resultados en $PERF_OUT_DIR (results.jsonl, report.md)" >&2
