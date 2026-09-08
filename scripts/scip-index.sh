#!/usr/bin/env bash
# Regenerate a SCIP index for a project so scip-producer can consume it.
# `graph-server --scip-index` does the same thing automatically (into the
# system temp dir); this script is for fixtures and for keeping an index in
# a place of your choosing.
#
#   scripts/scip-index.sh <rust|ts|go> <project-dir> [out]
#
# `out` defaults to <project-dir>/index.scip. Indexers are expected on PATH:
#   rust  rust-analyzer      (rustup component add rust-analyzer)
#   ts    scip-typescript    (run via npx; a tsconfig.json must exist in <dir>)
#   go    scip-go            (go install github.com/scip-code/scip-go/cmd/scip-go@latest)
#
# The fixtures under crates/scip-producer/tests/fixtures/* commit their
# index.scip; tests never invoke an indexer. Re-run this after editing a
# fixture's sources. A Rust fixture nested inside this repo needs its own
# `[workspace]` table so cargo does not attach it to the root workspace.
set -euo pipefail

lang=${1:?usage: scip-index.sh <rust|ts|go> <dir> [out]}
dir=${2:?usage: scip-index.sh <rust|ts|go> <dir> [out]}
out=${3:-$dir/index.scip}
out=$(cd "$(dirname "$out")" && pwd)/$(basename "$out")

cd "$dir"
case "$lang" in
  rust)
    # Keep build artifacts out of the fixture tree.
    CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(mktemp -d)} rust-analyzer scip . --output "$out"
    ;;
  ts)
    npx --yes @sourcegraph/scip-typescript index --output "$out"
    ;;
  go)
    scip-go --output "$out"
    ;;
  *)
    echo "unknown language: $lang (expected rust, ts or go)" >&2
    exit 2
    ;;
esac
echo "wrote $out"
