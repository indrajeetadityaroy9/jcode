# Crate Ownership and Modularization Boundaries

This document defines the target structure for keeping `jcode` modular without turning shared crates into a dumping ground. It is intentionally practical: use it when deciding whether to move a type, helper, or behavior out of the root crate.

## Goals

Primary goal: make normal development and `--profile selfdev` builds faster by shrinking the recompilation surface of the three spine crates (`jcode-base` -> `jcode-app-core` -> `jcode-tui`). Structural cleanliness is valuable because it supports that compile-time goal.

- Move stable DTOs and protocol-safe state into small crates so changes in spine behavior do not recompile those contracts, and changes in contracts recompile only focused dependents.
- Keep dependency-light crates dependency-light so they compile quickly and do not pull large runtime/TUI/provider graphs into unrelated builds.
- Keep spine-crate behavior — storage, process, TUI, server, and provider runtime logic — where it is until a full dependency boundary can move without increasing dependency fan-out.
- Avoid cyclic dependencies and hidden coupling through broad `jcode-core` re-exports, or through the whole-crate glob ladder (`crates/jcode-app-core/src/lib.rs:24`, `crates/jcode-tui/src/lib.rs:23`, `src/lib.rs:22`).
- Preserve serde compatibility and compatibility re-exports during migrations unless all call sites are intentionally updated.
- Measure success by compile impact: fewer spine edits, fewer spine-owned DTOs, smaller dependency fan-out, and faster `cargo check --profile selfdev` after common changes.

## Ownership rules

### Type crates own stable data contracts

A `*-types` crate should contain:

- Plain data structures used by multiple crates or protocol layers.
- Serialization shape and small pure helper methods tied to the data contract.
- No filesystem, network, process, TUI, provider client, global state, or storage access.
- Dependencies limited to serde, chrono, and other type crates where necessary.

Examples: `jcode-session-types`, `jcode-side-panel-types`, `jcode-dev-types`, `jcode-background-types`.

### Spine-crate modules own runtime behavior

There is no root `src/` module layer any more: the root package holds only
`main.rs`, `lib.rs`, `cli/`, and `bin/`. Runtime behavior lives in the three
spine crates, and a module should stay there when it needs:

- `crate::storage`, `crate::config`, `crate::logging`, `crate::server`, or
  process spawning. Those `crate::` paths still resolve anywhere in the spine
  because each layer glob-re-exports the one below it.
- Provider HTTP clients and auth managers.
- Tokio runtime, background tasks, channels, global caches, file locks, or PID
  registries.
- TUI rendering and crossterm/ratatui state.

If a type has inherent methods that need these APIs, either leave the type in its
spine crate or move behavior and dependencies together into a domain crate. Do
not move only the struct if that forces illegal inherent impls behind it.

### `jcode-core` is for genuinely shared primitives

`jcode-core` should contain:

- Cross-domain primitives that do not have an obvious domain crate yet.
- Very small, dependency-light helpers used by many crates.
- Temporary DTO staging only when creating a new domain type crate would be premature.

`jcode-core` should not accumulate every extracted DTO indefinitely. Once a cluster grows, split it into a focused domain crate.

### Compile-speed decision rule

Prefer a split when it reduces root crate churn or dependency fan-out. Do not split just to make files look tidier if the new crate adds dependencies, increases rebuild fan-out, or forces frequent cross-crate edits. A good split has at least one of these compile-time benefits:

- Common root behavior edits no longer touch stable type definitions.
- A type-only change can be checked by compiling a small type crate plus focused dependents.
- Heavy dependencies stay out of DTO crates.
- Multiple downstream crates can use a small contract without depending on the root crate.

### Re-export policy

During migrations:

1. Move the type to the target crate.
2. Keep the old path as `pub use ...` to preserve call sites
   (`crates/jcode-base/src/protocol.rs:1` and
   `crates/jcode-base/src/storage.rs:3` are the pattern).
3. Validate focused tests and a full build.
4. Later, remove the obsolete re-export only after downstream crates can depend
   directly on the domain crate. Note that whole-crate globs of the
   `pub use jcode_base::*;` kind never reach step 4, which is why the wildcard
   re-export budget (`scripts/wildcard_reexport_budget.json`) exists to stop new
   ones appearing.

## Move checklist

Use this checklist for every type or pure-helper migration. Copy it into the PR/commit notes when a move is non-trivial.

