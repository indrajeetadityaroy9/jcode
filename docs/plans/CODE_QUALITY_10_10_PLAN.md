# Code Quality 10/10 Plan

Status: the guardrail infrastructure this plan asked for **exists and exceeds the
plan** — eight gates run from `scripts/check_guardrails.sh`, five ratcheted
against JSON baselines (four of them auto-rebaselinable). The plan's *CI* phases
are unreachable: this fork has no `.github/`, so every gate is a local pre-push
sweep. The refactor target paths below were written against the pre-crate-split
root `src/` tree and have been repointed at their current locations.

This document defines the quality target for jcode, the standards required to reach it, and the phased execution plan to get there without destabilizing the product.

## Goal

Raise jcode from its current state of roughly **7/10 overall code quality** to a sustained **9+/10 engineering standard**, with a practical target that feels like "10/10" in day-to-day development:

- clean builds
- clear module ownership
- small and maintainable files
- low-risk refactors
- strong tests
- predictable behavior under stress
- guardrail gates that prevent regressions (run locally; there is no CI in this fork)

Because jcode is a fast-moving product, "10/10" does **not** mean "perfect". It means:

1. defects are easier to prevent than to introduce
2. contributors can quickly understand where code belongs
3. the repo resists architectural drift
4. risky areas are well-tested and observable
5. quality does not depend on memory or heroics

## Current Problems

The main issues observed in the codebase today are:

### 1. Oversized modules

Several files are dramatically larger than they should be for long-term
maintainability. The live hotspot list is the oversized-file ratchet baseline
(`scripts/code_size_budget.json`: threshold 1200 LOC, 102 tracked files). Its
largest current entries:

- `crates/jcode-tui/src/tui/ui_messages.rs` (4417)
- `crates/jcode-tui/src/tui/app/inline_interactive.rs` (4337)
- `crates/jcode-tui/src/tui/app/input.rs` (4026)
- `crates/jcode-tui/src/tui/ui.rs` (3683)
- `crates/jcode-tui/src/tui/app/commands.rs` (3545)
- `crates/jcode-tui/src/tui/app/auth.rs` (3433)
- `src/cli/commands.rs` (3375)
- `crates/jcode-app-core/src/tool/communicate.rs` (3351)
- `crates/jcode-app-core/src/server/client_lifecycle.rs` (3282)

The files this plan originally named have all moved, and several shrank:
`src/provider/openai.rs` is now a 143-line compatibility shim at
`crates/jcode-base/src/provider/openai.rs` (the runtime went to
`jcode-provider-openai-runtime`); `src/provider/mod.rs` is
`crates/jcode-base/src/provider/mod.rs` (2891 lines); `src/agent.rs` is
`crates/jcode-app-core/src/agent.rs` (998); `src/server.rs` is
`crates/jcode-app-core/src/server.rs` (2329); `src/tui/ui.rs` and
`crates/jcode-tui/src/tui/info_widget.rs` are under `crates/jcode-tui/src/tui/`; and
`tests/e2e/main.rs` is down to 17 lines after the suite split.

These files are doing too much at once and create review, testing, and onboarding friction.

### 2. Warning and dead-code debt

Largely addressed. The warning baseline is **8**
(`scripts/warning_budget.txt`), enforced by `scripts/check_warning_budget.sh`
(gate at `scripts/check_guardrails.sh:84`), which counts `^warning:` lines from
`cargo check -q` and fails when the count exceeds the baseline. Broad
`allow(dead_code)` suppressions are the remaining half of this item; nothing
counts them yet.

### 3. Inconsistent strictness around failure paths

Now measured and ratcheted rather than unbounded.
`scripts/check_panic_budget.py` counts production `.unwrap(`, `.expect(`,
`panic!`, `todo!`, and `unimplemented!` across `src/` and `crates/`
(`scripts/check_panic_budget.py:4-13, 27-28`); the baseline is **77 occurrences
across 20 files** (`scripts/panic_budget.json`). Swallowed errors are tracked
separately at **3187 across 450 files**
(`scripts/swallowed_error_budget.json`) — that is the large remaining debt.

