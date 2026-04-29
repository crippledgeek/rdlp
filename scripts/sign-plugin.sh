#!/usr/bin/env bash
# Sign an rdlp plugin manifest with a local Ed25519 key.
#
# Usage: scripts/sign-plugin.sh <wasm_path> <manifest_template_path>
#
# Generates an Ed25519 keypair at ~/.config/rdlp/keys/dev.ed25519 if missing
# (PEM/PKCS8). Invokes the example-sign-plugin Rust helper to perform the actual
# signing (canonical-bytes serialization MUST match rdlp_plugin::manifest exactly,
# so we share the implementation rather than duplicate it in shell).
#
# The signed plugin.toml is printed to stdout.

set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 <wasm_path> <manifest_template_path>" >&2
    exit 2
fi

WASM_PATH="$(realpath "$1")"
MANIFEST_PATH="$(realpath "$2")"

KEY_DIR="${RDLP_KEY_DIR:-${HOME}/.config/rdlp/keys}"
KEY_PATH="${KEY_DIR}/dev.ed25519"
mkdir -p "$KEY_DIR"
chmod 700 "$KEY_DIR"

if [[ ! -f "$KEY_PATH" ]]; then
    echo "generating new Ed25519 keypair at $KEY_PATH" >&2
    openssl genpkey -algorithm ed25519 -out "$KEY_PATH" >/dev/null
    chmod 600 "$KEY_PATH"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SIGNER_DIR="${SCRIPT_DIR}/../examples/plugins/example-extractor/tools/sign-plugin"

# Build once (silent unless errors), then invoke. --quiet keeps stdout clean for
# the printed TOML; cargo's progress goes to stderr.
cargo build --release --manifest-path "${SIGNER_DIR}/Cargo.toml" --quiet >&2
SIGNER_BIN="${SIGNER_DIR}/target/release/example-sign-plugin"

exec "$SIGNER_BIN" "$WASM_PATH" "$MANIFEST_PATH" "$KEY_PATH"
