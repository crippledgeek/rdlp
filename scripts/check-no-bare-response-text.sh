#!/usr/bin/env bash
# check-no-bare-response-text.sh — CI gate: verify every HTTP response body read in
# rdlp-extractor goes through the byte-capped `fetch_capped_text` helper
# (crates/rdlp-extractor/src/base/common/mod.rs), not a bare `.text().await`.
#
# Exit 0 = PASS (no offenders found).
# Exit 1 = FAIL (an uncapped response body read remains).
#
# Usage: bash scripts/check-no-bare-response-text.sh
#
# Rationale (#438): a bare `wreq::Response::text().await` buffers the entire body
# before any size check runs — an adversarial/compromised CDN can stream an
# unbounded payload and OOM the host. `fetch_capped_text` streams via
# `bytes_stream()` and aborts once `MAX_WEBPAGE_BYTES` is exceeded. This gate
# prevents a future call site from reintroducing the uncapped read.
#
# Detection is MULTILINE-aware (`rg -U`), like check-url-redaction.sh: a `.text()`
# followed by `.await` across a rustfmt-split chain
#     .text()
#     .await
# is caught exactly as the single-line `.text().await` form is — a split chain
# cannot evade the gate. Each match is reduced to its `.text()` anchor line.
#
# Scope: only `.text()` immediately followed by `.await` (the async HTTP-body
# read). `scraper`'s synchronous `element.text().collect()` has no `.await` and
# never matches.
#
# Escape hatch: a genuinely-safe raw read (e.g. a mockito loopback body inside a
# `#[cfg(test)]` module) carries an inline `// uncapped-ok: <reason>` marker on
# its `.text()` line, which this gate skips. Production reads MUST NOT use it —
# route them through `crate::base::common::fetch_capped_text(response, url)`.

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

# The guard below is deliberately NOT covered by a self-test canary, unlike the
# fixture-driven canaries in check-merge-exhaustive.sh and its siblings. Those
# feed a pure matcher a synthetic violating fixture -- input in, verdict out, no
# environment involved. A tool-presence guard has no fixture: proving it fires
# means re-executing this script under a manipulated PATH, and that harness
# produced four defects across four review rounds (it passed vacuously; it
# exited 127 on external binaries missing from its own stub PATH; it inherited
# the caller's cwd; it conflated cannot-run with failed) while the five lines
# below were never once wrong. Same call the kubernetes hack/ scripts make for
# their own tool-presence checks.
#
# ("self-test" is spelled without its dashes above on purpose: check-all.sh
# discovers canaries by grepping for that literal, and a script that merely
# MENTIONS it is reported FAILED for not implementing one.)
#
# The guard stays verifiable on demand. Seed a stub PATH with the binaries this
# script needs EXCEPT rg -- bash included, or the pipeline below dies 127 before
# reaching anything, which is the very trap that killed the canary:
#
#   stub=$(mktemp -d)
#   for t in bash git grep sed; do ln -s "$(command -v "$t")" "$stub/$t"; done
#   PATH="$stub" /bin/bash scripts/check-no-bare-response-text.sh            # expect: exit 2
#   grep -v '^require_tool rg$' scripts/check-no-bare-response-text.sh \
#       | PATH="$stub" /bin/bash -s                              # expect: PASS, exit 0
#
# The second form printing PASS is the silent false pass this guard prevents.
# --- required external tools -------------------------------------------------
# Verified failure mode (2026-07-21): with `rg` absent, `hits=$(rg ... || true)`
# swallowed rg's 127 and this gate printed PASS while checking nothing. A
# security gate that reports green when it cannot run is worse than no gate.
#
# Exit 2, not 1, so "tool missing" is distinguishable from "gate failed" by any
# caller that reads the status.
require_tool() {
    command -v "$1" >/dev/null 2>&1 && return 0
    printf 'error: %s: required tool %s not found in PATH\n' "${0##*/}" "$1" >&2
    printf '       This gate cannot run, and will NOT report a PASS it did not verify.\n' >&2
    exit 2
}
require_tool rg

TARGET="crates/rdlp-extractor/src"

# 1. `rg -U --pcre2` matches `.text()` + `.await` across optional whitespace/newlines.
# 2. Reduce to the `.text()` anchor line (drops the trailing `.await` line of a
#    multiline match).
# 3. Drop pure comment lines (the helper's own docstring references the pattern).
# 4. Drop lines carrying the explicit `uncapped-ok:` allow-marker.
hits=$(
    rg -Un --pcre2 '\.text\(\)\s*\.await' "$TARGET" 2>/dev/null \
        | rg '\.text\(\)' \
        | grep -vE ':[0-9]+:[[:space:]]*//' \
        | grep -v 'uncapped-ok:' \
        || true
)

if [[ -n "$hits" ]]; then
    echo "FAIL — bare (uncapped) response body read(s) found:"
    printf '%s\n' "  ${hits//$'\n'/$'\n'  }"
    echo ""
    echo "FIX: route the read through crate::base::common::fetch_capped_text(response, url)."
    echo "     For a genuinely-safe test-only loopback read, add an inline"
    echo "     '// uncapped-ok: <reason>' marker on the .text() line."
    exit 1
fi

echo "PASS — no bare .text().await response reads found in $TARGET"
exit 0
