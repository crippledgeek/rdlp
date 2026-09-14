#!/usr/bin/env bash
# Verify no production binary enables rdlp-extractor's `loopback-test-exemption`
# cargo feature.
#
# Why: that feature widens the SSRF gate's `#[cfg(test)]` mockito-loopback
# bypass in `base/common/manifest_url.rs` to non-test builds of
# rdlp-extractor, so a sibling crate's own test suite (rdlp-plugin, rdlp-api)
# can drive HLS expansion against a mockito server. It must never reach a
# binary a user actually runs -- that would reintroduce a loopback SSRF hole
# in production. This gate checks each of rdlp-cli/rdlp-desktop/rdlp-probe's
# non-dev dependency graph for the feature; a binary that does not depend on
# rdlp-extractor at all is reported as such, not silently skipped.
#
# Fix when this fails: move the feature dependency out of the offending
# binary's [dependencies] (it belongs only in a sibling crate's
# [dev-dependencies], if anywhere) and re-run this gate.
#
# Usage: scripts/check-loopback-feature-not-in-release.sh [--self-test]
#   --self-test: prove the grep this gate relies on actually matches the
#                real `cargo tree -e features` line shape, against canned
#                lines rather than a live feature flip (flipping the feature
#                on a scratch copy of the workspace is too heavy for a canary).

set -euo pipefail

# Pin the C locale: standing practice across every gate in this script family
# (#621) so `grep`'s byte-for-byte matching on cargo's output can't drift with
# the invoking shell's locale, even though this script's patterns are plain
# literals with no range expressions today.
export LC_ALL=C

# Anchor to the repo root: `cargo tree -p` below assumes it is invoked from
# inside the workspace. `|| exit 2` distinguishes "cannot run" from "gate
# failed" (exit 1).
cd "$(git rev-parse --show-toplevel)" || exit 2

FEATURE='loopback-test-exemption'

if [ "${1:-}" = "--self-test" ]; then
    # `cargo tree -e features` prints one feature per line as
    # `<crate> feature "<name>"` (verified against a real
    # `cargo tree -p rdlp-cli -e features,no-dev -i rdlp-extractor` run).
    # Assert the gate's grep both fires on a real positive line AND stays
    # silent on a real negative one (the default-feature line every binary
    # actually prints today) -- a matcher broad enough to fire on "default"
    # too would pass this gate on every binary vacuously.
    if ! echo 'rdlp-extractor feature "loopback-test-exemption"' | grep -q -- "$FEATURE"; then
        echo "SELF-TEST FAILED: the gate's matcher no longer fires on a known feature line."
        exit 1
    fi
    if echo 'rdlp-extractor feature "default"' | grep -q -- "$FEATURE"; then
        echo "SELF-TEST FAILED: the gate's matcher fires on an unrelated feature line."
        exit 1
    fi
    echo "SELF-TEST OK: the gate's matcher fires on the real feature line and only that line."
    exit 0
fi

for bin in rdlp-cli rdlp-desktop rdlp-probe; do
    # A `-p` that names a non-member silently falls through to cargo's
    # default query and reports on the wrong package with exit 0 -- confirm
    # the binary actually exists in this workspace first, or a typo'd name
    # here would report "ok" having checked nothing.
    if ! cargo pkgid -p "$bin" >/dev/null 2>&1; then
        echo "error: '$bin' is not a workspace member -- cannot check its dependency graph" >&2
        exit 2
    fi

    # `-e features,no-dev`: the property this gate proves is about a
    # RELEASE build's dependency graph, so dev-dependencies (where the
    # feature is meant to be enabled, in a sibling crate's own test suite)
    # are excluded on purpose. `2>&1` (not `2>/dev/null`): `-i rdlp-extractor`
    # fails with a non-zero exit and an stderr message when `$bin` does not
    # depend on rdlp-extractor at all, and discarding stderr would make that
    # look identical to "checked and found nothing" -- exactly the fail-open
    # this gate exists to close.
    out=$(cargo tree -p "$bin" -e features,no-dev -i rdlp-extractor 2>&1) && rc=0 || rc=$?
    if [ "$rc" -ne 0 ]; then
        if printf '%s' "$out" | grep -q 'did not match any packages'; then
            echo "note: $bin does not depend on rdlp-extractor -- nothing to check"
            continue
        fi
        echo "error: 'cargo tree -p $bin -e features,no-dev -i rdlp-extractor' failed:" >&2
        printf '%s\n' "$out" >&2
        exit 2
    fi

    if printf '%s' "$out" | grep -q -- "$FEATURE"; then
        echo "error: $bin enables rdlp-extractor/$FEATURE in a non-test build" >&2
        exit 1
    fi
done

echo "ok: loopback-test-exemption is not enabled by any binary"
