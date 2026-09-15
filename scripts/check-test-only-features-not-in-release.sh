#!/usr/bin/env bash
# Verify no production binary enables a test-only cargo feature in its
# non-dev dependency graph.
#
# Why: some crates gate test-support code that must exist OUTSIDE
# `#[cfg(test)]` (because a SIBLING crate's own integration test links the
# lib as a regular external dependency and cannot see `cfg(test)` items)
# behind a cargo feature instead, so the extra dependencies that code needs
# (e.g. `tempfile`, `temp-env`) never reach a release binary. Two such
# features exist today:
#
#   - rdlp-extractor's `loopback-test-exemption` — widens the `#[cfg(test)]`
#     mockito-loopback bypass in `base/common/manifest_url.rs` to non-test
#     builds of that crate, so rdlp-plugin/rdlp-api's own tests can drive
#     HLS expansion against a mockito server. Reaching a production binary
#     would reintroduce a loopback SSRF hole.
#   - rdlp-plugin's `test-support` — gates `pub mod test_support` (the
#     signer/identity/config-dir-isolation helpers rdlp-plugin's and
#     rdlp-api's own tests share). Reaching a production binary would link
#     `tempfile`/`temp-env` into every user's `rdlp`/`rdlp-desktop` binary
#     for no runtime purpose.
#
# Both are meant to be enabled ONLY as a dev-dependency (or a dev-dependency
# feature unification, or a self-referential dev-dependency) in a sibling
# crate's own test suite. This gate derives the workspace's binary crates
# from `cargo metadata` (never a hardcoded list -- the same drift class
# CLAUDE.md calls out for check-all.sh: a new binary crate added later must
# be checked automatically, not silently skipped) and checks each one's
# non-dev dependency graph for every registered feature; a binary that does
# not depend on the gated crate at all is reported as such, not silently
# skipped.
#
# Fix when this fails: move the feature dependency out of the offending
# binary's [dependencies] (it belongs only in a sibling crate's
# [dev-dependencies], if anywhere) and re-run this gate.
#
# Adding a new test-only feature: append a "<crate>:<feature>" entry to the
# CHECKS array below.
#
# Usage: scripts/check-test-only-features-not-in-release.sh [--self-test]
#   --self-test: prove the grep this gate relies on actually matches the
#                real `cargo tree -e features` line shape, against canned
#                lines rather than a live feature flip (flipping a feature
#                on a scratch copy of the workspace is too heavy for a canary);
#                also prove the binary-list derivation itself is non-empty and
#                includes a binary known to exist on this tree, so a `jq`
#                filter that silently stopped matching anything is caught here
#                rather than by every binary quietly going unchecked. Runs
#                the self-test once per registered CHECKS entry.

set -euo pipefail

# Pin the C locale: standing practice across every gate in this script family
# (#621) so `grep`'s byte-for-byte matching on cargo's output can't drift with
# the invoking shell's locale, even though this script's patterns are plain
# literals with no range expressions today.
export LC_ALL=C

# Anchor to the repo root: `cargo tree -p` / `cargo metadata` below assume
# they are invoked from inside the workspace. `|| exit 2` distinguishes
# "cannot run" from "gate failed" (exit 1).
cd "$(git rev-parse --show-toplevel)" || exit 2

command -v cargo >/dev/null 2>&1 || { echo "ERROR: cargo not found -- cannot run this gate." >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "ERROR: jq not found -- cannot run this gate." >&2; exit 2; }

# Every "<crate>:<feature>" pair that must never reach a release binary's
# non-dev dependency graph. Append here, never hardcode a second script.
CHECKS=(
    "rdlp-extractor:loopback-test-exemption"
    "rdlp-plugin:test-support"
)

# Whether `cargo tree -e features` output enables `feature` on `crate`: the
# `<crate> feature "<name>"` token it prints, preceded by the tree-drawing
# prefix (`├── `, `└── `, `│   `) and optionally followed by
# ` (command-line)` -- measured on real `cargo tree -p rdlp-plugin -e
# features -i rdlp-extractor` output, e.g.
# `└── rdlp-extractor feature "loopback-test-exemption"`. The token is
# whitespace-bounded on both sides so a line that merely mentions the
# feature name inside a path or a comment cannot match, and so a line
# starting with a tree prefix (which a `^` anchor would reject, failing
# this gate OPEN) does.
feature_enabled() {
    local crate="$1" feature="$2" output="$3"
    grep -qE "(^|[[:space:]])${crate} feature \"${feature}\"([[:space:]]|$)" <<<"$output"
}

# Every workspace package with at least one `bin` target -- the set of crates
# whose dependency graph is a real release build a user runs, as opposed to a
# library crate nothing ships standalone. Read from `cargo metadata` rather
# than named in this script, so a new binary crate is picked up the moment it
# exists instead of needing this file edited.
list_workspace_binaries() {
    cargo metadata --no-deps --format-version 1 \
        | jq -r '.packages[] | select(any(.targets[]; .kind[] == "bin")) | .name'
}

