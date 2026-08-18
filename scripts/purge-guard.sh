#!/usr/bin/env bash
# Fail if any purged subsystem has been reintroduced into this fork.
#
# This fork deliberately removes several upstream subsystems (see
# docs/FORK_WORKFLOW.md). Upstream keeps developing them, so every sync risks
# pulling them back in — often with NO merge conflict, because upstream adds a
# call site to a file we kept. This guard is the check the compiler cannot make.
#
# Usage: run from the repo root.
#   ./scripts/purge-guard.sh            # exit 0 = clean, 1 = reintroduction
#   ./scripts/purge-guard.sh --all      # also fail on test-only hits
#
# Keep the patterns TIGHT. A guard with false positives gets ignored. Bare
# `request_permission` matches an unrelated Grok provider method; the quoted
# tool name and the type name do not.
set -uo pipefail

ALL=false
[ "${1:-}" = "--all" ] && ALL=true

# Test code is excluded by default: this fork does not maintain the test suite,
# and stale test references are known and accepted.
filter_tests() {
  if [ "$ALL" = true ]; then cat; else grep -vE '(^|/)tests?/|/tests?\.rs:|ui_tests/'; fi
}

fail=0
section() { printf '\n== %s ==\n' "$1"; }
verdict() { if [ -n "$1" ]; then printf '%s\n' "$1"; fail=1; else echo "  clean"; fi; }

section "deleted trees must stay deleted"
verdict "$(for p in \
    ios telemetry-worker .github \
    crates/jcode-telemetry-core crates/jcode-tui-permissions crates/jcode-gateway-types \
    crates/jcode-base/src/gateway.rs crates/jcode-base/src/gateway \
    crates/jcode-base/src/sponsors.rs crates/jcode-base/src/sponsors \
    crates/jcode-app-core/src/tool/discover.rs \
    crates/jcode-app-core/src/tool/discover_secrets.rs \
    crates/jcode-setup-hints/src/windows_setup.rs \
    crates/jcode-setup-hints/src/windows_hotkeys.rs \
    crates/jcode-transport/src/windows.rs \
    TELEMETRY.md docs/SAFETY_SYSTEM.md docs/IOS_APP.md docs/WINDOWS.md
  do [ -e "$p" ] && echo "  RESURRECTED: $p"; done)"

section "deleted crates must not reappear in any manifest"
verdict "$(grep -rn 'jcode-telemetry-core\|jcode-tui-permissions\|jcode-gateway-types' \
    Cargo.toml crates/*/Cargo.toml 2>/dev/null | sed 's/^/  /')"

# `transcript_telemetry`/`upload_transcript` are here because the v0.76.0 sync
# merged a `transcript_telemetry_sent` struct field and its initializer with no
# conflict marker. Neither carries a `crate::telemetry::` prefix, so every
# pattern above was blind to them and only the unused-field warning would have
# surfaced it.
section "deleted Rust APIs must have no call sites"
verdict "$(grep -rnE \
    'crate::telemetry::|jcode_telemetry_core|crate::gateway::|jcode_gateway_types|jcode_tui_permissions|crate::sponsors|DiscoverToolsTool|record_permission_via_file|register_permission_notifier|RequestPermissionTool|safety::(PermissionRequest|PermissionResult|ActionTier|Urgency)|\.record_decision\(|\.pending_requests\(\)|transcript_telemetry|upload_transcript|handle_support_command|handle_feedback_command|SUPPORT_EMAIL' \
    --include='*.rs' crates/ src/ tests/ 2>/dev/null | filter_tests | sed 's/^/  /')"

section "removed tool / CLI registrations"
verdict "$(grep -rnE \
    '"integration_tools"|"request_permission"|Command::(Pair|Permissions)|handle_telemetry_command|commands_remote|SummaryPill::(Subscription|Telemetry)|TelemetryChoice|TelemetryLevel|"/support"|"/feedback"' \
    --include='*.rs' crates/ src/ 2>/dev/null | filter_tests | sed 's/^/  /')"

# The Windows launcher/hotkey port and the PowerShell installer are purged: this
# fork is macOS-only. Upstream never touched these files across the 236 commits
# of the v0.75.3 and v0.76.0 syncs, so a hit here means a sync reintroduced them
# rather than that they drifted back in gradually.
section "Windows launcher port must stay deleted"
verdict "$( { find scripts -name '*.ps1' 2>/dev/null | sed 's/^/  RESURRECTED: /'
    grep -rnE 'mod windows_(setup|hotkeys)|windows_(setup|hotkeys)::|listen_windows_hotkey' \
      --include='*.rs' crates/ src/ 2>/dev/null | filter_tests | sed 's/^/  /'; } )"

section "network egress endpoints"
verdict "$(grep -rn 'telemetry\.jcode\.sh\|api\.jcode\.sh/v1/discovery' \
    --include='*.rs' --include='*.sh' --include='*.ps1' . 2>/dev/null | sed 's/^/  /')"

echo
if [ "$fail" -eq 0 ]; then echo "PURGE GUARD: clean"; else echo "PURGE GUARD: REINTRODUCTION DETECTED"; fi
exit $fail
