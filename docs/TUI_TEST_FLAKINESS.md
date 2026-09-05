# jcode-tui test flakiness

Status: the process-global render-state race described below is **fixed**
(reentrant lock, `crates/jcode-tui/src/tui/ui.rs:1446-1545`). One flakiness
class remains and it is not a race: tests that read the developer's real
credential state. Both are documented here.

## Fixed: the process-global render-state race

`cargo test -p jcode-tui --lib` used to fail 1-4 tests per run with a varying
set, while `--test-threads=1` passed and each failing test passed alone.
Counts were taken on 2026-07-27 (2006/2006 passing serially, 16 ignored) and
have drifted since.

### Root cause

`create_test_app()` (and its `create_named_provider_test_app` sibling) in
`crates/jcode-tui/src/tui/app/tests/support_failover/part_01.rs:178` calls
`crate::tui::ui::clear_test_render_state_for_tests()`, which wipes
**process-global** render state: flicker frame history, layout snapshots,
status-area snapshots, copy targets, and scroll positions
(`clear_test_render_state_locked`, `ui.rs:1520-1545`).

Rendering tests guard exactly that state with `render_state_test_lock()`. The
clear ran *without* taking the lock, so any of its call sites could reset a
concurrently-running render test's state mid-assertion. The mechanism for the
most frequent victim (`test_changelog_overlay_repeated_renders_are_stable`) is
a recorded flicker event adding a "⚠ flicker detected" notification line to
later renders, shifting every layout-sensitive assertion by a row
(`ui.rs:1529-1532`).

Bisecting the `tui::app::tests::` suite against the changelog test identified
`test_tui_login_providers_have_real_tui_handlers`, which calls
`create_test_app()` once per login provider. Running just those two did not
reproduce; the race needed enough concurrent load to interleave, which is why
it presented as order-dependent flakiness.

### The fix

Reentrant ownership tracking, not serialization. `render_state_test_lock()`
(`ui.rs:1454-1463`) returns a `RenderStateTestGuard` that sets a thread-local
`RENDER_STATE_LOCK_HELD` flag (`ui.rs:1501-1511`) and clears it on drop
(`ui.rs:1473-1478`). `clear_test_render_state_for_tests` now routes through
`with_render_state_lock` (`ui.rs:1492-1499`), which takes the lock only when
this thread does not already hold it:

- render tests, which already hold the lock, get a nested no-op instead of a
  self-deadlock;
- `create_test_app` call sites, which hold nothing, block for the duration of
  one reset instead of wiping a peer's state.

The same commit collapsed two independently-defined private locks into this
one, which is what made the guard meaningful — two locks serialized nothing
between them (`ui.rs:1446-1453`, issue #593).

This supersedes the earlier "make it thread-local" and "skip the clear in
`create_test_app`" proposals; neither is needed.

### What was measured and rejected

**Taking `render_state_test_lock` unconditionally inside `create_test_app`.**
Correct but serializes every call site (`create_test_app(` appears 908 times
across `crates/jcode-tui/src`, spread over three same-named helpers: the
shared one above plus local ones in `app/remote_tests.rs:40` and
`ui_header.rs:1041`): suite runtime went from ~12s to over 10 minutes.
Measured, then reverted. The reentrant guard gets the correctness without the
serialization.

**Asserting a floor instead of an exact count** in the changelog test's
`buffered_samples` check, and **calling `clear_test_render_state_for_tests`**
at the top of that test. Both measured over 5 runs: the test still failed 5/5
with *and* without the change. Reverted rather than committed as churn.

## Live failure class: tests that read the real credential state

This one is not a race and does not depend on thread count. It fails
deterministically on a developer machine that has providers configured, and
passes on a clean machine or in a clean `HOME`.

Known failing test:

```
tui::app::tests::login_openai_phase_is_default_when_no_imports
```

(`crates/jcode-tui/src/tui/app/tests/onboarding_flow.rs:328-350`)

It asserts that a fresh install with nothing importable lands on
`OnboardingPhase::LoginOpenAi { yes_highlighted: true }`. The test *is*
sandboxed — `with_temp_jcode_home`
(`app/tests/support_failover/part_01.rs:400-428`) points `JCODE_HOME` at a
tempdir, and `user_home_path` (`crates/jcode-storage/src/lib.rs:204-219`)
redirects every file-backed external credential source under
`$JCODE_HOME/external/`. But the sandbox does not cover everything
`begin_onboarding_flow_at_login`
(`crates/jcode-tui/src/tui/app/onboarding_flow_control.rs:218-246`) probes via
`pending_external_auth_review_candidates`
(`crates/jcode-app-core/src/external_auth.rs:234-322`):

- `auth::claude::native_credentials_present()`
  (`crates/jcode-base/src/auth/claude.rs:784-807`) shells out to
  `/usr/bin/security find-generic-password`, which reads the *login keychain*
  — resolved from the OS user, not from `JCODE_HOME`.
- The same function short-circuits on the `CLAUDE_CODE_OAUTH_TOKEN`
  environment variable, which the sandbox also does not clear.

Either one makes `import` `Some(..)`, so the flow lands on
`OnboardingPhase::Login { import }` and the `LoginOpenAi` assertion fails.

Workaround while running the suite locally:

```sh
HOME=/tmp/jcode-clean-home cargo test -p jcode-tui --lib
```

An empty `HOME` denies the `security` lookup its keychain, so the probe
reports no candidates and the test sees the fresh-install state it asserts.
Unset `CLAUDE_CODE_OAUTH_TOKEN` too if it is exported in your shell.

The real fix belongs in the sandbox, not in the test: `with_temp_jcode_home`
should also neutralize the non-file credential sources (keychain probe and
`CLAUDE_CODE_OAUTH_TOKEN`) so the hermetic-home helper is actually hermetic.
Until then, treat a failure of this test on a configured machine as
environmental, and confirm it under a clean `HOME` before investigating.

## Scope note

The render-state race was pre-existing and independent of the render-path
performance work in commits `0ba0154c6`, `2b8e78e34`, `8b44fc83b`,
`8142f1a0b`. Verified at the time by stashing those changes and reproducing
the same failure rate.
