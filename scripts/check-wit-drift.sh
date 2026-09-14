#!/usr/bin/env bash
# Verify the example plugins' vendored WIT files are byte-identical to the
# host crate's authoritative WIT contract.
#
# Why: `cargo-component` requires a plugin to vendor a copy of its host's WIT
# under `wit/deps/<host>/`. Without this guard a host WIT bump that forgets
# to refresh the example creates a plugin built against an older contract --
# silently passes type-check but the contract lock is broken.
#
# Scope: ONLY `*.wit` files are the contract and are compared. The host
# directory also carries non-.wit policy docs beside the contract (e.g.
# COMPATIBILITY.md) that are never vendored -- an earlier version of this
# script ran `diff -r` over the whole directory, so adding such a doc made
# the gate fail on a file that was never part of what "vendored" means here.
# Each host `.wit` file must exist, byte-identical, in every vendored dir; a
# vendored `.wit` file with no host counterpart (a stale contract file a
# rename left behind) is drift too.
#
# Fix when this fails:
#   cp crates/rdlp-plugin/wit/*.wit \
#      examples/plugins/example-extractor/wit/deps/rdlp-plugin/
#   git add examples/plugins/example-extractor/wit/deps/rdlp-plugin/
#
# Usage: scripts/check-wit-drift.sh [--self-test]
#   --self-test: prove the comparison, in a scratch mktemp tree (never the
#                real directories), both (a) detects a modified byte in a
#                vendored .wit file and (b) does NOT flag a non-.wit file
#                that exists beside the host contract but is never vendored.
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

# Compare the `*.wit` files ONLY between $1 (host dir) and $2 (vendored dir).
# Prints one line per divergence to stdout and returns non-zero if any file
# is missing, byte-different, or a stale vendored-only leftover. Non-.wit
# files in either directory (COMPATIBILITY.md, a stray README) are never
# considered -- they are not part of the vendored contract.
compare_wit_dirs() {
    local host_dir="$1" vendored_dir="$2" ok=0 base f

    shopt -s nullglob
    for f in "$host_dir"/*.wit; do
        base=$(basename "$f")
        if [ ! -f "$vendored_dir/$base" ]; then
            echo "missing from $vendored_dir: $base"
            ok=1
            continue
        fi
        if ! diff -q "$f" "$vendored_dir/$base" >/dev/null; then
            echo "diverged: $base ($host_dir vs $vendored_dir)"
            ok=1
        fi
    done
    for f in "$vendored_dir"/*.wit; do
        base=$(basename "$f")
        if [ ! -f "$host_dir/$base" ]; then
            echo "stale vendored file with no host counterpart in $vendored_dir: $base"
            ok=1
        fi
    done
    shopt -u nullglob

    return "$ok"
}

if [ "${1:-}" = "--self-test" ]; then
    tmp=$(mktemp -d) || exit 2
    trap 'rm -rf "$tmp"' EXIT
    mkdir -p "$tmp/host" "$tmp/vendor"

    # A non-.wit policy doc beside the host contract, never vendored -- must
    # NOT be reported as drift (this is the exact shape that broke the old
    # whole-directory `diff -r`: adding COMPATIBILITY.md to the host dir made
    # the gate fail even though nothing about the .wit contract had changed).
    printf 'irrelevant policy text\n' > "$tmp/host/COMPATIBILITY.md"
    printf 'contract v1\n' > "$tmp/host/a.wit"
    printf 'contract v1\n' > "$tmp/vendor/a.wit"

    if ! compare_wit_dirs "$tmp/host" "$tmp/vendor" >/dev/null; then
        echo "SELF-TEST FAILED: a non-.wit file beside the host contract was wrongly flagged as drift."
        exit 1
    fi

    # Mutate one byte of the vendored .wit file -- must now be detected.
    printf 'contract v2\n' > "$tmp/vendor/a.wit"
    if compare_wit_dirs "$tmp/host" "$tmp/vendor" >/dev/null; then
        echo "SELF-TEST FAILED: a modified byte in a vendored .wit file was not detected."
        exit 1
    fi

    echo "SELF-TEST OK: a modified .wit byte is detected, and a non-.wit sibling of the host contract is not."
    exit 0
fi

HOST_DIR="crates/rdlp-plugin/wit"
VENDORED_DIRS=(
    "examples/plugins/example-extractor/wit/deps/rdlp-plugin"
    "examples/plugins/ytdlp-hello-world/wit/deps/rdlp-plugin"
)

if [ ! -d "$HOST_DIR" ]; then
    echo "error: host WIT directory not found at $HOST_DIR" >&2
    exit 2
fi

DRIFT=0
for VENDORED_DIR in "${VENDORED_DIRS[@]}"; do
    if [ ! -d "$VENDORED_DIR" ]; then
        echo "error: vendored WIT directory not found at $VENDORED_DIR" >&2
        exit 2
    fi
    if ! diff_output=$(compare_wit_dirs "$HOST_DIR" "$VENDORED_DIR"); then
        cat <<EOF >&2

ERROR: WIT vendor drift detected.

The host WIT contract files (*.wit) under $HOST_DIR have diverged from the
vendored copy at $VENDORED_DIR:
$diff_output

To fix:
    cp $HOST_DIR/*.wit $VENDORED_DIR/
    git add $VENDORED_DIR

Then re-run this check.
EOF
        DRIFT=1
    fi
done

if [ "$DRIFT" -eq 1 ]; then
    exit 1
fi

# Count with a glob rather than `ls | wc -l` (SC2012): `ls` output is not a
# reliable list for programmatic use, and this is two fewer processes.
# `nullglob` so an empty directory counts 0 instead of matching the literal
# pattern as one entry. Like `ls`, a bare `*` skips dotfiles, so the count is
# unchanged.
shopt -s nullglob
host_files=("$HOST_DIR"/*.wit)
shopt -u nullglob

echo "WIT vendor parity OK (${#host_files[@]} contract files × ${#VENDORED_DIRS[@]} vendored copies match)"
