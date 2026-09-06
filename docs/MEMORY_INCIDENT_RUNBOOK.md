# Jcode Server Memory Incident Runbook

Status: active operational runbook
Updated: 2026-09-06

## PLATFORM NOTE: what this runbook measures on macOS

**Read this before triaging anything.** This fork is macOS-only, and
`process_memory::snapshot_with_source` reads the process's memory from the kernel with two
calls (`crates/jcode-base/src/process_memory.rs`):

- `proc_pidinfo(PROC_PIDTASKINFO)` → `rss_bytes`, `virtual_bytes`, `thread_count`
- `task_info(TASK_VM_INFO)` → `peak_rss_bytes` (the kernel's own resident high-water mark, the
  macOS analogue of Linux `VmHWM`), plus `os.rss_anon_bytes` (`internal + reusable`),
  `os.rss_file_bytes` (`external`) and `os.swap_bytes` (`compressed`, i.e. compressor-held
  bytes — macOS compresses instead of swapping)
- `getrlimit(RLIMIT_STACK)` → `main_stack_bytes`

`os.rss_anon_bytes + os.rss_file_bytes` is the entire resident set. The system allocator's live
and mapped bytes come from `malloc_zone_statistics` across all zones, so
`allocator.stats.allocated_bytes` is populated on a default build, not only under
`--features jemalloc`.

### What is still unavailable on macOS

The PSS family has no macOS equivalent — proportional accounting is a Linux `smaps_rollup`
concept — so these stay `None` and are never guessed: `os.pss_bytes`, `os.pss_anon_bytes`,
`os.pss_file_bytes`, `os.pss_shmem_bytes`, `os.anon_huge_pages_bytes`, `os.rss_shmem_bytes`, and
the private/shared clean/dirty splits. Consequences:

- `server:memory-incident` takes `os.pss_bytes` **or** `rss_bytes`
  (`crates/jcode-app-core/src/server/debug_server_state.rs:414-430`), so every "PSS" number in
  its payload is really RSS on this platform. RSS counts shared pages in full, so it reads
  slightly higher than PSS would; the growth trend, the warning/critical thresholds and the
  `non_heap_or_mapping_growth` branch all work.
- `scripts/analyze_runtime_memory_log.py` applies the same fallback through
  `Sample.footprint_bytes` and, unlike the payload, **names the metric it read** in every
  label: `metric: RSS (rss_bytes; os.pss_bytes has no macOS source)`, `final RSS 47.5 MB`,
  `Top RSS spikes`, `coverage: vs RSS`. A macOS log therefore reports a real footprint instead
  of the `final PSS 0.0 MB` it printed while it keyed on `os.pss_bytes` alone. Genuinely
  PSS-only fields are reported as unavailable rather than as zero: the `PSS split` line is
  replaced by an `RSS split` line ending `PSS split n/a on macOS`. Spikes are never computed
  across a PSS/RSS metric change. The thresholds below are unchanged; see "Severity
  thresholds".
- The retained-resident estimate needs the allocator's `retained_bytes`, which libmalloc does
  not report, so on a default build `allocator_retained_resident_bytes` is `0` and the
  `allocator_retention` branch cannot fire. Rebuild with `--features jemalloc` for that one
  signal. `unattributed_live_heap` works either way, since it only needs live bytes.
- §5's actions use the macOS tools (`footprint`, `vmmap`, `sample`) rather than the Linux
  `/proc` commands, which do not exist here. Note that `footprint`'s "phys_footprint" is a
  different (ledger) number than `rss_bytes` and will not match exactly.
- `allocator:purge` works on both builds: jemalloc purges every initialised arena, libmalloc
  gets `malloc_zone_pressure_relief` across all zones. On macOS the recoverable memory sits on
  the `reusable` ledger — freeing a large block leaves its pages mapped and resident until
  something reclaims them — and a relief call returns it (measured: 128 MiB freed then relieved
  dropped resident size from 135 MB to 1.3 MB).

Per-session payload attribution (`jcode debug 'server:memory'`, `agent:memory`) is JSON-byte
accounting rather than an OS metric, and works unchanged.

