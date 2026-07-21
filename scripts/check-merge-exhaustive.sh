#!/usr/bin/env bash
# Asserts the #585 exhaustiveness canary still asks the compiler for
# completeness. rustc's E0027 help suggests adding `..`, which would silently
# disable the canary — this gate makes that suggestion un-takeable.
#
# Usage: scripts/check-merge-exhaustive.sh [--self-test]
#   --self-test: prove the gate still detects `..` in the canary, by scanning
#                a synthetic violating fixture. Runs in CI every time, so
#                "canary-verified" stays true as the tree evolves.
set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like the `[a-z_]`
# classes used below are UNSPECIFIED outside the C locale -- they "might fail to
# match any character". A correctness fix, NOT a speed one (measured: no
# difference). Full quote and rationale in #621.
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)" || exit 2

FILE="crates/rdlp-cli/src/config_tests.rs"
FN="every_config_field_is_classified"

check() {
    local file="$1"
    local body
    body=$(awk "/fn $FN\(\)/,/^}/" "$file")
    if [ -z "$body" ]; then
        echo "error: canary fn '$FN' not found in $file — was it renamed or deleted?" >&2
        return 1
    fi
    if grep -qE '\.\.' <<<"$body"; then
        echo "error: '..' found in the $FN canary." >&2
        echo "       That disables the compile-time completeness check (#585)." >&2
        echo "       Classify the new field into merge_fields! or the exceptions" >&2
        echo "       block instead, then name it in the pattern." >&2
        return 1
    fi
    return 0
}

if [ "${1:-}" = "--self-test" ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    printf 'fn %s() {\n    let Config { a: _, .. } = x;\n}\n' "$FN" >"$tmp/bad.rs"
    if check "$tmp/bad.rs" 2>/dev/null; then
        echo "SELF-TEST FAILED: gate accepted a canary containing '..'" >&2
        exit 1
    fi
    printf 'fn %s() {\n    let Config { a: _, b: _ } = x;\n}\n' "$FN" >"$tmp/good.rs"
    if ! check "$tmp/good.rs"; then
        echo "SELF-TEST FAILED: gate rejected a valid canary" >&2
        exit 1
    fi
    echo "SELF-TEST OK"
    exit 0
fi

check "$FILE"
echo "merge exhaustiveness canary intact"
