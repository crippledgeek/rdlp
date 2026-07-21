#!/usr/bin/env bash
# CI guard: fail if any production code uses std::process::Command or
# spawns external FFmpeg processes.  Test code is allowed to use
# std::process for non-FFmpeg purposes (e.g. exit codes).
#
# Usage: scripts/check-no-cli.sh

set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like the `[a-z_]`
# classes used below are UNSPECIFIED outside the C locale -- they "might fail to
# match any character". A correctness fix, NOT a speed one (measured: no
# difference). Full quote and rationale in #621.
export LC_ALL=C

# Anchor to the repo root: every path below is relative, so without this the
# gate scans NOTHING and reports success when run from any other directory --
# the same fail-open class as the missing-tool guard. `|| exit 2` distinguishes
# "cannot run" from "gate failed" (exit 1). See #621.
cd "$(git rev-parse --show-toplevel)" || exit 2

FOUND=0

# Allowlist: build-time tooling that legitimately invokes external CLIs
# (NOT FFmpeg / NOT runtime extraction). Each entry must have a comment
# explaining why the exception is justified.
#
# - rdlp-cli/src/plugin_cmd/build_from_ytdlp.rs: invokes componentize-py
#   (Python toolchain) at plugin-build time. Not FFmpeg. Not runtime.
ALLOWLIST_REGEX='^crates/rdlp-cli/src/plugin_cmd/build_from_ytdlp\.rs:'

# Search production source directories only (exclude tests/ and comment lines)
for pattern in 'std::process::Command' 'Command::new("ffmpeg")' 'Command::new("ffprobe")'; do
    matches=$(grep -rn "$pattern" crates/*/src/ 2>/dev/null \
        | grep -v '^\([^:]*:[^:]*:\)\s*//' \
        | grep -Ev "$ALLOWLIST_REGEX" || true)
    if [ -n "$matches" ]; then
        echo "$matches"
        echo "ERROR: Forbidden CLI pattern found: $pattern"
        FOUND=1
    fi
done

if [ "$FOUND" -eq 1 ]; then
    echo ""
    echo "This project enforces pure libav-only FFmpeg usage."
    echo "See crates/rdlp-ffmpeg/src/lib.rs for the CLI Usage Policy."
    exit 1
fi

echo "OK: No CLI usage found in production code."
