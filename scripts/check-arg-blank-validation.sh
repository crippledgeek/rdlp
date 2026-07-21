#!/usr/bin/env bash
# Verify every string- and path-valued CLI argument rejects blank input.
#
# Why: a blank value reaches the domain layer looking like a real one.
# `--recode-audio=` became `RecodeAudioMode::Encoder { name: "" }` and failed
# inside FFmpeg *after* the download completed; `--cookies="   "` would be a
# path that cannot exist. Validating at the parse boundary means clap reports it
# with the flag name, the offending value and usage text (#540).
#
# clap has no built-in for this and no struct-level hook: `NonEmptyStringValueParser`
# tests `OsStr::is_empty()` only, so `"   "` passes it, and the derive applies a
# `value_parser` per field. The rule therefore has to be attached 38 times, which
# means the real risk is the 39th argument silently omitting it. This gate is
# what makes the invariant enforced rather than remembered.
#
# Usage: scripts/check-arg-blank-validation.sh [--self-test]
#   --self-test: prove the gate still fires, by scanning a synthetic violating
#                file in a temp dir. Runs in CI every time, so "canary-verified"
#                stays true as the tree evolves instead of being a one-off claim.
#
# The parser is deliberately STRICT: the dangerous failure mode is silent. If a
# field declaration cannot be classified, or an `#[arg(...)]` spans multiple
# lines (which would break the contiguous-attribute assumption the same way it
# broke check-ts-enum-drift.sh), the script exits 2 and demands extension rather
# than skipping the field and reporting OK.

set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like the `[a-z_]`
# classes used below are UNSPECIFIED outside the C locale -- they "might fail to
# match any character". A correctness fix, NOT a speed one (measured: no
# difference). Full quote and rationale in #621.
export LC_ALL=C

# Anchor to the repo root: the target path below is relative, so running from
# anywhere else would scan nothing and cheerfully report OK. A gate that passes
# having scanned nothing is worse than no gate.
cd "$(git rev-parse --show-toplevel)" || exit 2

