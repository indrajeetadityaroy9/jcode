# Memory Regression Budget

Status: active guardrail
Updated: 2026-09-05

This document defines the current memory regression budget for jcode.

For live server triage and cause-specific remediation, see
[Jcode Server Memory Incident Runbook](./MEMORY_INCIDENT_RUNBOOK.md).

The goal is not to freeze memory usage forever. The goal is to make memory changes:
- measurable
- reviewable
- intentionally justified

Where possible, budgets below are tied to counters and caps already exposed by the codebase rather than guessed RSS numbers.

## How to collect the metrics

Use existing debug surfaces instead of ad hoc instrumentation:

- TUI aggregate memory profile: `:debug memory`
- TUI memory sample history: `:debug memory-history`
- Markdown cache profile: `:debug markdown:memory`
- Mermaid cache profile: `:debug mermaid:memory`
- Agent/session memory profile via debug socket: `agent:memory`
- Fast server incident classification: `server:memory-incident`
- Full server attribution: `server:memory`
- Process-lifetime timeline: `python3 scripts/analyze_runtime_memory_log.py --days 1`

The *process* figures in those reports (RSS, peak RSS, virtual, thread stacks, anonymous heap,
system total/available, swap, load, battery) are **real on macOS**:
`process_memory::snapshot_with_source` reads them through `proc_pidinfo`,
`task_info(TASK_VM_INFO)`, `getrlimit`, `getrusage`, `sysctlbyname` and `host_statistics64`
(`crates/jcode-base/src/process_memory.rs:186`).

