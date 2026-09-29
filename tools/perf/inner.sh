#!/usr/bin/env bash
# Runs inside sway (exec from sway.config), in the container started by
# run.sh. Measures each app of $PERF_APPS following spec 08 §3.6 and writes
# one JSON line per measurement to /out/results.jsonl. Ends sway when done.
#
# Environment (set by run.sh):
#   PERF_MODE        full | smoke
#   PERF_APPS        e.g. "cincel zed antigravity"
#   PERF_CMD_<app>   executable inside the container (/apps/...)
#   PERF_ARGS_<app>  extra arguments before the project (optional)
#   PERF_SETUP_<app> keys injected once the window has content (optional)
#   PERF_REPS        cold and warm starts per app (full mode, default 10)
#   PERF_BENCH_ARGS  arguments of `cincel --bench` (full mode)
set -uo pipefail

PERF=/bench/cincel-perf
OUT=/out
RESULTS=$OUT/results.jsonl
LOGS=$OUT/logs
PROJECT=/corpus/medio
FILES=(cinco-mil.rs un-mega.rs cincuenta-mil.rs)
MODE=${PERF_MODE:-smoke}
REPS=${PERF_REPS:-10}
status=0

mkdir -p "$LOGS"

note() {
    printf '%s\n' "$*" >>"$OUT/inner.log"
}

record() {
    # One or more JSON lines from cincel-perf into the results file.
    if [ -n "$1" ]; then
        printf '%s\n' "$1" >>"$RESULTS"
    fi
}

profile_dir() {
    printf '/perfiles/%s-%s-%s' "$1" "$2" "$3"
}

app_command() {
    local app=$1 cmd_var args_var
    cmd_var="PERF_CMD_$app"
    args_var="PERF_ARGS_$app"
    # shellcheck disable=SC2206 # extra arguments are space separated on purpose
    APP_CMD=("${!cmd_var}" ${!args_var:-})
}

# launch APP TAG N [extra launch options...]
launch() {
    local app=$1 tag=$2 n=$3
    shift 3
    local out setup_var setup=()
    setup_var="PERF_SETUP_$app"
    [ -z "${!setup_var:-}" ] || setup=(--setup-keys "${!setup_var}")
    out=$("$PERF" launch --name "$app" --tag "$tag" \
        --profile "$(profile_dir "$app" "$tag" "$n")" \
        --app-log "$LOGS/$app-$tag-$n.log" "${setup[@]}" "$@" -- "${APP_CMD[@]}" "$PROJECT")
    local code=$?
    record "$out"
    [ $code -eq 0 ] || { note "launch $app $tag $n falló ($code)"; status=1; }
    LAST_LINE=$out
    return $code
}

environment() {
    {
        printf '{"kind":"bench_env",'
        printf '"sway":%s,' "$(sway --version | jq -R .)"
        printf '"output":%s}\n' "$(swaymsg -t get_outputs -r | jq -c '[.[] | {name, current_mode}]')"
    } >>"$RESULTS"
    for app in $PERF_APPS; do
        app_command "$app"
        local version
        case $app in
            cincel) version=$("${APP_CMD[0]}" --version 2>/dev/null | head -n1) ;;
            zed) version=$(/apps/zed/bin/zed --version 2>/dev/null | awk 'NR == 1 {print $1, $2}') ;;
            antigravity | vscode)
                local app_dir=/apps/$app/resources/app
                version=$(jq -r '"\(.nameShort) \(.ideVersion // .version // "")"' "$app_dir/product.json")
                version="$version (VS Code $(jq -r .version "$app_dir/package.json"))"
                ;;
            *) version="" ;;
        esac
        jq -nc --arg app "$app" --arg version "$version" '{kind: "app_version", app: $app, version: $version}' >>"$RESULTS"
    done
}

measure_files() {
    local app=$1 n=$2 reps=$3 count=$4 seconds=$5
    for file in "${FILES[@]}"; do
        if ! launch "$app" archivo "$n-$file" --settle-ms 0 --idle-ms 0 --keep-open; then
            continue
        fi
        local pid
        pid=$(jq -r .pid <<<"$LAST_LINE")
        record "$("$PERF" quick-open --app "$app" --file "$file" --repeat "$reps")" || status=1
        record "$("$PERF" type --app "$app" --file "$file" --count "$count")" || status=1
        record "$("$PERF" scroll --app "$app" --file "$file" --seconds "$seconds")" || status=1
        record "$("$PERF" stop --pid "$pid" --profile "$(profile_dir "$app" archivo "$n-$file")")"
        if [ "$MODE" = smoke ]; then
            break
        fi
    done
}

full() {
    local app=$1
    for i in $(seq 1 "$REPS"); do
        "$PERF" evict "/apps/$app" >>"$RESULTS"
        launch "$app" cold "$i" --settle-ms 0 --idle-ms 0
    done
    for i in $(seq 1 "$REPS"); do
        launch "$app" warm "$i" --settle-ms 0 --idle-ms 0
    done
    launch "$app" reposo 1 --open-file cinco-mil.rs --settle-ms 30000 --idle-ms 60000
    measure_files "$app" 1 5 100 3
    if [ "$app" = cincel ] && [ -n "${PERF_BENCH_ARGS:-}" ]; then
        # Internal measurements (§3.3) on a writable copy of the corpus.
        local scratch
        scratch=$(mktemp -d /perfiles/bench.XXXXXX)
        cp -a /corpus/. "$scratch/"
        local profile
        profile=$(profile_dir cincel bench 1)
        mkdir -p "$profile/config/cincel"
        printf '{\n  "editor": { "cursor_blink": false }\n}\n' >"$profile/config/cincel/settings.json"
        # shellcheck disable=SC2086 # PERF_BENCH_ARGS is a word list
        (cd "$scratch" && HOME=$profile CINCEL_CONFIG_DIR=$profile/config/cincel \
            XDG_DATA_HOME=$profile/data XDG_STATE_HOME=$profile/state XDG_CACHE_HOME=$profile/cache \
            timeout 900 "${APP_CMD[0]}" --bench $PERF_BENCH_ARGS) >>"$RESULTS" 2>"$LOGS/cincel-bench.log" \
            || { note "cincel --bench falló"; status=1; }
        rm -rf "$scratch" "$profile"
    fi
}

smoke() {
    local app=$1
    "$PERF" evict "/apps/$app" >>"$RESULTS"
    launch "$app" cold 1 --settle-ms 2000 --idle-ms 5000
    measure_files "$app" 1 1 10 1
}

environment
for app in $PERF_APPS; do
    app_command "$app"
    note "== $app: ${APP_CMD[*]}"
    case $MODE in
        full) full "$app" ;;
        *) smoke "$app" ;;
    esac
done

rm -rf /perfiles/*
printf '%s\n' "$status" >"$OUT/inner.status"
swaymsg exit >/dev/null 2>&1
