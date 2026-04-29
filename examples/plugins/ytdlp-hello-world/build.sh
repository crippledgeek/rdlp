#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ACTIVATE="../../../tools/ytdlp-compat/.venv/bin/activate"
if [ ! -f "$ACTIVATE" ]; then
    echo "ERROR: tools/ytdlp-compat/.venv not found. Run Task 1 Step 7 first." >&2
    exit 1
fi
# shellcheck disable=SC1090
source "$ACTIVATE"

# Clear any prior bindings — componentize-py-pin@0.17.2 errors out with
# "File exists (os error 17)" if the destination dir is already populated.
rm -rf extractor_plugin

# Generate Python bindings — --world-module is critical (default would name the
# package `wit_world`, breaking every import in extractor.py).
componentize-py -d wit -w hello-extractor \
    --world-module extractor_plugin bindings .

# componentize-py-pin@0.17.2 does NOT auto-create the output dir (#162); ensure it exists.
mkdir -p out

# Componentize the extractor.py into a .wasm.
#
# --stub-wasi replaces WASI 0.2 imports with trapping stubs so the resulting
# component links cleanly against rdlp-plugin's host (which intentionally wires
# only the 6 capability interfaces — see crate-level docs "Known Limitations").
# Plain Python that doesn't touch os/time/random at runtime won't trip the stubs.
componentize-py -d wit -w hello-extractor \
    --world-module extractor_plugin componentize --stub-wasi extractor -o out/plugin.wasm

echo "Built: $SCRIPT_DIR/out/plugin.wasm"
ls -lh out/plugin.wasm
