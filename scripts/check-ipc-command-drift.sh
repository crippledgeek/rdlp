#!/usr/bin/env bash
# Verify the desktop's typed IPC command map still matches the commands the
# Rust side registers, and that every command call goes through it (#611).
#
# Why: `crates/rdlp-desktop/src/api/ipc.ts` `interface IpcCommands` is the ONE
# owner of command names and payload types on the TypeScript side; `invokeTyped`
# is keyed by it. Registering a command in `generate_handler![]` without adding
# it to the map means the frontend cannot call it; removing one from Rust while
# the map still lists it means a call compiles and fails at runtime with a
# Tauri "command not found". Neither breaks either build on its own.
#
# The Rust set is the last path segment of every `commands::…::name,` line
# inside `tauri::generate_handler![ … ]` in src-tauri/src/lib.rs; the TS set is
# the top-level keys of `export interface IpcCommands { … }`. Both parsers are
# STRICT: a line they cannot classify exits 2 (extend the script) rather than
# being skipped — a skipped entry on both sides would let the sets match on the
# very drift this exists to catch.
#
# A second invariant: `import { invoke` from `@tauri-apps/api/core` appears in
# `src/api/invokeClient.ts` only. A raw `invoke` elsewhere bypasses the map.
#
# Fix when this fails: add/remove the `IpcCommands` entry (with its `args` and
# `result` types read from the Rust signature), or route the raw `invoke`
# through `invokeTyped`.
#
# Usage: scripts/check-ipc-command-drift.sh [--self-test]
#   --self-test: prove the set diff still fails on a synthetic extra command on
#                each side, and the raw-invoke sweep on a synthetic import, in a
#                temp dir. check-all.sh runs it every time.

set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like `[a-z_]` are
# UNSPECIFIED outside the C locale. A correctness fix (#621).
export LC_ALL=C

# Anchor to the repo root: every path below is relative. `|| exit 2` separates
# "cannot run" from "gate failed" (exit 1). See #621.
cd "$(git rev-parse --show-toplevel)" || exit 2

RUST_FILE="crates/rdlp-desktop/src-tauri/src/lib.rs"
TS_FILE="crates/rdlp-desktop/src/api/ipc.ts"
TS_SRC_DIR="crates/rdlp-desktop/src"
INVOKE_OWNER="crates/rdlp-desktop/src/api/invokeClient.ts"

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Command names registered in `generate_handler![ … ]`, sorted.
rust_commands() {
    awk -v file="$1" '
        /tauri::generate_handler!\[/ { inside = 1; next }
        inside && /^[[:space:]]*\]\)/ { exit }
        inside && /^[[:space:]]*$/ { next }
        inside && /^[[:space:]]*\/\// { next }
        inside && /^[[:space:]]*commands::[a-z_]+(::[a-z_]+)*::[a-z_]+,$/ {
            sub(/,$/, ""); n = split($0, parts, "::"); print parts[n]
            next
        }
        inside {
            printf "error: unparseable line in generate_handler![] (%s):\n  %s\n", file, $0 > "/dev/stderr"
            print  "       Expected `commands::<module>::<name>,`. Extend this script." > "/dev/stderr"
            exit 2
        }
    ' "$1" | sort
}

# Top-level keys of `export interface IpcCommands { … }`, sorted.
ts_commands() {
    awk -v file="$1" '
        index($0, "export interface IpcCommands {") == 1 { inside = 1; next }
        inside && /^}/ { exit }
        inside && /^[[:space:]]*$/ { next }
        inside && /^    (\/\/|\/\*\*|\*)/ { next }              # 4-space comments
        inside && /^    [a-z_][a-z0-9_]*: \{/ {                 # entry start (one line or multi-line)
            sub(/^    /, ""); sub(/:.*$/, ""); print
            next
        }
        inside && /^        / { next }                          # a multi-line entry body
        inside && /^    \};$/ { next }                          # a multi-line entry end
        inside {
            printf "error: unparseable line in IpcCommands (%s):\n  %s\n", file, $0 > "/dev/stderr"
            print  "       Expected `<snake_name>: { args: …; result: … };`. Extend this script." > "/dev/stderr"
            exit 2
        }
    ' "$1" | sort
}

# 0 = match, 1 = drift, 2 = cannot parse.
check_command_set() {
    local rust_file=$1 ts_file=$2 rust_set ts_set diff_out
    if ! rust_set=$(rust_commands "$rust_file"); then return 2; fi
    if ! ts_set=$(ts_commands "$ts_file"); then return 2; fi
    if [ -z "$rust_set" ]; then
        echo "error: parsed zero commands from generate_handler![] in $rust_file" >&2
        return 2
    fi
    if [ -z "$ts_set" ]; then
        echo "error: parsed zero keys from IpcCommands in $ts_file" >&2
        return 2
    fi
    if ! diff_out=$(diff <(echo "$rust_set") <(echo "$ts_set")); then
        cat <<EOM >&2

ERROR: generate_handler![] ($rust_file) and interface IpcCommands ($ts_file)
disagree. '<' lines are Rust-only (registered, not in the map); '>' lines are
TS-only (in the map, not registered).

$diff_out
EOM
        return 1
    fi
    echo "generate_handler![] ↔ IpcCommands OK ($(echo "$rust_set" | wc -l) commands)"
}

