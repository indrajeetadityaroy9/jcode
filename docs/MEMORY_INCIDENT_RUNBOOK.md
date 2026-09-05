# Jcode Server Memory Incident Runbook

Status: active operational runbook
Updated: 2026-09-05

## PLATFORM CAVEAT: this runbook does not work on macOS

**Read this before triaging anything.** Every RSS/PSS/smaps number this runbook depends on is
collected only on Linux. `process_memory::snapshot_with_source` is
`#[cfg(target_os = "linux")]` and parses `/proc/self/status` plus `smaps_rollup`
(`crates/jcode-base/src/process_memory.rs:148`). The non-Linux build is a stub that logs
`using default non-linux implementation` and returns `ProcessMemorySnapshot::default()`
(`crates/jcode-base/src/process_memory.rs:180`), i.e. `rss_bytes`, `peak_rss_bytes`,
`virtual_bytes`, `thread_count`, `main_stack_bytes` and the whole `os` block (PSS, anon PSS, file
PSS, private/shared dirty, swap) are all `None`. This fork is macOS-only, so in practice:

- `server:memory-incident` reports `pss = 0` and `pss_growth = 0`: the payload takes
  `os.pss_bytes` or `rss_bytes` and falls back to `0`
  (`crates/jcode-app-core/src/server/debug_server_state.rs:414-430`).
- Consequently the PSS warning/critical thresholds and the `non_heap_or_mapping_growth` branch can
  never fire (`debug_server_state.rs:365`, `:371-376`), and severity reports `healthy` unless the
  live-session count alone crosses a threshold.
- On a default (system-allocator) build `allocator_live_bytes` is also `0`: `glibc_malloc_stats` is
  a `None`-returning stub off glibc (`process_memory.rs:290`), so `allocator_retention` and
  `unattributed_live_heap` cannot fire either.
- `allocator:purge` fails outright with `allocator purge unavailable on this platform: rebuild with
  --features jemalloc` (`process_memory.rs:336-339`).
- The `/proc`-based commands in §5 (`smaps_rollup`, `pmap`, `ps -T`) do not exist on macOS.

### What actually works on macOS

| Need | macOS substitute |
|---|---|
| Live session population, swarm attribution, status counts | Works unchanged — pure application state (`debug_server_state.rs:441-491`). This is the one decision-tree branch that stays valid. |
| Per-session payload attribution (transcript / provider cache / tool results / blobs) | Works unchanged — `jcode debug 'server:memory'` and `agent:memory` are JSON-byte accounting, not OS metrics. |
| Allocator live/retained bytes, purge A/B | Requires a rebuild with `--features jemalloc` (or `jemalloc-prof`). The jemalloc feature is not target-gated, so a macOS jemalloc build does populate `allocated/active/resident/retained` and enables arena purge (`process_memory.rs:200-211`, `:295-316`). Note the retained-resident estimate degrades to raw `retained` because anon PSS is unavailable to cap it (`runtime_memory_log.rs:734-737`). |
| Process RSS / footprint | **No in-product substitute.** Use the OS directly: `ps -o rss=,vsz= -p <pid>`, `footprint -p <pid>`, or `vmmap <pid>`. jcode will not log or report it, and the JSONL memory logs will record zeros, so `analyze_runtime_memory_log.py` PSS trends and spike lists are empty by construction. |
| Mapping / thread growth | `vmmap <pid>` and `sample <pid>` in place of `pmap`/`ps -T`. |

Until `process_memory` grows a Darwin implementation (`task_info`/`proc_pid_rusage` for RSS,
`mach_vm_region` for mapping detail), treat the sections below as Linux-only procedure. The
session-population and payload-attribution paths are the only parts an operator can act on here.

This runbook answers two questions:

1. What is using the jcode server's memory?
2. What is the safest next action for that specific cause?

The goal is not to react to every high RSS value. The goal is to distinguish live application state, allocator retention, and non-heap mappings before changing or stopping anything.

## One-command triage

Run:

```bash
jcode debug 'server:memory-incident'
```

This is the first command during an incident. It is intentionally lightweight. It does not lock or serialize every Agent transcript, so it remains useful when thousands of sessions are resident.

The report includes:

- RSS, PSS, anonymous PSS, and allocator live bytes
- 15-minute PSS growth
- live, headless, detached, and connected session counts
- status counts for resident sessions
- the swarms with the most resident Agents
- a severity and primary-cause classification
- ordered, cause-specific actions

Preserve the JSON output in the incident notes before changing state.

## Offline timeline analysis

Runtime memory logging is enabled by default and writes daily JSONL files under:

```text
~/.jcode/logs/memory/
```

Analyze the latest server process lifetime:

```bash
python scripts/analyze_runtime_memory_log.py --days 1
```

The analyzer selects the latest server and client process instances by default. This is important because comparing PSS across a server reload produces false spikes. Use `--all-instances` only for explicit cross-instance forensics.

List recorded process lifetimes and select a pre-reload incident directly:

```bash
python scripts/analyze_runtime_memory_log.py --days 1 --list-instances
python scripts/analyze_runtime_memory_log.py --days 1 --instance <server-instance-id>
```

Prefer `--instance` for postmortems. It preserves one coherent process lifetime without mixing a high-memory server with its low-memory replacement.

For machine-readable output:

```bash
python scripts/analyze_runtime_memory_log.py --days 1 --json > /tmp/jcode-memory-analysis.json
```

## Severity thresholds

