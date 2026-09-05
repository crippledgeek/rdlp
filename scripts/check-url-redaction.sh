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

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# The TOOL-PRESENCE guard below remains uncovered by the canary, and that is
# still deliberate. The matcher canary added above feeds a pure matcher a
# synthetic fixture -- input in, verdict out, no environment involved. A
# tool-presence guard has no fixture: proving it fires means re-executing this
# script under a manipulated PATH, and that harness produced four defects
# across four review rounds (it passed vacuously; it exited 127 on external
# binaries missing from its own stub PATH; it inherited the caller's cwd; it
# conflated cannot-run with failed) while the five lines below were never once
# wrong. Same call the kubernetes hack/ scripts make for their own checks.
#
# (This script now genuinely implements --self-test, so the former note about
# spelling the flag without its dashes to stay out of check-all.sh's discovery
# no longer applies -- being discovered is now correct.)
#
# The guard stays verifiable on demand. Seed a stub PATH with the binaries this
# script needs EXCEPT rg -- bash included, or the pipeline below dies 127 before
# reaching anything, which is the very trap that killed the canary:
#
#   stub=$(mktemp -d)
#   for t in bash git grep sed; do ln -s "$(command -v "$t")" "$stub/$t"; done
#   PATH="$stub" /bin/bash scripts/check-url-redaction.sh            # expect: exit 2
#   grep -v '^require_tool rg$' scripts/check-url-redaction.sh \
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

EXTRACTOR_TARGET="crates/rdlp-extractor/src"
PIPELINE_TARGET="crates/rdlp-postprocess/src"
API_TARGET="crates/rdlp-api/src"

# Every crate's sources, so a NEW crate is gated the day it is added rather
# than the day someone remembers to list it here. The per-crate targets above
# stay: they run extra, crate-specific patterns.
ALL_TARGETS=(crates/*/src crates/rdlp-desktop/src-tauri/src)

# The one documented exclusion. `io_diag.rs` dumps `AVFormatContext.url` for the
# OUTPUT context, which is the local muxer output PATH, not a network URL —
# verified at the call site, and the reason rdlp-ffmpeg is called out as
# ungated in CLAUDE.md. Excluded by path so the rest of the crate stays gated.
EXCLUDED_PATH="crates/rdlp-ffmpeg/src/ffmpeg/normalize/io_diag.rs"

# The workspace patterns, named so --self-test drives the SAME strings rather
# than a copy that can drift away from what CI actually runs.
#
# `format!` is absent from the macro class ON PURPOSE: workspace-wide it is the
# URL *constructor* (`format!("{base_url}/{slug}")` occurs ~20 times across the
# extractors), so including it reports construction as a leak. The
# error-surfacing `format!` shapes stay gated by the message:/reason: patterns,
# which anchor on the field name.
PAT_LOG_MACRO='(anyhow!|bail!|error!|warn!|info!|debug!|trace!)\(\s*"(?:[^"\\]|\\.)*\{[a-z_]*url'
PAT_POSITIONAL='(anyhow!|bail!|error!|warn!|info!|debug!|trace!)\(\s*"(?:[^"\\]|\\.)*[a-z_]*url=\{\}'
PAT_STRUCT_KV='[a-z_]*url[a-z_]*:[?%]\s*='


FAIL=0

# Helper: run rg -U (multiline), filter out already-compliant lines and test files,
# report hits.
# Args: <label> <rg-pattern> [target-dir]   (target defaults to the extractor crate)
check() {
    local label="$1"
    local pattern="$2"
    shift 2
    # Multiple targets, passed as separate args: "${ALL_TARGETS[@]}" must reach
    # rg as N paths. Joining them into one string ("${ALL_TARGETS[@]}") hands rg
    # a single nonexistent path whose name is every directory concatenated —
    # which rg reports on stderr, silently making the check pass on everything.
    local targets=("$@")
    if [[ ${#targets[@]} -eq 0 ]]; then
        targets=("$EXTRACTOR_TARGET")
    fi
    local hits
    # One alternation, not six chained greps: the exclusion set is then a single
    # auditable predicate rather than a conjunction the reader must assemble.
    #
    # The former chain also carried a `grep -v '^\s*//'` arm intended to drop
    # commented-out code. It was dead: `rg -n` over a directory prefixes every
    # line with `path:line:`, so `^\s*//` could never match (verified: 0 hits).
    # Dropped rather than repaired -- a comment containing a raw URL in a format
    # string is worth seeing, and the compliant-wrapper filters already cover
    # the real false positives.
    #
    # `rg -U` reports EVERY line of a multi-line match, including the bare
    # `debug!(` opener. Such a line carries no url token and cannot leak
    # anything, so it is dropped before the compliance filter runs -- keeping
    # it produced a FAIL whose quoted line contained no URL at all.
    #
    # What this does NOT do is accept a wrapper written on a LATER line than
    # the url token: the filter is per line, so `debug!(\n "url={}",\n
    # RedactedUrl::new(x))` is still reported. That is the intended
    # convention, not an oversight -- bind `let safe_url = RedactedUrl::new(x)`
    # first and interpolate that, which is what the FIX line tells authors and
    # what the rest of the tree already does. The self-test pins the convention
    # (st:safe_url-binding) so it stays a decision rather than a surprise.
    hits=$(rg -U --type rust -n "$pattern" "${targets[@]}" 2>/dev/null \
        | grep -E '[a-z_]*url' \
        | grep -Ev 'RedactedUrl|sanitize_for_logging|safe_url|/tests/|#\[cfg\(test\)\]' \
        | grep -Fv "$EXCLUDED_PATH" \
        || true)
    if [[ -n "$hits" ]]; then
        echo "FAIL [$label] — raw URL interpolation(s) found:"
        printf '%s\n' "  ${hits//$'\n'/$'\n'  }"
        FAIL=1
    fi
}

