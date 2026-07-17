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
    echo "$hits" | sed 's/^/  /'
    echo ""
    echo "FIX: route the read through crate::base::common::fetch_capped_text(response, url)."
    echo "     For a genuinely-safe test-only loopback read, add an inline"
    echo "     '// uncapped-ok: <reason>' marker on the .text() line."
    exit 1
fi

echo "PASS — no bare .text().await response reads found in $TARGET"
exit 0
