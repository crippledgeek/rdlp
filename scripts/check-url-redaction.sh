#!/usr/bin/env bash
# check-url-redaction.sh — CI gate: verify all operator-visible URL interpolations in
# rdlp-extractor are wrapped with rdlp_redact::RedactedUrl (or sanitize_for_logging).
#
# Exit 0 = PASS (no offenders found).
# Exit 1 = FAIL (raw URL interpolations remain).
#
# Usage: bash scripts/check-url-redaction.sh
#
# All rg invocations use -U (multiline) so that format! arguments split across lines
# (e.g. `message: format!(\n    "... {url}"`) are caught.  Patterns 1-3 use
# "(?:[^"\\]|\\.)*" to stay inside the format-string literal, which also eliminates
# the need to filter out RedactedUrl wraps: compliant code passes the URL as a
# positional argument *outside* the string, so the in-string {*url} pattern never
# fires on it.

set -euo pipefail

TARGET="crates/rdlp-extractor/src"
FAIL=0

# Helper: run rg -U (multiline), filter out already-compliant lines and test files,
# report hits.
# Args: <label> <rg-pattern>
check() {
    local label="$1"
    local pattern="$2"
    local hits
    hits=$(rg -U --type rust -n "$pattern" "$TARGET" 2>/dev/null \
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
#    "(?:[^"\\]|\\.)*" matches only inside the string literal (stops at closing "),
#    so a following `url: None` field is never caught as a false positive.
check "message:format" \
    'message:\s*format!\(\s*"(?:[^"\\]|\\.)*\{[a-z_]*url'

# 2. reason: format!("... {url / *_url ...}")
check "reason:format" \
    'reason:\s*format!\(\s*"(?:[^"\\]|\\.)*\{[a-z_]*url'

# 3. RdlpError::(extraction|network|download)(format!("... {*url ..."), ...)
check "RdlpError::*_ctor:format" \
    'RdlpError::(extraction|network|download)\s*\(\s*(&)?format!\(\s*"(?:[^"\\]|\\.)*\{[a-z_]*url'

# 4. log_if_verbose positional interpolation of *_url
#    [^;]* is bounded by the statement terminator; -U ensures any wrapped call is caught.
check "log_if_verbose:format" \
    'log_if_verbose\s*\([^;]*\{[a-z_]*url'

# 5. Structured-kv log fields: any *url* field used with :? or :% = <bare var>
#    Durable class [a-z_]*url[a-z_]* catches future names (manifest_url, master_url, …).
check "structured-kv:url_field" \
    '[a-z_]*url[a-z_]*:[?%]\s*='

if [[ $FAIL -eq 0 ]]; then
    echo "PASS — no raw URL interpolations found in $TARGET"
    exit 0
else
    echo ""
    echo "FIX: wrap each raw URL with rdlp_redact::RedactedUrl::new(...)"
    exit 1
fi
