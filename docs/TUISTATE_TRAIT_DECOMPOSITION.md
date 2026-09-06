# TuiState Trait Decomposition Plan

Status: **unrealized proposal.** Zero sub-traits have been extracted. The only
part of this plan in the tree is step 1 (the section-header comments inside the
trait). Everything from "Proposed target shape" down is design, not code.

This document audits the `TuiState` trait (`crates/jcode-tui/src/tui/mod.rs`) and
proposes a safe, incremental decomposition. It is the Phase 1.5 follow-on to the
`App` god-object decomposition (see `plans/CLIENT_CORE_PRESENTATION_SPLIT_PLAN.md`).

## Current state

- `pub trait TuiState` (`crates/jcode-tui/src/tui/mod.rs:174`) exposes **135
  methods** (`awk '/pub trait TuiState/,/^}/' crates/jcode-tui/src/tui/mod.rs |
  grep -c '    fn '`).
- Implementors: 2 (`App` in `tui/app/tui_state.rs:525`, and `TestState` in
  `tui/ui_tests/mod.rs:156`).
- Consumers: 92 `dyn TuiState` mentions across 18 files; 68 render-function
  parameters are spelled `app: &dyn TuiState`.

It is the presentation-layer counterpart to the `App` god-object: a single wide
interface that couples every render module to the entire client surface.

## Why a naive sub-trait split has limited value

Two structural facts constrain the refactor:

1. **`App` implements the whole surface regardless.** Splitting `TuiState` into
   `TuiTranscriptState + TuiInputState + ...` does not reduce what `App` must
   implement, and (because the trait is presentation-only data access) it does
   not change crate-level compile coupling. The win is intent/navigability, not
   decoupling of `App`.

2. **`&dyn TuiState` does not compose.** Render functions take trait objects.
   Rust has no stable `&dyn (A + B)`, so any consumer that needs methods from
   more than one domain must take a supertrait that re-aggregates them. The
   widest consumers (`ui.rs`, `ui_prepare.rs`, `ui_input.rs`,
   `redraw_schedule.rs`, `ui_viewport.rs`) use methods from most domains, so
   they would keep the full supertrait bound.

Measured (approximate: per-file scan for calls to each trait method name, so a
name shared with an inherent method can inflate a row): 17 modules besides the
trait definition take `&dyn TuiState`, and only **4** of them stay inside a
single domain:

| Sections used | Modules |
|---:|---|
| 11 | `ui.rs` |
| 10 | `ui_prepare.rs`, `ui_input.rs` |
| 8 | `redraw_schedule.rs` |
| 7 | `ui_viewport.rs` |
| 6 | `ui_frame_metrics.rs` |
| 4 | `ui_header.rs` |
| 3 | `ui_overlays.rs`, `ui_file_diff.rs` |
| 2 | `ui_status.rs`, `ui_pinned.rs`, `ui_inline.rs`, `ui_inline_interactive.rs` |
| 1 | `ui_transitions.rs`, `ui_pinned_selection.rs`, `ui_onboarding.rs`, `ui_animations.rs` |

So the majority of consumers are multi-category and would need a re-aggregating
supertrait anyway. The narrowing win is real only for the four single-domain
leaves; the headline god-interface stays wide.

Conclusion: the split is worthwhile for readability and for narrowing leaf
render-module bounds, but it is **not** a compile-coupling win and should be done
incrementally to avoid a high-conflict big-bang across 18 files.

## Proposed target shape

```
trait TuiState:
    TuiTranscriptState + TuiInputState + TuiScrollState + TuiStreamStatusState
    + TuiProviderState + TuiSessionServerState + TuiWorkspaceState
    + TuiDiagramPaneState + TuiDiffPaneState + TuiSidePanelState
    + TuiInlineState + TuiOverlayState + TuiCopySelectionState
    + TuiOnboardingState + TuiMiscState
{}
```

`App` and `TestState` keep a single `impl` per sub-trait (mechanical move). The
wide renderers take `&dyn TuiState` (the supertrait). Only the four
single-domain leaves can narrow to one sub-trait; everything else narrows to a
smaller aggregate at best.

## Method categorization (all 135)

This is a transcription of the section-header comments that *do* exist in the
trait today (`crates/jcode-tui/src/tui/mod.rs:174-659`), not an aspiration.
Several methods sit under a header that no longer describes them — the trait
grew and new accessors were appended to whichever section had the cursor in it.
Any real extraction has to re-home these first; they are marked below.

### TuiTranscriptState — `// ---- Transcript ----` (9)
display_messages, display_user_message_count, compacted_hidden_user_prompts,
has_display_edit_tool_messages, side_pane_images,
side_pane_images_signature, display_messages_version, streaming_text,
pinned_todos_payload

Misfiled: `streaming_text` belongs with stream/status.

### TuiInputState — `// ---- Input ----` (6)
input, cursor_pos, is_processing, queued_messages, interleave_message,
pending_soft_interrupts

Misfiled: `is_processing` belongs with stream/status. The command-suggestion
and queue accessors this section should own drifted into stream/status and
session/server.

### TuiScrollState — `// ---- Scroll ----` (8)
scroll_offset, auto_scroll_paused, terminal_clear_collapsed,
pending_history_anchor_lines_from_bottom, chat_overscroll_active,
chat_overscroll_pinned, chat_overscroll_remaining,
copy_selection_edge_autoscroll_active

### TuiProviderState — `// ---- Provider ----` (8)
provider_name, provider_model, upstream_provider, connection_type,
status_detail, mcp_servers, available_skills, active_dual_credential