# ---------------------------------------------------------------------------
# --self-test: prove the matcher still fires, and still does NOT fire on the
# three shapes that produced false verdicts while this gate was being widened.
# Runs in CI via check-all.sh, so "canary-verified" stays true as the tree
# evolves instead of being a claim made once in a commit message.
# ---------------------------------------------------------------------------
if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    mkdir -p "$tmp/one/src" "$tmp/two/src"

    # A real leak — but placed in the SECOND target only. Passing the targets
    # as one space-joined string ("${ALL_TARGETS[*]}") hands rg a single path
    # named by every directory concatenated; rg reports that on stderr, which
    # this script discards, so the gate would scan NOTHING and pass. This
    # fixture is the regression test for that fail-open.
    cat > "$tmp/two/src/leak.rs" <<'FIXTURE'
fn fetch(video_url: &str) {
    log::warn!("fetching {video_url}");
}
FIXTURE

    # Shapes that must NOT be reported:
    #  - URL construction. `format!` is the constructor workspace-wide.
    #  - The `safe_url` binding: the repo's convention for logging a redacted
    #    value, and the remedy the FIX line points authors at.
    cat > "$tmp/one/src/ok.rs" <<'FIXTURE'
fn build(base_url: &str, slug: &str) -> String {
    format!("{base_url}/{slug}")
}

fn log_it(u: &str) {
    let safe_url = rdlp_redact::RedactedUrl::new(u);
    log::debug!("fetching url={safe_url}");
}
FIXTURE

    st_fail=0
    st_expect() {  # <expectation: hit|clean> <label> <pattern> <dirs...>
        local want="$1" label="$2" pattern="$3"
        shift 3
        local out
        out=$(check "$label" "$pattern" "$@")
        case "$want:${out:+hit}" in
            hit:hit | clean:) : ;;
            *)
                echo "SELF-TEST case failed [$label]: expected $want"
                [ -n "$out" ] && printf '%s\n' "$out"
                st_fail=1
                ;;
        esac
    }

    # 1. The matcher fires at all, AND reaches a target that is not the first.
    st_expect hit "st:multi-target" "$PAT_LOG_MACRO" "$tmp/one/src" "$tmp/two/src"

    # 2. URL construction is not a leak.
    st_expect clean "st:construction-not-a-leak" "$PAT_LOG_MACRO" "$tmp/one/src"

    # 3. The `safe_url` convention passes both url-shaped patterns.
    st_expect clean "st:safe_url-binding" "$PAT_POSITIONAL" "$tmp/one/src"
    st_expect clean "st:safe_url-binding-kv" "$PAT_STRUCT_KV" "$tmp/one/src"

    if [ "$st_fail" -eq 0 ]; then
        echo "SELF-TEST OK: the gate fires on a leak in a non-first target, and"
        echo "  stays quiet on URL construction and the safe_url binding."
        exit 0
    fi
    echo "SELF-TEST FAILED: the gate no longer behaves as documented."
    exit 1
