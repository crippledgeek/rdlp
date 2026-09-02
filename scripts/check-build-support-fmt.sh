#!/usr/bin/env bash
# CI guard: format-check the `include!`d build-support sources that
# `cargo fmt` cannot see.
#
# rustfmt formats compilation targets and does not descend into files pulled in
# by `include!`, so `crates/*/build_support/*.rs` is invisible to both
# `cargo fmt` and `cargo fmt --check`. Measured on 2026-09-02: a non-conformant
# line sat in `crates/rdlp-ffmpeg/build_support/pkgconfig_intent.rs` while
# `cargo fmt --check` reported the tree clean (#655).
#
# Usage: scripts/check-build-support-fmt.sh [--self-test]

set -euo pipefail

export LC_ALL=C

# Anchor to the repo root: every path below is relative, so without this the
# gate scans NOTHING and reports success from any other directory. `|| exit 2`
# distinguishes "cannot run" from "gate failed" (exit 1). See #621.
cd "$(git rev-parse --show-toplevel)" || exit 2

command -v rustfmt >/dev/null 2>&1 || {
    echo "ERROR: rustfmt not found — cannot run this gate."
    exit 2
}

# rustfmt has no manifest to read here, because these files are not compilation
# targets, so the edition is passed explicitly and must track the workspace.
# Asserted rather than parsed: on an edition bump this fails loudly instead of
# silently format-checking under the old one and staying green on a file
# rustfmt would now reformat.
EDITION=2024
grep -q "^edition = \"$EDITION\"\$" Cargo.toml || {
    echo "ERROR: EDITION=$EDITION no longer matches the workspace Cargo.toml."
    echo "       Update this gate to match, or it checks under the wrong edition."
    exit 2
}

# THE check. Both the real run and the canary go through this one function, so
# the canary proves *this gate* can fail rather than merely that rustfmt can.
run_check() {
    rustfmt --edition "$EDITION" --check "$@"
}

case "${1:-}" in
    --self-test)
        # Canary: prove a rustfmt invocation that silently ignored its input, or
        # a `run_check` edited into a no-op, cannot pass forever. (A *moved*
        # path is a different failure, caught by the zero-files guard below.)
        tmp=$(mktemp -d) || exit 2
        trap 'rm -rf "$tmp"' EXIT
        printf 'fn f()->u8{\n1\n}\n' > "$tmp/bad.rs"
        if run_check "$tmp/bad.rs" >/dev/null 2>&1; then
            echo "SELF-TEST FAILED: the check accepted deliberately misformatted input"
            exit 1
        fi
        printf 'fn f() -> u8 {\n    1\n}\n' > "$tmp/good.rs"
        if ! run_check "$tmp/good.rs" >/dev/null 2>&1; then
            echo "SELF-TEST FAILED: the check rejected conformant input"
            exit 1
        fi
        echo "SELF-TEST OK"
        exit 0
        ;;
    "") ;;
    *)
        # A typo'd flag must not fall through to a normal scan and exit 0 —
        # that reads as a passed canary. This gate's whole subject is checks
        # that pass without checking what you thought.
        echo "ERROR: unknown argument '$1'"
        echo "Usage: scripts/check-build-support-fmt.sh [--self-test]"
        exit 2
        ;;
esac

mapfile -t FILES < <(git ls-files 'crates/*/build_support/*.rs')

if [ "${#FILES[@]}" -eq 0 ]; then
    # Fail open loudly rather than silently: zero files means the layout moved
    # and this gate is now checking nothing.
    echo "ERROR: no crates/*/build_support/*.rs tracked — has the layout moved?"
    echo "       This gate would otherwise pass while checking nothing."
    exit 2
fi

if ! run_check "${FILES[@]}"; then
    echo ""
    echo "ERROR: build-support sources are not rustfmt-clean."
    echo "       \`cargo fmt\` does NOT reach these files (they are include!d,"
    echo "       not compilation targets). Format them explicitly:"
    echo "         rustfmt --edition $EDITION ${FILES[*]}"
    exit 1
fi

exit 0
