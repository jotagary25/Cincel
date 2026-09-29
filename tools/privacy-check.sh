#!/bin/sh
# Personal-data guard, run before publishing the repository
# (docs/specs/08-etapa6-cierre-1-0.md D20, §7.1, §7.4).
#
# What it looks for:
#   1. Identity values read LIVE from the environment / `git config` for
#      THIS checkout: the system username, the machine's hostname ("el
#      nombre del equipo") and the git author name/email configured for
#      this repository. None of these values is ever hard-coded here —
#      writing the real value into this script would be publishing it,
#      which is exactly what D20 says to avoid.
#   2. Generic patterns that do not depend on knowing the author's real
#      values ahead of time: `/home/<name>/` paths whose `<name>` is not
#      one of the repo's own generic example names, email addresses
#      outside the allowed placeholder/no-reply domains, and known
#      secret-token prefixes with a realistic length.
#
# A line that only mentions an identity value as part of a GitHub URL
# (`github.com/<value>/...`) is citing a public org or repo, not leaking
# the author's own account — this keeps a hostname that happens to match a
# well-known public GitHub org name (e.g. a Linux distribution's org) from
# being a false positive, without hard-coding that org name here.
#
# Where it looks: the working tree (tracked and untracked files, skipping
# whatever `.gitignore` already excludes) AND the full git history (every
# commit's added lines, plus every commit's author/committer identity),
# because a value removed from the tree can still live in an old commit.
# The actual scanning is done with `awk` (one pass per input, in C), not a
# shell loop per line: the tree alone is >150k lines and a per-line shell
# loop would take minutes.
#
# A finding in the WORKING TREE (or in an unpacked package, mode 1) is a
# bug: it makes this script exit 1, and must be fixed before publishing.
# A finding only in git HISTORY is reported the same way but never affects
# the exit code and this script never rewrites history — deciding whether
# to publish it as-is or start a fresh single-commit history is the
# author's call (`docs/publicacion.md`, "decidir la historia de git";
# `08-etapa6-cierre-1-0.md` §7.4: "sale con 0 sobre el árbol de trabajo...
# la historia se informa, la decide el autor").
#
# Usage:
#   tools/privacy-check.sh              # scans the working tree + history
#   tools/privacy-check.sh PATH...       # scans only these paths as plain
#                                         # files/directories (e.g. an
#                                         # unpacked package), no git history
#
# Exit 0: nothing found in the working tree / given paths (git history, if
# scanned, may still have printed informational findings). Exit 1: at
# least one working-tree/path finding, printed as
# "<location>: <what>: <masked value>" — a hit never repeats the value in
# full, only its first 3 and last 2 characters.
set -eu

REPO_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

# Two separate markers: PRIMARY_MARKER gates the exit code (tree, or the
# paths given on the command line); HIST_MARKER is informational only
# (git history findings never fail this script, see header).
PRIMARY_MARKER=$(mktemp)
HIST_MARKER=$(mktemp)
trap 'rm -f "$PRIMARY_MARKER" "$HIST_MARKER"' EXIT INT TERM

# Repo-local placeholder allowlists (never the author's real values, see
# header): example names this repo itself uses in `/home/<name>/...` test
# fixtures and docs, and email domains meant to be published as-is.
EXAMPLE_NAMES="user,username,u,ana,alguien,tester,demo,someone,person,jdoe,example,nobody"
ALLOWED_DOMAINS="example.com,example.invalid,users.noreply.github.com"

SYS_USER=$(id -un 2>/dev/null || true)
SYS_HOST=$(hostname 2>/dev/null || true)
GIT_NAME=$(git config user.name 2>/dev/null || true)
GIT_EMAIL=$(git config user.email 2>/dev/null || true)

