#!/usr/bin/env bash
# Verify the desktop's `EffectiveNetwork` mirror and its consumers have not
# drifted from the Rust owner of the nine network/download defaults (#611).
#
# Two invariants:
#
#  (a) FIELD SET — the TS `interface EffectiveNetwork` in
#      crates/rdlp-desktop/src/types/index.ts has exactly the field names of
#      `pub struct EffectiveNetwork` in crates/rdlp-types/src/effective_network.rs
#      (serde default naming: the Rust identifier IS the wire key). Adding a
#      Rust field does not break the TS build — it just ships a key the TS type
#      says does not exist, and a placeholder that can never be derived from it.
#
#  (b) NO LITERAL PLACEHOLDERS — DownloadSection.tsx / NetworkSection.tsx must
#      not carry a numeric placeholder literal (`placeholder="8"`, or a
#      `byteFieldPlaceholder(x, "10")`-style trailing string argument). Every
#      NumericField placeholder derives from the IPC-sourced payload; a literal
#      is a fifth copy of a default, which is exactly the drift #611 removed.
#
# The Rust side is parsed from the struct body itself, never a hand-copied
# list, so the gate cannot silently agree with a stale mirror. Both parsers are
# STRICT: a body line they cannot classify exits 2 (extend the script) rather
# than being skipped — a skipped field on both sides would let the sets match
# on the very drift this exists to catch.
#
# Fix when this fails: (a) edit the TS interface to match the reported Rust
# field set; (b) replace the literal with `String(defaults.<field>)` (or
# `defaults.<field>` for the byte-field helpers).
#
# Usage: scripts/check-effective-network-drift.sh [--self-test]
#   --self-test: prove both checks still FAIL on a synthetic field mismatch and
#                a synthetic literal placeholder, in a temp dir. check-all.sh
#                runs it every time.

set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like `[a-z_]` are
# UNSPECIFIED outside the C locale. A correctness fix (#621).
export LC_ALL=C

# Anchor to the repo root: every path below is relative. `|| exit 2` separates
# "cannot run" from "gate failed" (exit 1). See #621.
cd "$(git rev-parse --show-toplevel)" || exit 2

RUST_FILE="crates/rdlp-types/src/effective_network.rs"
TS_FILE="crates/rdlp-desktop/src/types/index.ts"
SECTION_FILES=(
    "crates/rdlp-desktop/src/views/settings/sections/DownloadSection.tsx"
    "crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx"
)

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Field identifiers of `pub struct EffectiveNetwork {` in $1, sorted.
rust_fields() {
    awk -v file="$1" '
        index($0, "pub struct EffectiveNetwork {") == 1 { inside = 1; next }
        inside && /^}/ { exit }
        inside && /^[[:space:]]*$/ { next }                 # blank
        inside && /^[[:space:]]*\/\// { next }              # `//` or `///`
        inside && /^[[:space:]]*#\[/ {
            if ($0 ~ /serde/) {
                printf "error: per-field serde attribute in %s:\n  %s\n", file, $0 > "/dev/stderr"
                print  "       the wire key no longer follows from the identifier. Extend the script." > "/dev/stderr"
                exit 2
            }
            next
        }
        inside && /^    pub [a-z_][a-z0-9_]*: [A-Za-z0-9_<>]+,$/ {
            sub(/^    pub /, ""); sub(/:.*$/, "")
            print
            next
        }
        inside {
            printf "error: unparseable line in EffectiveNetwork (%s):\n  %s\n", file, $0 > "/dev/stderr"
            print  "       Expected `pub <snake_ident>: <Type>,`. Extend this script." > "/dev/stderr"
            exit 2
        }
    ' "$1" | sort
}

# Property identifiers of `export interface EffectiveNetwork {` in $1, sorted.
ts_fields() {
    awk -v file="$1" '
        index($0, "export interface EffectiveNetwork {") == 1 { inside = 1; next }
        inside && /^}/ { exit }
        inside && /^[[:space:]]*$/ { next }                        # blank
        inside && /^[[:space:]]*(\/\/|\/\*|\*)/ { next }           # `//`, `/**`, ` *`, ` */`
        inside && /^    [a-z_][a-z0-9_]*: number;$/ {
            sub(/^    /, ""); sub(/:.*$/, "")
            print
            next
        }
        inside {
            printf "error: unparseable line in TS EffectiveNetwork (%s):\n  %s\n", file, $0 > "/dev/stderr"
            print  "       Expected `<snake_ident>: number;` (optional `?` is drift too:" > "/dev/stderr"
            print  "       every Rust field is concrete). Extend this script." > "/dev/stderr"
            exit 2
        }
    ' "$1" | sort
}