1. Classify the candidate.
   - [ ] Is it a stable data contract or pure helper rather than spine runtime behavior?
   - [ ] Does it have inherent methods?
   - [ ] Do those methods require spine-only APIs such as storage, network clients, TUI state, process management, or globals?
   - [ ] If behavior must move too, can the full dependency boundary move without increasing fan-out?
2. Check compatibility.
   - [ ] Does its serde representation stay identical?
   - [ ] Are defaults, skips, renames, and enum discriminants preserved?
   - [ ] Are all field visibilities still appropriate?
   - [ ] Can the old crate keep a compatibility re-export?
3. Check crate health.
   - [ ] Does the target crate already have the needed dependency policy?
   - [ ] Are new dependencies limited to type-crate-appropriate libraries, usually `serde`, `serde_json`, `chrono`, or sibling type crates?
   - [ ] Is the target crate still acyclic?
   - [ ] Did `cargo metadata`/`cargo check` avoid pulling spine, provider, storage, or process dependencies into the type crate?
4. Validate.
   - [ ] Is there a focused test filter that covers the moved type?
   - [ ] Did `cargo check --profile selfdev -p <type-crate> -p jcode --bin jcode` pass?
   - [ ] Did relevant focused root tests pass?
   - [ ] Did `cargo fmt` pass?
   - [ ] Did a full build pass from a clean committed HEAD?

## Dependency boundary guard

Run this guard after adding or changing any type crate dependency:

```sh
python3 scripts/check_dependency_boundaries.py
```

It is not optional: it runs as a gate from `scripts/check_guardrails.sh:89`, so a
violation fails the guardrail sweep.

What it actually blocks: a crate whose name matches `jcode-*-types`
(`scripts/check_dependency_boundaries.py:61-62`) may not directly depend on any
of the 18 crates in `FORBIDDEN_INTERNAL_DEPS` (`:28-47`):

`jcode`, `jcode-agent-runtime`, `jcode-azure-auth`, `jcode-core`,
`jcode-embedding`, `jcode-pdf`, `jcode-plan`, `jcode-protocol`,
`jcode-provider-core`, `jcode-provider-gemini`, `jcode-provider-metadata`,
`jcode-provider-openrouter`, `jcode-terminal-launch`, `jcode-tui-core`,
`jcode-tui-markdown`, `jcode-tui-mermaid`, `jcode-tui-render`,
`jcode-tui-workspace`.

`jcode-message-types` is the only allowed internal dependency
(`ALLOWED_INTERNAL_TYPE_DEPS`, `:21-23`); external lightweight libraries are
unrestricted. `jcode-core` is on the forbidden list deliberately, so it cannot
become the backdoor catch-all for DTO crates (`:25-27`).

Two things the guard does **not** do, and you should not assume it does:

- The forbidden list contains no spine crate. `jcode-base`, `jcode-app-core`, and
  `jcode-tui` are absent, so a type crate that depends on one of them — the worst
  possible direction — passes the gate. Treat that as a review rule, not a
  checked one.
- It says nothing about any pair of non-`-types` crates. Provider, TUI, and
  runtime crates can depend on each other freely as far as tooling is concerned.

The companion static report is **advisory**:

```sh
python3 scripts/compile_isolation_report.py
```

It prints LOC, inline-test, `async_trait`, and target-state dependency
advisories, and exits non-zero only when `--strict-target-state` is passed
(`scripts/compile_isolation_report.py:4-5`, `:174-178`, `:244-246`). It is not
wired into `scripts/check_guardrails.sh`.

## Test policy

Prefer focused filters for validation. Broad filters often select unrelated stateful, timing-sensitive, or benchmark tests.

Known broad-filter hazards observed during modularization:

- `side_panel` selects unrelated pinned UI/layout and latency benchmark tests.
- `usage` selects app-display tests in addition to pure usage tests.
- `session::` selects live-attach server tests and picker behavior beyond session persistence.
- `ambient` selects TUI/helper integration tests with config and schedule state beyond ambient module persistence/runtime tests.

Document precise filters next to each domain crate/module. Broad filters are still useful for periodic sweeps, but they should not block a DTO-only extraction when precise tests and compile checks pass.

Focused validation matrix after the current DTO splits:

