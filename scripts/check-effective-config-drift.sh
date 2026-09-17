#!/usr/bin/env bash
# Verify the desktop's mirrors of the engine's "effective" (materialised)
# config structs, and their consumers, have not drifted from the Rust owners
# of the defaults (#611).
#
# Four struct/interface pairs today: `EffectiveNetwork` (the nine
# network/download defaults), `EffectiveNormalize` (the preset-dependent
# normalization defaults), and the preset catalogue `LoudnormPresetInfo` /
# `LoudnormTargets` served to the preset picker. Add a row to PAIRS for the
# next one.
#
# Two invariants:
#
#  (a) FIELD SET — the TS `export interface <Name>` in
#      crates/rdlp-desktop/src/types/index.ts has exactly the field names of
#      `pub struct <Name>` in its Rust file (serde default naming: the Rust
#      identifier IS the wire key). Adding a Rust field does not break the TS
#      build — it just ships a key the TS type says does not exist, and a
#      placeholder that can never be derived from it. Every Rust field must
#      also be CONCRETE (no `Option<_>`/generics): this is the materialised
#      layer, and the GUI treats its values as final. A TS field is `number`
#      or a named union type (`LoudnormPreset`); an optional `?` is drift.
#
#  (b) NO LITERAL PLACEHOLDERS — the settings sections listed in
#      SECTION_FILES must not carry a numeric placeholder literal:
#      `placeholder="8"`, `placeholder: "-14.0"` in a field table, or a
#      `byteFieldPlaceholder(x, "10")`-style trailing string argument. Every
#      placeholder derives from the IPC-sourced payload; a literal is another
#      copy of a default, which is exactly the drift #611 removed — and for the
#      loudnorm targets the copy was Streaming-only, so it was WRONG under any
#      other preset.
#
# The Rust side is parsed from the struct body itself, never a hand-copied
# list, so the gate cannot silently agree with a stale mirror. Both parsers are
# STRICT: a body line they cannot classify exits 2 (extend the script) rather
# than being skipped — a skipped field on both sides would let the sets match
# on the very drift this exists to catch.
#
# Fix when this fails: (a) edit the TS interface to match the reported Rust
# field set; (b) replace the literal with `String(payload.<field>)` (or
# `payload.<field>` for the byte-field helpers).
#
# Usage: scripts/check-effective-config-drift.sh [--self-test]
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

TS_FILE="crates/rdlp-desktop/src/types/index.ts"

# rust_file:struct_name — the TS interface carries the same name.
PAIRS=(
    "crates/rdlp-types/src/effective_network.rs:EffectiveNetwork"
    "crates/rdlp-types/src/effective_normalize.rs:EffectiveNormalize"
    "crates/rdlp-types/src/loudnorm_preset.rs:LoudnormTargets"
    "crates/rdlp-types/src/loudnorm_preset.rs:LoudnormPresetInfo"
)

SECTION_FILES=(
    "crates/rdlp-desktop/src/views/settings/sections/DownloadSection.tsx"
    "crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx"
    "crates/rdlp-desktop/src/views/settings/sections/NormalizationSection.tsx"
)

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Field identifiers of `pub struct <$2> {` in $1, sorted.
rust_fields() {
    awk -v file="$1" -v name="$2" -v decl="pub struct $2 {" '
        index($0, decl) == 1 { inside = 1; next }
        inside && /^}/ { exit }
        inside && /^[[:space:]]*$/ { next }                 # blank
        inside && /^[[:space:]]*\/\// { next }              # `//` or `///`
        inside && /^[[:space:]]*#\[/ {
            if ($0 ~ /serde/) {
                printf "error: per-field serde attribute in %s (%s):\n  %s\n", name, file, $0 > "/dev/stderr"
                print  "       the wire key no longer follows from the identifier. Extend the script." > "/dev/stderr"
                exit 2
            }
            next
        }
        inside && /^    pub [a-z_][a-z0-9_]*: [A-Za-z0-9_]+,$/ {  # concrete type only
            sub(/^    pub /, ""); sub(/:.*$/, "")
            print
            next
        }
        inside && /^    pub [a-z_][a-z0-9_]*: .*[<>].*,$/ {
            # A generic (`Option<u64>`, `Vec<_>`) is a CONTRACT violation, not a
            # parser gap: the effective struct is the materialised layer, every
            # field concrete. Exit 1 (gate failed) naming the field.
            field = $0; sub(/^    pub /, "", field); sub(/:.*$/, "", field)
            printf "\nERROR: %s.%s in %s has a generic type:\n  %s\n", name, field, file, $0 > "/dev/stderr"
            printf "%s is the resolved layer -- every field is a concrete value.\n", name > "/dev/stderr"
            print  "An Option here would put \"inherit\" back into the payload the GUI treats as final." > "/dev/stderr"
            exit 1
        }
        inside {
            printf "error: unparseable line in %s (%s):\n  %s\n", name, file, $0 > "/dev/stderr"
            print  "       Expected `pub <snake_ident>: <Type>,`. Extend this script." > "/dev/stderr"
            exit 2
        }
    ' "$1" | sort
}

