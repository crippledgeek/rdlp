#!/usr/bin/env bash
# check-error-attr-redaction.sh — CI gate: free text rendered by an error type
# must be routed through `redact(...)`.
#
# Exit 0 = PASS (no offenders).
# Exit 1 = FAIL (an attribute renders free text unredacted).
# Exit 2 = CANNOT RUN (a tool or a scanned file is missing).
#
# Usage: bash scripts/check-error-attr-redaction.sh [--self-test]
#
# Why this gate exists
# --------------------
# The scanned error types carry operator-visible free text assembled by
# `format!("…: {e}")` at hundreds of call sites. A stringified `wreq::Error`
# renders the request URI verbatim, credentials included, so that text can hold
# a URL — inside an opaque string, which is why `check-url-redaction.sh` scans
# some of these files and finds nothing: there is no URL-shaped token to match.
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

set -euo pipefail
export LC_ALL=C

# check-all.sh asserts this exact literal at line start, so a gate that merely
# MENTIONS --self-test cannot report a canary it never ran.
SENTINEL='SELF-TEST OK'

# Anchor to the repo root so relative paths resolve wherever this is invoked
# from (check-url-redaction.sh does the same, per #621).
cd "$(git rev-parse --show-toplevel)" || exit 2

command -v awk >/dev/null 2>&1 || {
    echo "check-error-attr-redaction: awk not found" >&2
    exit 2
}

FILES=(
    "crates/rdlp-core/src/error.rs"
    "crates/rdlp-api/src/errors.rs"
    "crates/rdlp-api/src/orchestrator/errors.rs"
)

# Types that reach the frontend through a DERIVED `Serialize`, which reads each
# field directly — so Display/Debug redaction does nothing for them. Every
# free-text `String` field needs `#[serde(serialize_with = "serialize_redacted")]`.
# Both files below have already leaked once through a field that lacked it.
# Scope is ERROR and EVENT payload types, deliberately not data models. On a
# data model a `url: String` is the payload — the frontend needs it to fetch —
# so the rule "every free-text String is redacted" inverts there and would
# break the feature it is meant to protect. `rdlp-types::SubtitleDiagnostic`
# carries error text and is guarded on its field by hand for that reason;
# `SubtitleTrack` beside it is a model and is correctly not.
SERDE_FILES=(
    "crates/rdlp-desktop/src-tauri/src/error.rs"
    "crates/rdlp-desktop/src-tauri/src/events.rs"
)

# Field names on those types that are not operator-assembled free text.
# Field names on those types that are not operator-assembled free text:
# identifiers, enum-ish tags, local filesystem paths, and formatted numbers.
# Every entry must be a name actually present in a SERDE_FILES type — a stale
# entry is a hole nobody is watching, which is why `retry_after_ms` and eight
# others were removed.
#
# `url` is deliberately NOT exempt: a URL-carrying field must be typed
# `RedactedUrlBuf` (which self-redacts and is not a `String`, so it never
# matches the field rule at all). A `url: String` should be flagged.
#
# Both exemption lists match by NAME, not type — the gate cannot see types. A
# new field named for something on these lists must be checked by its author to
# actually be the harmless thing the name implies.
SERDE_EXEMPT_RE='job_id|field|level|stage|speed|eta|filepath|unit_title'


# Placeholder names that cannot carry operator-assembled free text: the
# self-redacting URL type, and scalars. Every entry must be a name actually
# present in a scanned file — a stale entry is a hole nobody is watching.
#   url, source_url : RedactedUrlBuf, redacts in its own Display AND Debug
#   status          : u16
#   path            : PathBuf — a local filesystem path, never a URL we fetched
EXEMPT_RE='url|source_url|status|path'

# `#[error(transparent)]` renders the wrapped value with no placeholder of its
# own, so placeholder counting cannot see it. Each one must be listed here with
# the reason its payload is safe, so adding a transparent variant is a
# deliberate act rather than an invisible exemption.
#   (none — OrchestratorError::Other was converted to an explicit redaction,
#    because `anyhow::Error` forwards our own `.context(...)` strings.)
TRANSPARENT_ALLOW_RE='^$'

