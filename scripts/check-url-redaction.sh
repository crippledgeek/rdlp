#!/usr/bin/env bash
# check-url-redaction.sh — CI gate: verify all operator-visible URL interpolations in
# rdlp-extractor AND the post-processing pipeline (rdlp-postprocess) are wrapped with
# rdlp_redact::RedactedUrl (or sanitize_for_logging).
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
#
# Scope rationale (#422): the fatal post-process stages (Merge/Remux/Recode/
# AudioExtract/Normalize) author the `.context(...)` strings that
# `classify_pipeline_err` flattens via `{e:#}` into `OrchestratorError::
# PostProcessingFailed` → `Event::Failed`.  Today those contexts are static
# literals (no URL), but a future stage that fetches a URL (e.g. a remote-subtitle
# or plugin post-processor) could inline one.  Gating rdlp-postprocess closes that
# regression window at the authoring surface.  The classifier itself lives in
# rdlp-api but cannot introduce a URL (it only formats `{e:#}` + a static string),
# so it needs no separate gate.  rdlp-ffmpeg is intentionally NOT gated: its only
# `url` token is `AVFormatContext.url`, which for a file muxer is the local output
# path (not a network URL) and appears only in diagnostic logs.

set -euo pipefail

EXTRACTOR_TARGET="crates/rdlp-extractor/src"
PIPELINE_TARGET="crates/rdlp-postprocess/src"
API_TARGET="crates/rdlp-api/src"
FAIL=0

# Helper: run rg -U (multiline), filter out already-compliant lines and test files,
# report hits.
# Args: <label> <rg-pattern> [target-dir]   (target defaults to the extractor crate)
check() {
    local label="$1"
    local pattern="$2"
    local target="${3:-$EXTRACTOR_TARGET}"
    local hits
    hits=$(rg -U --type rust -n "$pattern" "$target" 2>/dev/null \
        | grep -v 'RedactedUrl' \
        | grep -v 'sanitize_for_logging' \
        | grep -v 'safe_url' \
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

# ---------------------------------------------------------------------------
# Post-processing pipeline (rdlp-postprocess) — #422 defense-in-depth.
#
# Fatal-stage `.context(...)` strings reach Event::Failed via classify_pipeline_err's
# `{e:#}` flatten, so a URL inlined into any error/log macro here would surface to
# the operator unredacted.  Two durable classes:
#
#   P1. Any error/log-producing macro whose string literal inlines `{*url}` —
#       covers `.context(format!("…{url}"))`, `.with_context(|| format!(…{url}))`,
#       bare `anyhow!`/`bail!`, and the `error!`/`warn!`/`info!`/`debug!`/`trace!`/
#       `panic!` log macros.  `format!` is in the alternation, so any `.context(...)`
#       built from a `format!` is caught regardless of the surrounding call.
#   P2. Structured-kv `*url*` fields (same class as #5, re-run against the pipeline).
# ---------------------------------------------------------------------------

# P1. error/log macro inlining {*url} in the string literal.
check "pp:macro:format_url" \
    '(format!|anyhow!|bail!|error!|warn!|info!|debug!|trace!|panic!)\(\s*"(?:[^"\\]|\\.)*\{[a-z_]*url' \
    "$PIPELINE_TARGET"

# P2. Structured-kv log fields in the pipeline.
check "pp:structured-kv:url_field" \
    '[a-z_]*url[a-z_]*:[?%]\s*=' \
    "$PIPELINE_TARGET"

# ---------------------------------------------------------------------------
# API layer (rdlp-api) — #427 defense-in-depth (DELIBERATELY NARROW).
#
# Tiered controls protect rdlp-api's URL surface; each catches what the others
# cannot (see CODING_RULES.md "URL Redaction — Controls (tiered)"):
#
#   1. THE TYPE SYSTEM is the PRIMARY guard for error-enum URL fields. The
#      `url` field of `RdlpApiError::UnsupportedUrl` / `OrchestratorError::
#      {NoExtractor,NoDownloader}` is `rdlp_redact::RedactedUrlBuf`, whose
#      `Display` redacts — so `#[error("…{url}")]` and `format!("…{url}")`
#      auto-redact (#427). This grep CANNOT verify that surface: a redacted
#      `{url}` (RedactedUrlBuf) and a raw `{url}` (String) are the SAME token
#      in source. The compliant precedent `#[error("…{source_url}…")]`
#      (errors.rs) would false-positive under an error-attr/format pattern.
#      Industry standard: redact at source via a secret type (CWE-532;
#      secrets-as-types). So we do NOT add a `{*url}` error-attr/format check
#      here — the type + the unit tests guard it.
#
#   2. THIS GREP is the SECONDARY guard, for the one surface the type cannot
#      cover: a raw URL passed to a STRUCTURED-KV log field (`url:? = <var>`).
#      A `.expose()`-then-log bypass lands here. Only the `:?`/`:%` form is
#      gated; the positional no-sigil `url = <var>` form is added by #428
#      (after orchestrator/thumbnail.rs is wrapped — it is the one remaining
#      raw `url = thumbnail_url` site today).
# ---------------------------------------------------------------------------

# A1. Structured-kv `:?`/`:%` url fields in rdlp-api (the .expose()-then-log surface).
check "api:structured-kv:url_field" \
    '[a-z_]*url[a-z_]*:[?%]\s*=' \
    "$API_TARGET"

if [[ $FAIL -eq 0 ]]; then
    echo "PASS — no raw URL interpolations found in $EXTRACTOR_TARGET, $PIPELINE_TARGET, or $API_TARGET"
    exit 0
else
    echo ""
    echo "FIX: wrap each raw URL with rdlp_redact::RedactedUrl::new(...)"
    exit 1
fi
