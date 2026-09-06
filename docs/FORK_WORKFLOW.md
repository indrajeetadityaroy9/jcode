# Fork Maintenance Runbook

> Operational guide for this **personal fork** of [`1jehuang/jcode`](https://github.com/1jehuang/jcode).
> Written so a future Claude Code instance (or a human) can repeat the full cycle
> without rediscovering the traps. Every pitfall listed in the Appendix was hit
> for real; none is hypothetical.

**Scope of this document**
1. What this fork removes, and why that must survive every sync
2. Full uninstall / clean slate
3. Sync with upstream — filtering out the purged subsystems **and** confirming upstream's new
   features actually arrived. Both directions matter: a sync that quietly loses features is as
   much a failure as one that quietly reabsorbs telemetry.
4. Build
5. Install
6. Verification gates
7. Appendix — pitfalls, each with the symptom that revealed it

---

## 0. Orientation

| | |
|---|---|
| Upstream | `https://github.com/1jehuang/jcode` (branch `master`) |
| This fork's origin | `git@github.com:indrajeetadityaroy9/jcode.git` |
| Local checkout | `/Users/indrajeetadityaroy/jcode` (`$MAIN` throughout this doc) |
| **Last upstream merge** | **`c4cdc6768` — v0.76.0** (merged 2026-08-16 as `2fb0f158e`) |
| Rollback tag convention | `pre-merge-YYYY-MM-DD` |
| Install layout | `~/.jcode/builds/versions/<hash>/` + `stable`/`current` symlinks + `~/.local/bin/jcode` |

> ⚠️ **The "Last upstream merge" row is load-bearing.** §3.2 classifies
> `BASE..upstream/master`, and `BASE` comes from this row. If it goes stale the next
> sync classifies against the wrong base and silently under-reports the work.
> **§3.6 updates it — do not skip that step.**

**Architecture facts that shape this runbook**

- jcode is **client/server**. `jcode serve` outlives the TUI. Closing the window does *not*
  stop the daemon — check `pgrep -fl jcode`, not the window.
- Hot reload is `exec()` into the new binary **on the same socket**; clients auto-reconnect
  (`docs/SERVER_ARCHITECTURE.md`).
- The install store is **immutable versioned dirs + symlink flip**. The previous version
  *is* the rollback. Never delete it before installing a new one.
- Per `AGENTS.md`: a freshly built binary is **inert** until the symlink is repointed and the
  daemon restarted. Testing via `jcode` on PATH while an old daemon is alive measures the old
  code.

---

## 1. The purge invariant

This fork deletes the following. **Upstream keeps developing them**, so every sync tries to
pull them back — frequently with *no merge conflict*, because upstream adds a call site to a
file we kept.

| Subsystem | What was removed |
|---|---|
| **iOS + WebSocket gateway** | `ios/`, `crates/jcode-gateway-types`, `jcode-base/src/gateway*`, `jcode pair`, `/remote`, `scripts/remote/`, `scripts/phone-server/` |
| **Sponsored discovery** | `jcode-base/src/sponsors*`, `tool/discover*.rs`, `integration_tools` tool, `[sponsors]` config, discovery benchmarks/docs |
| **Telemetry** | `crates/jcode-telemetry-core`, `telemetry-worker/`, `TELEMETRY.md`, all `record_*` call sites, `/telemetry`, the onboarding consent screen, **and the independent shell telemetry in `scripts/install.sh`** |
| **Ambient permissions** | `crates/jcode-tui-permissions`, `jcode permissions`, `request_permission` tool, permission half of `safety.rs`, remote approve/deny over Telegram/Discord/email/Jade |
| **CI** | `.github/` |
| **Self-development subsystem** | Removed in two passes. First the tool: `jcode-app-core/src/tool/selfdev/` (canary build queue, clone/setup/status/launch), `src/cli/selfdev.rs`, the `jcode self-dev` subcommand, the `selfdev` agent tool, and its prompt layer. Then everything that still referenced it: the `/selfdev` slash command (+ help and autocomplete), the `Cmd+Shift+'` self-dev launch hotkey and the `self_dev` field on `LaunchHotkeyEntry`, `--no-selfdev` and the repo-auto-detect that set `JCODE_CLIENT_SELFDEV_MODE` (`jcode_dev_types::{CLIENT_SELFDEV_ENV, client_selfdev_requested}`), the `self-dev` argv every reload/hot-restart/post-update re-exec appended, `spawn_selfdev_in_new_terminal*`, the `selfdev` field on `Request::Subscribe` and the `is_canary` field on `ServerEvent::History`, `RestartSnapshotSession::is_selfdev`, `create_session:selfdev[:<path>]`, `prefer_selfdev_binary` on the reload signal, `SelfDevBuildCommand`/`SelfDevBuildTarget`/`selfdev_build_command*`/`run_selfdev_build`/`selfdev_binary_path`/`SELFDEV_CARGO_PROFILE`, the `is_selfdev_session` parameter on `client_update_candidate`/`shared_server_update_candidate`/`preferred_reload_candidate`, `Registry::register_dev_tools()` with the `debug_socket` **agent tool** it registered, the `[self-dev]`/`jcode:selfdev`/`jcode:d:` process- and window-title variants, and `session_search`'s `canary` filter. **The canary session marker went with it**: every production `Session::set_canary` call passed the literal `"self-dev"`, so `Session::{is_canary, testing_build, set_canary, clear_canary, is_self_dev}`, `Agent::{is_canary, set_canary}`, `EnvSnapshot::{is_selfdev, is_canary, testing_build}`, `SessionInfo::is_canary` and the TUI's `remote_is_canary` + `(canary/self-dev)` badge are all gone. **Kept:** `[profile.selfdev]` and `target/selfdev` (a plain cargo profile `dev_cargo.sh` and the release scripts use), `jcode-build-support` and `jcode-dev-types` (build publish/activation DTOs), the `~/.jcode/builds/canary` channel + `CanaryStatus` manifest + promote/publish machinery, the debug socket itself (`jcode debug`, `display.debug_socket`), and `ReloadContext` in `server/reload_context.rs` — that is the reconnect path, not self-dev. **Behavior deliberately dropped with the gates:** the client no longer re-execs itself onto a newer binary after an in-process server reload, onboarding no longer special-cases such sessions, and `shared_server_update_candidate` no longer honors a `shared-server` pin whose version marker matches neither `stable` nor `current`. See `docs/plans/SELFDEV_EXTRACTION.md` for the first pass. |
| **Support / feedback commands** | `/support` (mailto to upstream carrying account id, email, tier) and `/feedback` (a no-op since the telemetry purge removed `record_feedback`) |
| **Windows launcher port** | `jcode-setup-hints/src/windows_{setup,hotkeys}.rs`, `jcode-transport/src/windows.rs` (named-pipe IPC), all six `scripts/*.ps1`, `docs/WINDOWS.md`, `--listen-windows-hotkey`, the Windows branch of `scripts/install.sh`, and Section D of `setup_friction_eval.sh` |
| **Desktop app** | `crates/jcode-desktop2` (winit + wgpu + Vello + Parley GPU client, ~42k lines) and `crates/jcode-math` (its TeX layout engine, no other consumer). Also `SelfDevBuildTarget::Desktop2` in `jcode-dev-types` + its routing in `jcode-build-support/src/paths.rs`, the `vello`/`wgpu`/`parley`/`fontique`/`skrifa`/`read-fonts`/`font-types`/`harfrust`/`peniko`/`color`/`zeno`/`naga` `[profile.*.package.*]` pins in the root `Cargo.toml`, the already-dead legacy-desktop pins (`cosmic-text`, `swash`, `yazi`, `unicode-linebreak`), `scripts/{check_desktop2_reload.py,desktop2_mutation_sweep.sh,desktop2_visual_check.sh}`, the desktop2 frame-budget gate in `check_guardrails.sh`, `captures/` in `.gitignore`, and all `docs/DESKTOP*.md`. **Kept:** `jcode-harness-api`, `jcode-harness-api-server` (`jcode api-bridge`) and `jcode-sdk` — a general programmatic surface, not desktop-only; `jcode-render-core` (shared with the TUI); and the `kurbo`/`rustybuzz`/`ttf-parser`/`fontdb` pins, which the TUI's mermaid SVG rasterizer still needs. `docs/HARNESS_API_AND_DESKTOP_REWRITE.md` was trimmed to Part 1 and renamed `docs/HARNESS_API.md`. |
| **AWS Bedrock provider** | `crates/jcode-provider-bedrock` (native Converse/ConverseStream client, IAM SigV4 + `AWS_BEARER_TOKEN_BEDROCK` bearer auth, ~2k lines) and the whole 20-crate `aws-sdk-*` dependency stack it pulled in. Also the `bedrock` cargo feature and its forwarding through `jcode-base`/`jcode-app-core`/`jcode-tui`, `provider::bedrock` re-export, `ActiveProvider::Bedrock` / `RuntimeKey::Bedrock` / `ModelRouteApiMethod::Bedrock` / `LoginProviderTarget::Bedrock` / `LoginProviderAuthStateKey::Bedrock` / `ProviderChoice::Bedrock`, `AuthStatus.bedrock` + `probe_bedrock_status`, `BEDROCK_LOGIN_PROVIDER`, `ALL_BEDROCK_MODELS`, the `bedrock:` model-spec prefix and its route/prefetch/failover arms, `jcode login --provider bedrock` (CLI + TUI API-key flow), the Bedrock branch of the provider doctor's native-driver matrix, the picker's Bedrock model-id prettifier in `tui/app/helpers/model_names.rs`, `JCODE_BEDROCK_{ENABLE,PROFILE,MODEL}`, and `docs/AWS_BEDROCK_PROVIDER.md`. **Kept:** every `Bedrock` mention that describes someone *else's* upstream — the hosted jcode router's server-side routing notes in `subscription_catalog.rs`, OpenRouter's upstream list in `jcode-provider-openrouter/src/request.rs`, and `docs/JCODE_CLOUD_AWS.md` (EC2 deployment, not a local provider). |
| **Global launch hotkeys** | The OS-level shortcuts that launched a new jcode window, and everything built to install and promote them: `jcode-setup-hints/src/{launch_hotkeys,linux_env,linux_niri,cli_launch_hints}.rs` (+ `linux_niri_fuzz_corpus.txt`, `scripts/fuzz/niri_insert_point_fuzz.py`), the whole hotkey half of `jcode-setup-hints/src/lib.rs` (2548 → 530 lines: `run_setup_hotkey`, `run_macos_hotkey_listener_main_thread` + its Carbon/Core-Foundation run loop, the LaunchAgent installer/uninstaller/migrator, `HOTKEY_LISTENER_VERSION`, `MacHotkeyAction`, `record_launch_dirs`, `record_launch_hotkey_use`, `launch_hotkey_notice_lines`, `reinstall_launch_hotkeys_after_config_change`, and every GNOME/KDE/XFCE/Cinnamon/MATE/niri/Hyprland/sway dconf+config-file installer), `jcode setup-hotkey` with `--listen-macos-hotkey`/`--uninstall`/`--notify-cli-launch` and its `main.rs` pre-Tokio entry points, the global `--spawn-hotkey <CHORD>` flag, `[launch_hotkeys]` (`LaunchHotkeysConfig`, `LaunchHotkeyEntry`, `Config::{set_launch_hotkeys, bake_launch_hotkeys_once}`, the `launch_hotkeys` restart-required section), `repo_ranking::{PlannedHotkey, DEFAULT_LAUNCH_HOTKEY_CHORDS, build_launch_hotkey_plan, plan_launch_hotkeys_from_sessions}`, the `~/.jcode/hotkey/` support dir (`last_dir`, `last_repo`, `plan.json`), the "Launch hotkeys" startup notice + its single-line renderer, the Claude-Code/Codex SessionStart hook that nagged about the shortcut, and the now-unused `global-hotkey` dependency in all five manifests. **Kept:** everything about *in-app* keys — `[keybindings]`, `hotkey_feedback.rs` (rare-chord hints + near-miss suggestions), the `/hotkeys` usage list, `[dictation].key`, and `jcode-setup-hints/src/keymap/**`, which still detects terminal/macOS shortcuts that intercept jcode's own chords. Also kept: `jcode setup-launcher` + `macos_launcher.rs` (the Spotlight/Dock `Jcode.app`), `macos_terminal.rs`, `terminal.preferred`, the terminal-capability nudges, and the repo-*ranking* half of `repo_ranking.rs` that onboarding uses for its "recent project" suggestion. |
| **Grok / xAI providers** | Both access paths. (1) **Grok Build**, the Jcode-managed subscription provider: `crates/jcode-provider-grok-build-runtime` (Agent Client Protocol over the stdio of an xAI `grok` binary Jcode downloaded itself), `jcode-base/src/auth/grok_build.rs` (version pin, download, `ensure_cli`, cached-login probe), `GROK_BUILD_LOGIN_PROVIDER`, `LoginProviderTarget::GrokBuild`, `LoginProviderAuthStateKey::GrokBuild`, `RuntimeProviderId::GrokBuild`, `AuthStatus.grok_build`, `GROK_BUILD_PROFILE_ID`, `external::GROK_BUILD_RUNTIME` + its composition-root registration, the `grok-build:` model prefix and the `grok-build-acp` route rows, `jcode login --provider grok-build`, the TUI `start_grok_build_login` managed-OAuth flow and `PendingLogin::GrokBuild`, and `JCODE_GROK_CLI_PATH`. (2) The **`xai` OpenAI-compatible profile** (`XAI_PROFILE`, `XAI_LOGIN_PROVIDER`, `api.x.ai/v1`, `XAI_API_KEY`, `xai.env`, default model `grok-code-fast-1`, aliases `x.ai`/`x-ai`/`grok`) — its entire content is Grok, so it went with the rest. Also gone: `ProviderChoice::{Xai, GrokBuild}` (so `--provider xai|grok-build|grok` no longer parse), the `grok-code-fast` 256K context-window rule in `jcode-provider-core/src/models.rs`, the `xai` pricing key, the `grok-code-fast-1` entry in the `firmware` curated catalog, and the xAI row in `docs/audits/provider-model-catalog-audit.md`. `OPENAI_COMPAT_PROFILES` 38 → **37**, `LOGIN_PROVIDERS` 49 → **47**. **Kept:** the `XAI_API_KEY` entry in the transcript secret-redaction denylist (`jcode-base/src/message.rs`) — provider-independent secret hygiene, useful even with no xAI provider; and `groq`, an unrelated vendor that merely matches the substring. |
| **Browser automation** | Every path that drove a browser on the agent's behalf, in three layers. (1) The **`browser` agent tool**: `jcode-app-core/src/tool/{browser.rs,browser_tests.rs}` (22 actions over the bridge binary) and its `base_tools` registration — the model-callable tool count drops 29 → 28 on macOS; the TUI's `browser_summary`/`browser_target_summary` renderers and the `"browser"` transcript-compaction arm went with it. (2) The **Firefox agent bridge**: `jcode-base/src/{browser.rs,browser_tests.rs}` (GitHub-release download of `firefox-agent-bridge`, native-messaging host install, extension-compat probes, per-session `BROWSER_SESSION` tabs), the `jcode browser setup\|status` subcommand (`Command::Browser` + `run_browser`), and the auto-rewrite hook in the `bash` tool that silently redirected browser commands to the installed binary. (3) The **ChatGPT-web provider route**: `jcode-provider-openai-runtime/src/chatgpt_web.rs` (~915 lines driving a logged-in ChatGPT tab), `CHATGPT_WEB_MODEL` (`gpt-5.6-pro[web]`) in `jcode-provider-core` + its `ALL_OPENAI_MODELS` entry, `build_chatgpt_web_route` and the `chatgpt-web` api-method routes, `OpenAIProvider::new_browser_only()` and the whole `browser_only` flag (with it gone the OpenAI runtime only registers when Codex credentials load), plus tokio's `process` feature in that crate. Also the endorsed **`firefox-browser` skill** in `jcode-base/src/skill.rs` and `docs/BROWSER_PROVIDER_PROTOCOL.md`. **Kept:** everything that opens the *user's* browser for OAuth (`open_auth_browser`, `--no-browser`, `NO_BROWSER`/`JCODE_NO_BROWSER`, device-approval copy), the `EnvFacts.browser` probe, the `open` tool's Firefox desktop-entry matching, and "browser-style" text-selection comments in the TUI. |
| **Gmail integration** | The whole email-agent surface. The **`gmail` tool** (`jcode-app-core/src/tool/gmail.rs`, 12 actions: list/read/search/draft/send/attachments/labels) and its `base_tools` registration — the model-callable tool count drops 28 → 27 on macOS. The **Gmail REST backend** `jcode-base/src/gmail.rs` (~1.2k lines) with *both* of its transports: `GmailBackend::Direct` against `gmail.googleapis.com/gmail/v1`, and `GmailBackend::Composio` brokering the same calls through Composio's `proxy-execute` (`ComposioConfig`, connect-link flow, `COMPOSIO_API_KEY`/`COMPOSIO_GMAIL_AUTH_CONFIG_ID`, `JCODE_GMAIL_BACKEND`). The **Gmail-only Google OAuth store** `jcode-base/src/auth/google.rs` (`gmail.readonly`/`compose`/`send`/`modify` scopes, `GmailAccessTier`, `GoogleCredentials`, `~/.jcode/google_oauth.json`), `AuthStatus.{google,google_can_send}`, `probe_google_status`, `GOOGLE_LOGIN_PROVIDER` (`LOGIN_PROVIDERS` 47 → 46), `LoginProviderTarget::Google`, `LoginProviderAuthStateKey::Google`, `jcode login --provider google\|gmail` with its interactive client-ID/secret paste flow, the `--google-access-tier full\|readonly` flag (`GoogleAccessTierArg`), the `jcode auth-test` Google probe (and `AuthTestTarget::supports_smoke`, which existed only to mark Gmail as non-model), the TUI Gmail tool-summary renderer + `"gmail"` compaction arm, `docs/GMAIL_COMPOSIO_BACKEND.md`, and the "Google / Gmail OAuth" section of `OAUTH.md`. **Kept:** `jcode-base/src/auth/google_oauth.rs` — the *shared* token-refresh helper that `auth/gemini.rs` and `auth/antigravity.rs` depend on; every Gemini / Antigravity login path (Code Assist OAuth also says "Google"); the many places where `"google"` is a **Gemini alias** (`provider-core/src/selection.rs`, `auth/external.rs`'s `GEMINI_API_KEY => ["google","gemini"]`, TUI provider colors); and, at the time, `crates/jcode-notify-email` with its `smtp.gmail.com`/`imap.gmail.com` config examples, on the grounds that it was generic SMTP/IMAP rather than the Gmail API — superseded by the next row, which removed that channel too. |
| **Email notification channel** | The SMTP/IMAP half of ambient notifications, which the Gmail purge had left as the last email surface. `crates/jcode-notify-email` (267 lines: `send_email`/`SendEmailRequest` over `lettre`, `poll_imap_once`/`ReplyAction` over `imap` + `mail-parser`, markdown→HTML email bodies via `pulldown-cmark`) and its workspace member + `jcode-app-core` dependency. In `notifications.rs`: the email dispatch arm of `send_all`, the `send_all_with_email_override` wrapper and its `email_html_override`/`cycle_id` (Message-ID reply-tracking) plumbing, and the whole `imap_reply_loop` — plus its spawn block in `ambient/runner.rs`. Config: the nine `SafetyConfig.email_*` fields and defaults in `jcode-config-types`, the `JCODE_SMTP_PASSWORD` / `JCODE_EMAIL_TO` / `JCODE_IMAP_HOST` / `JCODE_EMAIL_REPLY_ENABLED` env overrides + their `KNOWN_ENV` entries, the `[safety]` email/IMAP block in the config template, and the "Email"/"Email replies" rows of `/config`. Also the now-vacuous `linux-compat-vendored-openssl` feature chain (root → `jcode-tui` → `jcode-app-core` → `jcode-notify-email`, which existed only to vendor OpenSSL for `imap`'s native-tls) and its `--features` flag in `scripts/build_linux_compat.sh`. Dropping `lettre`/`imap`/`mail-parser` retired three triaged advisories (`RUSTSEC-2026-0141`, `RUSTSEC-2023-0086`, `RUSTSEC-2026-0049`) and their `security_preflight.sh` ignores: `imap`/`rustls-connector` held the last rustls 0.22 stack, so the graph now resolves a single `rustls-webpki 0.103.13`. **Kept:** every other notification transport — ntfy.sh, desktop/macOS Notification Center, and the Telegram/Discord/Jade message channels — including the shared `ambient/directives.rs` reply→directive store, which those channels still feed through `AmbientRunnerHandle::inject` (its docs no longer claim email as the source). |
| **macOS desktop control** | The `macos_computer_use` tool and everything under it: `crates/jcode-app-core/src/tool/computer/` (2,966 lines across `mod.rs` + `ax.rs`/`win.rs`/`sys.rs`/`screen.rs`/`input.rs`/`keys.rs`/`osa.rs`/`setup.rs`/`discover.rs` and two test modules) and its `#[cfg(target_os = "macos")]` `base_tools` registration — the model-callable tool count drops 26 → **25**. It was the second-widest tool in the harness at 29 schema properties, covering screenshot/OCR, the Accessibility (AX) tree, window and system queries, synthetic mouse/keyboard input, AppleScript/JXA execution, a permission-setup flow, and progressive schema discovery. Its exclusive dependency went with it: the `[target.'cfg(target_os = "macos")'.dependencies]` block in `crates/jcode-app-core/Cargo.toml` carrying `core-graphics 0.23` with the `highsierra` feature (the `CGEventCreateScrollWheelEvent2` binding used for scroll). Also deleted the two design docs it shipped from, `docs/proposals/computer-use-tool.md` and `docs/proposals/computer-use-maximal-control.md`. **Kept:** the `open` tool (open/reveal a file, folder, or URL *for the user* — no synthetic input, no screen capture), the macOS menu-bar helper and its `objc2*` stack in the root `Cargo.toml`, and the Notification Center broker. With this gone the harness no longer requests Accessibility or Screen Recording permissions for tool use. |
| **Bundled documentation search** | The `jcode_docs` tool (`crates/jcode-app-core/src/tool/jcode_docs.rs`, 285 lines: `search`/`read` over version-matched in-binary docs, `DEFAULT_LIMIT` 5 / `MAX_LIMIT` 10 / `MAX_SECTION_CHARS` 4,000) and its `base_tools` registration — the model-callable tool count drops 25 → **24**. With it went `crates/jcode-app-core/build.rs`, whose only job was generating the corpus: it walked `README.md` + every `docs/*.md` and emitted a `JCODE_DOCS: &[(&str, &str)]` table of `include_str!`s into `OUT_DIR`. Two consequences worth knowing: the binary no longer embeds a copy of the docs tree, and `docs/**` is no longer a `rerun-if-changed` input for `jcode-app-core`, so editing a doc stops triggering a rebuild of the second-largest crate in the workspace. Also dropped the `agent_tests.rs` assertion that pinned `jcode_docs` as model-visible. **Kept:** `docs/` itself and the `read`/`agentgrep` path a model uses to reach it in-repo; the separate `jcode-build-meta/build.rs` (version stamping) is untouched. |
| **Purge residue sweep** | Not a subsystem — the inert leftovers of the rows above, found by auditing what no longer resolves. (1) **2,467 lines of test files that never compiled**: `jcode-app-core/src/{protocol_tests.rs, protocol_tests/ (5 files, 1,648 lines), usage_tests.rs (596), stdin_detect_tests.rs (207)}`. This crate wires unit tests with explicit `#[cfg(test)] #[path = "x_tests.rs"] mod …`, so when protocol/usage/stdin_detect moved down into `jcode-protocol`/`jcode-base`/`jcode-core` the sibling files lost their `mod` line and silently stopped being compiled — appending garbage to one produced zero `cargo check` errors. The live copies are `jcode-protocol/src/protocol_tests{,/}`, `jcode-base/src/usage/tests.rs`, `jcode-core/src/stdin_detect_tests.rs`. (2) **26 stale ratchet keys** across `scripts/{code_size,test_size,panic,swallowed_error}_budget.json`, naming files deleted by the self-dev, telemetry, sponsored-discovery, Gmail-OAuth, gateway, tui-permissions and setup-hints purges. Two carried real slack: `panic_budget.total` 74 → 72 and `swallowed_error_budget.total` 3,171 → 3,013 (`totals_by_pattern` decremented to match), so the ratchets had been under-counting by 2 and 158. (3) **Dead assets**: `tests/desktop-gallery-golden/` (15 golden PNGs, 1.8 MB, for the purged `jcode-desktop2`), `assets/niri-screenshot.png` (the niri WM integration removed with the global launch hotkeys), `docs/reddit_dashboard.py` + `docs/jcode_reddit_dashboard.png` (964 KB one-off matplotlib script, against `docs/README.md`'s "describes current state" rule), the empty `scripts/fuzz/` (its only file went with the niri fuzz corpus), and `scripts/repro/tls-bad-record-mac/` (a non-member crate whose README documents a shipped fix). (4) **Broken references**: the `test_launcher_prompt_names_neither_target_tool_nor_discovery` test in `scripts/test_demo_shop.py` read `scripts/launch_agentcard_discovery_demo.sh`, purged with sponsored discovery, so the suite errored 1/7 — dropped, 6/6 now pass; and `docs/plans/COMPILE_PERFORMANCE_PLAN.md` gained a third caveat for the `scripts/bench_selfdev_checkpoints.sh` harness its 2026-04-18 entries still cite. **Kept:** `scripts/demo_shop.py` (still exercised by the 6 remaining tests). |
| **Dead-code sweep** | Code that could not execute, proven case by case rather than inferred from a warning. (1) **The `subagent` tool that was never registered.** `jcode_tool_types::resolve_tool_name` mapped `task`/`task_runner`/`Agent` → `subagent`, and no such tool is in the registry (24 tools), so a model calling `task` resolved to a name that could only fail. The aliases are gone; the "Unknown tool" reply is unchanged because `Registry::execute` reports the *original* name and computes its did-you-mean suggestions from it. (2) **Three curated Anthropic-OAuth builtins that were never advertised.** `Agent`, `Glob` and `Grep` in `jcode-provider-anthropic::format_tools`, plus the `subagent`/`glob`/`grep` rows in `OAUTH_BUILTIN_LOCAL_TOOLS`. The `has_backing` guard (#572) suppresses a curated definition when no local tool backs it, and none of those three names is registered — verified by calling `format_tools(&real_24_tool_defs, is_oauth=true)`: `Agent`/`Glob`/`Grep` absent, 24 names advertised, identical before and after. Dropping them from `OAUTH_BUILTIN_LOCAL_TOOLS` also means a registry that *did* carry those names would now be forwarded with its real schema under the mapped name, which is the same fix `schedule` already got. (3) **Zero-caller items**: the 154-line SVG artifact generator in `app/tests/onboarding_golden.rs` (6 functions, roots unreferenced), `seed_git_info_cache_for_tests`, a 39-line duplicate `shorten_model_name` in `info_widget_model.rs` (the live copy is `ui_status.rs:71`, reachable through `ui.rs`), `auth::external::preferred_unconsented_api_key_source`, `provider::models_catalog::fetch_openai_context_limits`, `provider::models::model_unavailability_detail_for_account` (both with their `pub use` rows), `TokenHashIndex::is_empty`, and the unused `proc_pidfdinfo` + `PROC_PIDFD{VNODEPATHINFO,SOCKETINFO,PIPEINFO}` libproc bindings in `jcode-core/src/stdin_detect.rs`. (4) **De-duplication**: `stable_hash_str`/`stable_hash_json` existed as byte-identical private copies in `jcode-app-core/src/agent.rs` and `jcode-tui/src/tui/app.rs` — now `pub` in the former and imported by the latter, with a doc note that these are process-local `DefaultHasher` fingerprints and *not* `jcode_provider_core::stable_hash_str`, which is SHA-256 because its output crosses processes. `provider::anthropic::AVAILABLE_MODELS` was a 12-entry copy of `jcode_provider_core::ALL_CLAUDE_MODELS` and is now a re-export (list verified identical at runtime). **Kept, and worth knowing why:** the `"Task" ⇄ "subagent"` maps in `jcode-provider-claude-cli-runtime` and the `task`/`task_runner` alias in `jcode-tui-tool-display` are *live* — the Claude CLI runs its own tools and `to_internal_tool_name` (lib.rs:393,503) renames its `Task` calls to `subagent` for display, which is also what makes `local.rs:373` reachable. `Disposition::Gap` in `jcode-harness-api`'s capability ledger is an unused-but-documented taxonomy variant, not residue. `copy_to_clipboard_osc52`, `is_file_controlled_debug_client` and `remove_registry_if_same` all *have* callers behind `cfg` gates that macOS compiles out; deleting them would break other targets, so they belong to the platform-residue question, not here. |
| **Platform residue** | Every non-macOS code path, in three waves. (1) **Windows.** All `cfg(windows)`/`cfg(not(unix))` code: `jcode-storage`'s 348-line deferred Windows-ACL hardening subsystem (`SECRET_HARDEN_*`, `schedule_windows_path_hardening`, `run_windows_hardening_worker`), `jcode-base`'s `WindowsPowerGuard` and 10 `platform.rs` dispatch arms (incl. the `taskkill.exe` tree-kill), `bash.rs`'s `cmd.exe /D /S /C` shell and `WINDOWS_SHELL_TOOL_DESCRIPTION`, `terminal-launch`'s `wt`/`cmd` spawners + `windows_arg_quote`/`windows_command_line` + `windows_portable_tests.rs`, the named-pipe halves of the transport/socket abstractions, `src/main.rs`'s 8 MB-stack shim, the Windows arm of `spawn_server`'s 120 s bootstrap poll, and every `windows-sys` manifest table (root, `jcode-base`, `jcode-core`). Five tests went with it, all asserting Windows behavior from a macOS host (`command_candidates_adds_extension_on_windows`, `windows_vt_mouse_modes_*`, `windows_guard_is_long_lived_*`, `windows_command_line_quotes_*`, and the `#[cfg(not(unix))]` half of the #715 resume-spawn pair — its behavioral half and two stronger arg-vector tests survive). (2) **Linux and other unix.** `cfg(target_os = "linux")` and `cfg(all(unix, not(target_os = "macos")))` code: `jcode-app-core/src/tool/open.rs`'s 322-line xdg/gio opener + niri browser-window-raising subsystem (`query_niri_windows`, `select_window_to_focus`, `normalize_desktop_entry_to_stems`, …), `reload_state.rs`'s 113-line inotify handoff waiter, `session_launch.rs`'s wmctrl/xdotool focus fallback, `network_retry.rs`'s `ip monitor`, `notifications.rs`'s `notify-send`, the `/proc` halves of `perf.rs`/`overnight.rs`/`ui_frame_metrics.rs`/`process_memory.rs`/`claude_live.rs` (pidfd + `/proc/<pid>/stat` identity), `power_inhibit.rs`'s `systemd-inhibit` platform, `jcode-core`'s `stdin_detect::linux`, `terminal-launch`'s gnome-terminal/konsole/xterm/foot spawners, and the wl-copy/xclip/xsel clipboard chains. (3) **Ungated residue** — the part no `cfg` marked and no compiler could find: `prompt.rs`'s entire `hardware_context` subsystem (`/sys/devices/virtual/dmi`, `/proc/cpuinfo`, `/proc/meminfo`, `lspci` — so the system prompt's "Hardware:" block has never rendered on this fork), `dictation.rs`'s `/proc` client-PID scan (`proc_children_map` → every `ClientCandidate` unreachable), `auth/commands.rs`'s WSL2 detection (`/proc/version`, so its `/mnt/c` DrvFs PATH filter was a constant `true`), `jcode-logging`'s watchdog resource reporting (`/proc/self/{statm,task,fd}` — every stall dump printed `rss_mb: 0 threads: 0 open_fds: 0`), `jcode-core`'s `process_fd_diagnostic_snapshot` (`/proc/self/fd`, so the EMFILE diagnostic was a constant all-zero string, and `connect_socket`'s EMFILE arm degenerated into the catch-all it sat above), `server/util.rs`'s `strip_deleted_suffix` (an identity function off Linux), and the `WAYLAND_DISPLAY`-gated clipboard read/paste paths. **Two pre-existing macOS gaps were exposed, not created, and deliberately left as gaps:** `process_memory::snapshot_with_source` returned `ProcessMemorySnapshot::default()` on every non-Linux target *at HEAD before this session*, so `/debug memory`, the runtime-memory log, `scripts/analyze_runtime_memory_log.py` and `docs/MEMORY_INCIDENT_RUNBOOK.md` have always read `None` here (a reader would be `proc_pidinfo`/`task_info`); likewise `overnight.rs`'s `detect_memory`/`detect_load`/`detect_battery` had `None` stubs for macOS (`sysctlbyname`/`getloadavg`/IOKit would fill them). Deleting those fields would have meant a cross-crate teardown of a debugging surface whose real fix is a reader, so the code was left in place and its lying `/proc` doc comments were rewritten to say plainly what is unpopulated and which macOS API would populate it. **Kept deliberately:** `DISPLAY` passthrough (XQuartz sets it on macOS), `$XDG_RUNTIME_DIR`/`$XDG_CONFIG_HOME` as explicit user overrides, `jcode-transport`'s `#[cfg(not(unix))] compile_error!` guard, `command-risk`'s `/proc`+`/sys` protected-path list, and standalone `#[cfg(unix)]` gates (stripped only where their Windows twin's deletion made them vacuous). The one **user-visible** removal is in dictation, which the extended guard below caught after the cfg-driven waves had passed over it: `jcode-base/src/dictation.rs` reached the focused window through `niri msg -j focused-window` and typed transcripts with `wtype`, both Wayland-only binaries, so on macOS `focused_jcode_session()` always returned `None` and `type_text()` could only fail with "failed to launch `wtype`". Gone with them: `NiriFocusedWindow`, `resolve_session_from_window_title` + its two title-parsing helpers and their tests, the `focused_jcode_session` branch of `server/debug.rs`'s transcript-target resolution (the `last_focused_session()` file-cache branch under it is the live path and is unchanged), and the **`jcode dictate --type` flag** — `Args::Dictate` is now a unit variant and `run_dictate_command()` takes no argument. `run_configured`/`run_command` (the configured STT command) and `remember_last_focused_session` are untouched, so `jcode dictate` itself still works exactly as documented. This wave also settles the three symbols the *Dead-code sweep* row deferred to it: `is_file_controlled_debug_client` and `remove_registry_if_same` are **deleted** (their only callers were the Linux `PR_SET_PDEATHSIG` and pidfd blocks; `arm_debug_client_parent_death_signal` had already been an empty fn on macOS and is gone with its call site), while `copy_to_clipboard_osc52` is **kept** — it is the live tail of `copy_to_clipboard`'s macOS chain (arboard → pbcopy → OSC 52) and is now `#[cfg(not(test))]`, matching the only block that calls it. Net across the three waves: **~10,700 lines deleted**; suite 6,480 → 6,453 passing, 0 failed, with all 27 lost tests accounted for as platform-behavior assertions. |
| **First-run onboarding** | The whole guided first-launch subsystem, ~6,000 lines. Deleted files: `jcode-tui/src/tui/app/onboarding_{flow,flow_control,graph,repair,sim}.rs` (the `OnboardingPhase` state machine, the flow driver with its tick watchdog and import/repair orchestration, the descriptive state graph, the "press H to have another agent fix this" repair brief, and the `--onboarding-sim` screen simulator), `ui_onboarding.rs` (the welcome cards), and five test files including the 3,293-line `onboarding_eval.rs` scoring harness. With them went: 11 `App` fields, `SessionPickerMode::Onboarding` and the picker's whole action-only overlay mode (`OnboardingAction`, `PickerResult::StartNewSession`/`::ReviewRecentProject`, `render_onboarding_band`, `SessionFilterMode::ExternalClis`), the `/onboarding-sim` + `/onboarding-preview` dev commands and the `--onboarding-sim` CLI flag, `ui.rs`'s full-column takeover early-return, `BusEvent::OnboardingModelValidated`, and `jcode-import-core`'s 518-line `repo_ranking` module (its only production caller was the onboarding recent-project prefetch). **`prefer_strongest` came out of the protocol entirely** (`jcode-protocol` `Request::NotifyAuthChanged`, the server's `handle_notify_auth_changed` branch, `apply_auth_route_to_agent`, `Agent::set_route_selection_from_auth`, `auth::lifecycle::globally_preferred_default_route`): onboarding was its only `true` producer, every other call site already passed `false`. **The general surfaces onboarding sat on top of survived, collapsed:** `suggestion_prompts` keeps only its unauthenticated arm (the `("Log in to get started", "/login")` pair) because its new-user cards were gated on `is_new_user_install()`, which needs `launch_count ≤ 5` and was therefore already dead on any established install; `ui_prepare.rs`'s `is_initial_empty` is now purely "empty transcript, not processing, not streaming"; the resume picker keeps navigation, filtering, Enter-to-resume and rendering. `/login` was never routed through onboarding (`/login` → `show_interactive_login` → `open_login_picker_inline`) and is untouched. **Kept deliberately:** `scripts/onboarding_sandbox.sh` + `scripts/auth_fixture.sh` — a live general auth sandbox named in a user-facing error string at `jcode-base/src/auth/login_diagnostics.rs:112`, so the filenames stay accurate; and Gemini Code Assist's `onboardUser` REST API in `jcode-provider-gemini*`, which is Google's endpoint and unrelated. `docs/ONBOARDING_STATE_GRAPH.md` was deleted only after its §2.2/§2.3 — the sole documentation of the live `CredState` lifecycle, the rejected-fingerprint rule, `EnvFacts`/`Tri` and the `preferred_auth_method` table — was moved into `docs/AUTH_CREDENTIAL_SOURCES.md`. Also removed: the `onboarding state-space invariants` gate in `scripts/check_guardrails.sh` (it ran `cargo test … onboarding_graph::`) and the already-dead `scripts/capture_onboarding.sh`, whose one command invoked a test that no longer existed anywhere in the repo. Two tests were **rescued rather than deleted** into `app/tests/api_key_login.rs`: the API-key prompt's endpoint/no-static-default assertions, and the OpenRouter typed-key path (key reaches the input buffer, Enter submits without reopening the picker, key is persisted to `openrouter.env` and exported) — both lived in the onboarding test file but assert only general login behavior. |
| **First-party `jcode` subscription provider** | The curated "jcode subscription" product, ~3,900 lines: `jcode-base/src/{provider/jcode.rs (JcodeProvider), subscription_api.rs (device-code login, token poll, /v1/me, revoke, tier cache), subscription_catalog.rs (curated catalog, JcodeTier, JCODE_* runtime env)}`, `src/cli/login/jcode_device.rs` + tests, `src/cli/account.rs` with the entire `jcode account` CLI command (`Command::Account`, `AccountCommand::{Login,Status,Manage,Logout}` — all four ran on `subscription_api`/`subscription_catalog`, nothing third-party was in that file), and `jcode-tui`'s `subscribe_nudge.rs` (the `/subscribe`, `/hosted` and `/subscription` hosted-model pitch, whose delivery rule was literally "never for users who already hold jcode account credentials"). Vocabulary removed across crates: `ProviderChoice::Jcode`, `LoginProviderTarget::Jcode`, `LoginProviderAuthStateKey::Jcode`, `RuntimeProviderId::Jcode`, `RuntimeKey::JcodeSubscription`, `ModelRouteApiMethod::JcodeSubscription`, `OpenRouterTransportState::JcodeSubscription`, `NativeProviderKind::Jcode`, the `id: "jcode"` login descriptor (`LOGIN_PROVIDERS` 46 → 45), `JCODE_LOGIN_PROVIDER`, `AuthStatus::jcode`, `probe_jcode_status`, and the `"jcode"`/`"jcode-subscription"`/`"subscription"` aliases wherever they were parsed. Chain consequences: with both jcode arms gone, `disable_subscription_runtime_mode()` and its `_preserving_active_provider_profile` variant were unopposed at all 24 call sites and went with the runtime-env mechanism they wrapped; `set_model_on_jcode_subscription` and `ensure_model_allowed_for_subscription` lost their only caller; `is_jcode_subscription_runtime()` was the sole gate on two OpenRouter transport special cases. **Data compatibility, verified not assumed:** a persisted `provider_key = "jcode"` is a plain `Option<String>`, not an enum, so an old session still loads and renders; `ModelRouteApiMethod::parse("jcode-subscription")` now falls through its pre-existing `_ =>` to `Other(..)` rather than panicking; `RuntimeKey` is a live wire type with no persistence site in the repo. **Kept, and the distinction that governs this row:** every *third-party* subscription auth path — Claude Pro/Max via `auth/claude.rs`, ChatGPT/Codex OAuth, Copilot, Gemini Code Assist's free tier, `PremiumMode` — plus the TUI's multi-provider `/account` centre, which is a different surface from the deleted CLI command and is what the README advertises for multi-account switching. `channel_subscriptions` in `jcode-app-core` (241 of that crate's 267 `subscription` hits) is swarm chat pub/sub and was never related. |

**Deliberately *not* removed** (easy to delete by mistake):

- `jcode-base/src/login_qr.rs` + `qrcode` dep — OAuth device-code login, 14 references across 7 files
- `jcode-base/src/session/load_telemetry.rs` — session-load burst detection, **not** analytics
- Provider KV-cache "telemetry" in `info_widget.rs` / `state_ui.rs` — the cold-cache warning
- `ApiEvent::PermissionRequest` in `jcode-harness-api` / `jcode-sdk` — harness API wire protocol
- All 18 remaining `jcode-provider-*` crates and every OAuth flow

**Also excluded by choice** (not purged, just not merged from upstream):
discovery reframing.

**Why the Windows purge was judged safe, and where it deliberately stops.** Measured before
deleting: across the 236 upstream commits of the v0.75.3 and v0.76.0 syncs, every deleted file
saw **zero** changes except `install.ps1`, which saw one — and all ten existed at both range
starts, so that is a real figure rather than an artifact of recently-added files. The structural
argument is stronger still: `#[cfg(windows)]` code is never compiled on macOS, so deleting it
cannot change the macOS binary. Only four things could: removing a helper that is *not*
Windows-gated, removing a file still named by a `mod` macOS evaluates, changing a predicate
macOS evaluates, or breaking a shell script macOS runs.

The second of those is the live hazard and is worth remembering: `mod windows_hotkeys;` was
gated `cfg(any(test, windows))`, and that `test` term makes it **active under `cargo test` on
macOS**. A `cfg(windows)`-only module is invisible here, but a `cfg(any(test, …))` sibling is
not. `cargo test -p jcode-setup-hints` is the detector.

Formerly left alone, **now also purged** (see the *Platform residue* row in §1): the root and
per-crate `windows-sys` dependency tables, `src/main.rs`'s `cfg(windows)` 8 MB-stack shim, and
all Linux/other-unix code. The judgment that reversed it: those manifest entries were kept
because target-gated deps are never built here and churn against them is pure conflict cost,
but once the *code* behind every gate was gone the entries could only mislead — and a stale
`[target.'cfg(windows)'.dependencies]` table is precisely the hook an upstream merge uses to
re-land the code. The two manifests edited by the original pass — `jcode-setup-hints` and
`jcode-transport` — both had zero churn over the same 236 commits and their tables were
entirely dead.

What that purge cannot protect against is a *reintroduced* platform branch: `cfg(windows)` and
`cfg(target_os = "linux")` code still compiles away silently on macOS, so neither the compiler
nor the test suite objects to it coming back. That is the guard's job below.

### The guard

`scripts/purge-guard.sh` encodes the invariant. Run it after every sync.

```bash
./scripts/purge-guard.sh          # non-test code; exit 0 = clean
./scripts/purge-guard.sh --all    # include tests (this fork does not maintain tests)
```

Keep its patterns **tight**. A guard that cries wolf gets ignored — bare `request_permission`
matches an unrelated Grok provider method; the quoted tool name and the type name do not.

### The three scripts, and why they must move together

| Script | Direction | Answers |
|---|---|---|
| `purge-guard.sh` | defensive | is purged code present in the tree? |
| `classify-upstream.sh` | defensive | which incoming commits *risk* reintroducing it? |
| `upstream-features.sh` | **offensive** | what did upstream add, and did it actually land? |

The Windows purge extended all three together: a `find scripts -name '*.ps1'` section plus
`mod windows_{setup,hotkeys}` / `listen_windows_hotkey` identifiers in the guard, the same paths
in `PURGED_PATHS` so future upstream commits bucket as `PURE_PURGED`, and a **tight**
`PURGED_DESC` in the feature script. Tight matters here more than usual: a bare `windows` would
match Windows Terminal detection, `wt.exe` launching, and `cmd.exe` shell selection — all of
which §1 keeps — and a false `PURGED` label hides real upstream work, which is exactly the
failure that script exists to prevent.

The desktop-app and Bedrock purges were wired in the same three places: deleted trees and
`docs/{DESKTOP*,AWS_BEDROCK_PROVIDER}.md` in the guard's tree scan, the three deleted crates
added to its manifest grep, `jcode_desktop2` / `jcode_math` / `jcode_provider_bedrock` /
`provider::bedrock` / `BedrockProvider` / `AWS_BEARER_TOKEN_BEDROCK` / `JCODE_BEDROCK_` in the
API-call-site pattern, and the `::Bedrock` enum variants plus `SelfDevBuildTarget::Desktop2` in
the registration pattern. Bedrock demanded the tightest patterns in the file: a bare `bedrock`
matches three things §1 explicitly keeps — the hosted jcode router's "routed server-side to
Amazon Bedrock" model notes, OpenRouter's upstream list, and `docs/proposals/JCODE_CLOUD_AWS.md` — so the
prose pattern is `aws bedrock|bedrock provider|bedrock api key|--provider bedrock` and never the
bare word. `desktop` is the same trap in the other direction: it would match the macOS
desktop-notification stack, so the prose pattern says `desktop app`/`desktop2`/`vello`/`parley`.

All three encode the same §1 invariant in three different vocabularies — a tree scan, a commit
walk, and a changelog/structural read. **Change one, change all three.** A classifier that lags
the guard reports work as safe that the guard later rejects; a feature script that lags either
one reports purged code as missing upstream work.

One subtlety worth stating, because it has already caused a false alarm: prose patterns and
identifier patterns are *not* interchangeable. A changelog line says "pair your phone"; the
manifest says `jcode-gateway-types`; the config type says `GatewayConfig`. Each subsystem needs
a **bare stem** in the identifier pattern, not just its crate name.

### Fork-local replacements (not purges, but sync-fragile)

Deletions are only half the divergence. This fork also *replaces* upstream implementations in
place, and those are easier to lose in a sync than a purge is: there is no guard entry to trip,
because the symbol upstream expects still exists somewhere in the tree.

| Subsystem | Upstream | This fork |
|---|---|---|
| `agentgrep` **grep mode** | `agentgrep::search::run_grep` spawns an `rg` subprocess and silently falls back to a hand-rolled walker when the binary is absent; results render through `agentgrep::render::render_grep_output`; the tool schema exposes no search options beyond query/path/glob/type/hidden/no_ignore/paths_only | `crates/jcode-app-core/src/tool/agentgrep/rg.rs` links ripgrep's own crates (`grep-searcher`, `grep-regex`, `grep-pcre2`, `grep-matcher`, `ignore`, `globset` — all published from `BurntSushi/ripgrep/crates/*`) and searches in-process with no subprocess, no `PATH` lookup, and no walker fallback. Grouping and rendering are reimplemented here because upstream's `MatchGroup::match_indices` is private with no public constructor, so an external caller cannot build a grouped result. Six engine options are layered on top: `case` (smart by default, so an all-lowercase query is case-insensitive — upstream is always case-sensitive), `word`, `multiline`, `context_lines` (0-5), `max_matches_per_file` (default 1000), and `engine` (`rust` by default, `pcre2` opt-in for look-around and backreferences). Engine selection is never inferred; a Rust-engine refusal that PCRE2 could take appends `retry with engine="pcre2"`, gated on `regex-syntax`'s structured `ErrorKind::UnsupportedLookAround`/`UnsupportedBackreference` so a merely malformed pattern is not redirected to an engine that would accept it and match nothing. `find`/`outline`/`trace` still route through the upstream crate. |

**What a sync must check:** if `execute_linked_agentgrep`'s `"grep"` arm goes back to calling
`run_grep`/`render_grep_output`, the subprocess dependency and the silent walker fallback
return, and the six options above vanish from the tool surface (`AgentGrepInput` carries the
fields, but only `build_grep_request` reads them). Parity is verifiable — `rg.rs`'s tests use an
installed `rg` as an oracle where present, and one test runs with `PATH=/nonexistent` to prove
the search path is in-process.

PCRE2 costs three crates (`grep-pcre2`, `pcre2`, `pcre2-sys`) and a C build step, and it has no
linear-time guarantee, which is why it is opt-in per call and never the default.
`pcre2-sys` links Homebrew's `libpcre2-8.dylib` whenever
pkg-config finds one, which would make the installed binary depend on a brew formula, so
`.cargo/config.toml` sets `PCRE2_SYS_STATIC = "1"` to force the vendored static build. Check a
built binary with `otool -L target/<profile>/jcode | grep pcre` — it must print nothing.

---

## 2. Full uninstall (clean slate)

`scripts/uninstall.sh` alone is **not sufficient**. Run these in order.

```bash
# 1. Retire any LaunchAgent left behind by a pre-purge install. The global
#    launch-hotkey subsystem is gone from this fork, but a KeepAlive=1 +
#    RunAtLoad=1 agent installed by an older build respawns forever, including
#    after you delete the binary it points at.
launchctl bootout gui/$UID/com.jcode.hotkey 2>/dev/null
rm -f ~/Library/LaunchAgents/com.jcode.hotkey.plist
rm -rf ~/.jcode/hotkey

# 2. Stop processes uninstall.sh does not match.
#    Its pkill pattern is 'jcode( .*)? serve' — menubar survives.
pkill -f 'jcode menubar'; pkill -f 'jcode setup-hotkey'

# 3. Uninstall.
bash scripts/uninstall.sh --yes            # binaries + apps, KEEPS ~/.jcode
bash scripts/uninstall.sh --purge --yes    # ALSO wipes ~/.jcode (see warning)

# 4. Stale sockets (macOS puts them under /var/folders, not /tmp).
rm -f /var/folders/*/*/T/jcode*.sock

# 5. Artifacts OUTSIDE ~/.jcode that no jcode uninstall path touches.
#    A pre-purge `jcode setup-hotkey` ADDED a SessionStart hook to Claude Code
#    and Codex; after the binary is gone it fails on every session start.
python3 - <<'PY'
import json, os
p = os.path.expanduser('~/.claude/settings.json')
d = json.load(open(p)); h = d.get('hooks', {})
ss = [e for e in h.get('SessionStart', []) if 'jcode' not in json.dumps(e)]
if ss: h['SessionStart'] = ss
else: h.pop('SessionStart', None)
if not h: d.pop('hooks', None)
json.dump(d, open(p, 'w'), indent=2, sort_keys=True); open(p, 'a').write('\n')
PY
grep -c -i jcode ~/.codex/config.toml   # check Codex too
```

> **`--purge` destroys live state, not build artifacts.** It deletes all of `~/.jcode`:
> `auth.json` (every provider login), all sessions, the memory graph, `config.toml`, and the
> ~87 MB embedding model. **None of that is needed for a clean rebuild.** Only use `--purge`
> when a genuinely fresh first-run state is the goal. Confirm with the user explicitly.

**Verify:**
```bash
for p in ~/.jcode ~/.local/bin/jcode ~/Applications/Jcode.app \
         ~/Library/LaunchAgents/com.jcode.hotkey.plist; do
  [ -e "$p" ] && echo "PRESENT: $p"; done
launchctl list | grep -c jcode; pgrep -c jcode; command -v jcode
```

---

## 3. Sync with upstream

All analysis happens in a **throwaway clone**. The main repo is not touched until the merge
is verified.

### 3.1 Stage in tmp

```bash
MAIN=/Users/indrajeetadityaroy/jcode
WS=/tmp/jcode-sync && rm -rf "$WS" && mkdir -p "$WS"

cd "$MAIN"
git status --porcelain        # MUST be empty

# Push BEFORE you start. The tmp clone below copies $MAIN, so the sync works
# unpushed — but then the merge you are about to attempt, and the rollback tag
# that protects it, exist on exactly one disk. Pushing also makes the next
# sync's BASE reproducible from a second checkout.
git push origin master
git tag -a "pre-merge-$(date +%F)" -m "rollback point" HEAD   # -a: annotated tags may be forced
git push origin "pre-merge-$(date +%F)"                       # tags are NOT pushed by `git push`

git rev-list --left-right --count origin/master...master      # MUST print "0	0"

git clone -q --no-hardlinks "$MAIN" "$WS/fork"
cd "$WS/fork"
git remote add upstream https://github.com/1jehuang/jcode.git
git fetch --no-tags upstream master

BASE=$(git rev-parse HEAD)
git reset --hard "$BASE" && git clean -fd     # pristine start — see Pitfall 3
git merge --no-commit --no-ff upstream/master
test -f .git/MERGE_HEAD || echo "MERGE DID NOT START"
```

### 3.2 Classify before resolving

Most upstream commits never touch purged code. Classifying tells you how much judgement is
actually required, and — more importantly — **which shas to hand-audit**.

`scripts/classify-upstream.sh` does this. Run it from inside the tmp clone:

```bash
cd "$WS/fork"
./scripts/classify-upstream.sh "$BASE" upstream/master --list-mixed
```

| Bucket | Meaning | Action |
|---|---|---|
| `PURE_KEPT` | no purged path, no purged identifier added | none — the merge handles it |
| `MIXED` | touches files we keep **and** adds purged identifiers | **hand-audit every one** (§3.3) |
| `PURE_PURGED` | touches only purged paths | `DU` conflicts → `git rm` |
| `EMPTY` | no file changes | none |

Its patterns are deliberately kept in sync with `scripts/purge-guard.sh`. **Change one, change
both** — a classifier that lags the guard reports work as safe that the guard will later reject.

**Validation.** Replaying the previous sync's range reproduces its total exactly and identifies
the commits that actually caused damage:

```
$ ./scripts/classify-upstream.sh f3f48aa3d fd1ff012c
  PURE_KEPT    137     MIXED  12     PURE_PURGED  6     EMPTY  4     TOTAL 159
```

Its `MIXED` list contains both build-breakers that §3.4 previously caught only *after* the
compiler failed — `25463c35c` (todo traceability + telemetry call) and `659b8cc15` (Grok Build
login + `record_auth_surface_blocked`) — plus the four subscription-onboarding commits §1
excludes by choice.

> An earlier hand-rolled pass recorded this range as 136/9/10/4. That number is **not
> reproducible** and is superseded by the script. The script buckets a commit touching both
> purged and kept paths as `MIXED`/`PURE_KEPT` rather than `PURE_PURGED`, which is why it
> reports more `MIXED` and fewer `PURE_PURGED`. Erring toward `MIXED` is the safe direction.

### 3.2b Inventory what upstream added

§3.2, `purge-guard.sh`, and the compiler are all **defensive** — they detect purged code that
should not be present. None of them can detect the opposite failure: **upstream work that should
be present and is not.** A feature dropped during conflict resolution leaves no trace. No
conflict marker, no compile error, no guard hit. It simply never arrives, and nobody looks for
it because nobody read what upstream shipped.

That is how a sync quietly turns into a downgrade.

```bash
cd "$WS/fork"
./scripts/upstream-features.sh inventory "$BASE" upstream/master
```

The authoritative source is upstream's own `changelog/vX.Y.Z.json` files — human-written
`highlights` / `improvements` / `fixes` per release. Commit subjects are a weak substitute:
in the v0.75.3→v0.76.0 range only 5 of 77 commits are tagged `feat`, and 21 use no conventional
prefix at all.

Each line is marked:

| Mark | Meaning |
|---|---|
| `KEEP` | must be present after the merge — verify it |
| `PURGED` | §1 deletes this subsystem; its absence is correct |
| `EXCLUDED` | §1 "excluded by choice" (e.g. the subscription onboarding pill) |

The script also lists the **structural surface** upstream grew — new workspace members, CLI
subcommands, registered tools, config fields, scripts — because those are mechanically
checkable later, unlike prose.

> **Cross-reference the `KEEP` list against §3.2's `MIXED` shas.** A `KEEP` feature whose commit
> landed in `MIXED` is the highest-risk item in the whole sync: that commit needed hand-surgery,
> so it is exactly where a feature gets stripped along with the purged code sharing its hunk.

After the transfer (§3.5), assert the structural items actually landed:

```bash
cd "$MAIN"
./scripts/upstream-features.sh verify "$BASE" upstream/master   # exit 0 = nothing dropped
```

Replaying the *previous* sync through this reports `all KEEP items present` — its one KEEP
structural item (`jcode-provider-grok-build-runtime`) is on disk, and the gateway / telemetry /
permissions crates, the `Pair` and `Permissions` subcommands, and the `GatewayConfig` /
`SponsorsConfig` fields are all correctly skipped rather than reported as losses.

### 3.3 Resolve

**`DU` (deleted by us / modified upstream)** — mechanical, keep the deletion:
```bash
git status --porcelain | awk '$1=="DU"{print $2}' | while read -r f; do git rm -q "$f"; done
```

**`UU` (both modified)** — ⚠️ **this is where the fork gets damaged.**

> **Never use `git checkout --ours -- <file>`.** It is **file-level, not hunk-level**: it
> discards *every* upstream change to that file, not just the conflicting hunk. Doing this on
> 11 files silently dropped ~800 lines of upstream work; the compiler caught only 6 errors and
> the rest would have shipped as silent feature loss.

Correct method — **take upstream's file, then strip only the purged parts**:
```bash
git checkout <upstream-sha> -- <file>
# then delete just the purged blocks (telemetry calls, discovery registration, ...)
```

Typical strips: `mod discover;` · the `integration_tools` registration · `RequestPermissionTool`
registration · `record_tool_execution` · `todo_telemetry_update` + the `record_todo_gate` loop ·
the `[sponsors]` config template block.

Mixed hunks need judgement. Example from the last sync — upstream added a real feature *and*
telemetry in one hunk:
```rust
// upstream
self.append_user_context_message_with_display_role(user_message, images, display_role)?;
crate::telemetry::record_turn();
// resolution: keep line 1 (feature), drop line 2 (telemetry)
```

**Guardrail JSON baselines** (`scripts/*_budget.json`) — take upstream's; ours are already stale.

### 3.4 Catch silent reintroduction

Auto-merged files can gain purged references with **no conflict marker**. Last sync this found
two build-breakers that no gate would otherwise have caught:

- `state_ui_input_helpers.rs` referencing `SummaryPill::Subscription` (upstream grew onboarding
  from 2 pills to 4)
- `auth.rs` — upstream's new **GrokBuild** login target shipped with a `record_auth_surface_blocked()`
  call. Keep the feature, drop the call.

Both are now covered by `scripts/purge-guard.sh` (`SummaryPill::(Subscription|Telemetry)` and
the `record_*` patterns), and §3.2's classifier flags their commits up front. Run all three
checks — they fail differently:

```bash
# 1. Purged code reintroduced anywhere in the tree.
./scripts/purge-guard.sh

# 2. Module declarations without files — catches a `mod foo;` kept while foo.rs was purged.
python3 scripts/check_module_files.py

# 3. Upstream work that went MISSING. This is the check that caught the --ours
#    damage, and no other gate detects it: the guard only finds code that should
#    not be there, never code that should be and isn't.
git diff "$BASE"...upstream/master --stat > /tmp/upstream-expected.txt
git diff "$BASE"..HEAD             --stat > /tmp/ours-actual.txt
diff /tmp/upstream-expected.txt /tmp/ours-actual.txt   # every delta must be a deliberate purge
```

Then hand-audit each sha from §3.2's `--list-mixed` output. A `MIXED` commit that produced no
conflict is the most dangerous case in this entire runbook: it merged clean and carries purged
code.

### 3.5 Commit in tmp, then transfer

```bash
git add -A

# Gate 4 runs HERE, not in §6. `--staged` scans the index, and after the commit
# below there is nothing staged — running it in §6 silently scans nothing and
# passes. This is the only point where the incoming upstream files are staged.
gitleaks git --staged --redact --no-banner -v

git commit --no-verify -m "Merge upstream <ver>, excluding purged subsystems"
#          ^^^^^^^^^^^ gitleaks pre-commit hook false-positives on upstream's
#          keyboard-shortcut table (`key: "Ctrl+Shift+Tab"`). REVIEW the findings
#          from the scan above first — you are importing hundreds of upstream
#          files — then bypass.

git log -1 --pretty=%p        # MUST print TWO hashes. One = not recorded as a
                              # merge; future syncs will re-conflict everything.
```

Transfer the **verified commit** — never replay resolutions by hand. §3.1 clones `$MAIN`, so
the tmp clone is on **`master`**; there is no `merge-upstream` branch to fetch:
```bash
cd "$MAIN"
git fetch "$WS/fork" master:verified
git merge --ff-only verified
```

> **`--ff-only` constrains ordering for the whole sync.** It succeeds only while `$MAIN`'s HEAD
> is still the merge's first parent. Any commit landed in `$MAIN` between the §3.1 clone and
> this transfer breaks it. Stash unrelated work (tooling fixes, doc edits) and commit it
> *after* the transfer, not before.

### 3.6 Update this document

The merge is not finished until the runbook describes the state it produced. Skipping this is
what makes the *next* sync classify against a stale base.

```bash
cd "$MAIN"
git log -1 --pretty='%h' verified^2      # the upstream parent = new "last upstream merge"
grep -m1 '^version' Cargo.toml           # the new version string
```

1. **§0 — update the "Last upstream merge" row** to that sha and version.
2. **Appendix — add any new pitfall** this sync cost you, with the symptom that revealed it.
3. **§1 — record any newly purged or newly kept subsystem**, and update all three scripts
   together if the pattern set changed: `purge-guard.sh`, `classify-upstream.sh`,
   `upstream-features.sh` (both its prose *and* identifier patterns).
4. **Skim the `KEEP` inventory from §3.2b one last time.** Gate 10 only checks the
   mechanically verifiable items — new crates, subcommands, config fields. Prose highlights
   like "streams handle transient failures more reliably" cannot be asserted by a script; a
   human read is the only check they get.

Commit the doc update as part of the sync, before building — §4 requires a clean tree.

> **Every commit must land before §4.** §5 derives the install directory from
> `git rev-parse --short HEAD`, and §4 bakes a hash into the binary. A commit made *after*
> the build — even a docs-only one — desynchronises the two, and the only fix is a **full
> LTO rebuild** to re-stamp. The same applies to editing a tracked file while a build is in
> flight: `jcode-build-meta` computes the dirty flag when its build script runs, so a
> mid-build edit can stamp `-dirty` and cost another rebuild. Finish all writing, commit,
> *then* build. This cost two of the three rebuilds in the v0.76.0 sync.

---

## 4. Build

```bash
cd "$MAIN"
git status --porcelain        # empty, so the version string is reproducible

# Pin the hash. Without this the binary embeds whatever hash the build script
# last cached — see Pitfall 10; committing does NOT refresh it.
JCODE_BUILD_GIT_HASH="$(git rev-parse --short HEAD)" \
  cargo build --profile release-lto 2>&1; echo "CARGO_EXIT=$?"
```

- **Never pipe cargo through `tail`/`head`** — you get the *pager's* exit code and a failed
  build reports success. Echo `$?` from cargo directly.
- `cargo build` does **not** compile `tests/` or `#[cfg(test)]`. A green build says nothing
  about the test suite. This fork does not maintain tests; treat `cargo check --all-targets`
  as advisory only (it always fails here).
- `release-lto` = thin LTO, ~5–7 min cold on an M-series laptop at the repo's pinned
  `jobs = 4` (`.cargo/config.toml`, deliberate RAM cap).
- **Commit before building, but do not rely on it for the stamp.** A clean tree keeps the
  `-dirty` suffix off, which is real — but the embedded *hash* comes from a build-script cache
  that git activity deliberately does not invalidate. Pin it with `JCODE_BUILD_GIT_HASH` as
  above. `touch crates/jcode-build-meta/build.rs` also works and costs the same full LTO
  rebuild, but the env override is the mechanism the build script documents as intended.
- **A failed build still poisons the stamp.** The cache survives the failure, so a later
  successful build silently inherits the *earlier* HEAD. This is why the env override matters
  more than build ordering.
- **Do not touch tracked files while the build runs**, and make no further commits until §5 is
  done. Each violation costs a full re-stamp rebuild (~6–10 min at the pinned `jobs = 4`).

---

## 5. Install

Nothing running? Then this is pure file operations. `scripts/install_release.sh` does the same
copy/symlink dance and additionally, on macOS, runs `jcode setup-launcher` (installing
`Jcode.app` plus the turn-notification broker, best-effort), calls `jcode server reload`, and
**edits your shell rc files** via `jcode_configure_path` (`scripts/lib/configure_path.sh`) — do
it manually to skip those. It no longer registers global launch hotkeys: that subsystem was
purged (§1).

```bash
cd "$MAIN"
bin="$PWD/target/release-lto/jcode"
hash="$(git rev-parse --short HEAD)"
[ -n "$(git status --porcelain)" ] && hash="${hash}-dirty"

builds="$HOME/.jcode/builds"; vdir="$builds/versions/$hash"
mkdir -p "$vdir" "$builds/stable" "$builds/current" "$HOME/.local/bin"
install -m 755 "$bin" "$vdir/jcode"
ln -sfn "$vdir/jcode" "$builds/stable/jcode"
ln -sfn "$vdir/jcode" "$builds/current/jcode"
printf '%s\n' "$hash" > "$builds/stable-version"
printf '%s\n' "$hash" > "$builds/current-version"
ln -sfn "$builds/current/jcode" "$HOME/.local/bin/jcode"
```

**Always kill the two long-running helpers** — they are separate processes holding the old
binary and no reload reaches them:
```bash
pkill -f 'jcode menubar'; pkill -f 'jcode setup-hotkey'
```

**If a `jcode serve` daemon is running** (`pgrep -f 'jcode( .*)? serve'`), adopt the new binary:
```bash
jcode server reload        # exec()s into the new binary on the same socket
python3 - <<'PY'
import json, os
p = os.path.expanduser('~/.jcode/servers.json')
d = json.load(open(p)) if os.path.exists(p) else {}
if not d:
    print("no registered servers — nothing to adopt (expected when no daemon runs)")
else:
    for name, s in d.items():
        print(f"{name}: {s.get('git_hash', '<absent>')}")
PY
# ^ every git_hash must change — that is the proof the daemon adopted the new binary
```

> `servers.json` is `{}` whenever no daemon is registered. The previous one-liner here indexed
> `list(d)[0]` unconditionally and died with `IndexError` in exactly that case, which is the
> normal state after a clean shutdown. The block above reports instead of crashing.

**Rollback:** repoint `current`/`stable` at the previous `versions/<hash>/` and reload. No
rebuild. (After a `--purge` there is no previous version, so rollback becomes
`git reset --hard pre-merge-<date>` + rebuild.)

**Keep at least one previous `versions/<hash>/`.** That directory *is* the rollback; the store
is append-only by design. Prune older ones only when more than two are present.

---

## 6. Verification gates

Gates are **not all run at the same point.** Two of them can only pass at one specific moment;
running them in this table's position instead makes them vacuous. The "When" column is binding.

| # | Gate | When | Command | Pass |
|---|---|---|---|---|
| 1 | Purge invariant | §3.4, tmp | `./scripts/purge-guard.sh` | exit 0 |
| 2 | Module decls resolve | §3.4, tmp | `python3 scripts/check_module_files.py` | exit 0 |
| 3 | Lockfile coherent | §3.4, tmp | `cargo metadata --locked --format-version 1 >/dev/null` | exit 0 |
| 4 | Secret scan of incoming | **§3.5, staged, pre-commit** | `gitleaks git --staged --redact --no-banner -v` | reviewed |
| 5 | Merge recorded | **§3.5, post-commit** | `git log -1 --pretty=%p` | **two** hashes |
| 6 | Build | §4, `$MAIN` | `cargo build --profile release-lto; echo $?` | 0 |
| 7 | Smoke, isolated | §4, `$MAIN` | `./target/release-lto/jcode --no-update --socket /tmp/verify.sock run 'hi'` | see below |
| 8 | Removed CLI absent | §5, post-install | `jcode --help \| grep -E '^\s+(pair\|permissions\|self-dev\|setup-hotkey)\b'` | no match |
| 9 | Version reproducible | §5, post-install | `jcode --version` | matches HEAD, no `-dirty` |
| 10 | **No upstream work dropped** | §3.5, post-transfer | `./scripts/upstream-features.sh verify "$BASE" upstream/master` | exit 0 |

- **Gate 4 must precede the commit.** `--staged` scans the index; after `git commit` the index
  is empty and the scan passes having examined nothing.
- **Gate 5 must follow the commit** — it inspects `HEAD`'s parents.
- **Gates 8–9 must follow the symlink flip.** They invoke `jcode` from `PATH`, which resolves
  through `~/.local/bin/jcode` → `builds/current`. Before §5 they measure the *old* binary.
- **Gate 10 is the only gate that can fail on absence.** Gates 1–9 all check that something
  wrong is not there; gate 10 checks that something right *is*. Do not skip it because the
  build is green — a dropped feature compiles perfectly.

Gate 7 uses an **isolated socket** deliberately (`AGENTS.md`): it proves the *new* binary works
rather than an old daemon answering. Two distinct passes, depending on whether `~/.jcode/auth.json`
holds credentials:

| State | Expected output | Verdict |
|---|---|---|
| Credentials present | a real model reply, then a `[Tokens] upload: … download: …` line | **pass** (stronger — the full turn loop ran) |
| No credentials | stops at `No credentials configured for provider auto-detection` | **pass** (reached the provider layer) |

Anything else — a panic, a hang, a socket error, a missing-tool error — is a failure. Remove
`/tmp/verify.sock` afterwards so the next run does not adopt a stale socket.

The ratchet scripts (`check_code_size_budget.py`, `check_panic_budget.py`,
`check_swallowed_error_budget.py`, …) **already fail at upstream HEAD**. Verify against a
pristine `git worktree` before treating any as a regression, and never "fix" one with
`--update` — that silently absorbs every pre-existing violation.

---

## Appendix — pitfalls

Each of these cost real time. The symptom is what made it visible.

1. **`git checkout --ours <file>` is file-level.** *Symptom:* 6 `cannot find function` errors;
   actual damage ~800 lines across 11 files, mostly silent. *Fix:* take upstream's file, strip
   the purged parts.

2. **Piping cargo masks its exit code.** `cargo build … | tail -80` returns tail's status.
   *Symptom:* harness reported success on a build that failed with 9 errors.

3. **A merge can silently record as a non-merge.** After repeated abort/reset cycles,
   `git commit` produced a **one-parent** commit — content merged, history didn't, so re-merging
   re-conflicted everything. *Fix:* pristine `reset --hard && clean -fd` before merging; assert
   `git log -1 --pretty=%p` shows two hashes.

4. **`cargo build` skips test targets.** A green build coexists with a test suite that does not
   compile.

5. **A pre-purge `com.jcode.hotkey` LaunchAgent has `KeepAlive=1`.** The launch-hotkey
   subsystem is gone, but `uninstall.sh` never touched that agent and launchd respawns a
   deleted binary forever. Retire it *before* removing the binary (§2 step 1). The old
   `setup-hotkey --uninstall` path also *added* SessionStart hooks to Claude Code and Codex,
   so re-check `~/.claude/settings.json` and `~/.codex/config.toml` on any machine that ran it.

6. **The gitleaks pre-commit hook blocks the merge commit** on upstream's keyboard-shortcut
   table (`key: "Ctrl+Shift+Tab"`, `generic-api-key`, entropy 3.52). Review, then `--no-verify`.

7. **Closing the TUI does not stop the daemon.** `jcode serve` persists; check `pgrep`.

8. **`macos_notification_broker.rs` used `jcode::` instead of `crate::`** — a pre-existing
   upstream bug, invisible on Linux, that breaks the first macOS build. Fixed in this fork;
   re-check after syncs.

9. **`jcode-build-meta` caches the version stamp — and "commit first" does NOT fix it.**
    This entry previously said to commit before building. That advice is wrong: the build
    script *deliberately* does not declare `.git/HEAD` or `.git/index` as `rerun-if-changed`
    inputs, because doing so turned every `git add`/`git status` into a full-tree recompile
    (see the long comment in `crates/jcode-build-meta/build.rs`). Committing therefore does not
    invalidate the stamp at all.
    *Symptom:* after committing `a9ff3a78b` and rebuilding, `jcode --version` still reported
    `5c2f81b0e` — the hash cached by an **earlier failed build** in the same session. Gate 9
    fails and no amount of committing or rebuilding changes it.
    *Fix:* use the env overrides, which **are** declared `rerun-if-env-changed`:
    ```bash
    JCODE_BUILD_GIT_HASH="$(git rev-parse --short HEAD)" cargo build --profile release-lto
    ```
    The build script's own comment confirms this is the intended path: release/dist builds set
    `JCODE_RELEASE_BUILD=1` / `JCODE_BUILD_SEMVER` precisely so "released binaries always embed
    the exact version/hash", while ordinary dev builds accept hash lag as cosmetic.
    **Any build you are about to install is a release build, not a dev build** — §5 keys the
    install directory off the hash, so a binary that self-reports a different commit than the
    `versions/<hash>/` it lives in is exactly the incoherence gate 9 exists to catch.

10b. **A wrapper's trailing `echo` masks cargo's exit code too.** Pitfall 2 covers piping
    through `tail`. The same failure wears a second disguise: running
    `cargo build … > log; echo "CARGO_EXIT=$?" >> log` in a background task makes the *task*
    exit 0 because `echo` succeeded, and the harness then reports **"completed (exit code 0)"**
    for a build that failed with 101. *Fix:* never trust the task-completion status; read
    `CARGO_EXIT=` out of the log, and grep the log for `^error`.

10. **`tail -f` monitors never self-terminate.** For "tell me when the build finishes", use a
    background command that exits — the harness notifies on completion by itself.

11. **`~/.jcode` reappears** from any `jcode --version` invocation (migration markers + a log).
    Harmless; not a failed purge.

12. **`gitleaks --staged` after committing scans nothing and passes.** The gate table used to
    list it as step 4 of a post-merge block, by which point §3.5 had already committed.
    *Symptom:* a "reviewed" secret gate on a sync that imported 133 upstream files without ever
    examining one. *Fix:* gate 4 now runs inside §3.5 while the index is populated.

13. **`git push` does not push tags.** The rollback tag is the entire recovery story for §3, and
    `git push origin master` leaves it local. *Symptom:* `git ls-remote --tags origin` empty
    while `git tag` listed `pre-merge-…`. *Fix:* §3.1 pushes the tag explicitly.

14. **GitHub orders commit history by author date, not push time.** After pushing a merge whose
    commits were authored days earlier, nothing appears at today's date and the contribution
    graph stays blank. *Symptom:* "the push does not appear on the repo commit history" when
    `git ls-remote` and the GitHub API both confirmed it had landed. Not a fault — verify with
    `gh api repos/<owner>/<repo>/commits?sha=master` before debugging a push.

15. **`servers.json` is `{}` when no daemon is registered**, which is the normal post-shutdown
    state — not an error. The old §5 proof one-liner indexed `list(d)[0]` and raised
    `IndexError` there. *Fix:* iterate and report (§5).

16. **A classifier that drifts from the guard is worse than none.** `classify-upstream.sh` and
    `purge-guard.sh` duplicate the pattern set by necessity (one walks commits, one walks the
    tree). If only one is updated, the classifier reports work as safe that the guard later
    rejects — after you have already resolved it. Change both together.

17. **Every gate was defensive; none checked that upstream work arrived.** A feature dropped in
    resolution produces no conflict, no compile error and no guard hit — a clean green sync that
    is silently a downgrade. *Symptom:* none, which is the point; it was found by reasoning about
    what the gates could not see, not by an incident. *Fix:* §3.2b + gate 10.

18. **A crate-name pattern does not match a type name.** `jcode-gateway-types` in the identifier
    list left `GatewayConfig` unmatched, so three purged config fields were reported as dropped
    upstream work. *Symptom:* `MISS config field bind_addr` on a sync that had correctly deleted
    it. *Fix:* bare stems (`gateway`, not `jcode-gateway-types`).

19. **BSD `sed` does not understand `\s`.** On macOS the extraction silently returns the entire
    diff line instead of the identifier, and every downstream check reports a false MISS.
    *Symptom:* `expected /^\s++    Pair {\s*[({,]/`. *Fix:* `[[:space:]]` in every `sed`
    expression. GNU-only regex shorthands are a recurring hazard in this repo's scripts —
    `grep -E` accepts `\s` here, `sed -E` does not.

20. **v0.76.0 shipped its headline feature *inside* a purged subsystem.** "Opt-in transcript
    telemetry" meant the sync's biggest diff was code we delete, while its real keepers (Grok
    Build TUI login, Anthropic-compatible profiles) were small. *Lesson:* commit count is a poor
    proxy for effort — 77 commits with 3 `MIXED` took longer than the prior 159 with 12, because
    the telemetry surface reached into `agent.rs`, `args.rs`, `dispatch.rs`, `startup.rs` and
    `mod.rs` and **almost none of it conflicted**.

21. **`git rm` refuses on merge-added files.** Purged files that arrive as clean `A` additions
    (`.github/`, `src/cli/telemetry.rs`) are already staged, so `git rm` errors with "changes
    staged in the index". *Fix:* `git rm -r -f`. Do not `--cached`, which keeps them on disk.

22. **The tmp clone is on `master`, not `merge-upstream`.** §3.5 documented a branch §3.1 never
    creates. *Symptom:* `fatal: couldn't find remote ref merge-upstream`. Fixed in §3.5.

23. **Deleting a purged struct field by line leaves its doc comment, and E0585 cascades.**
    Stripping `transcript_telemetry_sent` with a line filter orphaned the `///` above it. That
    is a *parse* error, so the whole `Agent` struct mis-parsed and **12 additional bogus E0308 /
    E0277 "mismatched types" errors** appeared in files the merge never touched
    (`turn_execution.rs`), which reads exactly like a real semantic merge break.
    *Fix:* delete the doc comment with the field, and always fix the **first** error before
    believing any that follow. `cargo check -p <crate>` confirms in seconds what a full LTO
    build takes minutes to re-prove.

24. **Committing after the build costs a full rebuild.** A docs-only commit moved HEAD from
    `a9ff3a78b` to `4c25acf91`; §5 would then have installed a binary stamped `a9ff3a78b` into
    `versions/4c25acf91`. *Symptom:* nothing at build time — it surfaces only as a gate 9
    mismatch, or worse, silently as a mislabelled install directory that the next rollback
    trusts. *Fix:* land every commit before §4; see the callout in §3.6.

25. **`jcode menubar` has no KeepAlive and stays down until launched again.** Unlike the
    pre-purge hotkey LaunchAgent (pitfall 5), nothing restarts it after the §5 symlink flip,
    so relaunch it by hand if you want the session-count indicator back.

26. **A `cfg(windows)`-only module is invisible to macOS `cargo test`; a `cfg(any(test, …))`
    sibling is not.** Planning the Windows purge, `windows_hotkeys.rs` looked orphaned once
    `windows_setup.rs` (its only caller) was deleted — so the plan claimed the deletion would
    surface as `dead_code`. Wrong twice over. `windows_setup.rs` is `cfg(windows)` with no `test`
    term, so it never compiled here to begin with; and `windows_hotkeys.rs` carries its own
    `#[cfg(test)] mod tests` with 8 tests, four of its functions existing purely to be exercised
    off-Windows. *Symptom:* a plan that reasons about dead code on a platform where the code in
    question is not compiled at all. *The real hazard runs the other way:* `mod windows_hotkeys;`
    was gated `cfg(any(test, windows))`, which **is** active under `cargo test` on macOS, so
    deleting the file without its declaration breaks the test build while `cargo check` stays
    green. Check the `mod` line's predicate, not the file's, and run `cargo test -p <crate>`.

27. **`scripts/warning_budget.txt` was a dead gate, and the purge is what proved it.** The
    baseline read `0` while `cargo check -q` reported 10, all of it pre-existing fork-purge
    fallout (an import whose enum variants were purged, a `cfg(unix)` const whose only user is
    `cfg(target_os = "linux")`, and Linux/macOS dead code upstream never trips). A gate that can
    only fail tells you nothing. Measured at pre-purge `f6038aae2`: **10**; after the Windows
    purge plus two in-scope fixes: **8** — so the purge added none. The baseline is now 8, an
    honest ratchet that will actually catch the next regression. Do not "fix" it back to 0 by
    lowering the number; lower it by removing warnings.
