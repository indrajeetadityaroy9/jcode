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
    crates/jcode-app-core/src/tool/computer \
    crates/jcode-app-core/src/tool/jcode_docs.rs crates/jcode-app-core/build.rs \
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
    docs/plans/SELFDEV_EXTRACTION.md docs/plans/UNIFIED_SELFDEV_SERVER_PLAN.md \
    docs/proposals/computer-use-tool.md docs/proposals/computer-use-maximal-control.md \
    crates/jcode-tui/src/tui/app/onboarding_flow.rs \
    crates/jcode-tui/src/tui/app/onboarding_flow_control.rs \
    crates/jcode-tui/src/tui/app/onboarding_graph.rs \
    crates/jcode-tui/src/tui/app/onboarding_repair.rs \
    crates/jcode-tui/src/tui/app/onboarding_sim.rs \
    crates/jcode-tui/src/tui/ui_onboarding.rs \
    crates/jcode-tui/src/tui/ui_tests/onboarding.rs \
    crates/jcode-import-core/src/repo_ranking.rs \
    scripts/capture_onboarding.sh docs/ONBOARDING_STATE_GRAPH.md
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
    'crate::telemetry::|jcode_telemetry_core|crate::gateway::|jcode_gateway_types|jcode_tui_permissions|crate::sponsors|DiscoverToolsTool|record_permission_via_file|register_permission_notifier|RequestPermissionTool|safety::(PermissionRequest|PermissionResult|ActionTier|Urgency)|\.record_decision\(|\.pending_requests\(\)|transcript_telemetry|upload_transcript|handle_support_command|handle_feedback_command|SUPPORT_EMAIL|tool::selfdev|SelfDevTool|run_self_dev|client_selfdev_requested|CLIENT_SELFDEV_ENV|JCODE_CLIENT_SELFDEV_MODE|SelfDevBuild(Command|Target)|selfdev_build_command|run_selfdev_build|selfdev_binary_path|SELFDEV_CARGO_PROFILE|spawn_selfdev_in_new_terminal|register_dev_tools|set_canary|is_self_dev\(|prefer_selfdev_binary|run_setup_hotkey|run_macos_hotkey_listener|record_launch_hotkey_use|record_launch_dirs|reinstall_launch_hotkeys|launch_hotkey_notice_lines|LaunchHotkeysConfig|LaunchHotkeyEntry|bake_launch_hotkeys_once|plan_launch_hotkeys_from_sessions|build_launch_hotkey_plan|MacHotkeyAction|HOTKEY_LISTENER_VERSION|jcode_provider_grok_build_runtime|auth::grok_build|GrokBuildProvider|GROK_BUILD_(PROFILE_ID|RUNTIME|LOGIN_PROVIDER)|XAI_PROFILE|XAI_LOGIN_PROVIDER|jcode_desktop2|jcode_math|jcode_provider_bedrock|provider::bedrock|BedrockProvider|AWS_BEARER_TOKEN_BEDROCK|JCODE_BEDROCK_|BrowserTool|chatgpt_web|CHATGPT_WEB_MODEL|ensure_browser_ready|browser_binary_path|is_browser_command|ensure_browser_session|firefox_agent_bridge|build_chatgpt_web_route|new_browser_only|crate::gmail|jcode_base::gmail|auth::google::|GmailAccessTier|GoogleCredentials|GmailBackend|GmailClient|ComposioConfig|google_can_send|jcode_notify_email|poll_imap_once|send_email|imap_reply_loop|SendEmailRequest|email_reply_enabled|email_imap_host|email_smtp_host|ComputerTool|tool::computer|core_graphics|JcodeDocsTool|tool::jcode_docs|JCODE_DOCS' \
    --include='*.rs' crates/ src/ tests/ 2>/dev/null | filter_tests | sed 's/^/  /')"