scan_file() {
    local file="$1"
    [ -f "$file" ] || {
        echo "check-error-attr-redaction: missing $file" >&2
        return 2
    }
    awk -v exempt="$EXEMPT_RE" -v transparent_allow="$TRANSPARENT_ALLOW_RE" '
        # Skip doc comments: an #[error(...)] shown as an EXAMPLE is not code.
        /^[ \t]*\/\// { next }

        # Join a rustfmt-wrapped #[error(...)] into one logical record.
        /#\[error\(/ { collecting = 1; attr = ""; attr_line = NR }
        collecting { attr = attr $0 }
        collecting && /\)\]/ {
            collecting = 0
            if ((getline payload) <= 0) payload = ""

            if (attr ~ /transparent/) {
                if (payload !~ transparent_allow) {
                    printf "%d:%s   [transparent, payload not allowlisted]\n", attr_line, squash(attr)
                }
                next
            }

            # Split the attribute at the first `",` — everything after it is
            # the argument list. `redact(` is only counted there, so a mention
            # in a trailing comment cannot satisfy the gate.
            args = attr
            if (sub(/^[^"]*"([^"\\]|\\.)*"[ \t]*,?/, "", args) == 0) args = ""
            # Drop everything from the attribute close onward: a trailing
            # comment is not routing, and `redact(` written there must not
            # satisfy the gate.
            sub(/\)\].*$/, "", args)
            # Identifier-boundary anchored: a bare substring match would let
            # `unredact(` or `maybe_redact(` satisfy the count.
            n_redacted = gsub(/(^|[^A-Za-z0-9_])redact\(/, "", args)

            # Which positional fields are typed sources? Only THOSE placeholders
            # are exempt — a tuple variant may pair `#[source]` with a free-text
            # field, and exempting the whole attribute would miss it.
            n_typed_positional = 0
            split(payload, parts, ",")
            for (i in parts) {
                if (parts[i] ~ /#\[(source|from)\]/) typed_idx[i - 1] = 1
            }

            literal = attr
            gsub(/\{\{|\}\}/, "", literal)
            n_unguarded = 0
            while (match(literal, /\{[^}]*\}/)) {
                ph = substr(literal, RSTART + 1, RLENGTH - 2)
                literal = substr(literal, RSTART + RLENGTH)
                sub(/:.*$/, "", ph)                       # drop the format spec
                if (ph ~ ("^(" exempt ")$")) continue
                if (ph ~ /^[0-9]+$/ && (ph in typed_idx)) continue
                n_unguarded++
            }
            delete typed_idx

            if (n_unguarded > n_redacted) {
                printf "%d:%s\n", attr_line, squash(attr)
            }
        }

        function squash(t) { gsub(/[ \t]+/, " ", t); return t }
    ' "$file"
}

# A `String` field on a Serialize type must carry the redacting serializer.
scan_serde_file() {
    local file="$1"
    [ -f "$file" ] || {
        echo "check-error-attr-redaction: missing $file" >&2
        return 2
    }
    awk -v exempt="$SERDE_EXEMPT_RE" '
        /^[ \t]*\/\// { next }
        # Must BE the attribute, not merely contain its text: a trailing comment
        # mentioning it is not a guard. Same defect this gate closed for
        # `redact(` one function above.
        /^[ \t]*#\[serde\(/ && /serialize_with[ \t]*=[ \t]*"([A-Za-z_]+::)*serialize_redacted"/ {
            guarded = 1; next
        }
        # A rustfmt-wrapped attribute spans lines; keep the guard alive across
        # its continuation rather than letting the closing `)]` clear it.
        guarded_open && /\)\]/ { guarded_open = 0; guarded = 1; next }
        guarded_open { next }
        /^[ \t]*#\[serde\(/ && !/\)\]/ { guarded_open = 1; next }
        /^[ \t]*(pub(\([a-z]+\))?[ \t]+)?[a-z_]+[ \t]*:[ \t]*(Option<)?String[,>]/ {
            name = $0
            sub(/^[ \t]*(pub(\([a-z]+\))?[ \t]+)?/, "", name)
            sub(/[ \t]*:.*$/, "", name)
            if (!guarded && name !~ ("^(" exempt ")$")) {
                line = $0; gsub(/[ \t]+/, " ", line)
                printf "%d:%s   [String field with no serialize_redacted]\n", NR, line
            }
            guarded = 0
            next
        }
        # Any other non-blank line clears a pending attribute.
        /[^ \t]/ { guarded = 0 }
    ' "$file"
}