The built-in incident report uses these operational thresholds
(`crates/jcode-app-core/src/server/debug_server_state.rs:17-22`, applied in
`classify_memory_incident`, `:371-376`):

| Signal | Warning | Critical | Constant |
|---|---:|---:|---|
| PSS | 1 GiB | 2 GiB | `MEMORY_WARNING_PSS_BYTES` / `MEMORY_CRITICAL_PSS_BYTES` |
| PSS growth in 15 minutes | 256 MiB | 1 GiB | `MEMORY_WARNING_GROWTH_BYTES` / `MEMORY_CRITICAL_GROWTH_BYTES` |
| Resident Agent sessions | 128 | 512 | `MEMORY_WARNING_LIVE_SESSIONS` / `MEMORY_CRITICAL_LIVE_SESSIONS` |

A threshold starts an investigation. It does not authorize destructive cleanup by itself.

The two classifiers emit different cause sets. `classify_memory_incident`
(`debug_server_state.rs:349-390`) emits exactly `runaway_live_session_population`,
`allocator_retention`, `unattributed_live_heap`, `non_heap_or_mapping_growth`, or
`within_normal_operating_range`. The offline analyzer additionally emits
`session_payload_growth` (`scripts/analyze_runtime_memory_log.py:746`) because it can see the
per-session attribution walk. §3 below therefore only appears in analyzer output, never in
`server:memory-incident`.

## Decision tree

### 1. `runaway_live_session_population`

Evidence:

- live Agent count is high or rising rapidly
- headless or detached sessions greatly outnumber attached clients
- allocator live bytes rise with session count
- one or more swarms dominate `top_live_swarms`

Actions:

1. Pause or cap the producer creating sessions.
2. Run `jcode debug 'swarm:list'` and inspect the largest live swarm.
3. From the owning coordinator, use `swarm list` and `swarm cleanup` to remove workers it no longer needs.
4. Do not destroy sessions blindly. Confirm that active work is disposable first.
5. Re-run `server:memory-incident`. Require live sessions, allocator live bytes, and PSS to fall together.
6. Only then run `allocator:purge` if freed-but-held memory remains high.

Why: allocator purge cannot free live Agent runtimes.

### 2. `allocator_retention`

Evidence:

- allocator retained-resident estimate is at least 256 MiB
- retained-resident memory is at least 25% of PSS
- allocator live bytes are materially below anonymous PSS

Actions:

```bash
jcode debug 'server:memory-incident' > /tmp/before.json
jcode debug 'allocator:purge'
jcode debug 'server:memory-incident' > /tmp/after.json
```

A large PSS drop confirms allocator retention. If it repeatedly regrows, inspect allocation churn and allocator decay rather than raising memory budgets.

### 3. `session_payload_growth` (offline analyzer only)

Evidence:

- tracked transcript/provider-cache/tool/blob bytes explain at least half of allocator live memory
- one or more sessions dominate `top_by_json_bytes`

Actions:

1. Run `jcode debug 'server:memory'` for the full attribution walk.
2. Inspect provider cache, tool results, large blobs, and payload text.
3. Compact, summarize, truncate, or move large artifacts out of line.
4. Add or tighten a hard cap before accepting a larger steady state.

### 4. `unattributed_live_heap`

Evidence:

- allocator live bytes exceed 1 GiB
- session population and allocator retention do not explain the heap
- live-heap attribution coverage remains below 50%

Actions:

1. Capture `server:memory` and the runtime log analysis.
2. Add counters for any obvious missing owner.
3. If ownership is still unclear, use a `jemalloc-prof` build:

```bash
jcode debug 'allocator:profile:on'
jcode debug 'allocator:profile:dump /tmp/jcode-server.heap'
```

The normal system-allocator build cannot produce allocation-stack profiles. Do not claim heap ownership from RSS alone.

### 5. `non_heap_or_mapping_growth` (Linux only)

Evidence:

- PSS is high but allocator live bytes are not
- file-backed, shared-memory, or thread-stack mappings are growing

Actions:

```bash
cat /proc/<server-pid>/smaps_rollup
pmap -x <server-pid> | sort -k3 -nr | head -40
ps -T -p <server-pid> -o pid,tid,%cpu,time,comm,wchan:32
```

Investigate model mappings, shared memory, thread creation, or large anonymous mappings outside the allocator.

On macOS none of the three commands above exist and this cause can never be classified (PSS is
always 0). Use `vmmap <server-pid>` for mapping detail and `sample <server-pid>` for thread
activity instead.

## Escalation ladder

Use the cheapest reliable evidence first:

1. `server:memory-incident`, normally sub-second and non-blocking
2. runtime JSONL analyzer, process-lifetime trend and incident classification
3. `server:memory`, expensive per-Agent attribution
4. allocator purge A/B test, only for retention
5. jemalloc heap profile, only for unexplained live heap
6. OS mapping and CPU profiler correlation

## Required incident record

Save:

- server ID, version, git hash, and uptime
- PSS, anonymous PSS, allocator live, and retained-resident bytes
- live/headless/connected session counts
- top live swarms and status counts
- 15-minute growth
- the chosen action and before/after measurements
- whether active work was preserved

## Resolution criteria

An incident is resolved only when one of these is true:

- the identified live owner was reduced and PSS fell accordingly
- allocator purge proved retention and the recurrence mechanism was corrected
- mapping growth was identified and bounded
- a heap profile identified an owner and a regression test or cap was added
- the high steady state was proven intentional, documented, and given an explicit budget

Do not close an incident with only “memory dropped after restart.” A restart erases evidence and does not identify the cause.
