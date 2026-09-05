# Test isolation: flakiness, deadlock, and platform assumptions

Status: **resolved**. `cargo test --workspace --lib` reports 6480 passed / 0
failed, repeatably. Four independent defects had to be fixed to get there; each
is documented below with its evidence, because every one of them will come back
if the mechanism is undone.

The suites in this repo read and write process-global state — `JCODE_HOME`-derived
paths, the config cache, the provider context-limit cache, the TUI's render-state
globals. That is the root cause behind three of the four items.

## 1. Fixed: the two test locks deadlocked each other

`cargo test -p jcode-tui --lib` never finished. It stalled permanently after
**497 of 2167** tests (`exit=124` at 180s, 1500s and 3000s alike), so nobody
ever saw a summary line. `--test-threads=1` completed in 44s, which is why the
hang looked like slowness rather than a deadlock.

`sample` on the stalled process named it exactly:

```
Thread …875: remote_reasoning_delta_burst_is_paced_not_dumped
  with_temp_jcode_home                  <- HOLDS jcode_base::storage::lock_test_env
    create_test_app -> clear_test_render_state_for_tests
      render_state_test_lock -> Mutex::lock -> __psynch_mutexwait   <- WAITS on render lock
Thread …460, …785, …611: with_temp_jcode_home
      lock_test_env -> __psynch_mutexwait                           <- WAIT on env lock
```

Two global mutexes, acquired in opposite orders by different tests: 5 tests took
render-then-env (they render, then swap `JCODE_HOME`), 8 took env-then-render
(they swap `JCODE_HOME`, then build an app — `create_test_app` takes the render
lock internally). Textbook AB-BA.

**The fix:** there is now one lock, and it is reentrant per thread.
`jcode_base::storage::lock_test_env` (`crates/jcode-base/src/storage.rs:24-88`)
returns a `TestEnvGuard` that holds the mutex only for the outermost acquisition
on a thread and counts depth for nested ones.
`crate::tui::ui::render_state_test_lock` (`crates/jcode-tui/src/tui/ui.rs`)
delegates to it instead of owning a private mutex.

Why reentrancy rather than ordering rules: `create_test_app` appears ~908 times
across `crates/jcode-tui/src` and acquires the lock several layers down from the
helper that already holds it. A non-reentrant lock self-deadlocks on the inner
call; an ordering convention cannot be enforced at 908 call sites. One reentrant
lock cannot be acquired out of order, so the failure mode is gone by
construction rather than by discipline.

This supersedes the previous per-thread `RENDER_STATE_LOCK_HELD` flag, which
solved the nested-clear problem for one lock but left the two-lock inversion in
place. That machinery is deleted.

## 2. Fixed: parallel-only races on config and catalog globals

With the deadlock gone, the suite ran to completion and exposed what the hang
had been hiding: 3-6 failures per run with a *shifting* set
(`test_model_picker_*`, `test_tui_cerebras_paste_key_lifecycle_*`,
`render_system_message_uses_scheduled_task_card`, …). Each passed alone and
passed serially.

Two changes fixed it:

- `ensure_test_jcode_home_if_unset` (`app/remote_tests.rs`, `ui_header.rs`) used
  to set `JCODE_HOME` and return nothing — 25 call sites fired it and forgot it,
  so nothing serialized the test body that followed. It now returns the
  `TestEnvGuard` and is `#[must_use]`; callers bind it for the test's duration.
- `.cargo/config.toml` sets `RUST_TEST_THREADS = "1"`. Tests within one binary
  share process globals and are only isolated when they run one at a time.
  Cargo still runs the crates' test binaries in parallel (separate processes,
  separate globals), so the cost is small: `jcode-tui` measures **43s
  serialized vs 22s threaded**, and the whole workspace runs in ~171s. That buys
  a suite that reports the same result every run.

The per-test `lock_test_env()` guard is still required — the `test-support`
feature is consumed by callers that do run threaded.

## 3. Fixed: sequential order dependence via the context-limit cache

Serializing exposed the mirror-image bug: one `jcode-base` test passed in
parallel and failed sequentially.

```
provider::tests::test_resolve_model_capabilities_uses_provider_hint
  left: Some(272000)   right: Some(1000000)
```

Instrumenting the insert path (libtest names threads after tests, so the
culprit prints itself):

```
PROBE_INSERT gpt-5.4=272000 by Some("provider::tests::test_multi_provider_antigravity_routes_do_not_include_legacy_duplicate_entries")
```

