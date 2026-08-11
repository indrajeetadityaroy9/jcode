# Fork Maintenance Runbook

> Operational guide for this **personal fork** of [`1jehuang/jcode`](https://github.com/1jehuang/jcode).
> Written so a future Claude Code instance (or a human) can repeat the full cycle
> without rediscovering the traps. Every pitfall listed in the Appendix was hit
> for real; none is hypothetical.

**Scope of this document**
1. What this fork removes, and why that must survive every sync
2. Full uninstall / clean slate
3. Sync with upstream, filtering out the purged subsystems
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
| Last upstream merge | `fd1ff012c` — v0.75.3 |
| Rollback tag convention | `pre-merge-YYYY-MM-DD` |
| Install layout | `~/.jcode/builds/versions/<hash>/` + `stable`/`current` symlinks + `~/.local/bin/jcode` |

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

**Deliberately *not* removed** (easy to delete by mistake):

- `jcode-base/src/login_qr.rs` + `qrcode` dep — OAuth device-code login, 13 callers
- `jcode-base/src/session/load_telemetry.rs` — session-load burst detection, **not** analytics
- Provider KV-cache "telemetry" in `info_widget.rs` / `state_ui.rs` — the cold-cache warning
- `subscription_api.rs` / `subscription_catalog.rs` / `/subscription` / `jcode login --provider jcode` — provider auth
- `ApiEvent::PermissionRequest` in `jcode-harness-api` / `jcode-sdk` — desktop wire protocol
- All 19 `jcode-provider-*` crates and every OAuth flow

**Also excluded by choice** (not purged, just not merged from upstream):
subscription onboarding pill · discovery reframing · upstream's other onboarding changes.

### The guard

`scripts/purge-guard.sh` encodes the invariant. Run it after every sync.

```bash
./scripts/purge-guard.sh          # non-test code; exit 0 = clean
./scripts/purge-guard.sh --all    # include tests (this fork does not maintain tests)
```

Keep its patterns **tight**. A guard that cries wolf gets ignored — bare `request_permission`
matches an unrelated Grok provider method; the quoted tool name and the type name do not.

---

## 2. Full uninstall (clean slate)

`scripts/uninstall.sh` alone is **not sufficient**. Run these in order.

```bash
# 1. Retire the KeepAlive LaunchAgent FIRST — while the binary still exists.
#    KeepAlive=1 + RunAtLoad=1 means it respawns the hotkey listener forever,
#    including after you delete the binary it points at.
jcode setup-hotkey --uninstall
launchctl bootout gui/$UID/com.jcode.hotkey 2>/dev/null
rm -f ~/Library/LaunchAgents/com.jcode.hotkey.plist

# 2. Stop processes uninstall.sh does not match.
#    Its pkill pattern is 'jcode( .*)? serve' — menubar and the hotkey listener survive.
pkill -f 'jcode menubar'; pkill -f 'jcode setup-hotkey'

# 3. Uninstall.
bash scripts/uninstall.sh --yes            # binaries + apps, KEEPS ~/.jcode
bash scripts/uninstall.sh --purge --yes    # ALSO wipes ~/.jcode (see warning)

# 4. Stale sockets (macOS puts them under /var/folders, not /tmp).
rm -f /var/folders/*/*/T/jcode*.sock

# 5. Artifacts OUTSIDE ~/.jcode that no jcode uninstall path touches.
#    `jcode setup-hotkey --uninstall` ADDS a SessionStart hook to Claude Code
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
MAIN=/path/to/jcode
WS=/tmp/jcode-sync && rm -rf "$WS" && mkdir -p "$WS"

cd "$MAIN"
git status --porcelain        # MUST be empty
git tag -a "pre-merge-$(date +%F)" -m "rollback point" HEAD   # -a: annotated tags may be forced

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
actually required (last run: **136 pure-kept / 9 mixed / 10 pure-purged / 4 empty** out of 159).

```bash
# For each commit in BASE..upstream/master, bucket by the paths it touches and
# whether its added lines mention purged identifiers.
# PURE_KEPT  -> merge handles automatically
# PURE_PURGED-> skip (our deletions win)
# MIXED      -> real feature + purged code in one commit; needs surgery
```

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

```bash
./scripts/purge-guard.sh
# plus: diff upstream's added lines against our tree to find non-purged work that
# went missing (the check that caught the --ours damage).
```

### 3.5 Commit in tmp, then transfer

```bash
git add -A
git commit --no-verify -m "Merge upstream <ver>, excluding purged subsystems"
#          ^^^^^^^^^^^ gitleaks pre-commit hook false-positives on upstream's
#          keyboard-shortcut table (`key: "Ctrl+Shift+Tab"`). REVIEW the findings
#          first — you are importing hundreds of upstream files — then bypass.

git log -1 --pretty=%p        # MUST print TWO hashes. One = not recorded as a
                              # merge; future syncs will re-conflict everything.
```

Transfer the **verified commit** — never replay resolutions by hand:
```bash
cd "$MAIN"
git fetch "$WS/fork" merge-upstream:verified
git merge --ff-only verified
```

---

## 4. Build

```bash
cd "$MAIN"
git status --porcelain        # empty, so the version string is reproducible

cargo build --profile release-lto 2>&1; echo "CARGO_EXIT=$?"
```

- **Never pipe cargo through `tail`/`head`** — you get the *pager's* exit code and a failed
  build reports success. Echo `$?` from cargo directly.
- `cargo build` does **not** compile `tests/` or `#[cfg(test)]`. A green build says nothing
  about the test suite. This fork does not maintain tests; treat `cargo check --all-targets`
  as advisory only (it always fails here).
- `release-lto` = thin LTO, ~5–7 min cold on an M-series laptop at the repo's pinned
  `jobs = 4` (`.cargo/config.toml`, deliberate RAM cap).
- **Commit before building.** `jcode-build-meta` bakes the git hash in; building from a dirty
  tree yields an unreproducible binary. Forcing a re-stamp afterwards (`touch
  crates/jcode-build-meta/build.rs`) costs another full LTO rebuild.

---

## 5. Install

Nothing running? Then this is pure file operations. `scripts/install_release.sh` also
registers global hotkeys, installs `Jcode.app`, and **edits your shell rc files** — do it
manually to skip those.

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

**If a daemon is running** (`pgrep -fl jcode`), adopt the new binary afterwards:
```bash
jcode server reload        # exec()s into the new binary on the same socket
pkill -f 'jcode menubar'; pkill -f 'jcode setup-hotkey'   # long-running, keep old code
python3 -c "import json,os;print(json.load(open(os.path.expanduser('~/.jcode/servers.json')))
  [list(json.load(open(os.path.expanduser('~/.jcode/servers.json'))))[0]]['git_hash'])"
# ^ the git_hash must change — that is the proof the daemon adopted the new binary
```

**Rollback:** repoint `current`/`stable` at the previous `versions/<hash>/` and reload. No
rebuild. (After a `--purge` there is no previous version, so rollback becomes
`git reset --hard pre-merge-<date>` + rebuild.)

---

## 6. Verification gates

Run in order. Everything except the build is seconds.

| # | Gate | Command | Pass |
|---|---|---|---|
| 1 | Purge invariant | `./scripts/purge-guard.sh` | exit 0 |
| 2 | Module decls resolve | `python3 scripts/check_module_files.py` | exit 0 |
| 3 | Lockfile coherent | `cargo metadata --locked --format-version 1 >/dev/null` | exit 0 |
| 4 | Secret scan of incoming | `gitleaks git --staged --redact --no-banner -v` | reviewed |
| 5 | Merge recorded | `git log -1 --pretty=%p` | **two** hashes |
| 6 | Build | `cargo build --profile release-lto; echo $?` | 0 |
| 7 | Smoke, isolated | `./target/release-lto/jcode --no-update --socket /tmp/verify.sock run 'hi'` | reaches provider layer |
| 8 | Removed CLI absent | `jcode --help \| grep -E '^\s+(pair\|permissions)\b'` | no match |
| 9 | Version reproducible | `jcode --version` | matches HEAD, no `-dirty` |

Gate 7 uses an **isolated socket** deliberately (`AGENTS.md`): it proves the *new* binary
works rather than an old daemon answering. Without credentials it stops at
`No credentials configured for provider auto-detection` — that is a **pass**.

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

5. **`com.jcode.hotkey` LaunchAgent has `KeepAlive=1`.** `uninstall.sh` never touches it;
   launchd respawns a deleted binary forever. Retire it *before* removing the binary.

6. **`jcode setup-hotkey --uninstall` ADDS hooks** to Claude Code and Codex. An uninstall path
   that installs things — always re-check `~/.claude/settings.json` afterwards.

7. **The gitleaks pre-commit hook blocks the merge commit** on upstream's keyboard-shortcut
   table (`key: "Ctrl+Shift+Tab"`, `generic-api-key`, entropy 3.52). Review, then `--no-verify`.

8. **Closing the TUI does not stop the daemon.** `jcode serve` persists; check `pgrep`.

9. **`macos_notification_broker.rs` used `jcode::` instead of `crate::`** — a pre-existing
   upstream bug, invisible on Linux, that breaks the first macOS build. Fixed in this fork;
   re-check after syncs.

10. **`jcode-build-meta` caches the version stamp.** Committing after building leaves a stale
    hash in the binary; re-stamping forces a full LTO rebuild. Commit first.

11. **`tail -f` monitors never self-terminate.** For "tell me when the build finishes", use a
    background command that exits — the harness notifies on completion by itself.

12. **`~/.jcode` reappears** from any `jcode --version` invocation (migration markers + a log).
    Harmless; not a failed purge.
