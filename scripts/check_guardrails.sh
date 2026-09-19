#!/usr/bin/env bash
# Run every quality gate in one pass.
#
# Why this exists: the gates live in a dozen separate scripts with different
# invocations and flags, so the usual way to discover a failing one is to trip
# over it mid-change. Run this before committing instead.
#
# Usage:
#   scripts/check_guardrails.sh              # check only, non-zero on failure
#   scripts/check_guardrails.sh --fix        # rustfmt + rebaseline ratchets
#   scripts/check_guardrails.sh --skip-slow  # skip cargo check/clippy/machete

set -uo pipefail
cd "$(dirname "$0")/.."

FIX=false
SKIP_SLOW=false
for arg in "$@"; do
    case "$arg" in
        --fix) FIX=true ;;
        --skip-slow) SKIP_SLOW=true ;;
        -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown flag: $arg (try --help)" >&2; exit 2 ;;
    esac
done

FAILED=()
JOBS="${CARGO_BUILD_JOBS:-2}"

run_gate() {
    local label=$1
    shift
    printf '▸ %s' "$label"
    local output
    if output=$("$@" 2>&1); then
        printf '\r✅ %s\n' "$label"
        return 0
    fi
    printf '\r❌ %s\n' "$label"
    printf '%s\n' "$output" | tail -20 | sed 's/^/    /'
    FAILED+=("$label")
    return 1
}

# Ratchet scripts share a --update flag to accept intentional growth.
run_ratchet() {
    local label=$1 script=$2
    if $FIX; then
        python3 "scripts/$script" --update >/dev/null 2>&1
    fi
    run_gate "$label" python3 "scripts/$script"
}

echo "=== Format ==="
# Before rustfmt: a `mod x;` with no file makes rustfmt fail with "Error writing
# files: failed to resolve mod", which reads like a formatting problem and hides
# every gate behind it. Naming the real cause first turns a confusing Format
# failure into an obvious one (221159294).
run_gate "module declarations resolve" python3 scripts/check_module_files.py
if $FIX; then
    cargo fmt --all
fi
run_gate "cargo fmt --all --check" cargo fmt --all --check

echo ""
echo "=== Quality Guardrails ==="
if $SKIP_SLOW; then
    echo "⏭  cargo check / clippy / machete (--skip-slow)"
else
    run_gate "cargo check --all-targets --all-features" \
        cargo check --all-targets --all-features -j "$JOBS"
    run_gate "cargo clippy -- -D warnings" \
        cargo clippy --all-targets --all-features -j "$JOBS" -- -D warnings
fi

# A stale lockfile is otherwise invisible: every other gate resolves it happily
# and only a `--locked` build refuses.
run_gate "Cargo.lock is up to date" cargo metadata --locked --format-version 1
run_gate "warning budget" bash scripts/check_warning_budget.sh
run_ratchet "oversized-file ratchet" check_code_size_budget.py
run_ratchet "oversized-test ratchet" check_test_size_budget.py
run_ratchet "panic-prone usage ratchet" check_panic_budget.py
run_ratchet "swallowed-error usage ratchet" check_swallowed_error_budget.py
run_gate "crate dependency boundaries" python3 scripts/check_dependency_boundaries.py
run_gate "wildcard re-export ratchet" python3 scripts/check_wildcard_reexport_budget.py
run_gate "dead surface (orphan crates/bins/scripts)" python3 scripts/check_dead_surface.py
run_gate "slash command parity (advertised but undispatched)" python3 scripts/check_command_parity.py

if $SKIP_SLOW; then
    :
elif command -v cargo-machete >/dev/null 2>&1; then
    run_gate "unused dependencies (cargo machete)" cargo machete
else
    echo "⏭  cargo machete (not installed: cargo install cargo-machete --locked)"
fi

if (( ${#FAILED[@]} )); then
    echo ""
    echo "❌ ${#FAILED[@]} gate(s) failed:"
    for f in "${FAILED[@]}"; do
        echo "   - $f"
    done
    if ! $FIX; then
        echo ""
        echo "For formatting and intentional ratchet growth: scripts/check_guardrails.sh --fix"
    fi
    exit 1
fi

echo "✅ All guardrail gates pass."
