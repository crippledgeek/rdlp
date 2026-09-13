#!/usr/bin/env bash
# CI guard: every workspace member must declare the MSRV, by inheriting the
# workspace `rust-version`.
#
# The workspace root declares `rust-version` under `[workspace.package]`, but
# that value reaches a member ONLY if the member writes
# `rust-version.workspace = true` (Cargo reference, "Inheriting a dependency
# from a workspace" / "rust-version"). Before #728 no member did, so no crate
# actually declared a floor: `cargo` enforced nothing, and clippy -- whose
# `msrv` "Defaults to the `rust-version` field in `Cargo.toml`" (clippy lint
# configuration docs) -- assumed the running toolchain and demanded APIs newer
# than the declared floor (`Duration::from_mins`, stable 1.95, against 1.88).
# The tree answered with thirteen `#[allow(clippy::duration_suboptimal_units)]`
# suppressions, each carrying an MSRV comment that drifted (six still said 1.85
# after the floor moved to 1.88).
#
# Two invariants, both cheap and textual:
#   1. Every member listed in the root `[workspace] members` inherits
#      `rust-version` -- so the declared floor applies per crate, and clippy's
#      MSRV gating follows it with no second copy of the number anywhere.
#   2. No `duration_suboptimal_units` suppression exists under crates/. It is
#      the canary symptom of invariant 1 regressing: the only reason to write
#      one is that clippy lost sight of the floor.
#
# Usage: scripts/check-msrv-declared.sh [--self-test]
#   --self-test: prove both matchers still fire, against a synthetic workspace
#                in a temp dir. Runs in CI every time via check-all.sh.

set -euo pipefail

# Pin the C locale: range expressions in the character classes below are
# unspecified outside it (GNU grep manual; rationale in #621).
export LC_ALL=C

# Anchor to the repo root: the member paths in the root manifest are relative.
cd "$(git rev-parse --show-toplevel)" || exit 2

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Members of the workspace rooted at $1 that do NOT inherit rust-version.
# Reads the `members = [...]` block of the root manifest rather than globbing
# crates/*, so a member outside crates/ (rdlp-desktop/src-tauri) is covered and
# a non-member directory is not. Shared by the real run and the self-test so
# the canary exercises the SAME matcher.
members_missing_inheritance() {
    local root="$1" member
    sed -n '/^members = \[/,/^\]/p' "$root/Cargo.toml" \
        | grep -oE '"[^"]+"' | tr -d '"' \
        | while IFS= read -r member; do
            if ! grep -qE '^rust-version\.workspace = true$' "$root/$member/Cargo.toml" 2>/dev/null; then
                printf '%s\n' "$member"
            fi
        done
}

# Files under $1 that suppress the canary lint.
suppression_sites() {
    grep -rlE 'duration_suboptimal_units' --include='*.rs' "$1" 2>/dev/null || true
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    mkdir -p "$tmp/crates/good/src" "$tmp/crates/bad/src"
    cat > "$tmp/Cargo.toml" <<'FIXTURE'
[workspace]
members = [
    "crates/good",
    "crates/bad",
]
[workspace.package]
rust-version = "1.88"
FIXTURE
    printf '[package]\nname = "good"\nrust-version.workspace = true\n' > "$tmp/crates/good/Cargo.toml"
    printf '[package]\nname = "bad"\nedition = "2024"\n' > "$tmp/crates/bad/Cargo.toml"
    printf '#![allow(clippy::duration_suboptimal_units)]\n' > "$tmp/crates/bad/src/lib.rs"
    if [ "$(members_missing_inheritance "$tmp")" = "crates/bad" ] \
        && [ -n "$(suppression_sites "$tmp/crates")" ]; then
        echo "SELF-TEST OK: both matchers still fire on a synthetic violation."
        exit 0
    fi
    echo "SELF-TEST FAILED: a matcher did NOT flag a known violation - it is broken."
    exit 1
fi

# Refuse to report OK having examined nothing.
member_count=$(sed -n '/^members = \[/,/^\]/p' Cargo.toml | grep -cE '"[^"]+"' || true)
if [ "$member_count" -eq 0 ]; then
    echo "ERROR: found no workspace members in Cargo.toml - refusing to report OK."
    exit 1
fi

status=0
missing=$(members_missing_inheritance .)
if [ -n "$missing" ]; then
    echo "ERROR: workspace members that do not inherit rust-version (add 'rust-version.workspace = true'):"
    printf '  %s\n' "$missing"
    status=1
fi
sites=$(suppression_sites crates)
if [ -n "$sites" ]; then
    echo "ERROR: duration_suboptimal_units suppressed - clippy has lost sight of the MSRV; fix the floor, do not allow the lint:"
    printf '  %s\n' "$sites"
    status=1
fi
if [ "$status" -eq 0 ]; then
    echo "OK: all $member_count workspace members inherit rust-version; no MSRV-lint suppressions."
fi
exit "$status"
