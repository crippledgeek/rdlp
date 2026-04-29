#!/usr/bin/env bash
# Populates the sigstore plugin manifest template with the OIDC identity,
# issuer, and base64-encoded Sigstore bundle. Used by the plugin-release
# GitHub Actions workflow.
#
# usage: embed-sigstore-bundle.sh <template> <bundle.sigstore> <identity> <issuer>
#
# Prints the populated TOML to stdout.

set -euo pipefail

if [[ $# -ne 4 ]]; then
    echo "usage: $0 <template> <bundle.sigstore> <identity> <issuer>" >&2
    exit 2
fi

TEMPLATE="$1"
BUNDLE_FILE="$2"
IDENTITY="$3"
ISSUER="$4"

if [[ ! -f "$TEMPLATE" ]]; then
    echo "error: template not found: $TEMPLATE" >&2
    exit 1
fi
if [[ ! -f "$BUNDLE_FILE" ]]; then
    echo "error: bundle not found: $BUNDLE_FILE" >&2
    exit 1
fi

BUNDLE_B64=$(base64 -w0 < "$BUNDLE_FILE")

python3 - "$TEMPLATE" "$IDENTITY" "$ISSUER" "$BUNDLE_B64" <<'PY'
import sys

template_path, identity, issuer, bundle_b64 = sys.argv[1:5]
with open(template_path, "r", encoding="utf-8") as fh:
    text = fh.read()

text = text.replace("PLACEHOLDER_IDENTITY", identity)
text = text.replace("PLACEHOLDER_ISSUER", issuer)
text = text.replace("PLACEHOLDER_BUNDLE", bundle_b64)

sys.stdout.write(text)
PY
