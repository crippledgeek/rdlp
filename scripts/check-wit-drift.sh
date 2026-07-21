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

# Pin the C locale: GNU grep's manual says range expressions like the `[a-z_]`
# classes used below are UNSPECIFIED outside the C locale -- they "might fail to
# match any character". A correctness fix, NOT a speed one (measured: no
# difference). Full quote and rationale in #621.
export LC_ALL=C

# Anchor to the repo root: every path below is relative, so without this the
# gate scans NOTHING and reports success when run from any other directory --
# the same fail-open class as the missing-tool guard. `|| exit 2` distinguishes
# "cannot run" from "gate failed" (exit 1). See #621.
cd "$(git rev-parse --show-toplevel)" || exit 2

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

# Count with a glob rather than `ls | wc -l` (SC2012): `ls` output is not a
# reliable list for programmatic use, and this is two fewer processes.
# `nullglob` so an empty directory counts 0 instead of matching the literal
# pattern as one entry. Like `ls`, a bare `*` skips dotfiles, so the count is
# unchanged.
shopt -s nullglob
host_files=("$HOST_DIR"/*)
shopt -u nullglob

echo "WIT vendor parity OK (${#host_files[@]} files × ${#VENDORED_DIRS[@]} vendored copies match)"
