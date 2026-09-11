#!/usr/bin/env bash
# Refreshes ui/vendor/ from esm.sh. Maintenance only: the vendored files are
# committed and the binary include_str!s them, so nothing in the build or at
# runtime touches the network. Run this to bump a version, then re-run
# `cargo test -p graph-server` (the closure test) and check the diff in.
#
# Why two phases per module: esm.sh answers a package URL with HTTP 200 and a
# stub body whose exports point at the real file by a root-relative path
# (`export * from "/react@18.3.1/es2022/react.mjs"`). There is no redirect for
# `curl -L` to follow, and served from localhost that path would escape the
# `/ui/` prefix and 404. The resolved path is also in the `x-esm-path`
# response header, so: probe for the header, fetch that, then rewrite every
# root-relative and `./`-relative specifier to the vendored sibling.
#
# Version-range URLs (`/scheduler@^0.23.2?target=es2022` inside react-dom)
# and relative ones (`./react.mjs` inside jsx-runtime) both go through the
# same probe, so one row per real file is all the table needs; a specifier
# that resolves to a file with no row aborts the run with the row to add.
#
# `external=react,react-dom` on xyflow is load-bearing: without it esm.sh
# inlines its own React and every hook in the viewer breaks on a second
# instance. For the same reason `react-dom` and `react-dom/client` share one
# react-dom.js rather than a `?bundle-deps` client build (which exports only
# createRoot/hydrateRoot, and xyflow needs createPortal from `react-dom`).
set -euo pipefail

ESM=https://esm.sh
cd "$(dirname "$0")/vendor"

REACT=18.3.1
SCHEDULER=0.23.2
XYFLOW=12.11.6
DAGRE=1.1.8
GRAPHLIB=2.2.4
HTM=3.1.1
HLJS=11.11.1
# The distinct values of EXT_LANG in ui/code.js.
LANGS="rust go typescript javascript python ini json markdown xml css bash yaml sql c cpp java ruby kotlin swift lua"

# vendored file  esm.sh request
rows() {
  cat <<EOF
react.js              /react@$REACT?target=es2022
react-jsx-runtime.js  /react@$REACT/jsx-runtime?target=es2022
react-dom.js          /react-dom@$REACT?target=es2022
react-dom-client.js   /react-dom@$REACT/client?target=es2022
scheduler.js          /scheduler@$SCHEDULER?target=es2022
xyflow-react.js       /@xyflow/react@$XYFLOW?bundle-deps&external=react,react-dom&target=es2022
xyflow-react.css      /@xyflow/react@$XYFLOW/dist/style.css
dagre.js              /@dagrejs/dagre@$DAGRE?bundle-deps&target=es2022
graphlib.js           /@dagrejs/graphlib@$GRAPHLIB/lib?target=es2022
graphlib-alg.js       /@dagrejs/graphlib@$GRAPHLIB/lib/alg?target=es2022
graphlib-json.js      /@dagrejs/graphlib@$GRAPHLIB/lib/json?target=es2022
htm.js                /htm@$HTM?bundle-deps&target=es2022
hljs-core.js          /highlight.js@$HLJS/lib/core?bundle-deps&target=es2022
EOF
  for lang in $LANGS; do
    echo "hljs/$lang            /highlight.js@$HLJS/lib/languages/$lang?bundle-deps&target=es2022"
  done
}

# The real esm.sh path behind a request URL: the x-esm-path header when the
# response is a stub, the request path itself (minus query) when it is not.
resolve() {
  local hdr
  hdr=$(curl -sSfI "$ESM$1" | tr -d '\r' | awk 'tolower($1)=="x-esm-path:"{print $2}')
  if [ -n "$hdr" ]; then echo "$hdr"; else echo "${1%%\?*}"; fi
}

MAP=$(mktemp)
trap 'rm -f "$MAP"' EXIT

echo "fetching"
mkdir -p hljs
while read -r file req; do
  real=$(resolve "$req")
  printf '  %-22s %s\n' "$file" "$real"
  curl -sSf "$ESM$real" -o "$file"
  echo "$real $file" >> "$MAP"
done < <(rows)

vendored_for() { awk -v p="$1" '$1==p{print $2}' "$MAP"; }

# Path from the directory of $1 to $2, both relative to vendor/.
relative() {
  local from=$1 to=$2 up=""
  from=${from%/*}
  [ "$from" = "$1" ] && { echo "./$to"; return; }
  up=$(printf '%s' "$from/" | sed 's|[^/]*/|../|g')
  echo "$up$to"
}

sed_escape() { printf '%s' "$1" | sed 's/[][\.*^$|]/\\&/g'; }

echo "rewriting specifiers"
while read -r real file; do
  case "$file" in *.css) continue ;; esac
  dir=${real%/*}
  specs=$(grep -oE '(from|import)[[:space:]]*"(/|\./|\.\./)[^"]*"' "$file" | sed -E 's/^(from|import)[[:space:]]*"//; s/"$//' | sort -u || true)
  for spec in $specs; do
    case "$spec" in
      /*) abs=$spec ;;
      ./*) abs="$dir/${spec#./}" ;;
      *) echo "unsupported relative specifier $spec in $file" >&2; exit 1 ;;
    esac
    target=$(vendored_for "${abs%%\?*}")
    # Not a real file yet (a version range, say): ask esm.sh what it means.
    [ -z "$target" ] && target=$(vendored_for "$(resolve "$abs")")
    if [ -z "$target" ]; then
      echo "$file imports $spec, which is not vendored; add a row for it" >&2
      exit 1
    fi
    rel=$(relative "$file" "$target")
    printf '  %-22s %s -> %s\n' "$file" "$spec" "$rel"
    sed "s|\"$(sed_escape "$spec")\"|\"$rel\"|g" "$file" > "$file.tmp" && mv "$file.tmp" "$file"
  done
  # The maps are not vendored; the browser would 404 on them with devtools open.
  grep -v '^//# sourceMappingURL=' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
done < "$MAP"

echo "checking closure"
bad=0
while read -r _ file; do
  case "$file" in
    *.css) if grep -qE 'url\(|@import' "$file"; then echo "  $file references other files" >&2; bad=1; fi ;;
    *) if grep -qE '(from|import)[[:space:]]*"(/|https?:)' "$file"; then echo "  $file still has an absolute specifier" >&2; bad=1; fi ;;
  esac
done < "$MAP"
[ "$bad" = 0 ] || exit 1
echo "ok: $(wc -l < "$MAP" | tr -d ' ') files, $(cat $(awk '{print $2}' "$MAP") | wc -c | tr -d ' ') bytes"