section "removed tool / CLI registrations"
verdict "$(grep -rnE \
    '"integration_tools"|"request_permission"|Command::(Pair|Permissions)|handle_telemetry_command|commands_remote|SummaryPill::(Subscription|Telemetry)|TelemetryChoice|TelemetryLevel|"/support"|"/feedback"|"/selfdev"|"selfdev"|create_session:selfdev|no_selfdev|self_dev:|Command::SetupHotkey|listen_macos_hotkey|notify_cli_launch|spawn_hotkey|ProviderChoice::(Xai|GrokBuild)|(LoginProviderTarget|LoginProviderAuthStateKey|RuntimeProviderId)::GrokBuild|PendingLogin::GrokBuild|"grok-build"|(ActiveProvider|LoginProviderTarget|ProviderChoice|RuntimeKey|ModelRouteApiMethod|LoginProviderAuthStateKey)::Bedrock|BEDROCK_LOGIN_PROVIDER|SelfDevBuildTarget::Desktop2|Command::Browser|"firefox-browser"|GmailTool|"gmail"|GOOGLE_LOGIN_PROVIDER|(LoginProviderTarget|LoginProviderAuthStateKey)::Google|google_access_tier|GoogleAccessTierArg|JCODE_SMTP_PASSWORD|JCODE_IMAP_HOST|JCODE_EMAIL_TO|JCODE_EMAIL_REPLY_ENABLED|linux-compat-vendored-openssl|"macos_computer_use"|"jcode_docs"' \
    --include='*.rs' crates/ src/ 2>/dev/null | filter_tests | sed 's/^/  /')"

# The first-run onboarding subsystem is purged (docs/FORK_WORKFLOW.md §1). The
# patterns are the concrete symbols, never a bare `onboarding`: that word still
# appears legitimately in Gemini Code Assist's `onboardUser` REST API
# (jcode-provider-gemini*), in scripts/onboarding_sandbox.sh (a kept auth
# harness), and in prose. A guard that matches those cries wolf and gets
# ignored.
section "first-run onboarding must stay deleted"
verdict "$(grep -rnE \
    'Onboarding(Phase|Flow|Action|WelcomeKind|PendingValidation)|onboarding_(flow|graph|sim|repair|preview|welcome|banner|startup_checked|import|auto_model)|ui_onboarding|SessionPickerMode::Onboarding|is_new_user_install|prefer_strongest|repo_ranking|SessionFilterMode::ExternalClis|"/onboarding-sim"|"/onboarding-preview"|onboarding-sim' \
    --include='*.rs' crates/ src/ tests/ 2>/dev/null | sed 's/^/  /')"

# The first-party `jcode` subscription provider is purged (docs/FORK_WORKFLOW.md
# §1). Tight for the same reason as above and then some: a bare `subscription`
# would match `channel_subscriptions` (swarm chat pub/sub, 241 hits in
# jcode-app-core alone) and every third-party subscription-auth path — Claude
# Pro/Max, ChatGPT, Copilot, Gemini Code Assist — all of which §1 KEEPS as the
# owner's actual working logins.
section "first-party jcode subscription provider must stay deleted"
verdict "$(grep -rnE \
    'subscription_api|subscription_catalog|JcodeProvider|JCODE_LOGIN_PROVIDER|JCODE_ACCOUNT_URL|jcode_device|subscribe_nudge|(ProviderChoice|LoginProviderTarget|LoginProviderAuthStateKey|RuntimeProviderId)::Jcode\b|(RuntimeKey|ModelRouteApiMethod|OpenRouterTransportState|NativeProviderKind)::Jcode(Subscription)?\b|AccountCommand::Jcode|"jcode-subscription"|disable_subscription_runtime_mode|is_jcode_subscription_runtime' \
    --include='*.rs' crates/ src/ tests/ 2>/dev/null | sed 's/^/  /')"

