#!/usr/bin/env bash
# Assert config↔GUI parity between the rdlp-api option registry and the desktop
# AppSettings struct — the one surface pair no compiler check covers.
#
#   forward : every Gui::Control("x") names a real AppSettings field.
#   reverse : every AppSettings field is bound by exactly one Gui::Control
#             (catches orphaned settings — the #589 class), minus known orphans.
#   missing : every Gui::Missing("y") references an issue (#<digits>).
#
# Usage: scripts/check-gui-option-parity.sh [--self-test]
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

REGISTRY="crates/rdlp-api/src/options.rs"
APPSETTINGS="crates/rdlp-desktop/src-tauri/src/state/app_settings.rs"
# Known orphan AppSettings fields with no Config field, each with a tracking issue.
KNOWN_ORPHANS="default_search_provider" # #589-2

extract_controls() {   # -> AppSettings field names named by Gui::Control("…")
    grep -oE 'Gui::Control\("[a-z0-9_]+"\)' "$1" | grep -oE '"[a-z0-9_]+"' | tr -d '"' | sort -u
}
extract_missing_refs() {
    grep -oE 'Gui::Missing\("[^"]*"\)' "$1" | grep -oE '"[^"]*"' | tr -d '"' | sort -u
}
appsettings_fields() {  # -> field names of the AppSettings struct (pub <name>: within the struct block)
    awk '/pub struct AppSettings/{f=1} f&&/^\}/{f=0} f' "$2" \
        | grep -oE '^[[:space:]]*pub [a-z0-9_]+:' | grep -oE '[a-z0-9_]+:' | tr -d ':' | sort -u
}

check() {
    local registry="$1" appsettings="$2" fail=0
    local controls fields missing
    controls="$(extract_controls "$registry")"
    fields="$(appsettings_fields _ "$appsettings")"

    # forward: each control target must be a real AppSettings field
    while read -r c; do [ -z "$c" ] && continue
        grep -qxF "$c" <<<"$fields" || { echo "registry Gui::Control(\"$c\") names no AppSettings field" >&2; fail=1; }
    done <<<"$controls"

    # reverse: each AppSettings field (minus known orphans) must be bound by a control
    while read -r f; do [ -z "$f" ] && continue
        grep -qwF "$f" <<<"$KNOWN_ORPHANS" && continue
        grep -qxF "$f" <<<"$controls" || { echo "AppSettings field \`$f\` has no registry Gui::Control (orphan; add a mapping or list in KNOWN_ORPHANS with an issue)" >&2; fail=1; }
    done <<<"$fields"

    # missing: every Gui::Missing must reference #<digits>
    missing="$(extract_missing_refs "$registry")"
    while read -r m; do [ -z "$m" ] && continue
        [[ "$m" =~ ^#[0-9]+$ ]] || { echo "Gui::Missing(\"$m\") is not an issue ref (#<digits>)" >&2; fail=1; }
    done <<<"$missing"

    return $fail
}

if [ "${1:-}" = "--self-test" ]; then
    tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
    # good fixtures
    cat >"$tmp/good_reg.rs" <<'EOF'
Gui::Control("alpha")
Gui::Missing("#123")
EOF
    cat >"$tmp/good_as.rs" <<'EOF'
pub struct AppSettings {
    pub alpha: bool,
}
EOF
    check "$tmp/good_reg.rs" "$tmp/good_as.rs" || { echo "self-test: good fixture wrongly rejected" >&2; exit 1; }
    # bad: control names a nonexistent field
    cat >"$tmp/bad_reg.rs" <<'EOF'
Gui::Control("ghost")
EOF
    if check "$tmp/bad_reg.rs" "$tmp/good_as.rs" 2>/dev/null; then echo "self-test: bad control not caught" >&2; exit 1; fi
    # bad: orphan AppSettings field
    cat >"$tmp/orphan_as.rs" <<'EOF'
pub struct AppSettings {
    pub alpha: bool,
    pub stray: bool,
}
EOF
    if check "$tmp/good_reg.rs" "$tmp/orphan_as.rs" 2>/dev/null; then echo "self-test: orphan field not caught" >&2; exit 1; fi
    # bad: malformed Missing ref
    printf 'Gui::Missing("META")\n' >"$tmp/badmiss_reg.rs"
    if check "$tmp/badmiss_reg.rs" "$tmp/good_as.rs" 2>/dev/null; then echo "self-test: bad Missing ref not caught" >&2; exit 1; fi
    echo "SELF-TEST OK"
    exit 0
fi

check "$REGISTRY" "$APPSETTINGS"
echo "GUI option parity OK: registry ↔ AppSettings consistent"