Only the **PSS family** stays `None`: proportional set size is a Linux `smaps_rollup` concept
with no macOS equivalent, so `os.pss_bytes` and friends are absent and the analyzer falls back to
RSS, labelling which metric it used. See
[the runbook's platform note](./MEMORY_INCIDENT_RUNBOOK.md) for the full field-by-field list.

> This paragraph previously claimed every process figure was "always absent" because
> `snapshot_with_source` was `#[cfg(target_os = "linux")]`. That was true before the macOS
> readers were implemented; the function carries no `cfg` today, and the daemon's runtime-memory
> log records real `rss_bytes`.

Primary sources in code (post workspace split):
- `crates/jcode-tui/src/tui/app/debug_cmds.rs`
- `crates/jcode-tui/src/tui/memory_profile.rs`
- `crates/jcode-base/src/session.rs`
- `crates/jcode-tui-markdown/src/lib.rs`
- `crates/jcode-tui-mermaid/src/lib.rs`
- `crates/jcode-base/src/runtime_memory_log.rs`

## Budget model

We use two kinds of budgets:

1. Hard caps
- These are explicit limits already enforced by caches.
- Regressions here mean the code changed its bound or bypassed it.

2. Ratchet expectations
- These are expected relationships between memory counters.
- Regressions here are allowed only with explanation and updated docs/tests.

## Hard caps

### Markdown cache budget

Source: `crates/jcode-tui-markdown/src/lib.rs`

| Metric | Budget | Why |
|---|---:|---|
| `highlight_cache_entries` | `<= 256` | Explicit cache cap (`HIGHLIGHT_CACHE_LIMIT`, `lib.rs:168`) |

Required review action if violated:
- explain why the cache limit changed
- update this doc
- update any affected tests or benchmarks

### Mermaid cache budget

Sources:
- `crates/jcode-tui-mermaid/src/lib.rs`
- `crates/jcode-tui-mermaid/src/mermaid_cache_render.rs`

| Metric | Budget | Why |
|---|---:|---|
| `render_cache_entries` | `<= 512` | Explicit render-cache cap (`RENDER_CACHE_MAX`, `mermaid_cache_render.rs:12`) |
| `layout_cache_entries` | `<= 32` | Explicit layout-tier cap (`LAYOUT_CACHE_MAX`, `mermaid_cache_render.rs:30`) |
| `image_state_entries` | `<= 24` | Explicit protocol-state cap (`IMAGE_STATE_MAX`, `lib.rs:484`) |
| `image_state_source_limit_bytes` | `<= 48 MiB` | Decoded source bytes held by protocol states (`IMAGE_STATE_MAX_SOURCE_BYTES`, `lib.rs:492`) |
| `source_cache_entries` | `<= 16` | Explicit decoded-source cap (`SOURCE_CACHE_MAX`, `lib.rs:726`) |
| `source_cache_limit_bytes` | `<= 48 MiB` | Decoded bytes held by the source cache (`SOURCE_CACHE_MAX_BYTES`, `lib.rs:730`) |
| `fitted_source_cache_entries` | `<= 16` | Pre-scaled non-Kitty sources (`FITTED_SOURCE_CACHE_MAX`, `lib.rs:735`) |
| `fitted_source_cache_limit_bytes` | `<= 32 MiB` | Decoded bytes held by pre-scaled sources (`FITTED_SOURCE_CACHE_MAX_BYTES`, `lib.rs:736`) |
| `kitty_viewport_state_entries` | `<= 256` | Kitty virtual-placement states (`KITTY_VIEWPORT_STATE_MAX`, `lib.rs:506`) |
| `kitty_pending_transmit_limit_bytes` | `<= 32 MiB` | Not-yet-drawn Kitty transmit bytes (`KITTY_VIEWPORT_PENDING_MAX_BYTES`, `lib.rs:515`) |
| `active_diagrams` | `<= 128` | Explicit active-diagram cap (`ACTIVE_DIAGRAMS_MAX`, `lib.rs:602`) |
| `cache_disk_png_bytes` | `<= 50 MiB` | Explicit on-disk cache cap (`CACHE_MAX_SIZE_BYTES`, `lib.rs:1389`) |
| `cache_disk_max_age_secs` | `<= 259200` | 3-day expiry (`CACHE_MAX_AGE_SECS`, `lib.rs:1386`) |

Every metric name above is a field of `MermaidMemoryProfile` (`lib.rs:1214`), so `:debug
mermaid:memory` reports each value next to its own `*_limit` field.

Required review action if violated:
- document the new limit and reason
- verify eviction still works
- verify no unbounded growth path was introduced

## Ratchet expectations

### Session and transcript memory

Source: `crates/jcode-base/src/session.rs`, `crates/jcode-tui/src/tui/memory_profile.rs`

These are not strict caps yet, but they are expected relationships.

| Metric relationship | Expectation |
|---|---|
| `provider_messages_cache.count` vs `messages.count` | Should remain in the same order of magnitude for a single session, and normally track the transcript closely |
| `session_provider_cache_json_bytes` vs `canonical_transcript_json_bytes` | Should remain comparable for normal chat flows, not explode independently |
| `transient_provider_materialization_json_bytes` | Should return to zero or near-zero outside active materialization-heavy paths |
| `display_large_tool_output_bytes` | Large values require explanation because they usually mean raw tool output is being retained too aggressively in the UI |

Required review action if violated:
- show before/after memory profiles
- explain which retention path grew
- prefer fixing duplication before raising any budget

### Runtime memory log expectations

Source: `crates/jcode-base/src/runtime_memory_log.rs`

Runtime memory logs are the regression detection mechanism, not just a debug feature.

Expected behavior:
- server/client logs should be sufficient to explain large changes in:
  - session/transcript totals
  - provider cache totals
  - TUI display totals
  - side panel totals
- new large memory owners should emit attributable signals instead of appearing only as unexplained RSS growth

Required review action if violated:
- add or improve attribution before accepting the memory increase

## Review checklist for memory-affecting changes

When changing memory-heavy code, capture and include:

1. Which counters changed?
- aggregate `:debug memory`
- targeted `:debug markdown:memory` / `:debug mermaid:memory`
- `agent:memory` when session/provider cache behavior changes

2. Was a hard cap changed?
- if yes, explain why the old cap was insufficient

3. Did duplication increase?
- canonical transcript
- provider cache
- materialized provider view
- display copy
- side-panel copy

4. Did observability remain adequate?
- if memory grew, can logs/profiles explain where?

## Current initial budget summary

These are the concrete enforced limits today:

- Markdown highlight cache entries: 256
- Mermaid render cache entries: 512
- Mermaid layout cache entries: 32
- Mermaid protocol image-state entries: 24, decoded source bytes 48 MiB
- Mermaid decoded source-cache entries: 16, bytes 48 MiB
- Mermaid pre-scaled (fitted) source-cache entries: 16, bytes 32 MiB
- Mermaid Kitty viewport states: 256, pending transmit bytes 32 MiB
- Mermaid active diagrams: 128
- Mermaid on-disk PNG cache: 50 MiB, max age 3 days
- TUI side-panel render cache entries: 12 (`SIDE_PANEL_RENDER_CACHE_LIMIT`,
  `crates/jcode-tui/src/tui/ui_pinned.rs:533`)

Any intentional change to those limits must update this document in the same PR.