fi

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

# ---------------------------------------------------------------------------
# Positional `url={}` interpolation — the form P1 cannot see.
#
# P1 anchors on `{url` INSIDE the literal, so it catches `"...{url}"` but not
# `"...url={}", some_url` where the value arrives as a positional argument.
# A real site sat in that blind spot: `debug!("Downloading subtitle: lang={lang},
# url={}", sub.url)` wrote a token-bearing CDN URL. It was harmless while the
# desktop had no logger installed; once one was, it went to a file on disk.
#
# Deliberately NOT running the full P1 macro class against rdlp-api: three of
# its matches interpolate a `RedactedUrlBuf` (redacted by TYPE, so the line
# carries no "RedactedUrl" text for the filter to see), and one of those is the
# intentionally-named `user_message_unredacted`. This narrower pattern has no
# such false positives.
# ---------------------------------------------------------------------------
check "positional:url_eq_brace" \
    '(format!|anyhow!|bail!|error!|warn!|info!|debug!|trace!|panic!)\(\s*"(?:[^"\\]|\\.)*[a-z_]*url=\{\}' \
    "$API_TARGET"

# ---------------------------------------------------------------------------
# Workspace-wide sweep — every crate, both leak shapes.
#
# The per-crate checks above predate this and stay (they carry patterns
# specific to those crates). These three close the gap the crate-by-crate
# approach leaves: a URL logged from rdlp-downloader, rdlp-plugin,
# rdlp-jsinterp or a crate added tomorrow was gated by nobody.
# ---------------------------------------------------------------------------

# The macro class here deliberately EXCLUDES bare `format!`. Workspace-wide,
# `format!` is the URL *constructor* — `format!("{base_url}/{slug}")` appears
# ~20 times across the extractors and is not a log at all. Including it made
# the sweep report construction sites as leaks, which is how a gate teaches
# people to ignore it. The error-surfacing `format!` shapes ARE still gated,
# by the `message:`/`reason:` checks below, which anchor on the field name.
#
# `panic!` is excluded for a related reason: its in-production uses are absent
# and its test uses (`panic!("... for {url}")`) are assertions, which the
# `#[cfg(test)]` line filter cannot see from a non-adjacent line.
check "workspace:log-macro:format_url" "$PAT_LOG_MACRO" "${ALL_TARGETS[@]}"

check "workspace:positional:url_eq_brace" "$PAT_POSITIONAL" "${ALL_TARGETS[@]}"

# The two error-surface field shapes, workspace-wide. These were gated in the
# extractor only; a `message: format!("...{url}")` in any other crate reaches
# an operator the same way.
check "workspace:message:format" \
    'message:\s*format!\(\s*"(?:[^"\\]|\\.)*\{[a-z_]*url' \
    "${ALL_TARGETS[@]}"

check "workspace:reason:format" \
    'reason:\s*format!\(\s*"(?:[^"\\]|\\.)*\{[a-z_]*url' \
    "${ALL_TARGETS[@]}"

check "workspace:structured-kv:url_field" "$PAT_STRUCT_KV" "${ALL_TARGETS[@]}"

if [[ $FAIL -eq 0 ]]; then
    echo "PASS — no raw URL interpolations found in any crate"
    exit 0
else
    echo ""
    echo "FIX: wrap each raw URL with rdlp_redact::RedactedUrl::new(...)"
    exit 1
fi
