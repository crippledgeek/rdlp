#!/usr/bin/env bash
# CI guard: fail if rdlp-ffmpeg calls into an FFmpeg library whose ABI is not
# checked at startup by crates/rdlp-ffmpeg/src/ffmpeg/abi.rs.
#
# abi.rs compares each library's compile-time LIBAV*_VERSION_MAJOR against the
# major the loaded object reports, because FFmpeg-8 bindings link cleanly
# against FFmpeg-9 shared objects and then read every struct field at the wrong
# offset (#656). That check is only as complete as its list of libraries, and
# the list is hand-written.
#
# The unit test beside it (`every_library_this_crate_reads_is_observed`) derives
# its expectation from `FfmpegLibrary::ALL`, so it catches a library being
# dropped from the checked set. It cannot catch the opposite direction: the
# crate starting to call a library that has no variant at all. That moves
# neither side of the comparison, so the test stays green.
#
# That direction is not hypothetical. It is exactly what happened with
# libavfilter: `ffi_helpers/filter_graph.rs` writes AVFilterInOut's name,
# filter_ctx, pad_idx and next fields directly, on the live loudnorm/volume
# path, while abi.rs checked only avcodec/avutil/avformat and its own doc
# comment claimed the coverage was complete. libavfilter's major is 11 where
# the others are 62/60/62, so a skew there is neither rare nor bounded.
#
# ---------------------------------------------------------------------------
# WHAT THIS GATE DOES NOT CATCH -- read before trusting it.
#
#   1. `av_*`-PREFIXED SYMBOLS ARE NOT ATTRIBUTED. av_strdup and av_dict_set
#      are libavutil, but av_write_frame and av_read_frame are libavformat --
#      the prefix does not say which library a symbol belongs to. Only the
#      unambiguous library-named prefixes below are classified. Both libraries
#      that `av_*` can mean are already checked, so this costs nothing today;
#      it would matter if a future library also exported bare `av_*` symbols.
#   2. TYPE-ONLY USE. Naming a struct type without calling any function of its
#      library (e.g. reading an AVFilterInOut through a pointer obtained
#      elsewhere) is invisible here. In practice a library's types arrive via
#      its own alloc/init calls, which this does see.
#   3. TRANSITIVE USE. A library reached only through another crate's wrappers,
#      with no `ffmpeg_the_third::ffi::` mention in this crate, is not seen.
#
# Treat this as a guard against the specific drift that bit us -- a new direct
# FFI dependency on an unchecked library -- not as proof of completeness.
# ---------------------------------------------------------------------------
#
# Usage: scripts/check-ffmpeg-abi-coverage.sh [--self-test]
#   --self-test: prove the gate still fires, by scanning a synthetic source
#                file that calls an unchecked library. Runs in CI every time,
#                so "canary-verified" stays true as the tree evolves.

set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like the `[a-z_]`
# classes used below are UNSPECIFIED outside the C locale -- they "might fail to
# match any character". A correctness fix, NOT a speed one. See #621.
export LC_ALL=C

# Anchor to the repo root. The paths below are relative: run from anywhere else
# they resolve to nothing, the scan sees zero files, and the gate would
# cheerfully report OK. A gate that passes having scanned nothing is worse than
# no gate, so this is load-bearing, not tidiness.
cd "$(git rev-parse --show-toplevel)" || exit 2

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

ABI_RS='crates/rdlp-ffmpeg/src/ffmpeg/abi.rs'
SCAN_ROOT='crates/rdlp-ffmpeg/src'

# Library-named FFI symbol prefixes, and the library each implies. `av_*` is
# deliberately absent -- see limitation 1 above.
#
# swscale/swresample are listed so that starting to use one is REPORTED rather
# than silently unchecked: today neither appears in the sources, which is why
# abi.rs omits them.
PREFIXES='avcodec_:libavcodec
avformat_:libavformat
avfilter_:libavfilter
avdevice_:libavdevice
sws_:libswscale
swr_:libswresample
postproc_:libpostproc'

# Which libraries does `dir` actually call into?
used_libraries() {
    local prefix library
    while IFS=: read -r prefix library; do
        if grep -rqE "ffmpeg_the_third::ffi::${prefix}" "$1"; then
            printf '%s\n' "$library"
        fi
    done <<< "$PREFIXES"
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    # A library abi.rs does not name, called the way the real code calls one.
    cat > "$tmp/scaler.rs" <<'FIXTURE'
fn scale(ctx: *mut SwsContext) {
    unsafe { ffmpeg_the_third::ffi::sws_scale(ctx); }
}
FIXTURE
    if used_libraries "$tmp" | grep -Fxq 'libswscale'; then
        echo "SELF-TEST OK: the gate still detects a call into an unchecked library."
        exit 0
    fi
    echo "SELF-TEST FAILED: the gate did NOT classify a known sws_ call — it is broken."
    exit 1
fi

[ -f "$ABI_RS" ] || {
    echo "ERROR: $ABI_RS not found — cannot tell which libraries are checked." >&2
    exit 2
}
[ -d "$SCAN_ROOT" ] || {
    echo "ERROR: $SCAN_ROOT not found — nothing scanned." >&2
    exit 2
}

status=0
while IFS= read -r library; do
    [ -n "$library" ] || continue
    # abi.rs names each checked library in its FfmpegLibrary Display impl.
    if ! grep -Fq "\"$library\"" "$ABI_RS"; then
        echo "ERROR: rdlp-ffmpeg calls into $library, but $ABI_RS does not check it."
        echo "       An ABI skew in $library would go unreported, and its struct"
        echo "       layouts would be read at the wrong offsets (#656)."
        echo "       Fix: add an FfmpegLibrary variant for $library, with its"
        echo "       LIBAV*_VERSION_MAJOR / *_version() pair in observe()."
        status=1
    fi
done < <(used_libraries "$SCAN_ROOT")

exit "$status"
