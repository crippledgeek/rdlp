#!/usr/bin/env bash
# check-error-attr-redaction.sh — CI gate: every `#[error(...)]` attribute that
# interpolates a free-text field must pass it through `redact(...)`.
#
# Exit 0 = PASS (no offenders).
# Exit 1 = FAIL (an attribute interpolates free text unredacted).
# Exit 2 = CANNOT RUN (a tool or a scanned file is missing).
#
# Usage: bash scripts/check-error-attr-redaction.sh [--self-test]
#
# Why this gate exists
# --------------------
# `RdlpError` and `RdlpApiError` carry operator-visible free text assembled by
# `format!("…: {e}")` at hundreds of call sites. A stringified `wreq::Error`
# renders the request URI verbatim, credentials included, so that text can hold
# a URL — inside an opaque string, which is why `check-url-redaction.sh` scans
# these files and finds nothing: there is no URL-shaped token to match.
#
# Their `Debug` impls are hand-written `match`es, so the compiler forces a new
# variant to be handled there. The `#[error(...)]` attributes have no such
# backstop: adding
#
#     #[error("Thing failed: {0}")]
#     Thing(String),
#
# compiles, passes clippy, passes the unit tests, and leaks. This gate is that
# missing backstop.
#
# Scope: only the two error enums whose payloads are operator-assembled free
# text. Typed `#[from]` sources (`Io`, `UrlParse`, `Json`, `Regex`) render their
# own crate's message and cannot carry a URL we built, so `{0}` on those is
# allowed via the source-variant allowlist below.

set -euo pipefail
export LC_ALL=C

# check-all.sh asserts this exact literal at line start (see its
# sentinel convention) so a gate that merely MENTIONS --self-test cannot
# report a canary it never ran.
SENTINEL='SELF-TEST OK'

FILES=(
    "crates/rdlp-core/src/error.rs"
    "crates/rdlp-api/src/errors.rs"
    "crates/rdlp-api/src/orchestrator/errors.rs"
)

# A payload marked `#[source]` or `#[from]` is a typed error that renders its
# own crate's message, not text we assembled — detected structurally rather
# than by a list of variant names, which would go stale per-enum and silently
# stop exempting when a name changed.
TYPED_PAYLOAD_RE='#\\[(source|from)\\]'

# Placeholder names that cannot carry operator-assembled free text: the
# redacting URL type, and scalars. Two are named explicitly so the exception is
# visible rather than implied: `feature` is a String but only ever a
# compile-time constant, and `path` is a `PathBuf` — a local filesystem path,
# never a URL we fetched. `job_id` is deliberately absent: it lives on
# `AppError`, which has no `#[error]` attributes and is not scanned.
EXEMPT_RE='status|url|source_url|feature|retry_after_ms|path'

# Anchor to the repo root so relative FILES resolve wherever this is invoked
# from, matching check-url-redaction.sh's convention (#621).
cd "$(git rev-parse --show-toplevel)" || exit 2

command -v awk >/dev/null 2>&1 || {
    echo "check-error-attr-redaction: awk not found" >&2
    exit 2
}

