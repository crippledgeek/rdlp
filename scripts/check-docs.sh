#!/usr/bin/env bash
# CI gate: `cargo doc` must build the whole workspace without rustdoc warnings.
#
# No other gate runs `cargo doc`, so a broken intra-doc link is invisible to
# `scripts/check-all.sh`, the pre-PR gate, and review. One survived a full
# green gate and two rounds of code review during #658, and was found only by
# an agent going looking for stale citations by hand (#661).
#
# Usage: scripts/check-docs.sh [--self-test]

set -euo pipefail

# Pin the C locale: this script has no locale-sensitive matching today, but
# every sibling gate does it as standing practice (#621) so a future `grep`
# added here inherits the same correctness guarantee rather than silently not.
export LC_ALL=C

# Anchor to the repo root: `--workspace` below is relative to the cwd cargo is
# invoked from, so running from anywhere else would build the wrong (or no)
# workspace. `|| exit 2` distinguishes "cannot run" from "gate failed" (#621).
cd "$(git rev-parse --show-toplevel)" || exit 2

command -v cargo >/dev/null 2>&1 || {
    echo "ERROR: cargo not found — cannot run this gate."
    exit 2
}

# THE check. Both the real run and the canary go through this one function, so
# the canary proves *this gate* can fail rather than merely that `cargo doc`
# can. `-D warnings` promotes every rustdoc warning (including the
# `rustdoc::broken_intra_doc_links` / `rustdoc::invalid_html_tags` lint group)
# to a build error, matching the exit-code contract cargo already has for
# `cargo build`/`cargo check`.
run_check() {
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
}

case "${1:-}" in
    --self-test)
        # Canary: prove a rustdoc invocation that silently ignored its input,
        # or a `run_check` edited into a no-op, cannot pass forever. The
        # canary crate lives outside the workspace (its own Cargo.toml) so it
        # never touches the shared CARGO_TARGET_DIR or Cargo.lock.
        tmp=$(mktemp -d) || exit 2
        trap 'rm -rf "$tmp"' EXIT
        mkdir -p "$tmp/src"
        cat > "$tmp/Cargo.toml" <<'EOF'
[package]
name = "check-docs-selftest"
version = "0.0.0"
edition = "2024"
EOF
        cat > "$tmp/src/lib.rs" <<'EOF'
//! Deliberately broken intra-doc link: [`DoesNotExist`] cannot resolve.
pub fn f() {}
EOF
        if (cd "$tmp" && CARGO_TARGET_DIR="$tmp/target" RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --offline >/dev/null 2>&1); then
            echo "SELF-TEST FAILED: the check accepted a deliberately broken intra-doc link"
            exit 1
        fi
        cat > "$tmp/src/lib.rs" <<'EOF'
//! No broken links here.
pub fn f() {}
EOF
        if ! (cd "$tmp" && CARGO_TARGET_DIR="$tmp/target" RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --offline >/dev/null 2>&1); then
            echo "SELF-TEST FAILED: the check rejected conformant input"
            exit 1
        fi
        echo "SELF-TEST OK"
        exit 0
        ;;
    "") ;;
    *)
        # A typo'd flag must not fall through to a normal scan and exit 0 —
        # that reads as a passed canary.
        echo "ERROR: unknown argument '$1'"
        echo "Usage: scripts/check-docs.sh [--self-test]"
        exit 2
        ;;
esac

if ! run_check; then
    echo ""
    echo "ERROR: cargo doc reported rustdoc warnings (promoted to errors via"
    echo "       RUSTDOCFLAGS=\"-D warnings\")."
    echo "       Re-run: RUSTDOCFLAGS=\"-D warnings\" cargo doc --no-deps --workspace"
    echo "       Fix each warning at its cause — backticks instead of a link for"
    echo "       an item that isn't public, a correct intra-doc path for an"
    echo "       unresolved link, backticks around bare generics that rustdoc"
    echo "       parses as HTML tags. Never #[allow(rustdoc::...)] to silence it."
    exit 1
fi

exit 0
