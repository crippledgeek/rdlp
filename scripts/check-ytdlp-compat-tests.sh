#!/usr/bin/env bash
# Run the yt-dlp compat shim's Python test suite as part of the pre-PR gate.
#
# Why: the shim carries the Python half of contracts whose other half is in
# Rust -- `clean_html` and `og_search_property` each exist twice, once per
# runtime, because a WASM plugin reaches the host over WIT while a Python
# plugin calls the shim directly. Those pairs MUST stay behaviourally
# identical, and the only thing pinning the Python side is its pytest suite.
#
# Until this gate existed, `cargo test` covered the Rust half and nothing ran
# the Python half, so a drift bug would surface only in a plugin at runtime.
# That is not hypothetical: the entity-decoding change (#698) had to be
# applied twice to each pair, and the first pass changed only the Rust side
# in both cases -- with a green Python test asserting the OLD contract.
#
# Fix when this fails: read the failure, then fix the shim or the test. If the
# Rust half of a pair changed, the Python half almost certainly needs the same
# change -- see the reciprocal comments at each implementation.
#
# Run from the repository root.

set -euo pipefail

export LC_ALL=C

# Anchor to the repo root so relative paths resolve; `|| exit 2` distinguishes
# "cannot run" (2) from "gate failed" (1), per the convention in #621.
cd "$(git rev-parse --show-toplevel)" || exit 2

SHIM_DIR="tools/ytdlp-compat"

if [ ! -d "$SHIM_DIR" ]; then
    echo "error: shim directory not found at $SHIM_DIR" >&2
    exit 2
fi

if ! command -v uv > /dev/null 2>&1; then
    echo "error: uv not found; cannot run the shim test suite" >&2
    exit 2
fi

# `--extra dev` pulls pytest; without it uv resolves the runtime deps only and
# fails to spawn pytest, which would otherwise read as a gate failure.
if ! (cd "$SHIM_DIR" && uv run --extra dev pytest -q); then
    echo "check-ytdlp-compat-tests: shim test suite FAILED" >&2
    exit 1
fi

echo "check-ytdlp-compat-tests: OK"
