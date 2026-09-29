#!/bin/sh
# Checks that every relative Markdown link (and image) in the repo's own
# Markdown files points to a file that actually exists
# (docs/specs/08-etapa6-cierre-1-0.md §7.4: "Todos los enlaces relativos
# del README y de docs/usuario apuntan a archivos que existen"). Runs the
# same check over every tracked `.md` file, not just those two, since it
# costs nothing extra and catches the same class of bug everywhere.
#
# What counts as a "relative" link: anything that is not `scheme://...`
# (http, https, mailto, etc.) and is not anchor-only (`#foo`). A link that
# contains the repo's own `<usuario>` placeholder
# (`docs/publicacion.md`, D22) is skipped: it is deliberately incomplete
# until the author fills it in. A trailing `#anchor` on an otherwise
# relative link is stripped before checking that the file exists (anchors
# inside the target file are not verified).
#
# Usage: tools/check-links.sh
# Exit 0: every relative link resolves. Exit 1: at least one broken link,
# printed as "<file>:<line>: broken link -> <target>".
set -eu

REPO_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPO_ROOT"

BROKEN_MARKER=$(mktemp)
trap 'rm -f "$BROKEN_MARKER"' EXIT INT TERM

# Prints "<file><TAB><line><TAB><target>" for every Markdown link/image
# found in $1.
extract_links() {
    file=$1
    awk -v file="$file" '
    {
        rest = $0
        while (match(rest, /\[[^]]*\]\([^()[:space:]]+\)/)) {
            whole = substr(rest, RSTART, RLENGTH)
            rest = substr(rest, RSTART + RLENGTH)
            paren_start = index(whole, "(")
            target = substr(whole, paren_start + 1, length(whole) - paren_start - 1)
            printf "%s\t%d\t%s\n", file, FNR, target
        }
    }' "$file"
}

git -C "$REPO_ROOT" ls-files '*.md' |
    while IFS= read -r mdfile; do
        extract_links "$mdfile"
    done |
    while IFS="$(printf '\t')" read -r file line target; do
        case "$target" in
            http://* | https://* | mailto:* | '#'* | *'<usuario>'*) continue ;;
        esac
        path=${target%%#*}
        [ -n "$path" ] || continue
        dir=$(dirname "$file")
        resolved="$dir/$path"
        if [ ! -e "$resolved" ]; then
            echo "$file:$line: broken link -> $target"
            printf 'x' >>"$BROKEN_MARKER"
        fi
    done

if [ -s "$BROKEN_MARKER" ]; then
    exit 1
fi
echo "tools/check-links.sh: every relative link resolves"
exit 0