This runbook answers two questions:

1. What is using the jcode server's memory?
2. What is the safest next action for that specific cause?

The goal is not to react to every high RSS value. The goal is to distinguish live application state, allocator retention, and non-heap mappings before changing or stopping anything.

## Prerequisite: debug control must be enabled on the daemon

Every `jcode debug '...'` command in this runbook fails with
`Debug control is disabled` unless debug control is enabled **in the process that
serves the session** - the long-lived daemon, not your shell. Exporting
`JCODE_DEBUG_CONTROL=1` next to the `jcode debug` invocation does nothing if the
daemon was started without it.

During an incident the daemon is already running and you do not want to restart
it, so use the file toggle, which is read per request and needs no restart:

```bash
touch ~/.jcode/debug_control      # enable, no restart
rm ~/.jcode/debug_control         # disable when finished
```

The other two routes both require the daemon to start with them in effect:
`display.debug_socket = true` in `~/.jcode/config.toml` (persistent), or
`JCODE_DEBUG_CONTROL=1` in the daemon's own environment. All three are checked by
`debug_control_allowed()` (`crates/jcode-app-core/src/server/util.rs:13`).

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
python3 scripts/analyze_runtime_memory_log.py --days 1
```

The analyzer selects the latest server and client process instances by default. This is important because comparing PSS across a server reload produces false spikes. Use `--all-instances` only for explicit cross-instance forensics.

List recorded process lifetimes and select a pre-reload incident directly:

```bash
python3 scripts/analyze_runtime_memory_log.py --days 1 --list-instances
python3 scripts/analyze_runtime_memory_log.py --days 1 --instance <server-instance-id>
```

Prefer `--instance` for postmortems. It preserves one coherent process lifetime without mixing a high-memory server with its low-memory replacement.

For machine-readable output:

```bash
python3 scripts/analyze_runtime_memory_log.py --days 1 --json > /tmp/jcode-memory-analysis.json
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

These numbers were calibrated against Linux PSS and are **unchanged** on macOS, where the
footprint is RSS. Because RSS counts every shared page in full rather than proportionally, an
RSS footprint reads at or above the PSS value for the same process, so a fixed threshold trips
marginally earlier here. The gap is the process's share of shared mappings — tens of MB for
this binary against a 1 GiB warning — so the thresholds are left alone rather than quietly
discounted; the analyzer names the metric in its output so a reader can apply the discount
themselves. If shared-mapping growth ever makes the gap material, adjust the constants
deliberately rather than changing what the analyzer reports.

The two classifiers emit different cause sets. `classify_memory_incident`
(`debug_server_state.rs:349-390`) emits exactly `runaway_live_session_population`,
`allocator_retention`, `unattributed_live_heap`, `non_heap_or_mapping_growth`, or
`within_normal_operating_range`. The offline analyzer additionally emits
`session_payload_growth` (`scripts/analyze_runtime_memory_log.py:876`) because it can see the
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

### 5. `non_heap_or_mapping_growth`

Evidence:

- resident memory is high but allocator live bytes are not
- file-backed, shared-memory, or thread-stack mappings are growing
- `os.rss_file_bytes` and `process_diagnostics.thread_stack_estimate_bytes` are where the
  in-product evidence for this lives on macOS

Actions (all four binaries ship with macOS, in `/usr/bin`):

```bash
footprint -p <server-pid>                 # per-category dirty/clean/reclaimable ledger
vmmap <server-pid> | head -40             # individual mappings, largest first
vmmap -summary <server-pid>               # totals by region type
sample <server-pid> 5                     # thread activity over 5s, if threads are the suspect
```

Investigate model mappings, shared memory, thread creation, or large anonymous mappings outside the allocator.

`footprint` is the closest analogue to the Linux `smaps_rollup` output this section used to
recommend: its Reclaimable column is where freed-but-resident allocator pages show up, which is
the distinction this cause turns on. The Linux commands (`cat /proc/<pid>/smaps_rollup`,
`pmap -x`, `ps -T`) do not exist here.

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