| Area | Fast compile check | Focused tests used during split | Notes |
| --- | --- | --- | --- |
| Usage DTOs | `cargo check --profile selfdev -p jcode-usage-types -p jcode --bin jcode` | Prefer exact tests under usage/copilot usage modules. Avoid bare `usage` as a required gate because it selects display/UI tests too. | DTO crate owns report and local counter contracts. Runtime fetch/cache/display live in `crates/jcode-base/src/usage/` and `crates/jcode-app-core/src/usage_display.rs`. |
| Ambient DTOs | `cargo check --profile selfdev -p jcode-ambient-types -p jcode --bin jcode` | Scheduler/type consumers only. | Ambient DTO crate owns usage records only. Queue/runtime/prompt behavior lives in `crates/jcode-app-core/src/ambient/`. |
| Ambient behavior modules | `cargo check --profile selfdev -p jcode-app-core` | `cargo test --profile selfdev -p jcode-app-core ambient::ambient_tests --lib`; `cargo test --profile selfdev -p jcode-app-core ambient::scheduler::tests --lib`; `cargo test --profile selfdev -p jcode-app-core ambient::runner::runner_tests --lib` | Those three test modules are at `crates/jcode-app-core/src/ambient.rs:196-197`, `ambient/scheduler.rs:284`, and `ambient/runner.rs:1041`. Avoid bare `ambient` as a required gate for module-only refactors because it selects cross-module TUI/config state tests. |
| Memory activity DTOs | `cargo check --profile selfdev -p jcode-memory-types -p jcode --bin jcode` | `cargo test --profile selfdev -p jcode-base runtime_memory_log --lib`; `cargo test --profile selfdev -p jcode-tui tui::info_widget::tests --lib` | `memory::activity` matches no tests, so use consumer tests. The log tests are at `crates/jcode-base/src/runtime_memory_log.rs:823`; the widget tests at `crates/jcode-tui/src/tui/info_widget.rs:2110-2112`. |
| Goal/todo/catchup DTOs | `cargo check --profile selfdev -p jcode-base -p jcode-app-core` | Exact goal/todo/catchup filters if behavior changes. | These never got their own crates. The DTOs are inline in `crates/jcode-base/src/goal.rs`, `crates/jcode-base/src/todo.rs`, and `crates/jcode-app-core/src/catchup.rs`. |


## Compile baseline observations

Measured on 2026-04-30 with `scripts/dev_cargo.sh check --profile selfdev -p jcode --bin jcode`, **before** the root crate was split into the `jcode-base`/`jcode-app-core`/`jcode-tui` spine. The paths named in the table no longer exist (`src/usage.rs` is now `crates/jcode-base/src/usage.rs`, and `crates/jcode-core/src/usage_types.rs` was moved out to `jcode-usage-types`), so treat this as a historical datapoint. The *conclusion* still holds and is the reason `jcode-core` stayed a utility crate. This is a coarse mtime-touch benchmark, not a full statistical study.

| Scenario | Observed time | Interpretation |
| --- | ---: | --- |
| No-op check after recent doc-only commit | ~65.8s | Environment/cache state can dominate a first check. Treat as warmup/noise baseline, not pure no-op steady state. |
| Touch behavior module `src/usage.rs` (now `crates/jcode-base/src/usage.rs`) | ~6.25s | A behavior-only edit can be relatively cheap when dependencies are already built. |
| Touch `crates/jcode-core/src/usage_types.rs` (since moved to `jcode-usage-types`) | ~65.35s | Editing `jcode-core` invalidates broad downstream dependents. Avoid adding high-churn domain DTOs to `jcode-core`. |

Implication: the compile-speed target is not simply "move things out of the spine". Moving stable, low-churn contracts down is good, but putting many high-churn domain DTOs into `jcode-core` can be counterproductive because `jcode-core` has high fan-out. Prefer focused leaf crates such as `jcode-usage-types` and `jcode-ambient-types` for domain DTOs that are likely to change.

## `jcode-core` fan-out audit

`jcode-core` now has 10 direct Cargo dependents: `jcode-app-core`, `jcode-base`,
`jcode-build-support`, `jcode-logging`, `jcode-provider-env`,
`jcode-provider-openai`, `jcode-provider-openrouter`, `jcode-setup-hints`,
`jcode-storage`, and `jcode-tui`. Two of those are spine crates, and
`jcode-storage`/`jcode-logging` are themselves depended on broadly, so a touch to
`jcode-core` still invalidates most of the workspace. Treat it as a high-fan-out
crate.