### TuiStreamStatusState — `// ---- Stream / status ----` (19)
streaming_tokens, streaming_cache_tokens, output_tps, streaming_tool_calls,
elapsed, connection_phase_elapsed, status, command_suggestions,
advance_command_suggestions_epoch, command_suggestion_selected,
prompt_history_search, active_skill, subagent_status, batch_progress,
time_since_activity, client_focused, stream_message_ended,
total_session_tokens, session_compaction_count

Misfiled: the four command-suggestion/prompt-history accessors are input
concerns; `total_session_tokens` and `session_compaction_count` are session
concerns.

### TuiSessionServerState — `// ---- Session / server ----` (36)
is_remote_mode, is_replay, diff_mode, current_session_id,
session_display_name, server_display_name, server_display_icon,
server_display_version, server_sessions, connected_clients, status_notice,
time_since_user_interaction, learn_hint, hotkey_feedback,
active_experimental_feature_notice, remote_startup_phase_active,
has_pending_mouse_scroll_animation, animation_elapsed,
rate_limit_remaining, queue_mode, next_prompt_new_session_armed,
has_stashed_input, context_info, context_snapshot, context_limit,
info_widget_overlays_enabled, client_update_available,
server_update_available, info_widget_data, inline_swarm_gallery_active,
inline_swarm_members, swarm_members_for_transcript, swarm_panel_selected,
swarm_panel_focused, swarm_panel_full_page

This section is the dumping ground: 35 of the 135 methods, spanning diff pane,
scroll animation, input queue, context accounting, and the swarm panel. It is
the section that most needs splitting and the one this plan describes least
accurately. `is_canary` (listed in earlier revisions of this doc) is gone with
the self-development purge.

### TuiWorkspaceState — `// ---- Workspace ----` (7)
workspace_mode_enabled, workspace_map_rows, workspace_animation_tick,
render_streaming_markdown, centered_mode, auth_status, update_cost

Misfiled: `render_streaming_markdown`, `centered_mode`, `auth_status`, and
`update_cost` have nothing to do with the workspace map.

### TuiDiagramPaneState — `// ---- Diagram pane ----` (10)
diagram_mode, diagram_focus, diagram_index, diagram_scroll,
diagram_pane_ratio, diagram_pane_ratio_user_adjusted,
diagram_pane_animating, diagram_pane_enabled, diagram_pane_position,
diagram_zoom

The only section that is exactly what its header says. It is the natural
proof-of-pattern candidate.

### TuiDiffPaneState — `// ---- Diff pane ----` (4)
diff_pane_scroll, diff_pane_scroll_x, side_panel_image_zoom_percent,
diff_pane_focus

Misfiled: `side_panel_image_zoom_percent` is a side-panel concern;
`diff_mode` and `diff_line_wrap` live in other sections.

### TuiSidePanelState — `// ---- Side panel ----` (9)
side_panel, pin_images, inline_images_visible, image_expand_level,
expanded_images_version, pinned_images_auto_hide_remaining_secs,
chat_native_scrollbar, side_panel_native_scrollbar, diff_line_wrap

### TuiInlineState — `// ---- Inline ----` (3)
inline_interactive_state, inline_view_state, inline_ui_state

### TuiOverlayState — `// ---- Overlay ----` (7)
changelog_scroll, help_scroll, model_status_overlay, session_picker_overlay,
login_picker_overlay, account_picker_overlay, usage_overlay

### TuiMiscState — `// ---- Misc ----` (3)
working_dir, git_branch, now_millis

### TuiCopySelectionState — `// ---- Copy selection ----` (4)
copy_badge_ui, copy_selection_mode, copy_selection_range,
copy_selection_status

### TuiOnboardingState — `// ---- Onboarding ----` (6)
onboarding_preview_mode, onboarding_welcome_active, onboarding_welcome_kind,
suggestion_prompts, cache_ttl_status, has_notification

Misfiled: `suggestion_prompts`, `cache_ttl_status`, and `has_notification` are
not onboarding state; they were appended to the last section in the file.

## Incremental, low-conflict migration

Do **not** split all 15 sub-traits at once across 18 files. Recommended order:

1. Land the documented section headers in the trait definition. **Done** —
   the 15 `// ---- ... ----` comments are in
   `crates/jcode-tui/src/tui/mod.rs`. This is the only step that has landed.
2. Re-home the misfiled methods called out above, still as pure comment moves,
   so the categorization is true before anything depends on it. *Not started.*
3. Extract one leaf sub-trait as a proof of pattern. `TuiDiagramPaneState` is
   the best candidate (its section is already coherent);
   `TuiCopySelectionState` needs step 2 first because `ui_file_diff.rs` and
   `ui.rs` also read copy-selection state. Verify with
   `cargo check -p jcode-tui`. *Not started.*
4. Extract remaining leaf sub-traits one per commit, narrowing the corresponding
   leaf render module's bound in the same commit. Only `ui_transitions.rs`,
   `ui_pinned_selection.rs`, `ui_onboarding.rs`, and `ui_animations.rs` can
   narrow to a single sub-trait. *Not started.*
5. Keep the wide renderers (`ui.rs`, `ui_prepare.rs`, `ui_input.rs`,
   `redraw_schedule.rs`, `ui_viewport.rs`) on the `TuiState` supertrait
   throughout.

Each step is behavior-preserving (data accessors only) and compiles
independently, so it can be merged between other agents' work without a
big-bang conflict.

## Verification

- `cargo check -p jcode-tui` after each sub-trait extraction (TMPDIR must point
  at real disk, not the RAM-backed tmpfs, or ring/aws-lc-sys build scripts fail
  with "Disk quota exceeded").
- `cargo test -p jcode-tui --lib` once at the end. Note: the lib test suite had
  parallel-order flakiness unrelated to this trait; the process-global
  render-state race behind it is fixed (see `TUI_TEST_FLAKINESS.md`), but
  `HOME`-sensitive onboarding tests remain — that doc has the workaround.
