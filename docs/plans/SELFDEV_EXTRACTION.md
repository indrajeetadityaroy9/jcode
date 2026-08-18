# Spec: untangle and remove the self-development subsystem

Status: **not started** · Scope: 2 steps, do them in order · Est: step 1 is mechanical, step 2 is bulk deletion

## Why

This fork is edited externally (a coding agent drives the source, `FORK_WORKFLOW.md` §4 builds
it, §5 installs it). The `selfdev` *tool* — canary build queue, `jcode self-dev`, its own
clone/setup/status/launch machinery — duplicates that pipeline with a worse version. Its setup
path also runs `git clone https://github.com/1jehuang/jcode.git`, which would hand you an
**unpurged upstream tree**.

So the tool should go. But **`selfdev` cannot be deleted as a unit**, and this is the whole
reason the job needs a spec instead of a `git rm`.

## The naming trap

`ReloadContext` is defined at `crates/jcode-app-core/src/tool/selfdev/mod.rs:72`. It has nothing
to do with self-development. It is the save/restore record for **ordinary daemon reload and
client reconnect recovery** — the path that runs on *every* client connect and every
`jcode server reload`, which §5 mandates after every install.

Verified consumers of `selfdev::` symbols from outside the module (22 of them are `ReloadContext`):

```
crates/jcode-tui/src/tui/app/remote/reconnect.rs      load_reload_reconnect_hints ->
                                                       ReloadContext::peek_for_session
crates/jcode-tui/src/tui/app/remote/server_events.rs
crates/jcode-tui/src/tui/app.rs, app/commands.rs
crates/jcode-app-core/src/server/reload.rs
crates/jcode-app-core/src/server/reload_recovery.rs
crates/jcode-app-core/src/server/client_state.rs
crates/jcode-app-core/src/server/client_actions.rs
crates/jcode-app-core/src/server.rs
crates/jcode-app-core/src/agent/turn_execution.rs
+ 8 test files
```

Deleting the module breaks reconnect for every session, not just self-dev ones.

`ReloadRecoveryDirective` is worse: it lives in `crates/jcode-selfdev-types/src/lib.rs:20`, is
re-exported through `selfdev/reload.rs:2`, and is **aliased into the wire protocol** as
`jcode_protocol::ReloadRecoverySnapshot` (`crates/jcode-protocol/src/lib.rs:172`). It is part of
the client/server contract.

## Verified inventory

Measured, not estimated: **1,658 `selfdev`/`self_dev` references workspace-wide.**

### Tier A — load-bearing, must survive (extract, do not delete)

| Item | Where | Note |
|---|---|---|
| `ReloadContext` struct | `tool/selfdev/mod.rs:72` | 5 plain serde fields |
| `ReloadContext` impl, 12 methods | `tool/selfdev/reload.rs` (427 ln) | `save`/`load`/`peek_for_session`/`recovery_directive*`/`continuation_message*`/`log_recovery_outcome` |
| `persisted_background_tasks_note` | `tool/selfdev/reload.rs:173` | called from TUI |
| `ReloadRecoveryDirective` | `jcode-selfdev-types/src/lib.rs:20` | **in the wire protocol** |
| `CLIENT_SELFDEV_ENV`, `client_selfdev_requested()` | `jcode-selfdev-types/src/lib.rs:10,15` | used by `src/cli/proctitle.rs`, TUI |
| `selfdev: Option<bool>` | `jcode-protocol/src/wire.rs:126` | wire field; removing it is a protocol break |

`reload.rs` imports only `super::*` plus `jcode_selfdev_types::ReloadRecoveryDirective`, so it
carries no dependency on `build_queue`/`setup`/`status`/`launch`. **The extraction is clean.**

### Tier B — the actual tool (delete in step 2)

```
crates/jcode-app-core/src/tool/selfdev/build_queue.rs   1352
crates/jcode-app-core/src/tool/selfdev/tests.rs         1440
crates/jcode-app-core/src/tool/selfdev/mod.rs            842   minus ReloadContext
crates/jcode-app-core/src/tool/selfdev/setup.rs           354   <- clones UPSTREAM
crates/jcode-app-core/src/tool/selfdev/status.rs          301
crates/jcode-app-core/src/tool/selfdev/launch.rs          225
src/cli/selfdev.rs                                        234
src/cli/selfdev_tests.rs                                  352
crates/jcode-desktop2/src/selfdev_reload.rs               122
tests/test_selfdev_reload.py                              242
scripts/bench_selfdev_checkpoints.sh                      207
scripts/bench_selfdev_build.sh                            116
crates/jcode-base/src/prompt/selfdev_mode.txt              20
crates/jcode-base/src/prompt/selfdev_focus_{tui,desktop2}.txt  8
                                                        ------
                                                         ~5800
```

Plus registrations: `SelfDevTool` (`tool/mod.rs:270`, `:1136`), `Command::SelfDev`
(`src/cli/args.rs:259`), and the prompt plumbing in `jcode-base/src/prompt.rs`
(`SELFDEV_*_PROMPT` consts, `selfdev_chars`, `build_system_prompt_with_selfdev`).

### Tier C — must NOT be touched

