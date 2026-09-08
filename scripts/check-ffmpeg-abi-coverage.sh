#!/usr/bin/env bash
# CI guard: fail if rdlp-ffmpeg calls into an FFmpeg library whose ABI is not
# checked at startup by crates/rdlp-ffmpeg/src/ffmpeg/abi/.
#
# That module compares each library's compile-time LIBAV*_VERSION_MAJOR/MINOR
# against what the loaded object reports, because FFmpeg-8 bindings link cleanly
# against FFmpeg-9 shared objects and then read every struct field at the wrong
# offset (#656). The check is only as complete as its list of libraries, and
# that list is hand-written.
#
# Its unit tests derive their expectations from `FfmpegLibrary::ALL`, and the
# const block there makes a variant missing from `ALL` a compile error. Neither
# can catch the opposite direction: the crate starting to CALL a library that
# has no variant at all. That moves neither side of the comparison, so
# everything stays green.
#
# That direction is not hypothetical. It is exactly what happened with
# libavfilter: `ffi_helpers/filter_graph.rs` writes AVFilterInOut's name,
# filter_ctx, pad_idx and next fields directly, on the live loudnorm/volume
# path, while the module checked only avcodec/avutil/avformat and its own doc
# comment claimed the coverage was complete. libavfilter's major is 11 where
# the others are 62/60/62, so a skew there is neither rare nor bounded.
#
# ---------------------------------------------------------------------------
# WHAT THIS GATE DOES NOT CATCH -- read before trusting it.
#
#   1. BARE `av_*` IS NOT FULLY ATTRIBUTED. The `av_` prefix spans at least
#      four libraries: av_dict_set/av_frame_alloc are libavutil, av_write_frame
#      and av_read_frame are libavformat, av_buffersrc_*/av_buffersink_* are
#      libavfilter, and libavdevice exports av_* too. The unambiguous families
#      are classified individually below; a bare `av_something` outside them is
#      not. Every library `av_*` can mean here is already checked, so this
#      costs nothing today.
#   2. TYPE-ONLY USE. Naming a struct type without calling any function of its
#      library (e.g. reading an AVFilterInOut through a pointer obtained
#      elsewhere) is invisible here. In practice a library's types arrive via
#      its own alloc/init calls, which this does see.
#   3. TRANSITIVE USE. A library reached only through another crate's wrappers,
#      with no `ffi::` mention in this crate, is not seen.
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

ABI_DIR='crates/rdlp-ffmpeg/src/ffmpeg/abi'
SCAN_ROOT='crates/rdlp-ffmpeg/src'

# Unambiguous FFI symbol prefixes, and the library each implies.
#
# The `av_*` families below are each owned by exactly one library despite not
# carrying its name: av_dict/av_frame/av_opt/av_strdup/av_malloc are libavutil
# (dict.h, frame.h, opt.h, mem.h), and av_buffersrc/av_buffersink are
# libavfilter. They matter because libavutil is otherwise reached ONLY through
# such symbols -- `avutil_*` appears nowhere in this crate outside the abi module's
# own version call, so without these libavutil's coverage would be self-referential.
#
# avio_* is libavformat: avio.h ships in libavformat, not a library of its own.
#
# swscale/swresample are listed so that starting to use one is REPORTED rather
# than silently unchecked: today neither appears in the sources, which is why
# the abi module omits them.
PREFIXES='avcodec_:libavcodec
avformat_:libavformat
avio_:libavformat
avfilter_:libavfilter
av_buffersrc_:libavfilter
av_buffersink_:libavfilter
avutil_:libavutil
av_dict_:libavutil
av_frame_:libavutil
av_opt_:libavutil
av_strdup:libavutil
av_malloc:libavutil
avdevice_:libavdevice
sws_:libswscale
swr_:libswresample
postproc_:libpostproc'

# Every shape a symbol can be reached by. The crate's dominant style is the
# short `ffi::` form (measured at the time of writing: avformat_ appears 126
# times short-form against 2 fully-qualified), and muxer_defaults.rs imports
# items bare out of a MULTI-LINE `use ...::ffi::{ ... }` block -- so a matcher
# keyed on the fully-qualified form, or one that is line-based, sees almost
# nothing. `\bffi::` covers the fully-qualified form too, since that string
# contains it.
#
# The import form is matched with `grep -z`, which treats each FILE as one
# record so a `use` block wrapped across lines still matches. `[^;]*` keeps the
# match inside a single `use` statement rather than running to the next one.
#
# `--exclude-dir=abi` is load-bearing, not tidiness: the abi module's own
# `avcodec_version()`/`avutil_version()` calls are the checker asking the
# question, not the crate consuming the library. Counting them would make every
# library look "used" purely because it is checked -- self-referential -- and
# would hide the case where a library's last real consumer goes away.
symbol_used() {
    local prefix=$1 dir=$2
    grep -rqE --exclude-dir=abi "\bffi::${prefix}" "$dir" && return 0
    grep -rqzE --exclude-dir=abi "use[^;]*ffi::\{[^;}]*\b${prefix}" "$dir" && return 0
    return 1
}

# Which libraries does `dir` actually call into?
used_libraries() {
    local prefix library
    while IFS=: read -r prefix library; do
        if symbol_used "$prefix" "$1"; then
            printf '%s\n' "$library"
        fi
    done <<< "$PREFIXES"
}