# (a) Compare the two field sets. 0 = match, 1 = drift, 2 = cannot parse.
check_field_set() {
    local rust_file=$1 ts_file=$2 rust_set ts_set diff_out
    # awk's exit 2 must abort, not be swallowed by the assignment (pipefail).
    if ! rust_set=$(rust_fields "$rust_file"); then return 2; fi
    if ! ts_set=$(ts_fields "$ts_file"); then return 2; fi
    if [ -z "$rust_set" ]; then
        echo "error: parsed zero fields from EffectiveNetwork in $rust_file" >&2
        return 2
    fi
    if [ -z "$ts_set" ]; then
        echo "error: parsed zero fields from TS interface EffectiveNetwork in $ts_file" >&2
        return 2
    fi
    if ! diff_out=$(diff <(echo "$rust_set") <(echo "$ts_set")); then
        cat <<EOF >&2

ERROR: EffectiveNetwork ($rust_file) and the TypeScript interface ($ts_file)
disagree. '<' lines are Rust-only (missing from TS); '>' lines are TS-only
(no such Rust field).

$diff_out
EOF
        return 1
    fi
    echo "EffectiveNetwork ↔ TS interface OK ($(echo "$rust_set" | wc -l) fields)"
}

# (b) Reject numeric placeholder literals in the given section files.
# 0 = clean, 1 = a literal was found.
check_no_literal_placeholders() {
    local hits
    # `|| true`: grep exits 1 on zero matches, which is the PASSING case here.
    hits=$(grep -nE 'placeholder="[0-9]+"|, "[0-9]+"\)' "$@" || true)
    if [ -n "$hits" ]; then
        cat <<EOF >&2

ERROR: numeric placeholder literal(s) in the settings sections. Every
NumericField placeholder must derive from the IPC-sourced EffectiveNetwork
payload (\`String(defaults.<field>)\`), never a literal copy of a default (#611):

$hits
EOF
        return 1
    fi
    echo "no literal placeholders in ${#@} section file(s) OK"
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    # Matching pair — the parsers must accept the real layout (doc comments,
    # a JSDoc block, mixed types) and report a match.
    cat > "$tmp/good.rs" <<'FIXTURE'
pub struct EffectiveNetwork {
    /// Doc comment.
    pub alpha_secs: u64,

    pub beta: usize,
}
FIXTURE
    cat > "$tmp/good.ts" <<'FIXTURE'
export interface EffectiveNetwork {
    /** JSDoc. */
    alpha_secs: number;
    beta: number;
}
FIXTURE
    # Drifted pair — TS renamed one field.
    cat > "$tmp/bad.ts" <<'FIXTURE'
export interface EffectiveNetwork {
    alpha_secs: number;
    gamma: number;
}
FIXTURE
    # Section fixtures: clean, a bare literal, and a helper-argument literal.
    printf '<NumericField placeholder={String(defaults.alpha_secs)} />\n' > "$tmp/clean.tsx"
    printf '<NumericField placeholder="8" />\n' > "$tmp/literal.tsx"
    printf 'placeholder={byteFieldPlaceholder(draft.buffer_size, "10")}\n' > "$tmp/helper-literal.tsx"

    ok=1
    check_field_set "$tmp/good.rs" "$tmp/good.ts" >/dev/null 2>&1 || ok=0
    rc=0; check_field_set "$tmp/good.rs" "$tmp/bad.ts" >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0
    check_no_literal_placeholders "$tmp/clean.tsx" >/dev/null 2>&1 || ok=0
    rc=0; check_no_literal_placeholders "$tmp/literal.tsx" >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0
    rc=0; check_no_literal_placeholders "$tmp/helper-literal.tsx" >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0

    if [ "$ok" -eq 1 ]; then
        echo "SELF-TEST OK: field-set diff and literal-placeholder matcher both still fire on synthetic drift."
        exit 0
    fi
    echo "SELF-TEST FAILED: a check did NOT flag a known violation (or rejected a clean fixture) - it is broken."
    exit 1
fi

for f in "$RUST_FILE" "$TS_FILE" "${SECTION_FILES[@]}"; do
    if [ ! -f "$f" ]; then
        echo "error: source not found at $f" >&2
        exit 2
    fi
done

status=0
rc=0; check_field_set "$RUST_FILE" "$TS_FILE" || rc=$?
[ "$rc" -eq 2 ] && exit 2
[ "$rc" -ne 0 ] && status=1
check_no_literal_placeholders "${SECTION_FILES[@]}" || status=1

exit "$status"