### 4. Test concentration

There are many tests, which is good, but some test coverage is concentrated inside very large files and does not yet provide ideal fault isolation.

### 5. Guardrails now exceed this plan

This was the plan's weakest assumption. The repository now has eight gates wired
into one sweep, `scripts/check_guardrails.sh`:

| Gate | Script | Baseline | Wired at |
|---|---|---|---|
| module declarations resolve | `scripts/check_module_files.py` | — | `check_guardrails.sh:64` |
| warning budget | `scripts/check_warning_budget.sh` | `scripts/warning_budget.txt` (8) | `:84` |
| oversized-file ratchet | `scripts/check_code_size_budget.py` | `scripts/code_size_budget.json` (1200 LOC, 102 files) | `:85` |
| oversized-test ratchet | `scripts/check_test_size_budget.py` | `scripts/test_size_budget.json` (1200 LOC, 39 files) | `:86` |
| panic-prone usage ratchet | `scripts/check_panic_budget.py` | `scripts/panic_budget.json` (77) | `:87` |
| swallowed-error ratchet | `scripts/check_swallowed_error_budget.py` | `scripts/swallowed_error_budget.json` (3187) | `:88` |
| crate dependency boundaries | `scripts/check_dependency_boundaries.py` | — | `:89` |
| wildcard re-export ratchet | `scripts/check_wildcard_reexport_budget.py` | `scripts/wildcard_reexport_budget.json` (17) | `:90` |

The same sweep also runs `cargo fmt --all --check`, `cargo check`/`cargo clippy
-- -D warnings` across all targets and features, `cargo metadata --locked`,
`cargo machete`, and the onboarding state-space invariant tests
(`check_guardrails.sh:59-107`).

Guards that exist but are **not** in that sweep:
`scripts/check_startup_budget.sh` (run from `scripts/test_fast.sh:28`),
`scripts/memory_regression_gate.sh`, `scripts/purge-guard.sh`, and the advisory
`scripts/compile_isolation_report.py`.

What is still missing is not more scripts: it is a `dead_code` suppression count
and an automatic trigger. Every gate here is a local pre-push sweep, because
there is no `.github/` in this fork.

## Definition of Done for "10/10"

We will consider this program successful when the codebase reaches the following state:

### Build and lint quality

- `cargo check --all-targets --all-features` passes cleanly
- `cargo clippy --all-targets --all-features -- -D warnings` passes cleanly or is very close with narrow, justified exceptions
- `cargo fmt --all -- --check` passes
- warning count is near zero and actively ratcheted downward

### Structural quality

- no production file exceeds **1200 LOC** without a documented reason
- most production files are below **800 LOC**
- most functions stay below **100 LOC** unless complexity is clearly justified
- major domains have clear boundaries and ownership

### Reliability quality

- e2e tests are split by feature instead of concentrated in mega-files
- critical state transitions have targeted tests
- reload, streaming, tool execution, and swarm coordination have explicit failure-mode coverage
- long-running reliability checks exist for memory, socket lifecycle, and reconnect/reload behavior

### Safety quality

- production `unwrap` / `expect` usage is significantly reduced and justified where it remains
- broad `allow(dead_code)` suppressions are eliminated or reduced to narrow local allowances
- tool, shell, path, and credential boundaries are explicit and tested

### Contributor quality

- contributors can tell where code belongs
- refactor rules are documented
- the guardrail sweep makes regressions hard to land (it must be run manually — there is no CI)
- architecture docs match reality

## Non-Negotiable Principles

1. **No big-bang rewrite.** Refactor incrementally.
2. **Behavior-preserving changes first.** Extract, move, split, and test before changing logic.
3. **Quality must be enforceable.** Prefer CI guardrails over informal expectations.
4. **Delete dead code aggressively.** Simpler code is higher-quality code.
5. **Keep the product shippable throughout the program.**

## Metrics to Track

These metrics should be checked repeatedly during the program:

- warning count
- clippy violations
- count of broad `allow(dead_code)` suppressions
- count of production `unwrap` / `expect`
- top 20 largest Rust files
- test runtime and flake rate
- startup time, memory, and reload reliability