`CONTEXT_LIMIT_CACHE` (`crates/jcode-base/src/provider/models.rs:85`) is
process-global and gets seeded as a *side effect* of ordinary work:
`MultiProvider::model_routes()` publishes the active provider's catalog, and the
Antigravity catalog lists gpt-5.x at 272k. The dynamic cache outranks the static
classification tables, so a test asserting the static tables silently inherited
another test's catalog.

**The fix:** `clear_context_limit_cache()` (test/`test-support` only), called by
the four tests that assert static classification. Those tests now state their
precondition instead of depending on ordering.

## 4. Fixed: tests pinning one platform's literals

Six tests asserted the ASCII spelling of the Alt keycap and failed on the only
platform this fork supports. Commit `73913b0aa` ("tui: show ⌥ instead of Alt in
keybinding hints on macOS") introduced `alt_label()`
(`crates/jcode-tui-core/src/keybind.rs:10`) and updated 13 source files but no
tests, so they passed on upstream's Linux CI and failed here:

- `⌥+Enter` vs `"Alt+Enter"`, `⌥+Shift+I` vs `"Alt+Shift+I"`, `⌥+M`, `⌥+C`.
- Two copy-badge width tests, same cause arithmetically: `" [⌥] [⇧] [A]"` is
  **12** columns, `" [Alt] [⇧] [A]"` is **14**, and the tests hardcoded
  `30 - 14`.

They now assert through `alt_label()`/`alt_chord()` and
`copy_badge_reserved_width()`/`copy_badge_alt_badge()` — the same helpers the
renderer uses — so they describe behavior instead of one platform's rendering.

Two related fixes fell out of the same investigation:

- **`Ctrl+5` was dead on macOS** (a product bug, not a test bug).
  `Ctrl+]` arrives as byte 0x1D, which crossterm decodes as `Ctrl+5`, and
  `ctrl_bracket_fallback_to_esc` rewrote it unconditionally — so it never
  reached `ctrl_prompt_rank`, and the fifth slot of the `Ctrl+<digit>`
  prompt-recency jump fell through to the input handler, which snapped the
  transcript to the bottom. The rewrite is now gated on a diagram being on
  screen, which is the only consumer of `']'`.
- **The credential sandbox is now hermetic.** `login_openai_phase_is_default_when_no_imports`
  failed on any Mac with Claude Code logged in: `JCODE_HOME` redirects
  file-backed credential sources under `$JCODE_HOME/external/`, but the login
  Keychain is process-wide, so `native_credentials_present()` still found the
  host's real login (`keychain:Claude Code-credentials`). Both Keychain probes
  now return early when `JCODE_HOME` is set
  (`keychain_reads_sandboxed`, `crates/jcode-base/src/auth/claude.rs`). This is
  the fix this document previously called for; the `HOME=/tmp/clean` workaround
  is no longer needed.

## Deleted rather than re-pinned

`stdin_detect_tests::test_own_process_not_reading_stdin` asserted that the
*test process itself* was not reading stdin. That is a property of the harness:
`macos::check` reports `Reading` when fd 0 is a pipe or vnode and any thread
sits in `TH_STATE_WAITING`, both of which a libtest process launched from a
shell pipeline satisfies whenever a worker thread is parked. It passed or failed
depending on how the suite was invoked. Replaced with
`blocked_child_on_a_pipe_is_reported_as_reading`, which spawns `cat` on a pipe
and polls the real detector.

Writing that replacement surfaced a genuine limitation, now documented on
`macos::check`: a process whose stdin is `/dev/null` is indistinguishable from
one on a pty here (both are vnodes), so `Reading` is advisory. Separating them
needs the fd's vnode path via `PROC_PIDFDVNODEPATHINFO`, which is declared but
unused.

## Measured and rejected

- **Taking the render lock unconditionally inside `create_test_app`.** Correct
  but serializes 908 call sites; suite runtime went from ~12s to over 10
  minutes. The reentrant guard gets the correctness without it.
- **Asserting a floor instead of an exact count** in the changelog test's
  `buffered_samples` check, and calling `clear_test_render_state_for_tests` at
  the top of that test. Both measured over 5 runs; the test failed 5/5 with and
  without. Reverted rather than committed as churn.

## Verification

```sh
cargo test --workspace --profile selfdev --lib     # 6480 passed, 0 failed, ~171s
cargo test -p jcode-tui --profile selfdev --lib    # 2149 passed, 17 ignored, ~43s
```

The 17 ignored `jcode-tui` tests are deliberate developer benchmarks and
measurement harnesses (`#[ignore = "developer benchmark: …"]`), not disabled
coverage.