# Property identifiers of `export interface <$2> {` in $1, sorted.
ts_fields() {
    awk -v file="$1" -v name="$2" -v decl="export interface $2 {" '
        index($0, decl) == 1 { inside = 1; next }
        inside && /^}/ { exit }
        inside && /^[[:space:]]*$/ { next }                        # blank
        inside && /^[[:space:]]*(\/\/|\/\*|\*)/ { next }           # `//`, `/**`, ` *`, ` */`
        inside && /^    [a-z_][a-z0-9_]*: (number|[A-Z][A-Za-z0-9]*);$/ {
            sub(/^    /, ""); sub(/:.*$/, "")
            print
            next
        }
        inside {
            printf "error: unparseable line in TS %s (%s):\n  %s\n", name, file, $0 > "/dev/stderr"
            print  "       Expected `<snake_ident>: number;` or `<snake_ident>: <UnionType>;`" > "/dev/stderr"
            print  "       (optional `?` is drift too: every Rust field is concrete). Extend this script." > "/dev/stderr"
            exit 2
        }
    ' "$1" | sort
}

# (a) Compare the two field sets for one struct. 0 = match, 1 = drift, 2 = cannot parse.
check_field_set() {
    local rust_file=$1 ts_file=$2 name=$3 rust_set ts_set diff_out rc
    # awk's non-zero exit must propagate, not be swallowed by the assignment
    # (pipefail): 1 = a generic field (gate failed), 2 = unparseable (cannot run).
    rc=0; rust_set=$(rust_fields "$rust_file" "$name") || rc=$?
    [ "$rc" -ne 0 ] && return "$rc"
    if ! ts_set=$(ts_fields "$ts_file" "$name"); then return 2; fi
    if [ -z "$rust_set" ]; then
        echo "error: parsed zero fields from $name in $rust_file" >&2
        return 2
    fi
    if [ -z "$ts_set" ]; then
        echo "error: parsed zero fields from TS interface $name in $ts_file" >&2
        return 2
    fi
    if ! diff_out=$(diff <(echo "$rust_set") <(echo "$ts_set")); then
        cat <<EOM >&2

ERROR: $name ($rust_file) and the TypeScript interface ($ts_file)
disagree. '<' lines are Rust-only (missing from TS); '>' lines are TS-only
(no such Rust field).

$diff_out
EOM
        return 1
    fi
    echo "$name ↔ TS interface OK ($(echo "$rust_set" | wc -l) fields)"
}

