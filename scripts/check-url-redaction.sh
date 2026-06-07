#!/usr/bin/env bash
# check-url-redaction.sh — CI gate: verify all operator-visible URL interpolations in
# rdlp-extractor are wrapped with rdlp_redact::RedactedUrl (or sanitize_for_logging).
#
# Exit 0 = PASS (no offenders found).
# Exit 1 = FAIL (raw URL interpolations remain).
#
# Usage: bash scripts/check-url-redaction.sh

set -euo pipefail

TARGET="crates/rdlp-extractor/src"
FAIL=0

# Helper: run rg, filter out already-compliant lines and test files, report hits.
# Args: <label> <rg-pattern>
check() {
    local label="$1"
    local pattern="$2"
    # grep -v filters: lines already wrapped, test modules, comment lines
    local hits
    hits=$(rg --type rust -n "$pattern" "$TARGET" 2>/dev/null \
        | grep -v 'RedactedUrl' \
        | grep -v 'sanitize_for_logging' \
        | grep -v '/tests/' \
        | grep -v '#\[cfg(test)\]' \
        | grep -v '^\s*//' \
        || true)
    if [[ -n "$hits" ]]; then
        echo "FAIL [$label] — raw URL interpolation(s) found:"
        echo "$hits" | sed 's/^/  /'
        FAIL=1
    fi
}

# 1. message: format!("... {url / *_url ...}")
check "message:format" \
    'message:\s*format!\([^)]*\{[a-z_]*url'

# 2. reason: format!("... {url / *_url ...}")
check "reason:format" \
    'reason:\s*format!\([^)]*\{[a-z_]*url'

# 3. RdlpError::(extraction|network|download)(format!("... {*url ..."), ...)
check "RdlpError::*_ctor:format" \
    'RdlpError::(extraction|network|download)\s*\(\s*(&)?format!\("[^"]*\{[a-z_]*url'

# 4. log_if_verbose positional interpolation of *_url
check "log_if_verbose:format" \
    'log_if_verbose\s*\([^;]*\{[a-z_]*url'

# 5. Structured-kv log fields: (url|video_url|...) :? or :% = <bare var>
#    Must NOT be followed by rdlp_redact:: or sanitize_for_logging
check "structured-kv:url_field" \
    '(url|video_url|embed_url|m3u8_url|segment_url|cdn_url|api_url|full_url|page_url):[?%]\s*='

if [[ $FAIL -eq 0 ]]; then
    echo "PASS — no raw URL interpolations found in $TARGET"
    exit 0
else
    echo ""
    echo "FIX: wrap each raw URL with rdlp_redact::RedactedUrl::new(...)"
    exit 1
fi
