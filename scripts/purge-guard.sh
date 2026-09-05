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
    crates/jcode-provider-grok-build-runtime crates/jcode-base/src/auth/grok_build.rs \
    crates/jcode-base/src/browser.rs crates/jcode-base/src/browser_tests.rs \
    crates/jcode-base/src/gmail.rs crates/jcode-base/src/auth/google.rs \
    crates/jcode-notify-email \
    crates/jcode-app-core/src/tool/gmail.rs \
    crates/jcode-app-core/src/tool/browser.rs \
    crates/jcode-app-core/src/tool/browser_tests.rs \
    crates/jcode-provider-openai-runtime/src/chatgpt_web.rs \
    crates/jcode-setup-hints/src/launch_hotkeys.rs \
    crates/jcode-setup-hints/src/linux_env.rs \
    crates/jcode-setup-hints/src/linux_niri.rs \
    crates/jcode-setup-hints/src/cli_launch_hints.rs \
    crates/jcode-setup-hints/src/windows_setup.rs \
    crates/jcode-setup-hints/src/windows_hotkeys.rs \
    crates/jcode-transport/src/windows.rs \
    crates/jcode-app-core/src/tool/selfdev src/cli/selfdev.rs \
    crates/jcode-desktop2 crates/jcode-math crates/jcode-provider-bedrock \
    scripts/check_desktop2_reload.py scripts/desktop2_mutation_sweep.sh \
    scripts/desktop2_visual_check.sh \
    TELEMETRY.md docs/SAFETY_SYSTEM.md docs/IOS_APP.md docs/WINDOWS.md \
    docs/AWS_BEDROCK_PROVIDER.md docs/BROWSER_PROVIDER_PROTOCOL.md \
    docs/GMAIL_COMPOSIO_BACKEND.md \
    docs/plans/SELFDEV_EXTRACTION.md docs/plans/UNIFIED_SELFDEV_SERVER_PLAN.md
  do [ -e "$p" ] && echo "  RESURRECTED: $p"; done)"

section "deleted crates must not reappear in any manifest"
verdict "$(grep -rn 'jcode-telemetry-core\|jcode-tui-permissions\|jcode-gateway-types\|jcode-desktop2\|jcode-math\|jcode-provider-bedrock\|jcode-provider-grok-build-runtime|jcode-notify-email' \
    Cargo.toml crates/*/Cargo.toml 2>/dev/null | sed 's/^/  /')"

# `transcript_telemetry`/`upload_transcript` are here because the v0.76.0 sync
# merged a `transcript_telemetry_sent` struct field and its initializer with no
# conflict marker. Neither carries a `crate::telemetry::` prefix, so every
# pattern above was blind to them and only the unused-field warning would have
# surfaced it.
section "deleted Rust APIs must have no call sites"
verdict "$(grep -rnE \
    'crate::telemetry::|jcode_telemetry_core|crate::gateway::|jcode_gateway_types|jcode_tui_permissions|crate::sponsors|DiscoverToolsTool|record_permission_via_file|register_permission_notifier|RequestPermissionTool|safety::(PermissionRequest|PermissionResult|ActionTier|Urgency)|\.record_decision\(|\.pending_requests\(\)|transcript_telemetry|upload_transcript|handle_support_command|handle_feedback_command|SUPPORT_EMAIL|tool::selfdev|SelfDevTool|run_self_dev|client_selfdev_requested|CLIENT_SELFDEV_ENV|JCODE_CLIENT_SELFDEV_MODE|SelfDevBuild(Command|Target)|selfdev_build_command|run_selfdev_build|selfdev_binary_path|SELFDEV_CARGO_PROFILE|spawn_selfdev_in_new_terminal|register_dev_tools|set_canary|is_self_dev\(|prefer_selfdev_binary|run_setup_hotkey|run_macos_hotkey_listener|record_launch_hotkey_use|record_launch_dirs|reinstall_launch_hotkeys|launch_hotkey_notice_lines|LaunchHotkeysConfig|LaunchHotkeyEntry|bake_launch_hotkeys_once|plan_launch_hotkeys_from_sessions|build_launch_hotkey_plan|MacHotkeyAction|HOTKEY_LISTENER_VERSION|jcode_provider_grok_build_runtime|auth::grok_build|GrokBuildProvider|GROK_BUILD_(PROFILE_ID|RUNTIME|LOGIN_PROVIDER)|XAI_PROFILE|XAI_LOGIN_PROVIDER|jcode_desktop2|jcode_math|jcode_provider_bedrock|provider::bedrock|BedrockProvider|AWS_BEARER_TOKEN_BEDROCK|JCODE_BEDROCK_|BrowserTool|chatgpt_web|CHATGPT_WEB_MODEL|ensure_browser_ready|browser_binary_path|is_browser_command|ensure_browser_session|firefox_agent_bridge|build_chatgpt_web_route|new_browser_only|crate::gmail|jcode_base::gmail|auth::google::|GmailAccessTier|GoogleCredentials|GmailBackend|GmailClient|ComposioConfig|google_can_send|jcode_notify_email|poll_imap_once|send_email|imap_reply_loop|SendEmailRequest|email_reply_enabled|email_imap_host|email_smtp_host' \
    --include='*.rs' crates/ src/ tests/ 2>/dev/null | filter_tests | sed 's/^/  /')"

section "removed tool / CLI registrations"
verdict "$(grep -rnE \
    '"integration_tools"|"request_permission"|Command::(Pair|Permissions)|handle_telemetry_command|commands_remote|SummaryPill::(Subscription|Telemetry)|TelemetryChoice|TelemetryLevel|"/support"|"/feedback"|"/selfdev"|"selfdev"|create_session:selfdev|no_selfdev|self_dev:|Command::SetupHotkey|listen_macos_hotkey|notify_cli_launch|spawn_hotkey|ProviderChoice::(Xai|GrokBuild)|(LoginProviderTarget|LoginProviderAuthStateKey|RuntimeProviderId)::GrokBuild|PendingLogin::GrokBuild|"grok-build"|(ActiveProvider|LoginProviderTarget|ProviderChoice|RuntimeKey|ModelRouteApiMethod|LoginProviderAuthStateKey)::Bedrock|BEDROCK_LOGIN_PROVIDER|SelfDevBuildTarget::Desktop2|Command::Browser|"firefox-browser"|GmailTool|"gmail"|GOOGLE_LOGIN_PROVIDER|(LoginProviderTarget|LoginProviderAuthStateKey)::Google|google_access_tier|GoogleAccessTierArg|JCODE_SMTP_PASSWORD|JCODE_IMAP_HOST|JCODE_EMAIL_TO|JCODE_EMAIL_REPLY_ENABLED|linux-compat-vendored-openssl' \
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