# The awk program shared by every scan below. Reads whole files (tree
# mode, `loc_prefix` empty, uses FILENAME:FNR) or a stream already tagged
# with the commit it came from (history mode, `loc_prefix` set). Emits one
# tab-separated "category<TAB>location<TAB>description<TAB>raw-value" line
# per finding to stdout; masking happens in the shell, after this returns.
# shellcheck disable=SC2016 # single quotes are deliberate: this is the awk
# program's own source text, expanded by `-v name=value`, not by the shell.
AWK_SCAN='
function is_allowed_domain(d) { return (d in dm_ok) }
function is_allowed_name(n) { return (n in ex_ok) }
function looks_like_filename(d) {
    # A generic false-positive guard for source code: `foo@bar.rs`-style
    # text (mention parsing, doc comments, decorators) matches the email
    # regex but `bar.rs` is a Rust file, not a mail domain. Skipped only
    # when the whole domain is a single label ending in a common source
    # file extension (a real mail domain almost never looks like this).
    return d ~ /^[A-Za-z0-9_-]+\.(rs|py|ts|tsx|jsx|js|mjs|cjs|json|toml|md|txt|yml|yaml|sh|rb|go|c|h|hpp|cpp|cc|java|kt|swift|php|lock)$/
}
function is_github_mention(line, val,    needle) {
    needle = "github.com/" val
    return index(line, needle) > 0
}
BEGIN {
    n = split(example_names, ex, ",")
    for (i = 1; i <= n; i++) if (ex[i] != "") ex_ok[ex[i]] = 1
    m = split(allowed_domains, dm, ",")
    for (i = 1; i <= m; i++) if (dm[i] != "") dm_ok[dm[i]] = 1
    token_re = "sk-ant-[A-Za-z0-9_-]{24,}|ghp_[A-Za-z0-9]{30,}|gho_[A-Za-z0-9]{30,}|ghu_[A-Za-z0-9]{30,}|ghs_[A-Za-z0-9]{30,}|ghr_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{24,}|AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]{20,}|glpat-[A-Za-z0-9_-]{20,}|npm_[A-Za-z0-9]{30,}"
    email_re = "[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Za-z][A-Za-z]+"
    home_re = "/home/[A-Za-z0-9_.-]+/"
}
{
    line = $0
    loc = (loc_prefix != "") ? loc_prefix : (FILENAME ":" FNR)

    rest = line
    while (match(rest, token_re)) {
        val = substr(rest, RSTART, RLENGTH)
        printf "TOKEN\t%s\tknown secret-token prefix\t%s\n", loc, val
        rest = substr(rest, RSTART + RLENGTH)
    }

    rest = line
    while (match(rest, email_re)) {
        addr = substr(rest, RSTART, RLENGTH)
        rest = substr(rest, RSTART + RLENGTH)
        split(addr, parts, "@")
        if (!is_allowed_domain(parts[2]) && !looks_like_filename(parts[2]))
            printf "EMAIL\t%s\temail address outside the allowed placeholder domains\t%s\n", loc, addr
    }

    rest = line
    while (match(rest, home_re)) {
        seg = substr(rest, RSTART, RLENGTH)
        rest = substr(rest, RSTART + RLENGTH)
        name = seg
        sub(/^\/home\//, "", name)
        sub(/\/$/, "", name)
        if (!is_allowed_name(name))
            printf "HOMEPATH\t%s\t/home/<name>/ path with a name outside the repo example list\t%s\n", loc, seg
    }

    if (sys_user != "" && length(sys_user) >= 3 && index(line, sys_user) > 0 && !is_github_mention(line, sys_user))
        printf "IDENTITY\t%s\tsystem username found\t%s\n", loc, sys_user
    if (sys_host != "" && length(sys_host) >= 3 && index(line, sys_host) > 0 && !is_github_mention(line, sys_host))
        printf "IDENTITY\t%s\tmachine name found\t%s\n", loc, sys_host
    if (git_name != "" && length(git_name) >= 3 && index(line, git_name) > 0 && !is_github_mention(line, git_name))
        printf "IDENTITY\t%s\tgit author name found\t%s\n", loc, git_name
    if (git_email != "" && length(git_email) >= 3 && index(line, git_email) > 0 && !is_github_mention(line, git_email))
        printf "IDENTITY\t%s\tgit author email found\t%s\n", loc, git_email
}
'

# Prints at most the first 3 and last 2 characters of $1, the middle
# replaced by "...", so a hit is provable without repeating the secret.
mask() {
    value=$1
    len=$(printf '%s' "$value" | wc -c | tr -d '[:space:]')
    if [ "$len" -le 6 ]; then
        printf '***'
    else
        head=$(printf '%s' "$value" | cut -c1-3)
        tail=$(printf '%s' "$value" | rev | cut -c1-2 | rev)
        printf '%s...%s' "$head" "$tail"
    fi
}

# Reads AWK_SCAN's tab-separated output on stdin and turns each row into a
# report line via `mask`, marking the given marker file for every row seen.
# $1 = marker file to touch (PRIMARY_MARKER or HIST_MARKER).
emit_reports() {
    marker=$1
    while IFS="$(printf '\t')" read -r _category loc what value; do
        [ -n "$loc" ] || continue
        printf '%s: %s: %s\n' "$loc" "$what" "$(mask "$value")"
        printf 'x' >>"$marker"
    done
}

run_awk() {
    # $1 = loc_prefix ("" for tree mode); remaining args = files to scan,
    # or none to read the tagged stream from stdin.
    loc_prefix=$1
    shift
    awk \
        -v example_names="$EXAMPLE_NAMES" \
        -v allowed_domains="$ALLOWED_DOMAINS" \
        -v sys_user="$SYS_USER" \
        -v sys_host="$SYS_HOST" \
        -v git_name="$GIT_NAME" \
        -v git_email="$GIT_EMAIL" \
        -v loc_prefix="$loc_prefix" \
        "$AWK_SCAN" "$@"
}

is_text_like() {
    case "$1" in
        *.png|*.jpg|*.jpeg|*.gif|*.ico|*.woff|*.woff2|*.ttf) return 1 ;;
        *) return 0 ;;
    esac
}