if [ "${1:-}" = "--self-test" ]; then
    for check in "${CHECKS[@]}"; do
        crate="${check%%:*}"
        feature="${check#*:}"

        # Assert the gate's matcher both fires on the real positive line
        # shapes AND stays silent on a real negative one (the default-feature
        # line every binary actually prints today) -- a matcher broad enough
        # to fire on "default" too would pass this gate on every binary
        # vacuously, and one too narrow for the tree prefix would fail it
        # open.
        # The real line shapes: tree-prefixed, and tree-prefixed with the
        # `(command-line)` suffix cargo adds to a feature named on the
        # command line.
        if ! feature_enabled "$crate" "$feature" "└── ${crate} feature \"${feature}\""; then
            echo "SELF-TEST FAILED ($check): the gate's matcher no longer fires on a known feature line."
            exit 1
        fi
        if ! feature_enabled "$crate" "$feature" "│       ├── ${crate} feature \"${feature}\" (command-line)"; then
            echo "SELF-TEST FAILED ($check): the gate's matcher no longer fires on a command-line feature line."
            exit 1
        fi
        if feature_enabled "$crate" "$feature" "├── ${crate} feature \"default\""; then
            echo "SELF-TEST FAILED ($check): the gate's matcher fires on an unrelated feature line."
            exit 1
        fi
        # A comment or a dependency line that merely MENTIONS the feature
        # name is not the feature being enabled; only the exact
        # `<crate> feature "<name>"` line is.
        if feature_enabled "$crate" "$feature" "some-other-crate v1.0.0 (${feature} mentioned in a path)"; then
            echo "SELF-TEST FAILED ($check): the gate's matcher fires on a line that merely mentions the feature."
            exit 1
        fi
    done

    # The derivation itself must not be able to silently return nothing: a
    # `jq` filter that stopped matching (a metadata schema change, a typo'd
    # rewrite) would otherwise make the main loop iterate zero times and
    # print "ok" having checked no binary at all -- the same fail-open class
    # the per-binary `cargo tree` call guards against with its `2>&1` (see
    # below), now asserted for the list that feeds it.
    bins=$(list_workspace_binaries) || exit 2
    if [ -z "$bins" ]; then
        echo "SELF-TEST FAILED: the binary-crate derivation returned nothing."
        exit 1
    fi
    if ! printf '%s\n' "$bins" | grep -qx 'rdlp-cli'; then
        echo "SELF-TEST FAILED: the derivation did not include the known binary 'rdlp-cli'."
        exit 1
    fi

    echo "SELF-TEST OK: the matcher fires on the exact feature line and not on the default-feature or mention-only lines for every registered check, and the binary-crate derivation is non-empty and includes rdlp-cli."
    exit 0
fi

bins=$(list_workspace_binaries) || exit 2
if [ -z "$bins" ]; then
    echo "ERROR: cargo metadata + jq derived no workspace binary crates -- cannot run this gate." >&2
    exit 2
fi
mapfile -t bin_list <<< "$bins"

for bin in "${bin_list[@]}"; do
    # A `-p` that names a non-member silently falls through to cargo's
    # default query and reports on the wrong package with exit 0 -- confirm
    # the binary actually exists in this workspace first, or a typo'd name
    # here would report "ok" having checked nothing.
    if ! cargo pkgid -p "$bin" >/dev/null 2>&1; then
        echo "error: '$bin' is not a workspace member -- cannot check its dependency graph" >&2
        exit 2
    fi

    for check in "${CHECKS[@]}"; do
        crate="${check%%:*}"
        feature="${check#*:}"

        # `-e features,no-dev`: the property this gate proves is about a
        # RELEASE build's dependency graph, so dev-dependencies (where the
        # feature is meant to be enabled, in a sibling crate's own test
        # suite) are excluded on purpose. `2>&1` (not `2>/dev/null`): `-i
        # <crate>` fails with a non-zero exit and an stderr message when
        # `$bin` does not depend on the crate at all, and discarding stderr
        # would make that look identical to "checked and found nothing" --
        # exactly the fail-open this gate exists to close.
        out=$(cargo tree -p "$bin" -e features,no-dev -i "$crate" 2>&1) && rc=0 || rc=$?
        if [ "$rc" -ne 0 ]; then
            if printf '%s' "$out" | grep -q 'did not match any packages'; then
                echo "note: $bin does not depend on $crate -- nothing to check for $feature"
                continue
            fi
            echo "error: 'cargo tree -p $bin -e features,no-dev -i $crate' failed:" >&2
            printf '%s\n' "$out" >&2
            exit 2
        fi

        if feature_enabled "$crate" "$feature" "$out"; then
            echo "error: $bin enables $crate/$feature in a non-test build" >&2
            exit 1
        fi
    done
done

echo "ok: no test-only feature (${CHECKS[*]}) is enabled by any binary"
