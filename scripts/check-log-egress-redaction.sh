#!/usr/bin/env bash
# CI guard: every production log egress redacts the rendered record (#684).
#
# The ~100 `warn!("…: {e}")` sites are inert only because the SINK redacts:
# the CLI's `SuspendingWriter` writes through `rdlp_redact::redact_bytes` and
# the desktop's tauri-plugin-log root formatter renders through
# `rdlp_redact::redact_str`. A new sink — a third binary, an extra
# `with_writer`, an `env_logger` — would be an unredacted egress for all of
# them at once. This gate pins the two known sinks and refuses any other
# production sink, so "site 98" cannot be a new sink.
#
# Three checks:
#   1. rdlp-cli's `impl std::io::Write for SuspendingWriter` calls
#      `redact_bytes` (via `write_redacted`).
#   2. rdlp-desktop's tauri-plugin-log builder has a root `.format(` that
#      calls `redact_str`, and no `.clear_format()` (which would drop it).
#   3. No other logger sink is installed in production code
#      (`with_writer(`, `env_logger`, `set_boxed_logger`, `set_logger(`,
#      `TargetKind::Stdout|Stderr`) — test modules and `tests/` excluded.
#
# Usage: scripts/check-log-egress-redaction.sh [--self-test]

set -euo pipefail
export LC_ALL=C
cd "$(git rev-parse --show-toplevel)" || exit 2

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

CLI=crates/rdlp-cli/src/main.rs
DESKTOP=crates/rdlp-desktop/src-tauri/src/lib.rs

# Production code only: drop everything from the first `#[cfg(test)]` on.
prod() { sed '/#\[cfg(test)\]/,$d' "$1"; }

# Check 1 — CLI writer. Returns 0 when the writer impl's body mentions the
# redaction helper within 12 lines of the impl header.
cli_sink_redacts() {
    local file=$1 n
    n=$(grep -n 'impl std::io::Write for SuspendingWriter' "$file" | cut -d: -f1 | head -1)
    [ -n "$n" ] || return 1
    sed -n "${n},$((n + 12))p" "$file" | grep -qE 'write_redacted|redact_bytes'
}

# Check 2 — desktop root formatter.
desktop_root_redacts() {
    local file=$1
    ! prod "$file" | grep -q '\.clear_format()' || return 1
    # The root `.format(` is the one at the Builder level (indented less than
    # the per-target ones); require any `.format(` whose next 3 lines call
    # redact_str/redacted_message.
    local n hits=0
    # No `| grep -q` on the loop: under pipefail an early exit would SIGPIPE
    # the producer and report the check as failed.
    while read -r n; do
        if sed -n "${n},$((n + 3))p" "$file" | grep -qE 'redact_str|redacted_message'; then
            hits=$((hits + 1))
        fi
    done < <(prod "$file" | grep -nE '^\s*\.format\(' | cut -d: -f1)
    [ "$hits" -gt 0 ]
}

# Check 3 — no other production sink. `rdlp-extractor/src/log_capture.rs`
# is the `test-support` capture logger (feature-gated out of every release
# binary by check-test-only-features-not-in-release.sh), not a sink.
ALLOWLIST='crates/rdlp-extractor/src/log_capture.rs'
other_sinks() {
    local file
    while IFS= read -r file; do
        printf '%s\n' "$ALLOWLIST" | grep -Fxq "$file" && continue
        prod "$file" | grep -nE 'with_writer\(|env_logger|set_boxed_logger|[^_]set_logger\(|TargetKind::(Stdout|Stderr)' \
            | grep -vE '^\s*[0-9]+:\s*//' | sed "s|^|$file:|"
    done < <(find "$@" -name '*.rs' -type f)
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    cat > "$tmp/bad_cli.rs" <<'FIXTURE'
impl std::io::Write for SuspendingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::io::stderr().write(buf)
    }
}
FIXTURE
    cat > "$tmp/bad_desktop.rs" <<'FIXTURE'
fn run() {
    tauri_plugin_log::Builder::new()
        .clear_format()
        .build();
}
FIXTURE
    cat > "$tmp/bad_desktop_format.rs" <<'FIXTURE'
fn run() {
    tauri_plugin_log::Builder::new()
        .format(|out, message, _record| {
            out.finish(format_args!("{message}"));
        })
        .build();
}
FIXTURE
    mkdir -p "$tmp/sink/src"
    cat > "$tmp/sink/src/main.rs" <<'FIXTURE'
fn main() {
    env_logger::init();
}
FIXTURE
    if ! cli_sink_redacts "$tmp/bad_cli.rs" \
        && ! desktop_root_redacts "$tmp/bad_desktop.rs" \
        && ! desktop_root_redacts "$tmp/bad_desktop_format.rs" \
        && [ -n "$(other_sinks "$tmp/sink")" ] \
        && cli_sink_redacts "$CLI" \
        && desktop_root_redacts "$DESKTOP"; then
        echo "SELF-TEST OK: the gate flags an unredacted CLI writer, a cleared and a non-redacting desktop root, and a stray sink, and passes the real ones."
        exit 0
    fi
    echo "SELF-TEST FAILED: the gate misjudged a known fixture — it is broken."
    exit 1
fi

[ -f "$CLI" ] && [ -f "$DESKTOP" ] || { echo "ERROR: expected sink files missing — refusing to report OK."; exit 2; }

failed=0
if ! cli_sink_redacts "$CLI"; then
    echo "ERROR: $CLI: SuspendingWriter no longer writes through rdlp_redact::redact_bytes (#684)."
    failed=1
fi
if ! desktop_root_redacts "$DESKTOP"; then
    echo "ERROR: $DESKTOP: the tauri-plugin-log root formatter no longer redacts (or .clear_format() is back) (#684)."
    failed=1
fi
# The CLI's one sanctioned sink is the `with_writer(writer)` that feeds
# SuspendingWriter; any other `with_writer(` in main.rs is a stray too.
strays=$(other_sinks crates/*/src crates/rdlp-desktop/src-tauri/src | grep -v "^$CLI:[0-9]*:.*with_writer(writer)" || true)
if [ -n "$strays" ]; then
    echo "ERROR: a production log sink other than the two redacting egresses (#684):"
    printf '%s\n' "$strays" | sed 's/^/  /'
    failed=1
fi
[ "$failed" -eq 0 ] || exit 1
echo "OK: both log egresses redact and no other production sink exists."