# Compare the libraries used under `scan_dir` against those named in `abi_dir`,
# reporting any that are used but unchecked. Shared by the real run and the
# self-test, so the self-test exercises the SAME verdict logic rather than a
# copy of it -- the half that decides pass/fail, not just the half that detects.
report_unchecked() {
    local scan_dir=$1 abi_dir=$2 library status=0
    while IFS= read -r library; do
        [ -n "$library" ] || continue
        # Match the DECLARATION -- the `=> "libavutil",` arm of FfmpegLibrary's
        # Display impl -- not merely a mention of the name.
        #
        # A bare `"$library"` search is not equivalent, and this is not
        # hypothetical: deleting the Avutil arm left the gate GREEN, because
        # `tests.rs` asserts `message.contains("libavutil")` and that quoted
        # string satisfied the search. The verdict was reporting "checked" on
        # the strength of a test fixture. Excluding tests.rs as well keeps a
        # future test string from reintroducing it.
        if ! grep -rFq --exclude=tests.rs "=> \"$library\"" "$abi_dir"; then
            echo "ERROR: rdlp-ffmpeg calls into $library, but $abi_dir does not check it."
            echo "       An ABI skew in $library would go unreported, and its struct"
            echo "       layouts would be read at the wrong offsets (#656)."
            echo "       Fix: add an FfmpegLibrary variant for $library, with its"
            echo "       LIBAV*_VERSION_MAJOR / *_version() pair in observe()."
            status=1
        fi
        # `sort -u`: several prefixes map to one library (libavutil has five),
        # and without it a single unchecked library reports once per prefix.
    done < <(used_libraries "$scan_dir" | sort -u)
    return "$status"
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    # DETECTION, one fixture per call shape the crate actually uses. Checking
    # only the fully-qualified form would canary the rarest branch: the crate is
    # overwhelmingly short-form, and muxer_defaults.rs reaches libavcodec purely
    # through a multi-line item import.
    mkdir -p "$tmp/fq" "$tmp/short" "$tmp/import"
    cat > "$tmp/fq/a.rs" <<'FIXTURE'
fn scale(c: *mut SwsContext) { unsafe { ffmpeg_the_third::ffi::sws_scale(c); } }
FIXTURE
    cat > "$tmp/short/b.rs" <<'FIXTURE'
use ffmpeg_the_third::ffi;
fn scale(c: *mut SwsContext) { unsafe { ffi::sws_scale(c); } }
FIXTURE
    cat > "$tmp/import/c.rs" <<'FIXTURE'
use ffmpeg_the_third::ffi::{
    SwsContext, sws_scale,
};
fn scale(c: *mut SwsContext) { unsafe { sws_scale(c); } }
FIXTURE

    for shape in fq short import; do
        if ! used_libraries "$tmp/$shape" | grep -Fxq 'libswscale'; then
            echo "SELF-TEST FAILED: the gate did NOT classify a known sws_ call in the" \
                 "'$shape' form — it is blind to that shape."
            exit 1
        fi
    done

    # VERDICT. Detection alone is not the gate: `report_unchecked` decides
    # pass/fail by asking whether the abi module names the library. If that
    # comparison rots into always-matching -- the directory repointed, the
    # quoting changed, the Display impl refactored so the quoted names move --
    # detection still works and the gate reports OK for an unchecked library.
    # So drive the real verdict against an abi module that names nothing.
    mkdir -p "$tmp/abi-naming-nothing"
    echo '// deliberately names no FFmpeg library' > "$tmp/abi-naming-nothing/mod.rs"
    verdict=$(report_unchecked "$tmp/short" "$tmp/abi-naming-nothing" 2>&1) && verdict_rc=0 || verdict_rc=$?
    if [ "$verdict_rc" -ne 1 ]; then
        echo "SELF-TEST FAILED: the verdict half returned $verdict_rc, not 1, for a used" \
             "but unchecked library — a violation would be reported as a pass."
        exit 1
    fi
    case "$verdict" in
        *"does not check it"*) ;;
        *)
            echo "SELF-TEST FAILED: the verdict half exited 1 without naming the" \
                 "unchecked library, so an operator would not know what to fix."
            exit 1
            ;;
    esac

    # ...and that it stays quiet when the library IS named, so the check above
    # is not passing merely because the verdict always fails.
    mkdir -p "$tmp/abi-naming-swscale"
    echo 'Self::Swscale => "libswscale",' > "$tmp/abi-naming-swscale/mod.rs"
    if ! report_unchecked "$tmp/short" "$tmp/abi-naming-swscale" > /dev/null; then
        echo "SELF-TEST FAILED: the verdict half reported a violation for a library that" \
             "IS named — it would fail the build on correct code."
        exit 1
    fi

    # A MENTION IS NOT A DECLARATION. Measured 2026-09-08: deleting the real
    # Avutil arm left the gate green, because tests.rs asserts
    # `message.contains("libavutil")` and a bare name search accepted it. The
    # verdict must key on the Display arm and ignore tests.rs.
    mkdir -p "$tmp/abi-mentions-only"
    echo '// no Display arm here' > "$tmp/abi-mentions-only/mod.rs"
    echo 'assert!(message.contains("libswscale"));' > "$tmp/abi-mentions-only/tests.rs"
    if report_unchecked "$tmp/short" "$tmp/abi-mentions-only" > /dev/null; then
        echo "SELF-TEST FAILED: the verdict half accepted a library that is only MENTIONED" \
             "in a test, not declared — a deleted variant would report as checked."
        exit 1
    fi

    echo "SELF-TEST OK: the gate still detects all three call shapes, fails an unchecked" \
         "library, and passes a checked one."
    exit 0
fi

[ -d "$ABI_DIR" ] || {
    echo "ERROR: $ABI_DIR not found — cannot tell which libraries are checked." >&2
    exit 2
}
[ -d "$SCAN_ROOT" ] || {
    echo "ERROR: $SCAN_ROOT not found — nothing scanned." >&2
    exit 2
}

report_unchecked "$SCAN_ROOT" "$ABI_DIR"