if [ "${1:-}" = "--self-test" ]; then
    canary="$(mktemp)"
    trap 'rm -f "$canary"' EXIT

    check_flags() {
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
    check_flags "free text beside an exempt name" '    #[error("D {message} for {url}")]
    D { message: String, url: RedactedUrlBuf },'
    check_flags "positional payload" '    #[error("E failed: {0}")]
    E(String),'
    check_flags "rustfmt-wrapped attribute" '    #[error(
        "F failed: {message}"
    )]
    F { message: String },'
    check_flags "one redacted, one raw" '    #[error("{} at {leak}", redact(message))]
    G { message: String, leak: String },'
    check_flags "typed source PLUS a free-text field" '    #[error("H: {0} leaked at {1}")]
    H(#[from] std::io::Error, String),'
    check_flags "decoy identifier containing redact(" '    #[error("N failed: {0}", unredact(_0))]
    N(String),'
    check_flags "redact( only in a trailing comment" '    #[error("I failed: {0}")] // redact( is not routing
    I(String),'
    check_flags "transparent over a non-allowlisted payload" '    #[error(transparent)]
    J(#[from] anyhow::Error),'

    check_clean "compliant single placeholder" '    #[error("K failed: {}", redact(message))]
    K { message: String },'
    check_clean "compliant beside an exempt name" '    #[error("L failed for {url}: {}", redact(message))]
    L { message: String, url: RedactedUrlBuf },'
    check_clean "typed source alone" '    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),'
    check_clean "no placeholders at all" '    #[error("operation cancelled")]
    Cancelled,'
    serde_flags() {
        printf '%s\n' "$2" > "$canary"
        if [ -z "$(scan_serde_file "$canary")" ]; then
            echo "check-error-attr-redaction: SELF-TEST FAILED — serde missed: $1" >&2
            exit 1
        fi
    }
    serde_clean() {
        printf '%s\n' "$2" > "$canary"
        if [ -n "$(scan_serde_file "$canary")" ]; then
            echo "check-error-attr-redaction: SELF-TEST FAILED — serde false positive: $1" >&2
            exit 1
        fi
    }
    serde_flags "unguarded String field" '    pub(crate) message: String,'
    serde_flags "unguarded Option<String> field" '    pub(crate) reason: Option<String>,'
    serde_flags "serialize_with only in a trailing comment" '    pub(crate) leak: String, // serialize_with = "serialize_redacted"'
    serde_clean "rustfmt-wrapped serde attribute" '    #[serde(
        serialize_with = "serialize_redacted"
    )]
    pub(crate) message: String,'
    serde_clean "guarded field" '    #[serde(serialize_with = "serialize_redacted")]
    pub(crate) message: String,'
    serde_clean "exempt name" '    pub(crate) job_id: String,'
    serde_clean "non-String field" '    pub(crate) retryable: bool,'

    check_clean "an #[error(...)] inside a doc comment" '    /// Example: #[error("Thing failed: {0}")]
    /// Thing(String),
    #[error("M failed: {}", redact(message))]
    M { message: String },'

    echo "$SENTINEL"
    exit 0
fi

offenders=""
for f in "${FILES[@]}"; do
    found="$(scan_file "$f")" || exit 2
    if [ -n "$found" ]; then
        offenders="${offenders}${f}:"$'\n'"${found}"$'\n'
    fi
done

for f in "${SERDE_FILES[@]}"; do
    found="$(scan_serde_file "$f")" || exit 2
    if [ -n "$found" ]; then
        offenders="${offenders}${f}:"$'\n'"${found}"$'\n'
    fi
done

if [ -n "${offenders//[$'\n']/}" ]; then
    echo "check-error-attr-redaction: FAIL — error text rendered without redact():" >&2
    echo "$offenders" >&2
    echo "Fix: render as #[error(\"… {}\", redact(field))] so the text is stripped at the boundary." >&2
    exit 1
fi

exit 0