It is also on the boundary guard's forbidden list for type crates
(`scripts/check_dependency_boundaries.py:32`), specifically so it cannot become
the DTO backdoor.

The DTO-staging modules this audit was written about are gone: `jcode-core` now
contains only general utilities.

| Module | Contents | Status |
| --- | --- | --- |
| `console` | Terminal/console output helpers | stay in core |
| `env` | Environment variable helpers | stay in core |
| `fs` | Filesystem helpers | stay in core |
| `id` | ID helpers | stay in core |
| `output_style` | Output-style primitives | stay in core |
| `panic_util` | Panic formatting helpers | stay in core |
| `stdin_detect` | stdin detection helpers | stay in core |
| `util` | Misc utilities | audit later; should not become a catch-all |

Domain DTOs that used to be staged here have all left: `ambient_usage_types` ->
`jcode-ambient-types`; `copilot_usage_types` and `usage_types` ->
`jcode-usage-types`; `memory_types` -> `jcode-memory-types` (re-exported at
`crates/jcode-base/src/memory_types.rs`). `catchup_types`, `goal_types`, and
`todo_types` were never split into crates — their DTOs are inline in
`crates/jcode-app-core/src/catchup.rs`, `crates/jcode-base/src/goal.rs`, and
`crates/jcode-base/src/todo.rs`.

Compile-speed priority from this audit:

1. Keep clustered, likely-changing domain DTOs out of `jcode-core`.
2. Keep stable general utilities in `jcode-core`.
3. Do not add new domain DTOs to `jcode-core`; the guard will not stop you, but a
   focused leaf crate is the right home.

## Target domain type crates

Landed domain type splits:

1. `jcode-usage-types` — provider usage report DTOs and local Copilot counters
2. `jcode-ambient-types` — ambient scheduler usage records and rate-limit DTOs
3. `jcode-memory-types` — memory activity DTOs plus the memory graph

Not built, and no longer proposed: a task-state crate for goal/todo/catchup
DTOs. Those DTOs still live inline next to their behavior
(`crates/jcode-base/src/goal.rs`, `crates/jcode-base/src/todo.rs`,
`crates/jcode-app-core/src/catchup.rs`), which is fine while they stay small; the
existing `jcode-task-types` crate is the natural home if they ever need one.

Ambient state/request/result DTOs still have not moved, for the original reason:
`AmbientState::load/save/record_cycle` behavior would have to separate from the
struct first (see `crates/jcode-app-core/src/ambient/persistence.rs`).

## Big module refactor targets

These are not simple DTO moves. Refactor behavior boundaries first.

### `crates/jcode-base/src/session.rs` (1601 lines)

Target split — partly landed. Already extracted into
`crates/jcode-base/src/session/`: `model.rs`, `persistence.rs`, `journal.rs`,
`memory_profile.rs`, `storage_paths.rs`, `maintenance.rs`, `load_telemetry.rs`,
plus the pre-existing `render.rs` and `crash.rs`.

Still in the parent file and worth continuing:

- remaining metadata/session-model surface
- startup stubs and remote startup snapshots

### `crates/jcode-app-core/src/ambient.rs` (197 lines)

Target split — **landed.** `crates/jcode-app-core/src/ambient/` now holds
`persistence.rs`, `directives.rs`, `scheduler.rs`, `prompt.rs`, `manager.rs`,
`runner.rs`, and `paths.rs`, with `ambient_runner.rs`/`ambient_scheduler.rs` as
re-export modules.

Do not move `AmbientState` as a DTO until load/save/record behavior is separated
from the struct.

### `crates/jcode-base/src/usage.rs` (658 lines)

Target split — **landed.** `crates/jcode-base/src/usage/` holds
`provider_fetch.rs`, `openai_helpers.rs`, `cache.rs`, `display.rs`, `model.rs`,
`api_keys.rs`, and `accessors.rs`; the public report DTOs live in
`jcode-usage-types`. Account selection/guidance and the higher-level display
surface live in `crates/jcode-app-core/src/usage_display.rs`.

## Definition of “optimal enough”

The structure is good enough when:

- Each type crate has a clear domain and minimal dependency set.
- `jcode-core` contains only true primitives — as of this revision it does.
- Spine-crate modules no longer mix large DTO blocks, persistence, runtime orchestration, and rendering in one file.
- Every domain has focused validation commands.
- A full build works cleanly after every structural change.