# The self-updater, the Cloud/Jade integration, `setup-launcher` and the four
# user-facing `ambient` verbs are purged (docs/FORK_WORKFLOW.md §1).
#
# Deliberately NOT matched: `--no-update` (hot_exec still emits it into its own
# re-exec argv), every `reload`/`rebuild` path (`session_rebuild`, `jcode server
# reload` — the owner's real source-build route), `ambient run-visible` and the
# whole ambient *runner* (the model-callable `schedule` tool rides on it), and
# `create_desktop_shortcut`/`maybe_show_setup_hints`, which install the app
# bundle automatically on first launch.
section "updater / cloud-jade / setup-launcher must stay deleted"
verdict "$(grep -rnE \
    'jcode_update_core|jcode-update-core|update_metadata|update_rate_limit|should_auto_update|spawn_background_update_check|run_auto_update|claim_update_fetch_slot|reload_server_after_update|\bhot_update\b|BusEvent::UpdateStatus|ClientMaintenanceAction::Update\b|UpdateChannel|JCODE_CHECK_UPDATES|JCODE_UPDATE_CHANNEL|jade_relay|JadeRelayChannel|JCODE_JADE|Command::(Update|Cloud|SetupLauncher)\b|Cloud(Command|SessionsCommand|SessionViewFormat)|JadeCloudOptions|run_setup_launcher|setup-launcher|AmbientCommand::(Status|Log|Trigger|Stop)\b' \
    --include='*.rs' --include='*.sh' --include='*.toml' crates/ src/ scripts/ tests/ Cargo.toml 2>/dev/null \
    | grep -v 'scripts/purge-guard.sh' | sed 's/^/  /')"

# Dictation (speech-to-text) and the replay video encoder are purged
# (docs/FORK_WORKFLOW.md §1).
#
# Deliberately NOT matched, because the owner asked for these to stay: the
# `image` crate and every inline/pinned-image path; `jcode transcript` and
# `Request::Transcript`, a standalone text-injection API any external STT
# script can drive; `storage::{remember,last}_focused_session`, which routes
# that injection; `jcode replay` interactive playback with `--export`
# (timeline JSON), `--speed`, `--swarm`, `--timeline`, `--auto-edit` and
# `--centered`; and `IMAGE_PLACEHOLDER_MODE` in jcode-tui-mermaid, which is the
# renamed image-placeholder path, not the encoder.
section "video media assets must stay deleted"
verdict "$( { git ls-files | grep -iE '\.(mp4|mov|webm|gif|avi|mkv)$' | sed 's/^/  RESURRECTED: /'
    git ls-files assets/demos 2>/dev/null | grep -i timeline | sed 's/^/  RESURRECTED: /'; } )"

section "dictation / video encoder must stay deleted"
verdict "$(grep -rnE \
    'video_export|export_swarm_video|export_video|run_headless_replay|compose_swarm_buffers|VIDEO_EXPORT_MODE|write_video_export_marker|JMERMAID|rsvg-convert|\bffmpeg\b|mod dictation|DictationConfig|DictationRun|ActiveDictation|BusEvent::Dictation|dictation_key|Command::Dictate\b|run_dictate_command|JCODE_DICTATION|replay_recording\.sh|record_demo\.sh|capture_demo\.sh|EventRecorder|RecordedEvent \{ offset_ms|get_event_recorder|EVENT_RECORDER|start_recording|stop_recording|get_recorded_events_json|"/record"|turn_complete_sound|UNNotificationSound|sound name \\"|tui::screenshot|mod screenshot|screenshot::(enable|disable|signal_ready|clear_all_signals)|"/screenshot"|"/screenshot-mode"|screenshot_watcher|auto_screenshot|niri msg' \
    --include='*.rs' --include='*.sh' --include='*.toml' crates/ src/ scripts/ tests/ 2>/dev/null \
    | grep -v 'scripts/purge-guard.sh' \
    | grep -v 'config_tests.rs' | sed 's/^/  /')"

