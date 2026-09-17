#!/usr/bin/env bash
# CI guard: every encoder open in rdlp-ffmpeg goes through the experimental
# gate (#639).
#
# `codec_registry::enable_experimental_if_flagged(ctx, codec)` is the ONE
# place rdlp decides to set FF_COMPLIANCE_EXPERIMENTAL on an encoder context
# (only when the codec carries AV_CODEC_CAP_EXPERIMENTAL — libavcodec/avcodec.c
# refuses such an encoder at any stricter level). It has to be called before
# `open_as` / `open_as_with` at every encoder-open site; a site that forgets
# it compiles fine and fails only at run time, on a build where the chosen
# encoder happens to be experimental (native dca/vorbis/opus). That is the
# drift class this gate closes: a new open site must call the helper within
# the preceding WINDOW lines or the build fails.
#
# Usage: scripts/check-encoder-open-compliance.sh [--self-test]
#   --self-test: prove the gate still fires, on a synthetic file with an
#                unguarded open_as.

set -euo pipefail
export LC_ALL=C
cd "$(git rev-parse --show-toplevel)" || exit 2

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Lines of context before the open call within which the helper must appear.
# Every current site calls it immediately before; the window only tolerates
# the `.map_err(..)` / comment lines that sit between them.
WINDOW=8
OPEN_RE='\.open_as(_with)?\('
HELPER='enable_experimental_if_flagged'

# Print "file:line" for every open call not preceded by the helper.
scan() {
    local file n start
    while IFS= read -r file; do
        # Decoders open via `.decoder()...open()`; only encoder opens use
        # open_as/open_as_with, so the regex alone selects the right sites.
        # Comment lines are skipped: doc comments legitimately name the call.
        while IFS= read -r n; do
            start=$(( n - WINDOW )); [ "$start" -lt 1 ] && start=1
            if ! sed -n "${start},${n}p" "$file" | grep -q "$HELPER"; then
                printf '%s:%s\n' "$file" "$n"
            fi
        done < <(grep -nE "$OPEN_RE" "$file" | grep -vE '^[0-9]+:[[:space:]]*//' | cut -d: -f1)
    done < <(find "$@" -name '*.rs' -type f)
}

SCAN_ROOTS=(crates/rdlp-ffmpeg/src)

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    # Positive canary: an unguarded open must be flagged.
    mkdir -p "$tmp/bad" "$tmp/good"
    cat > "$tmp/bad/unguarded.rs" <<'FIXTURE'
fn open(ctx: Audio, codec: Codec) -> Result<Encoder> {
    ctx.open_as(codec)
}
FIXTURE
    # Negative canary: a guarded open, and a doc comment naming the call,
    # must NOT be flagged — otherwise a scanner that flags everything would
    # pass the positive check.
    cat > "$tmp/good/guarded.rs" <<'FIXTURE'
/// Call before `.open_as()`.
fn open(mut ctx: Audio, codec: Codec) -> Result<Encoder> {
    crate::ffmpeg::codec_registry::enable_experimental_if_flagged(&mut ctx, codec);
    ctx.open_as(codec)
}
FIXTURE
    if [ -n "$(scan "$tmp/bad")" ] && [ -z "$(scan "$tmp/good")" ]; then
        echo "SELF-TEST OK: the gate flags an unguarded encoder open and passes a guarded one."
        exit 0
    fi
    echo "SELF-TEST FAILED: the gate misjudged a known fixture — it is broken."
    exit 1
fi

file_count=$(find "${SCAN_ROOTS[@]}" -name '*.rs' -type f | wc -l)
if [ "$file_count" -eq 0 ]; then
    echo "ERROR: found no .rs files under crates/rdlp-ffmpeg/src — refusing to report OK."
    exit 2
fi

violations=$(scan "${SCAN_ROOTS[@]}")
if [ -n "$violations" ]; then
    echo "ERROR: encoder opened without codec_registry::enable_experimental_if_flagged (#639):"
    printf '%s\n' "$violations" | sed 's/^/  /'
    echo "Call the helper on the context immediately before open_as/open_as_with."
    exit 1
fi
echo "OK: every encoder open in rdlp-ffmpeg ($file_count files) is preceded by the experimental gate."