# --- Mode 1: plain path scan (no git), for a package's unpacked content ---
if [ "$#" -gt 0 ]; then
    files=""
    for target in "$@"; do
        while IFS= read -r f; do
            is_text_like "$f" && files="$files $f"
        done <<EOF
$(find "$target" -type f 2>/dev/null)
EOF
    done
    # shellcheck disable=SC2086 # word-splitting is intentional: a file list
    if [ -n "$(printf '%s' "$files" | tr -d '[:space:]')" ]; then
        run_awk "" $files 2>/dev/null | emit_reports "$PRIMARY_MARKER"
    fi
    if [ -s "$PRIMARY_MARKER" ]; then
        exit 1
    fi
    echo "tools/privacy-check.sh: nothing found in: $*"
    exit 0
fi

cd "$REPO_ROOT"

# --- Mode 2 (default): working tree + full git history ---

echo "==> Scanning the working tree"
tree_files=""
while IFS= read -r f; do
    [ -f "$f" ] || continue
    is_text_like "$f" && tree_files="$tree_files $f"
done <<EOF
$(git -C "$REPO_ROOT" ls-files --cached --others --exclude-standard)
EOF
# shellcheck disable=SC2086 # word-splitting is intentional: a file list
if [ -n "$(printf '%s' "$tree_files" | tr -d '[:space:]')" ]; then
    run_awk "" $tree_files 2>/dev/null | emit_reports "$PRIMARY_MARKER"
fi
if [ -s "$PRIMARY_MARKER" ]; then
    echo "tools/privacy-check.sh: found in the working tree (see above); fix before publishing"
else
    echo "tools/privacy-check.sh: nothing found in the working tree"
fi

echo "==> Scanning git history (all reachable commits; informational, never rewritten here)"

# Commit author/committer identity, all reachable commits: checked as
# plain field comparisons, not through the awk scanner (there is no line
# of file content to attribute a location to; the location IS the commit).
git -C "$REPO_ROOT" log --all --format='%H%x09%an%x09%ae%x09%cn%x09%ce' 2>/dev/null |
    while IFS="$(printf '\t')" read -r hash _ ae _ ce; do
        short=$(printf '%s' "$hash" | cut -c1-10)
        domain=${ae#*@}
        if ! printf '%s\n' "$ALLOWED_DOMAINS" | tr ',' '\n' | grep -qx "$domain"; then
            printf '%s: %s: %s\n' "history:$short" "author email not on an allowed placeholder domain" "$(mask "$ae")"
            printf 'x' >>"$HIST_MARKER"
        fi
        cdomain=${ce#*@}
        if ! printf '%s\n' "$ALLOWED_DOMAINS" | tr ',' '\n' | grep -qx "$cdomain"; then
            printf '%s: %s: %s\n' "history:$short" "committer email not on an allowed placeholder domain" "$(mask "$ce")"
            printf 'x' >>"$HIST_MARKER"
        fi
    done

# Full added-line content of every reachable commit: same generic patterns
# and identity substrings as the working tree, one `awk` pass per commit
# (there are a handful of commits; each pass is fast). This re-reports a
# working-tree finding once per commit that introduced or kept it —
# expected and useful if the author later rewrites history.
git -C "$REPO_ROOT" rev-list --all 2>/dev/null |
    while IFS= read -r commit; do
        short=$(printf '%s' "$commit" | cut -c1-10)
        git -C "$REPO_ROOT" show --format= --no-color "$commit" 2>/dev/null |
            grep -E '^\+[^+]' |
            cut -c2- |
            run_awk "history:$short" 2>/dev/null |
            emit_reports "$HIST_MARKER"
    done

if [ -s "$HIST_MARKER" ]; then
    echo "tools/privacy-check.sh: found in git history (see above) — informational only,"
    echo "does not fail this script; decide what to do in docs/publicacion.md before publishing"
else
    echo "tools/privacy-check.sh: nothing found in git history"
fi

if [ -s "$PRIMARY_MARKER" ]; then
    exit 1
fi
exit 0