# (b) Reject numeric placeholder literals in the given section files.
# 0 = clean, 1 = a literal was found.
check_no_literal_placeholders() {
    local hits
    # `|| true`: grep exits 1 on zero matches, which is the PASSING case here.
    # Three shapes: a JSX attribute literal, a `placeholder:` object-field
    # literal (the loudnorm target table), and a helper's trailing string arg.
    hits=$(grep -nE 'placeholder="-?[0-9.]+"|placeholder: "-?[0-9.]+"|, "-?[0-9.]+"\)' "$@" || true)
    if [ -n "$hits" ]; then
        cat <<EOM >&2

ERROR: numeric placeholder literal(s) in the settings sections. Every
placeholder must derive from the IPC-sourced effective-config payload
(\`String(payload.<field>)\`), never a literal copy of a default (#611):

$hits
EOM
        return 1
    fi
    echo "no literal placeholders in ${#@} section file(s) OK"
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    # Matching pair — the parsers must accept the real layout (doc comments,
    # a JSDoc block, a number and a union-typed field).
    cat > "$tmp/good.rs" <<'FIXTURE'
pub struct EffectiveThing {
    /// Doc comment.
    pub alpha_secs: u64,

    pub beta: usize,
    pub preset: SomePreset,
}
FIXTURE
    cat > "$tmp/good.ts" <<'FIXTURE'
export interface EffectiveThing {
    /** JSDoc. */
    alpha_secs: number;
    beta: number;
    preset: SomePreset;
}
FIXTURE
    # Contract violation — a Rust field that is not concrete. Every field of an
    # effective struct is a resolved value by contract; an `Option` would put
    # "inherit" back into the payload the GUI treats as final.
    cat > "$tmp/optional.rs" <<'FIXTURE'
pub struct EffectiveThing {
    pub alpha_secs: Option<u64>,
    pub beta: usize,
    pub preset: SomePreset,
}
FIXTURE
    # Drifted pair — TS renamed one field.
    cat > "$tmp/bad.ts" <<'FIXTURE'
export interface EffectiveThing {
    alpha_secs: number;
    gamma: number;
    preset: SomePreset;
}
FIXTURE
    # Optional TS field — `?` is drift (cannot run: the parser rejects it).
    cat > "$tmp/optional.ts" <<'FIXTURE'
export interface EffectiveThing {
    alpha_secs: number;
    beta?: number;
    preset: SomePreset;
}
FIXTURE
    # Section fixtures: clean, a bare literal, a negative-float object-field
    # literal, and a helper-argument literal.
    printf '<NumericField placeholder={String(defaults.alpha_secs)} />\n{ placeholder: String(effective.target_i) }\n' > "$tmp/clean.tsx"
    printf '<NumericField placeholder="8" />\n' > "$tmp/literal.tsx"
    printf '{ id: "x", placeholder: "-14.0" },\n' > "$tmp/field-literal.tsx"
    printf 'placeholder={byteFieldPlaceholder(draft.buffer_size, "10")}\n' > "$tmp/helper-literal.tsx"

    ok=1
    check_field_set "$tmp/good.rs" "$tmp/good.ts" EffectiveThing >/dev/null 2>&1 || ok=0
    rc=0; check_field_set "$tmp/good.rs" "$tmp/bad.ts" EffectiveThing >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0
    rc=0; check_field_set "$tmp/optional.rs" "$tmp/good.ts" EffectiveThing >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0
    rc=0; check_field_set "$tmp/good.rs" "$tmp/optional.ts" EffectiveThing >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 2 ] || ok=0
    check_no_literal_placeholders "$tmp/clean.tsx" >/dev/null 2>&1 || ok=0
    for f in literal field-literal helper-literal; do
        rc=0; check_no_literal_placeholders "$tmp/$f.tsx" >/dev/null 2>&1 || rc=$?
        [ "$rc" -eq 1 ] || ok=0
    done

    if [ "$ok" -eq 1 ]; then
        echo "SELF-TEST OK: field-set diff and literal-placeholder matcher both still fire on synthetic drift."
        exit 0
    fi
    echo "SELF-TEST FAILED: a check did NOT flag a known violation (or rejected a clean fixture) - it is broken."
    exit 1
fi

for f in "$TS_FILE" "${SECTION_FILES[@]}"; do
    if [ ! -f "$f" ]; then
        echo "error: source not found at $f" >&2
        exit 2
    fi
done

status=0
for pair in "${PAIRS[@]}"; do
    IFS=: read -r rust_file name <<<"$pair"
    if [ ! -f "$rust_file" ]; then
        echo "error: Rust source not found at $rust_file" >&2
        exit 2
    fi
    rc=0; check_field_set "$rust_file" "$TS_FILE" "$name" || rc=$?
    [ "$rc" -eq 2 ] && exit 2
    [ "$rc" -ne 0 ] && status=1
done
check_no_literal_placeholders "${SECTION_FILES[@]}" || status=1

exit "$status"