scan_file() {
    local file="$1"
    [ -f "$file" ] || {
        echo "check-error-attr-redaction: missing $file" >&2
        return 2
    }
    # Checked PER PLACEHOLDER, not per attribute. An earlier version accepted
    # any attribute containing `redact(` anywhere, which proves ONE placeholder
    # is routed and says nothing about a second: `#[error("{} at {leak}",
    # redact(message))]` passed it with `{leak}` raw.
    #
    # Every placeholder form is counted, because the compliant style in this
    # repo is `#[error("…: {}", redact(field))]` — so the likeliest regression
    # is someone copying that line and dropping the `redact(`, which leaves a
    # BARE `{}`. Named (`{x}`), positional (`{0}`), empty (`{}`) and
    # debug-formatted (`{x:?}`) all count.
    #
    # Multi-line attributes are joined first: rustfmt wraps long ones, and a
    # line-scoped matcher silently skips those.
    awk -v typed="$TYPED_PAYLOAD_RE" -v exempt="$EXEMPT_RE" '
        # Join a wrapped #[error(...)] into one logical record.
        /#\[error\(/ { collecting = 1; attr = ""; attr_line = NR }
        collecting { attr = attr $0 }
        collecting && /\)\]/ {
            collecting = 0
            line = attr
            # Count placeholders, ignoring `{{` / `}}` escapes.
            gsub(/\{\{|\}\}/, "", line)
            n_ph = gsub(/\{[^}]*\}/, "&", line)
            if (n_ph == 0) { next }
            # Exempt placeholders whose names cannot carry free text.
            tmp = line
            n_exempt = gsub("\\{(" exempt ")(:[^}]*)?\\}", "", tmp)
            # Placeholders demonstrably routed through redact(...).
            n_redacted = gsub(/redact\(/, "", line)
            if (n_ph - n_exempt <= n_redacted) { next }
            # Typed sources render their own crate message.
            if ((getline nextline) > 0) {
                if (nextline ~ typed) next
            }
            one = attr
            gsub(/[ \t]+/, " ", one)
            printf "%d:%s\n", attr_line, one
        }
    ' "$file"
}

if [ "${1:-}" = "--self-test" ]; then
    # Prove the matcher can still fail, in every shape it is meant to catch.
    # A textual gate that has rotted into a no-op reports PASS forever.
    canary="$(mktemp)"
    trap 'rm -f "$canary"' EXIT

    check_flags() { # description, source
        printf '%s\n' "$2" > "$canary"
        if [ -z "$(scan_file "$canary")" ]; then
            echo "check-error-attr-redaction: SELF-TEST FAILED — missed: $1" >&2
            exit 1
        fi
    }
    check_clean() {
        printf '%s\n' "$2" > "$canary"
        if [ -n "$(scan_file "$canary")" ]; then
            echo "check-error-attr-redaction: SELF-TEST FAILED — false positive: $1" >&2
            exit 1
        fi
    }

    check_flags "bare {} with no redact()" '    #[error("A failed: {}", message)]
    A { message: String },'
    check_flags "named placeholder" '    #[error("B failed: {message}")]
    B { message: String },'
    check_flags "debug-formatted placeholder" '    #[error("C failed: {message:?}")]
    C { message: String },'
    check_flags "second placeholder beside an exempt one" '    #[error("D {message} for {url}")]
    D { message: String, url: RedactedUrlBuf },'
    check_flags "positional payload" '    #[error("E failed: {0}")]
    E(String),'
    check_flags "rustfmt-wrapped attribute" '    #[error(
        "F failed: {message}"
    )]
    F { message: String },'
    check_flags "one redacted, one raw" '    #[error("{} at {leak}", redact(message))]
    G { message: String, leak: String },'

    check_clean "compliant single placeholder" '    #[error("H failed: {}", redact(message))]
    H { message: String },'
    check_clean "compliant beside an exempt name" '    #[error("I failed for {url}: {}", redact(message))]
    I { message: String, url: RedactedUrlBuf },'
    check_clean "typed source variant" '    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),'
    check_clean "no placeholders at all" '    #[error("operation cancelled")]
    Cancelled,'

    echo "$SENTINEL"
    exit 0
fi

offenders=""
for f in "${FILES[@]}"; do
    found="$(scan_file "$f")" || exit 2
    if [ -n "$found" ]; then
        offenders="${offenders}${found}"$'\n'
    fi
done

if [ -n "${offenders//[$'\n']/}" ]; then
    echo "check-error-attr-redaction: FAIL — #[error(...)] interpolates free text without redact():" >&2
    echo "$offenders" >&2
    echo "Fix: render as #[error(\"… {}\", redact(field))] so the text is stripped at the boundary." >&2
    exit 1
fi

exit 0
