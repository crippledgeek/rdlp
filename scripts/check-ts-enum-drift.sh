#!/usr/bin/env bash
# Verify the desktop TypeScript string-literal unions still match the Rust enums
# they mirror across the Tauri IPC boundary.
#
# Why: `crates/rdlp-desktop/src/types/index.ts` hand-mirrors several
# `#[serde(rename_all = "lowercase")]` enums. Adding a Rust variant does not
# break the TS build — it just lets Rust serialize a value the TS type declares
# impossible. That is a silent contract mismatch, and it is exactly what
# happened when `ContainerFormat::M4v` landed in #534 (see #542).
#
# The Rust side is derived from the enum body itself (variant identifiers,
# lowercased — what `rename_all = "lowercase"` produces), never from a
# hand-copied list, so the check cannot silently agree with a stale mirror.
#
# The parser is deliberately STRICT, because the dangerous failure mode is a
# silent one: if a variant it cannot classify were skipped, and the TS union
# were equally incomplete, the sets would match and the gate would print OK on
# the very drift it exists to catch. So every line inside an enum body must be
# blank, a comment, an attribute, or a plain unit variant — anything else
# (tuple/struct payload, explicit discriminant, per-variant `serde(rename)`)
# exits 2 and demands this script be extended.
#
# Fix when this fails: edit the union in index.ts to match the reported Rust set.
#
# Run from the repository root.

set -euo pipefail

TS_FILE="crates/rdlp-desktop/src/types/index.ts"

# rust_file:rust_enum:ts_union — each enum's `rename_all = "lowercase"` is
# asserted below, not assumed.
PAIRS=(
    "crates/rdlp-types/src/container.rs:ContainerFormat:ContainerFormat"
    "crates/rdlp-types/src/audio_format.rs:AudioFormat:AudioFormat"
    "crates/rdlp-types/src/subtitle_format.rs:SubtitleFormat:SubtitleFormat"
    "crates/rdlp-desktop/src-tauri/src/state/download_queue.rs:JobStatus:JobStatus"
)

if [ ! -f "$TS_FILE" ]; then
    echo "error: TypeScript type file not found at $TS_FILE" >&2
    exit 2
fi

# Variant identifiers of `enum <name>`, lowercased, one per line, sorted.
#
# Also asserts the two properties that make "lowercased identifier" equal to the
# serde wire value: the enum carries `rename_all = "lowercase"`, and no variant
# overrides it with its own `serde(rename = ...)`.
rust_variants() {
    local file=$1 name=$2
    awk -v enum_decl="pub enum $name {" -v enum_name="$name" -v file="$file" '
        index($0, enum_decl) == 1 {
            if (attrs !~ /rename_all[[:space:]]*=[[:space:]]*"lowercase"/) {
                printf "error: %s in %s is not #[serde(rename_all = \"lowercase\")];\n", enum_name, file > "/dev/stderr"
                print  "       this script maps variants to wire values by lowercasing the identifier," > "/dev/stderr"
                print  "       which is only correct under that attribute. Extend the script." > "/dev/stderr"
                exit 2
            }
            inside = 1
            next
        }

        # Attributes/doc comments IMMEDIATELY preceding the decl. Anything else
        # non-blank (a previous item body, a closing brace) clears the block —
        # "since the last blank line" would let a prior enum s rename_all bleed
        # into this one s assertion, which is the silent-wrong the check exists
        # to prevent.
        !inside && /^[[:space:]]*(\/\/|#\[)/ { attrs = attrs "\n" $0; next }
        !inside && /^[[:space:]]*$/          { next }
        !inside                              { attrs = ""; next }

        inside && /^}/ { exit }
        inside && /^[[:space:]]*$/ { next }          # blank line
        inside && /^[[:space:]]*\/\// { next }       # `//` or `///` comment

        inside && /^[[:space:]]*#\[/ {
            if ($0 ~ /serde[[:space:]]*\([^)]*(rename[[:space:]]*=|skip)/) {
                printf "error: %s in %s has a per-variant #[serde(rename = ...)] or #[serde(skip)]:\n  %s\n", enum_name, file, $0 > "/dev/stderr"
                print  "       rename: the wire value no longer follows from the identifier." > "/dev/stderr"
                print  "       skip:   the variant has no wire value at all, so counting it would" > "/dev/stderr"
                print  "               demand a TS member that can never be produced." > "/dev/stderr"
                print  "       Extend the script." > "/dev/stderr"
                exit 2
            }
            next                                     # any other attribute
        }

        inside && /^    [A-Z][A-Za-z0-9]*,$/ {       # plain unit variant
            gsub(/^ +|,$/, "")
            print tolower($0)
            next
        }

        inside {
            printf "error: unparseable line in %s (%s):\n  %s\n", enum_name, file, $0 > "/dev/stderr"
            print  "       Expected a plain unit variant. A tuple/struct variant, an explicit" > "/dev/stderr"
            print  "       discriminant, or an unusual layout is NOT safely comparable against a" > "/dev/stderr"
            print  "       TS string union — skipping it silently could let real drift pass." > "/dev/stderr"
            print  "       Extend this script to handle it." > "/dev/stderr"
            exit 2
        }
    ' "$file" | sort
}

# Quoted members of `export type <name> =` up to the terminating semicolon.
ts_union_members() {
    local name=$1
    awk -v decl="export type $name =" '
        index($0, decl) == 1 { inside = 1 }
        inside {
            while (match($0, /"[^"]+"/)) {
                print substr($0, RSTART + 1, RLENGTH - 2)
                $0 = substr($0, RSTART + RLENGTH)
            }
            if (/;/) exit
        }
    ' "$TS_FILE" | sort
}

status=0

for pair in "${PAIRS[@]}"; do
    IFS=: read -r rust_file rust_enum ts_union <<<"$pair"

    if [ ! -f "$rust_file" ]; then
        echo "error: Rust source not found at $rust_file" >&2
        exit 2
    fi

    # awk's exit 2 must abort the run, not be swallowed by the assignment.
    # This depends on `pipefail` (line 27): awk is piped into `sort`, so without
    # it the pipeline would report sort's 0 and the guard would be a no-op.
    if ! rust_set=$(rust_variants "$rust_file" "$rust_enum"); then
        exit 2
    fi
    ts_set=$(ts_union_members "$ts_union")

    if [ -z "$rust_set" ]; then
        echo "error: parsed zero variants from $rust_enum in $rust_file" >&2
        exit 2
    fi
    if [ -z "$ts_set" ]; then
        echo "error: parsed zero members from TS union $ts_union in $TS_FILE" >&2
        exit 2
    fi

    if ! diff_out=$(diff <(echo "$rust_set") <(echo "$ts_set")); then
        status=1
        cat <<EOF >&2

ERROR: $rust_enum ($rust_file) and the TypeScript union $ts_union ($TS_FILE)
disagree. '<' lines are Rust-only (missing from TS); '>' lines are TS-only
(no such Rust variant).

$diff_out
EOF
    else
        echo "$rust_enum ↔ TS $ts_union OK ($(echo "$rust_set" | wc -l) variants)"
    fi
done

exit "$status"
