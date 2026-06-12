#!/usr/bin/env bash
# check-log-redaction.sh — Semgrep gate (#428): no raw URL in a `log` kv field.
#
# Tier-2 backstop to the type-level RedactedUrl guard. Sanitizer-aware (Semgrep
# `generic` mode), so it distinguishes `url = raw` from `url = RedactedUrl::new(raw)`
# — which the regex gate (check-url-redaction.sh) fundamentally cannot. See
# CODING_RULES.md "URL Redaction — Controls (tiered)".
#
# Runner resolution: prefer an installed `semgrep`; else `uvx semgrep` (ephemeral).
# If neither is available: warn + skip locally (exit 0); CI installs semgrep so it
# runs fail-closed there (CI sets RDLP_REQUIRE_SEMGREP=1 to turn skip into failure).
set -euo pipefail

RULE="scripts/semgrep/log-url-redaction.yml"
SCAN="crates"

if command -v semgrep >/dev/null 2>&1; then
    RUNNER=(semgrep)
elif command -v uvx >/dev/null 2>&1; then
    RUNNER=(uvx semgrep)
else
    if [[ "${RDLP_REQUIRE_SEMGREP:-0}" == "1" ]]; then
        echo "FAIL: semgrep not found and RDLP_REQUIRE_SEMGREP=1 (CI)." >&2
        exit 1
    fi
    echo "SKIP: semgrep/uvx not installed; skipping log-redaction gate." >&2
    echo "      Install: 'uv tool install semgrep' or 'pipx install semgrep'." >&2
    exit 0
fi

echo "==> ${RUNNER[*]} scan ($RULE over $SCAN)"
# --error makes any finding a non-zero exit; --quiet trims banner noise.
"${RUNNER[@]}" --config "$RULE" --error --quiet "$SCAN"
echo "PASS — no raw URL in log kv fields."
