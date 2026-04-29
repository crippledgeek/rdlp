#!/usr/bin/env bash
# Verify the example-extractor plugin's vendored WIT files are byte-identical
# to the host crate's authoritative WIT contract.
#
# Why: `cargo-component` requires a plugin to vendor a copy of its host's WIT
# under `wit/deps/<host>/`. Without this guard a host WIT bump that forgets
# to refresh the example creates a plugin built against an older contract —
# silently passes type-check but the contract lock is broken.
#
# Fix when this fails:
#   cp crates/rdlp-plugin/wit/*.wit \
#      examples/plugins/example-extractor/wit/deps/rdlp-plugin/
#   git add examples/plugins/example-extractor/wit/deps/rdlp-plugin/
#
# Run from the repository root.

set -euo pipefail

HOST_DIR="crates/rdlp-plugin/wit"
VENDORED_DIRS=(
    "examples/plugins/example-extractor/wit/deps/rdlp-plugin"
    "examples/plugins/ytdlp-hello-world/wit/deps/rdlp-plugin"
)

if [ ! -d "$HOST_DIR" ]; then
    echo "error: host WIT directory not found at $HOST_DIR" >&2
    exit 2
fi

for VENDORED_DIR in "${VENDORED_DIRS[@]}"; do
    if [ ! -d "$VENDORED_DIR" ]; then
        echo "error: vendored WIT directory not found at $VENDORED_DIR" >&2
        exit 2
    fi
    if ! diff -r "$HOST_DIR" "$VENDORED_DIR"; then
        cat <<EOF >&2

ERROR: WIT vendor drift detected.

The host WIT under $HOST_DIR has diverged from the vendored copy at
$VENDORED_DIR.

To fix:
    cp $HOST_DIR/*.wit $VENDORED_DIR/
    git add $VENDORED_DIR

Then re-run this check.
EOF
        exit 1
    fi
done

echo "WIT vendor parity OK ($(ls "$HOST_DIR" | wc -l) files × ${#VENDORED_DIRS[@]} vendored copies match)"
