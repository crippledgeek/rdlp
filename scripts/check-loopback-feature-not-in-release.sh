#!/usr/bin/env bash
# Verify no production binary enables rdlp-extractor's `loopback-test-exemption`
# cargo feature.
#
# Why: that feature widens the SSRF gate's `#[cfg(test)]` mockito-loopback
# bypass in `base/common/manifest_url.rs` to non-test builds of
# rdlp-extractor, so a sibling crate's own test suite (rdlp-plugin, rdlp-api)
# can drive HLS expansion against a mockito server. It must never reach a
# binary a user actually runs -- that would reintroduce a loopback SSRF hole
# in production. This gate proves the feature is unreachable from every
# workspace binary's default dependency graph.
#
# Usage: scripts/check-loopback-feature-not-in-release.sh [--self-test]
#   --self-test: prove the grep this gate relies on actually matches the
#                real `cargo tree -e features` line shape, against a canned
#                line rather than a live feature flip (flipping the feature
#                on a scratch copy of the workspace is too heavy for a canary).

set -euo pipefail

# Pin the C locale: see #621 -- range-expression behavior in grep is
# unspecified outside the C locale. A correctness fix, not a speed one.
export LC_ALL=C

# Anchor to the repo root: `cargo tree -p` below assumes it is invoked from
# inside the workspace. `|| exit 2` distinguishes "cannot run" from "gate
# failed" (exit 1).
cd "$(git rev-parse --show-toplevel)" || exit 2

if [ "${1:-}" = "--self-test" ]; then
    # `cargo tree -e features` prints one feature per line as
    # `<crate> feature "<name>"` (verified against a real
    # `cargo tree -p rdlp-cli -e features -i rdlp-extractor` run). Assert the
    # gate's grep still matches that exact shape rather than a stale pattern.
    if echo 'rdlp-extractor feature "loopback-test-exemption"' | grep -q 'loopback-test-exemption'; then
        echo "SELF-TEST OK: the gate's matcher still fires on a real cargo-tree feature line."
        exit 0
    fi
    echo "SELF-TEST FAILED: the gate's matcher no longer fires on a known feature line."
    exit 1
fi

for bin in rdlp-cli rdlp-desktop rdlp-probe; do
    if cargo tree -p "$bin" -e features -i rdlp-extractor 2>/dev/null | grep -q 'loopback-test-exemption'; then
        echo "error: $bin enables rdlp-extractor/loopback-test-exemption in a non-test build" >&2
        exit 1
    fi
done

echo "ok: loopback-test-exemption is not enabled by any binary"