| Keep | Why |
|---|---|
| `[profile.selfdev]` + its `[profile.selfdev.package.*]` blocks in `Cargo.toml` | A **cargo build profile** for fast rebuilds. `scripts/dev_cargo.sh` uses it. Unrelated to the tool. |
| `crates/jcode-build-support` (3,401 ln) | **Not** selfdev-only. `read_build_progress`, `stable_binary_path`, `is_jcode_repo`, `preferred_reload_candidate`, `resolve_binary_payload`, `read_stable_version` are used across `jcode-tui` (status line, redraw scheduling, reload candidate choice). Only its `run_selfdev_build` / `selfdev_binary_path` / `selfdev_build_command*` / `SELFDEV_CARGO_PROFILE` / canary helpers are Tier-B candidates. |
| `crates/jcode-selfdev-types` as a crate | Consumed by `jcode-protocol`, `jcode-tui`, `jcode-build-support`. Rename it, do not delete it. |

## Step 1 — extract the reload machinery

Goal: nothing named `selfdev` remains on the reconnect path. **No behavior change.**

1. Create `crates/jcode-app-core/src/server/reload_context.rs`. Move `ReloadContext` (from
   `tool/selfdev/mod.rs:72`) and the whole `impl` block plus `persisted_background_tasks_note`
   (from `tool/selfdev/reload.rs`). Keep the on-disk format byte-identical — `path_for_session`
   and the serde field names must not change, or in-flight reload markers from an older binary
   stop resolving.
2. Re-export from `jcode-app-core` so consumers import
   `crate::server::reload_context::ReloadContext`. Update the 17 external files.
3. Rename `crates/jcode-selfdev-types` → `crates/jcode-reload-types` (or fold into
   `jcode-protocol`, which already aliases its main type). Decide which; folding removes a crate
   but widens `jcode-protocol`'s surface.
4. Leave a `pub use` shim at the old path **only if** step 2 is not landing immediately.

Verification for step 1:

```bash
cargo check --workspace                      # 0 errors
cargo test -p jcode-app-core -p jcode-tui -p jcode-protocol
./scripts/check_warning_budget.sh            # must not exceed the ratchet
# behavioral: reload must still recover a live session
jcode server reload && jcode --version
python3 scripts/test_reload.py               # exercises the reload path end to end
```

The real test is a live reload with an attached client: start a session, `jcode server reload`,
confirm the client reconnects and the transcript survives. A unit-green step 1 that breaks this
is the failure mode to watch for.

## Step 2 — delete the tool

Only after step 1 is committed and verified.

1. `git rm` every Tier-B file.
2. Drop `SelfDevTool` from the tool registry (`tool/mod.rs:270`, `:1136`) and `Command::SelfDev`
   from `src/cli/args.rs:259`, threading the removal through `dispatch.rs` and `proctitle.rs` —
   the same file set the telemetry and `--listen-windows-hotkey` purges used.
3. Remove the selfdev prompt files and their `prompt.rs` plumbing.
4. Remove the selfdev-only functions from `jcode-build-support` (Tier C note), keeping the rest.
5. Decide the fate of `selfdev: Option<bool>` in `wire.rs:126` — see open questions.
6. Extend the three guard scripts together, per §1 of `FORK_WORKFLOW.md`: paths in
   `purge-guard.sh` and `classify-upstream.sh`'s `PURGED_PATHS`, and a **tight** `PURGED_DESC` in
   `upstream-features.sh`. Tight matters: a bare `selfdev` would match `[profile.selfdev]` and
   `dev_cargo.sh`, which are kept.
7. Add the §1 purge-table row and an appendix entry.

Verification: full `FORK_WORKFLOW.md` §6 gate sweep, plus `cargo test --workspace`, plus a live
reload as in step 1.

## Open questions to settle before starting

1. **Wire field.** Is `selfdev: Option<bool>` (`wire.rs:126`) still meaningful with the tool gone?
   It is `Option`, so leaving it costs nothing and keeps the protocol stable against upstream.
   Recommend: **leave it**, remove only its producers.
2. **`jcode-selfdev-types` fate** — rename vs fold into `jcode-protocol`.
3. **`is_selfdev` session flag.** Threaded through `server/util.rs` (31 refs), `client_session.rs`
   (24). Some of it drives non-selfdev behavior (repo auto-detection via `is_jcode_repo`). Audit
   before assuming it goes.

## Risk

The one that matters: **`ReloadContext` is on the hot path for every reconnect.** A silent
regression here does not fail `cargo check` — it shows up as sessions failing to restore after a
reload, which is exactly the class of bug that took a full pty-level investigation to find last
time. Do step 1 alone, commit it, and exercise a real reload before touching step 2.

Second, the sync tax — **measured**, over the same 236 upstream commits used to justify the
Windows purge (v0.75.3 + v0.76.0 ranges):

```
  2   crates/jcode-app-core/src/tool/selfdev
  2   crates/jcode-build-support
  3   crates/jcode-desktop2/src/selfdev_reload.rs
  0   crates/jcode-selfdev-types, src/cli/selfdev.rs, jcode-base/src/prompt
 ---
  7   combined            (Windows purge targets, same window: 1)
```

7 touches in 236 commits is ~3% — low in absolute terms, but 7x the Windows surface, and unlike
that purge this code is *live on macOS*. So the `PURE_PURGED` bucket will do occasional real work
here. That is an argument for doing step 1 (which keeps the reload code, just renamed) even if
step 2 is deferred indefinitely: the extraction is what removes the *risk*, while the deletion
only removes *lines*.
