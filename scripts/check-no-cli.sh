#!/bin/bash
# CI guard: fail if any production code uses std::process::Command or
# spawns external FFmpeg processes.  Test code is allowed to use
# std::process for non-FFmpeg purposes (e.g. exit codes).
#
# Usage: scripts/check-no-cli.sh

set -euo pipefail

FOUND=0

# Search production source directories only (exclude tests/)
for pattern in 'std::process::Command' 'Command::new("ffmpeg")' 'Command::new("ffprobe")'; do
    if grep -rn "$pattern" crates/*/src/ 2>/dev/null; then
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
