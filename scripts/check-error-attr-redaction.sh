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
)

# Variants whose payload is a typed error, not free text we assembled.
TYPED_SOURCE_RE='Io|UrlParse|Json|Regex'

command -v rg >/dev/null 2>&1 || {
    echo "check-error-attr-redaction: ripgrep (rg) not found" >&2
    exit 2
}

scan_file() {
    local file="$1"
    [ -f "$file" ] || {
        echo "check-error-attr-redaction: missing $file" >&2
        return 2
    }
    # An offender interpolates a placeholder INSIDE the format literal without
    # routing it through redact(). Compliant attributes render as
    # `#[error("… {}", redact(field))]`, whose literal holds `{}` only.
    #
    # The variant sits on the line AFTER its attribute, so deciding whether a
    # payload is typed (allowed) or free text (must be redacted) needs a
    # lookahead — hence awk rather than a lone rg pattern.
    #
    # Fields that are already redacting types (`RedactedUrlBuf`) or plain
    # scalars are exempt by name: they cannot carry free text.
    awk -v typed="$TYPED_SOURCE_RE" '
        /#\[error\(/ {
            attr = $0; attr_line = NR
            # A placeholder inside the literal, not already routed via redact().
            if (attr !~ /\{[a-z_0-9]+\}/) next
            if (attr ~ /redact\(/) next
            if (attr ~ /\{(status|job_id|feature|url|source_url)\}/) next
            if ((getline nextline) <= 0) { printf "%d:%s\n", attr_line, attr; next }
            # Strip to the variant identifier that opens the next line.
            variant = nextline
            sub(/^[ \t]*/, "", variant)
            sub(/[^A-Za-z0-9_].*$/, "", variant)
            if (variant ~ ("^(" typed ")$")) next
            printf "%d:%s\n", attr_line, attr
        }
    ' "$file"
}

if [ "${1:-}" = "--self-test" ]; then
    # Prove the matcher can still fail. A textual gate that has rotted into a
    # no-op reports PASS forever; this canary is the only thing that notices.
    canary="$(mktemp)"
    trap 'rm -f "$canary"' EXIT
    cat > "$canary" <<'CANARY'
#[derive(Error)]
pub enum Canary {
    #[error("Thing failed: {message}")]
    Thing { message: String },
}
CANARY
    if [ -z "$(scan_file "$canary")" ]; then
        echo "check-error-attr-redaction: SELF-TEST FAILED — the matcher did not flag an unredacted attribute" >&2
        exit 1
    fi
    # And that it does not flag the compliant form.
    cat > "$canary" <<'CANARY'
#[derive(Error)]
pub enum Canary {
    #[error("Thing failed: {}", redact(message))]
    Thing { message: String },
}
CANARY
    if [ -n "$(scan_file "$canary")" ]; then
        echo "check-error-attr-redaction: SELF-TEST FAILED — the matcher flagged a compliant attribute" >&2
        exit 1
    fi
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