# The Windows launcher/hotkey port and the PowerShell installer are purged: this
# fork is macOS-only. Upstream never touched these files across the 236 commits
# of the v0.75.3 and v0.76.0 syncs, so a hit here means a sync reintroduced them
# rather than that they drifted back in gradually.
section "Windows launcher port must stay deleted"
verdict "$( { find scripts -name '*.ps1' 2>/dev/null | sed 's/^/  RESURRECTED: /'
    grep -rnE 'mod windows_(setup|hotkeys)|windows_(setup|hotkeys)::|listen_windows_hotkey' \
      --include='*.rs' crates/ src/ 2>/dev/null | filter_tests | sed 's/^/  /'; } )"

# Every non-macOS code path is purged (see docs/FORK_WORKFLOW.md §1, "Platform
# residue"). This is the one guard section the compiler cannot back up: a
# reintroduced cfg(windows) or cfg(target_os = "linux") block compiles away
# silently here, so neither `cargo check` nor the suite objects to it.
#
# Deliberately NOT matched, because they are live macOS behavior: bare
# `cfg(unix)`, `DISPLAY` (XQuartz sets it), `$XDG_RUNTIME_DIR`/`$XDG_CONFIG_HOME`
# as user overrides, jcode-transport's `cfg(not(unix)) compile_error!` guard, and
# command-risk's /proc+/sys protected-path list.
section "non-macOS platform code must stay deleted"
verdict "$( { grep -rnE \
      'cfg\(windows\)|cfg\(target_os = "(windows|linux)"\)|cfg\(not\(windows\)\)|cfg\(all\(unix, not\(target_os = "macos"\)\)\)|windows-sys|windows_sys::' \
      --include='*.rs' --include='Cargo.toml' crates/ src/ Cargo.toml 2>/dev/null \
      | grep -v 'jcode-transport/src/lib.rs' | sed 's/^/  /'
    grep -rnE \
      'notify-send|xdg-open|wl-copy|wl-paste|xclip|xsel|wmctrl|xdotool|systemd-inhibit|taskkill|cmd\.exe\b|WAYLAND_DISPLAY|niri msg|/proc/self/(status|statm|task|fd|limits|stat)|/proc/cpuinfo|/proc/meminfo|/proc/version|/sys/devices|lspci' \
      --include='*.rs' crates/ src/ 2>/dev/null \
      | grep -vE '^[^:]+:[0-9]+:[[:space:]]*(//|\*)' | sed 's/^/  /'; } )"

# The orphaned client SDK, the consumer-less bridge *binary target*, the
# deprecated legacy-provider smoke bin and the Windows-only e2e module are
# purged (docs/FORK_WORKFLOW.md §1). The SDK's only consumer was the deleted
# desktop app; the bridge binary's only spawner was the SDK.
#
# Deliberately NOT matched: `run_bridge`, the `jcode api-bridge` subcommand and
# the server's wire-identity string `"jcode-harness-api-bridge/<version>"`
# (jcode-harness-api-server/src/lib.rs) are all live protocol surface - only the
# standalone binary target is gone. So this checks for the deleted *files* and
# the `[[bin]]` declaration rather than grepping the name.
section "orphaned client SDK / bridge binary / deprecated smoke bin"
verdict "$( { grep -rn 'crates/jcode-sdk\|jcode_sdk::' \
      --include='*.rs' --include='*.toml' . 2>/dev/null \
      | grep -v '/target/' | sed 's/^/  /'
    grep -rn 'name = "jcode-harness-api-bridge"\|name = "test_api"' \
      --include='*.toml' . 2>/dev/null | sed 's/^/  /'
    ls crates/jcode-harness-api-server/src/bin/bridge.rs src/bin/test_api.rs \
       tests/e2e/windows_lifecycle.rs 2>/dev/null | sed 's/^/  /'; } )"

section "network egress endpoints"
verdict "$(grep -rn 'telemetry\.jcode\.sh\|api\.jcode\.sh/v1/discovery' \
    --include='*.rs' --include='*.sh' --include='*.ps1' . 2>/dev/null | sed 's/^/  /')"

echo
if [ "$fail" -eq 0 ]; then echo "PURGE GUARD: clean"; else echo "PURGE GUARD: REINTRODUCTION DETECTED"; fi
exit $fail