TARGET="crates/rdlp-cli/src/args.rs"

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Scan one file; echo one line per offending field. Shared by the real run and
# the self-test so the self-test exercises the SAME matcher, not a copy of it.
scan() {
    awk '
        # Remember the most recent attribute line, and reset on any other
        # non-blank, non-comment line so an attribute never binds to a field
        # further down the file.
        /^[[:space:]]*#\[arg\(/ {
            attr = $0
            # A multi-line attribute would leave the value_parser on a later
            # line where this matcher cannot see it. Refuse rather than guess.
            if ($0 !~ /\)\][[:space:]]*$/) {
                printf "MULTILINE_ATTR:%d:%s\n", NR, $0
            }
            next
        }
        /^[[:space:]]*(#\[|\/\/|$)/ { next }

        # Field declaration, matched LOOSELY on BOTH the name and the type so an
        # unusual spelling cannot slip past by failing the pattern. The name
        # pattern admits raw identifiers (`r#type`) and non-snake-case: a
        # stricter one skipped them silently, which is the failure direction
        # this gate exists to prevent. Anything that looks like a
        # field is then classified, and an unrecognised type is a hard error --
        # a matcher that silently skips what it does not understand reports OK
        # on exactly the drift it exists to catch.
        /^[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?(r#)?[A-Za-z_][A-Za-z0-9_]*:[[:space:]]/ {
            # The type must terminate on this line, or the type text this
            # matcher classifies is only a fragment of the real one.
            if ($0 !~ /,[[:space:]]*$/) {
                printf "MULTILINE_FIELD:%d:%s\n", NR, $0
                attr = ""
                next
            }

            line = $0
            sub(/^[[:space:]]*/, "", line)
            sub(/^pub([[:space:]]*\([^)]*\))?[[:space:]]+/, "", line)
            idx = index(line, ":")
            name = substr(line, 1, idx - 1)
            type = substr(line, idx + 1)
            gsub(/^[[:space:]]+|[[:space:]]*,[[:space:]]*$/, "", type)

            needs = ""
            if (type ~ /PathBuf/) {
                needs = "non_blank_path"
            } else if (type ~ /String/) {
                # Substring, not equality: this must also catch Vec<String>,
                # Option<Vec<String>> and fully-qualified std::string::String.
                needs = "non_blank"
            } else if (type ~ /^(Option<)?(bool|u8|u16|u32|u64|usize|i8|i16|i32|i64|isize|f32|f64)>?$/) {
                needs = ""      # numeric/flag: blank is impossible by construction
            } else if (type ~ /^(Option<)?(PluginCmd|PluginSubcommand)>?$/) {
                needs = ""      # subcommand: its own fields are checked individually
            } else {
                printf "UNCLASSIFIED:%d:%s:%s\n", NR, name, type
                attr = ""
                next
            }

            if (needs != "" && attr !~ ("value_parser[[:space:]]*=[[:space:]]*" needs "[[:space:]]*[,)]")) {
                printf "MISSING:%d:%s:%s:%s\n", NR, name, type, needs
            }
            attr = ""
            next
        }
        # Residue rule. Widening the prefix pattern above fixes the spellings
        # anticipated today; this catches the ones that are not. Any line that
        # LOOKS like a field declaration (has a colon, ends in a comma) but fell
        # through every rule above is reported rather than dropped, so the next
        # unanticipated spelling fails loudly instead of silently disabling the
        # check for that field. Verified to flag zero lines in the real args.rs.
        /:/ && /,[[:space:]]*$/ { printf "UNPARSED:%d:%s\n", NR, $0; attr = ""; next }
        { attr = "" }
    ' "$1"
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    cat > "$tmp/synthetic.rs" <<'SYNTH'
pub struct Args {
    /// Covered.
    #[arg(long, value_parser = non_blank)]
    pub good: Option<String>,

    /// Not covered — the gate must catch this.
    #[arg(long)]
    pub bad: Option<String>,

    /// Wrong parser for the type — must also be caught.
    #[arg(long, value_parser = non_blank)]
    pub bad_path: Option<PathBuf>,

    /// A repeatable optional arg. Caught only if the String test is a
    /// substring match; exact-equality against a literal list misses it, which
    /// is the false pass this fixture exists to prevent.
    #[arg(long)]
    pub repeatable: Option<Vec<String>>,

    /// Fully-qualified. Same trap, other spelling.
    #[arg(long)]
    pub qualified: Option<std::string::String>,

    /// A raw identifier. Skipped silently by a name pattern that assumes
    /// snake_case, which is how this shape got through the first matcher.
    #[arg(long)]
    pub r#type: Option<String>,

    /// Restricted visibility. `(pub )?` matches neither `pub(crate) ` nor
    /// nothing here, so a narrower prefix pattern dropped this silently —
    /// and tightening a field's visibility is an ordinary refactor.
    #[arg(long)]
    pub(crate) restricted: Option<String>,

    /// Irrelevant types — must be ignored, not flagged.
    #[arg(long)]
    pub flag: bool,
    #[arg(long)]
    pub count: Option<u64>,
    #[command(subcommand)]
    pub sub: Option<PluginSubcommand>,
}
SYNTH
    cat > "$tmp/unclassified.rs" <<'SYNTH'
pub struct Args {
    /// A type this matcher has never seen. It must STOP, not skip.
    #[arg(long)]
    pub exotic: Option<Box<str>>,
}
SYNTH
    cat > "$tmp/unparsed.rs" <<'SYNTH'
pub struct Args {
    /// A declaration spelling no rule anticipates. The residue rule must make
    /// this loud; dropping it is how a field silently escapes the gate.
    #[arg(long)]
    pub odd:String,
}
SYNTH

    found=$(scan "$tmp/synthetic.rs" || true)
    missing=$(printf '%s\n' "$found" | grep '^MISSING:' || true)
    missing_count=$(printf '%s\n' "$missing" | grep -c . || true)
    if [ "$missing_count" -ne 6 ]; then
        echo "self-test FAILED: expected 6 flagged fields (bad, bad_path, repeatable, qualified, r#type, restricted), got $missing_count" >&2
        printf '%s\n' "$found" >&2
        exit 1
    fi
    for expected in bad bad_path repeatable qualified "r#type" restricted; do
        if ! printf '%s\n' "$missing" | grep -q ":${expected}:"; then
            echo "self-test FAILED: matcher missed '$expected'" >&2
            printf '%s\n' "$found" >&2
            exit 1
        fi
    done
    if printf '%s\n' "$found" | grep -qE ':(good|flag|count|sub):'; then
        echo "self-test FAILED: the matcher flagged a compliant or irrelevant field" >&2
        printf '%s\n' "$found" >&2
        exit 1
    fi

    # The strictness claims must be exercised, not asserted.
    if ! scan "$tmp/unclassified.rs" | grep -q '^UNCLASSIFIED:'; then
        echo "self-test FAILED: an unknown field type was silently skipped instead of stopping the gate" >&2
        exit 1
    fi
    if ! scan "$tmp/unparsed.rs" | grep -q '^UNPARSED:'; then
        echo "self-test FAILED: an unrecognised declaration spelling was dropped instead of stopping the gate" >&2
        exit 1
    fi

    echo "SELF-TEST OK: flags unvalidated/mismatched/repeatable/qualified/raw-ident/restricted-visibility args, ignores irrelevant ones, stops on an unclassifiable type or an unparseable declaration"
    exit 0
fi

if [ ! -f "$TARGET" ]; then
    echo "error: $TARGET not found — has the CLI arg module moved?" >&2
    exit 2
fi

findings=$(scan "$TARGET" || true)

if printf '%s\n' "$findings" | grep -q '^MULTILINE_ATTR:'; then
    echo "error: multi-line #[arg(...)] in $TARGET — this matcher reads one attribute line" >&2
    printf '%s\n' "$findings" | grep '^MULTILINE_ATTR:' | while IFS=: read -r _ line rest; do
        echo "       line $line: $rest" >&2
    done
    echo "       Keep the attribute on one line, or extend this script." >&2
    exit 2
fi

if printf '%s\n' "$findings" | grep -q '^MULTILINE_FIELD:'; then
    echo "error: field declaration spans lines in $TARGET — the type this matcher" >&2
    echo "       would classify is only a fragment of the real one" >&2
    printf '%s\n' "$findings" | grep '^MULTILINE_FIELD:' | while IFS=: read -r _ line rest; do
        echo "       line $line: $rest" >&2
    done
    exit 2
fi

if printf '%s\n' "$findings" | grep -q '^UNPARSED:'; then
    echo "error: line looks like a field declaration but matched no rule in $TARGET." >&2
    echo "       Refusing to skip it — an unrecognised declaration spelling is how" >&2
    echo "       a field silently escapes this gate. Extend the matcher." >&2
    printf '%s\n' "$findings" | grep '^UNPARSED:' | while IFS=: read -r _ line rest; do
        printf '       %s:%s %s\n' "$TARGET" "$line" "$rest" >&2
    done
    exit 2
fi

if printf '%s\n' "$findings" | grep -q '^UNCLASSIFIED:'; then
    echo "error: unrecognised field type in $TARGET — refusing to guess whether it" >&2
    echo "       can hold a blank value. Classify it in this script's matcher." >&2
    printf '%s\n' "$findings" | grep '^UNCLASSIFIED:' | while IFS=: read -r _ line name type; do
        printf '       %s:%s  %s: %s\n' "$TARGET" "$line" "$name" "$type" >&2
    done
    exit 2
fi

missing=$(printf '%s\n' "$findings" | grep '^MISSING:' || true)
if [ -n "$missing" ]; then
    echo "error: CLI argument(s) accept blank values — add the value_parser:" >&2
    printf '%s\n' "$missing" | while IFS=: read -r _ line name type needs; do
        printf '       %s:%s  %s: %s  -> #[arg(..., value_parser = %s)]\n' \
            "$TARGET" "$line" "$name" "$type" "$needs" >&2
    done
    echo "       See non_blank / non_blank_path in $TARGET (#540)." >&2
    exit 1
fi

count=$(grep -cE 'value_parser = non_blank(_path)?\)?' "$TARGET" || true)
echo "OK ($count string/path arguments reject blank input)"