# 0 = clean, 1 = a raw `invoke` import outside the owner.
check_invoke_owner() {
    local src_dir=$1 owner=$2 hits
    # `|| true`: grep exits 1 on zero matches, which is the PASSING case here.
    hits=$(grep -rnE 'import \{[^}]*\binvoke\b[^}]*\} from "@tauri-apps/api/core"' "$src_dir" --include='*.ts' --include='*.tsx' \
        | grep -v "^$owner:" || true)
    if [ -n "$hits" ]; then
        cat <<EOM >&2

ERROR: raw \`invoke\` imported outside $owner. Every command call goes through
\`invokeTyped\`, keyed by the IpcCommands map (#611):

$hits
EOM
        return 1
    fi
    echo "raw invoke imported only in $(basename "$owner") OK"
}

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    cat > "$tmp/good.rs" <<'FIXTURE'
        .invoke_handler(tauri::generate_handler![
            commands::settings::settings,
            commands::download::start_download,
        ])
FIXTURE
    cat > "$tmp/good.ts" <<'FIXTURE'
export interface IpcCommands {
    // ---- settings.rs
    settings: { args: void; result: AppSettings };
    start_download: {
        args: { url: string };
        /** The job UUID. */
        result: string;
    };
}
FIXTURE
    cat > "$tmp/extra.rs" <<'FIXTURE'
        .invoke_handler(tauri::generate_handler![
            commands::settings::settings,
            commands::download::start_download,
            commands::download::job_options,
        ])
FIXTURE
    cat > "$tmp/extra.ts" <<'FIXTURE'
export interface IpcCommands {
    settings: { args: void; result: AppSettings };
    start_download: { args: { url: string }; result: string };
    pick_directory: { args: void; result: string | null };
}
FIXTURE
    # A handler list entry the Rust parser cannot classify: cannot run (2), not
    # a silent skip — mirrors `odd.ts` on the TS side.
    cat > "$tmp/odd.rs" <<'FIXTURE'
        .invoke_handler(tauri::generate_handler![
            commands::settings::settings,
            some_other::thing,
        ])
FIXTURE
    cat > "$tmp/odd.ts" <<'FIXTURE'
export interface IpcCommands {
    settings: { args: void; result: AppSettings };
    [key: string]: unknown;
}
FIXTURE
    mkdir -p "$tmp/src/api" "$tmp/src/views"
    printf 'import { invoke } from "@tauri-apps/api/core";\n' > "$tmp/src/api/invokeClient.ts"
    printf 'import { invoke } from "@tauri-apps/api/core";\n' > "$tmp/src/views/Rogue.tsx"

    ok=1
    check_command_set "$tmp/good.rs" "$tmp/good.ts" >/dev/null 2>&1 || ok=0
    rc=0; check_command_set "$tmp/extra.rs" "$tmp/good.ts" >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0
    rc=0; check_command_set "$tmp/good.rs" "$tmp/extra.ts" >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0
    rc=0; check_command_set "$tmp/good.rs" "$tmp/odd.ts" >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 2 ] || ok=0
    rc=0; odd_err=$(check_command_set "$tmp/odd.rs" "$tmp/good.ts" 2>&1 >/dev/null) || rc=$?
    [ "$rc" -eq 2 ] || ok=0
    [[ "$odd_err" == *"Extend this script"* ]] || ok=0
    rc=0; check_invoke_owner "$tmp/src" "$tmp/src/api/invokeClient.ts" >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 1 ] || ok=0
    rm "$tmp/src/views/Rogue.tsx"
    check_invoke_owner "$tmp/src" "$tmp/src/api/invokeClient.ts" >/dev/null 2>&1 || ok=0

    if [ "$ok" -eq 1 ]; then
        echo "SELF-TEST OK: command-set diff, both strict parsers and the raw-invoke sweep all still fire on synthetic drift."
        exit 0
    fi
    echo "SELF-TEST FAILED: a check did NOT flag a known violation (or rejected a clean fixture) - it is broken."
    exit 1
fi

for f in "$RUST_FILE" "$TS_FILE" "$INVOKE_OWNER"; do
    if [ ! -f "$f" ]; then
        echo "error: source not found at $f" >&2
        exit 2
    fi
done

status=0
rc=0; check_command_set "$RUST_FILE" "$TS_FILE" || rc=$?
[ "$rc" -eq 2 ] && exit 2
[ "$rc" -ne 0 ] && status=1
check_invoke_owner "$TS_SRC_DIR" "$INVOKE_OWNER" || status=1

exit "$status"