## Phased Plan

## Phase 0: Prevent Further Decay

**Objective:** stop quality from getting worse.

Tasks:

- ~~add stricter CI checks for clippy and all-target/all-feature builds~~ — done, but as local gates in `scripts/check_guardrails.sh:75-78`, not CI
- ~~ratchet warning policy downward~~ — done, baseline 8
- document code quality standards and file-size goals
- ~~establish a tracked todo list for the quality program~~ — superseded: the JSON ratchet baselines in `scripts/` (`code_size_budget.json`, `test_size_budget.json`, `panic_budget.json`, `swallowed_error_budget.json`, `wildcard_reexport_budget.json`) are the live tracker. Each one names every offending file and count, so the debt list cannot drift from the code.

Success criteria:

- no new warnings land unnoticed **when the sweep is run**
- no new giant files are added casually
- contributors can see the roadmap and standards in-repo

Caveat that applies to every phase below: with no `.github/`, none of these gates
are automatic. `scripts/check_guardrails.sh` before pushing is the whole
enforcement mechanism.

## Phase 1: Warning and Dead-Code Burn-Down

**Objective:** restore signal quality in builds.

Tasks:

- remove unused variables, methods, and stale helpers
- replace broad `#![allow(dead_code)]` with narrow scoped allows where truly needed
- delete abandoned code paths
- reduce dead code in TUI, memory, and provider modules

Success criteria:

- warning count materially reduced
- dead-code suppression becomes the exception, not the default

## Phase 2: Decompose the Biggest Files

**Objective:** eliminate the primary maintainability hazard.

Priority order (repointed at current paths):

1. ~~`tests/e2e/main.rs`~~ — **done**, now 17 lines
2. `crates/jcode-app-core/src/server.rs` (2329) plus
   `crates/jcode-app-core/src/server/client_lifecycle.rs` (3282) and
   `server/swarm.rs` (3170)
3. `crates/jcode-tui/src/tui/ui_messages.rs` (4417)
4. `crates/jcode-tui/src/tui/app/inline_interactive.rs` (4337) and
   `app/input.rs` (4026)
5. `crates/jcode-base/src/provider/mod.rs` (2891)
6. `crates/jcode-tui/src/tui/ui.rs` (3683)
7. `crates/jcode-tui/crates/jcode-tui/src/tui/info_widget.rs` (2239)
8. `src/cli/commands.rs` (3375)

`src/provider/openai.rs` has left this list: it is a 143-line shim now.

Approach:

- extract pure helpers first
- extract types and state machines second
- extract domain-specific submodules third
- keep public interfaces stable during moves

Success criteria:

- each hotspot file becomes materially smaller
- functionality remains stable
- tests remain green during each split

## Phase 3: Strengthen Error Handling

**Objective:** make failure modes explicit and recoverable.

Tasks:

- reduce production `unwrap` / `expect`
- improve error context with `anyhow` / `thiserror`
- classify retryable vs user-facing vs internal invariant failures
- add tests for malformed streams, reconnects, and tool interruption paths

Success criteria:

- fewer panic-prone production paths
- clearer logs and more diagnosable failures

## Phase 4: Rebalance the Test Pyramid

**Objective:** make failures faster, narrower, and more actionable.

Tasks:

- split e2e suites by feature
- add more unit tests for parsing, protocol, and state transitions
- add snapshot or golden tests for stable render outputs
- add property tests for serialization, tool parsing, and patch/edit invariants
- improve test support utilities and isolation

Success criteria:

- lower test maintenance cost
- failures localize to one subsystem quickly

## Phase 5: Reliability and Performance Guardrails

**Objective:** keep architectural quality aligned with runtime quality.

Tasks:

- add or strengthen memory and stress checks
- add repeated reload / attach / detach reliability tests
- track startup and idle resource regressions
- improve structured diagnostics around reload, sockets, and provider streaming

Success criteria:

- regressions are caught before release
- long-running behavior is measurably stable

## Phase 6: Finish the Ratchet

**Objective:** make quality self-sustaining.

Tasks:

- move from warning budget to effectively warning-free builds (baseline is already 8)
- enforce stricter clippy rules where practical
- document module ownership expectations — see
  [`../CRATE_OWNERSHIP_BOUNDARIES.md`](../CRATE_OWNERSHIP_BOUNDARIES.md)
- review and refresh architecture docs after refactors land
- add the one guard this plan wants and the repo lacks: a ratcheted count of
  broad `allow(dead_code)` suppressions

Success criteria:

- repo quality remains high without special cleanup pushes
- the codebase resists drift by default

## Immediate Execution Order

The remaining concrete actions:

1. ~~land this quality plan and a tracked todo list~~ — plan landed; the JSON
   ratchet baselines replaced the todo list
2. ~~tighten CI guardrails~~ — landed as local gates; a CI phase is not
   reachable without `.github/`
3. burn down the swallowed-error baseline (3187 across 450 files), the largest
   remaining ratchet
4. ~~split `tests/e2e/main.rs`~~ — done
5. continue into `crates/jcode-app-core/src/server.rs`

## Initial Target Refactors

### `tests/e2e/main.rs` — done

`tests/e2e/main.rs` is now a 17-line harness. The suite is split into
`tests/e2e/session_flow.rs`, `provider_behavior.rs`, `reload_multiclient.rs`,
`ambient.rs`, `burst_spawn.rs`, `transport.rs`, `binary_integration.rs`,
`windows_lifecycle.rs`, `mock_provider.rs`, and `tests/e2e/test_support/`. The
originally proposed `tool_execution.rs` and `swarm.rs` files were not created;
that coverage lives in the other suites.

### `crates/jcode-app-core/src/server.rs` (2329 lines)

The `crates/jcode-app-core/src/server/` submodule tree already exists (including
`socket.rs`, `client_lifecycle.rs`, `lifecycle.rs`, `debug.rs`, `swarm.rs`,
`client_api.rs`). The remaining work is shrinking the parent file into a
facade/composition module and splitting the two 3k-line submodules.

### `crates/jcode-app-core/src/agent.rs` (998 lines)

Largely done: `crates/jcode-app-core/src/agent/` holds the extracted submodules.
Remaining work is the turn-loop unification tracked as Phase 4 of
[`../REFACTORING.md`](../REFACTORING.md).

### `crates/jcode-base/src/provider/mod.rs` (2891 lines)

The per-concern modules this section proposed already exist as siblings:
`crates/jcode-base/src/provider/routing.rs`, `pricing.rs`, `selection.rs`,
`registry.rs`, `models.rs`, `failover.rs`, `state.rs`, `dispatch.rs`. The
`Provider` trait itself moved out to `jcode-provider-core`, and the eight
concrete runtimes to the `jcode-provider-*-runtime` crates. What is left in
`mod.rs` is composition glue that still needs splitting.

## Working Rules for the Refactor Program

- every step must compile or fail for a very obvious temporary reason
- prefer moving code without changing behavior
- avoid mixing cleanup and feature work in the same commit when possible
- when a file is touched, leave it cleaner than it was
- if a new broad allow-suppression is added, it must be documented in the PR

## Validation Matrix

Minimum validation during this program:

- `scripts/check_guardrails.sh --skip-slow` (format + every ratchet)
- `cargo check -q`
- `cargo test -q`
- targeted tests for touched areas

Full sweep before pushing:

- `scripts/check_guardrails.sh`

Stricter validation when touching core orchestration or provider code:

- `cargo check --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo test --test e2e`

## Ownership

This is an active engineering program, not a one-time cleanup document. The
expectation is:

- the plan is updated as milestones are completed
- the ratchet baselines in `scripts/*_budget.json` are rebaselined only after
  intentional cleanup. `scripts/check_guardrails.sh --fix` does that for the four
  `run_ratchet` gates (`check_guardrails.sh:50-57, 85-88`); the wildcard
  re-export budget is a plain gate (`:90`) and must be rebaselined by hand
- progress is visible in the repo
- each completed phase leaves behind stronger guardrails than before
