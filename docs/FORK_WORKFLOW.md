# Fork Maintenance Runbook

> Operational guide for this **personal fork** of [`1jehuang/jcode`](https://github.com/1jehuang/jcode).
> Written so a future Claude Code instance (or a human) can repeat the full cycle
> without rediscovering the traps. Every pitfall listed in the Appendix was hit
> for real; none is hypothetical.

**Scope of this document**
1. What this fork removes, and why that must survive every sync
2. Full uninstall / clean slate
3. Sync with upstream — filtering out the purged subsystems **and** confirming upstream's new
   features actually arrived. Both directions matter: a sync that quietly loses features is as
   much a failure as one that quietly reabsorbs telemetry.
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
| Local checkout | `/Users/indrajeetadityaroy/jcode` (`$MAIN` throughout this doc) |
| **Last upstream merge** | **`fd1ff012c` — v0.75.3** |
| Rollback tag convention | `pre-merge-YYYY-MM-DD` |
| Install layout | `~/.jcode/builds/versions/<hash>/` + `stable`/`current` symlinks + `~/.local/bin/jcode` |

> ⚠️ **The "Last upstream merge" row is load-bearing.** §3.2 classifies
> `BASE..upstream/master`, and `BASE` comes from this row. If it goes stale the next
> sync classifies against the wrong base and silently under-reports the work.
> **§3.6 updates it — do not skip that step.**

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

- `jcode-base/src/login_qr.rs` + `qrcode` dep — OAuth device-code login, 14 references across 7 files
- `jcode-base/src/session/load_telemetry.rs` — session-load burst detection, **not** analytics
- Provider KV-cache "telemetry" in `info_widget.rs` / `state_ui.rs` — the cold-cache warning
- `subscription_api.rs` / `subscription_catalog.rs` / `/subscription` / `jcode login --provider jcode` — provider auth
- `ApiEvent::PermissionRequest` in `jcode-harness-api` / `jcode-sdk` — desktop wire protocol
- All 20 `jcode-provider-*` crates and every OAuth flow

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

### The three scripts, and why they must move together

| Script | Direction | Answers |
|---|---|---|
| `purge-guard.sh` | defensive | is purged code present in the tree? |
| `classify-upstream.sh` | defensive | which incoming commits *risk* reintroducing it? |
| `upstream-features.sh` | **offensive** | what did upstream add, and did it actually land? |

All three encode the same §1 invariant in three different vocabularies — a tree scan, a commit
walk, and a changelog/structural read. **Change one, change all three.** A classifier that lags
the guard reports work as safe that the guard later rejects; a feature script that lags either
one reports purged code as missing upstream work.

One subtlety worth stating, because it has already caused a false alarm: prose patterns and
identifier patterns are *not* interchangeable. A changelog line says "pair your phone"; the
manifest says `jcode-gateway-types`; the config type says `GatewayConfig`. Each subsystem needs
a **bare stem** in the identifier pattern, not just its crate name.

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
MAIN=/Users/indrajeetadityaroy/jcode
WS=/tmp/jcode-sync && rm -rf "$WS" && mkdir -p "$WS"

cd "$MAIN"
git status --porcelain        # MUST be empty

# Push BEFORE you start. The tmp clone below copies $MAIN, so the sync works
# unpushed — but then the merge you are about to attempt, and the rollback tag
# that protects it, exist on exactly one disk. Pushing also makes the next
# sync's BASE reproducible from a second checkout.
git push origin master
git tag -a "pre-merge-$(date +%F)" -m "rollback point" HEAD   # -a: annotated tags may be forced
git push origin "pre-merge-$(date +%F)"                       # tags are NOT pushed by `git push`

git rev-list --left-right --count origin/master...master      # MUST print "0	0"

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
actually required, and — more importantly — **which shas to hand-audit**.

`scripts/classify-upstream.sh` does this. Run it from inside the tmp clone:

```bash
cd "$WS/fork"
./scripts/classify-upstream.sh "$BASE" upstream/master --list-mixed
```

| Bucket | Meaning | Action |
|---|---|---|
| `PURE_KEPT` | no purged path, no purged identifier added | none — the merge handles it |
| `MIXED` | touches files we keep **and** adds purged identifiers | **hand-audit every one** (§3.3) |
| `PURE_PURGED` | touches only purged paths | `DU` conflicts → `git rm` |
| `EMPTY` | no file changes | none |

Its patterns are deliberately kept in sync with `scripts/purge-guard.sh`. **Change one, change
both** — a classifier that lags the guard reports work as safe that the guard will later reject.

**Validation.** Replaying the previous sync's range reproduces its total exactly and identifies
the commits that actually caused damage:

```
$ ./scripts/classify-upstream.sh f3f48aa3d fd1ff012c
  PURE_KEPT    137     MIXED  12     PURE_PURGED  6     EMPTY  4     TOTAL 159
```

Its `MIXED` list contains both build-breakers that §3.4 previously caught only *after* the
compiler failed — `25463c35c` (todo traceability + telemetry call) and `659b8cc15` (Grok Build
login + `record_auth_surface_blocked`) — plus the four subscription-onboarding commits §1
excludes by choice.

> An earlier hand-rolled pass recorded this range as 136/9/10/4. That number is **not
> reproducible** and is superseded by the script. The script buckets a commit touching both
> purged and kept paths as `MIXED`/`PURE_KEPT` rather than `PURE_PURGED`, which is why it
> reports more `MIXED` and fewer `PURE_PURGED`. Erring toward `MIXED` is the safe direction.

### 3.2b Inventory what upstream added

§3.2, `purge-guard.sh`, and the compiler are all **defensive** — they detect purged code that
should not be present. None of them can detect the opposite failure: **upstream work that should
be present and is not.** A feature dropped during conflict resolution leaves no trace. No
conflict marker, no compile error, no guard hit. It simply never arrives, and nobody looks for
it because nobody read what upstream shipped.

That is how a sync quietly turns into a downgrade.

```bash
cd "$WS/fork"
./scripts/upstream-features.sh inventory "$BASE" upstream/master
```

The authoritative source is upstream's own `changelog/vX.Y.Z.json` files — human-written
`highlights` / `improvements` / `fixes` per release. Commit subjects are a weak substitute:
in the v0.75.3→v0.76.0 range only 5 of 77 commits are tagged `feat`, and 21 use no conventional
prefix at all.

Each line is marked:

| Mark | Meaning |
|---|---|
| `KEEP` | must be present after the merge — verify it |
| `PURGED` | §1 deletes this subsystem; its absence is correct |
| `EXCLUDED` | §1 "excluded by choice" (e.g. the subscription onboarding pill) |

The script also lists the **structural surface** upstream grew — new workspace members, CLI
subcommands, registered tools, config fields, scripts — because those are mechanically
checkable later, unlike prose.

> **Cross-reference the `KEEP` list against §3.2's `MIXED` shas.** A `KEEP` feature whose commit
> landed in `MIXED` is the highest-risk item in the whole sync: that commit needed hand-surgery,
> so it is exactly where a feature gets stripped along with the purged code sharing its hunk.

After the transfer (§3.5), assert the structural items actually landed:

```bash
cd "$MAIN"
./scripts/upstream-features.sh verify "$BASE" upstream/master   # exit 0 = nothing dropped
```

Replaying the *previous* sync through this reports `all KEEP items present` — its one KEEP
structural item (`jcode-provider-grok-build-runtime`) is on disk, and the gateway / telemetry /
permissions crates, the `Pair` and `Permissions` subcommands, and the `GatewayConfig` /
`SponsorsConfig` fields are all correctly skipped rather than reported as losses.

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

Both are now covered by `scripts/purge-guard.sh` (`SummaryPill::(Subscription|Telemetry)` and
the `record_*` patterns), and §3.2's classifier flags their commits up front. Run all three
checks — they fail differently:

```bash
# 1. Purged code reintroduced anywhere in the tree.
./scripts/purge-guard.sh

# 2. Module declarations without files — catches a `mod foo;` kept while foo.rs was purged.
python3 scripts/check_module_files.py

# 3. Upstream work that went MISSING. This is the check that caught the --ours
#    damage, and no other gate detects it: the guard only finds code that should
#    not be there, never code that should be and isn't.
git diff "$BASE"...upstream/master --stat > /tmp/upstream-expected.txt
git diff "$BASE"..HEAD             --stat > /tmp/ours-actual.txt
diff /tmp/upstream-expected.txt /tmp/ours-actual.txt   # every delta must be a deliberate purge
```

Then hand-audit each sha from §3.2's `--list-mixed` output. A `MIXED` commit that produced no
conflict is the most dangerous case in this entire runbook: it merged clean and carries purged
code.

### 3.5 Commit in tmp, then transfer

```bash
git add -A

# Gate 4 runs HERE, not in §6. `--staged` scans the index, and after the commit
# below there is nothing staged — running it in §6 silently scans nothing and
# passes. This is the only point where the incoming upstream files are staged.
gitleaks git --staged --redact --no-banner -v

git commit --no-verify -m "Merge upstream <ver>, excluding purged subsystems"
#          ^^^^^^^^^^^ gitleaks pre-commit hook false-positives on upstream's
#          keyboard-shortcut table (`key: "Ctrl+Shift+Tab"`). REVIEW the findings
#          from the scan above first — you are importing hundreds of upstream
#          files — then bypass.

git log -1 --pretty=%p        # MUST print TWO hashes. One = not recorded as a
                              # merge; future syncs will re-conflict everything.
```

Transfer the **verified commit** — never replay resolutions by hand:
```bash
cd "$MAIN"
git fetch "$WS/fork" merge-upstream:verified
git merge --ff-only verified
```

### 3.6 Update this document

The merge is not finished until the runbook describes the state it produced. Skipping this is
what makes the *next* sync classify against a stale base.

```bash
cd "$MAIN"
git log -1 --pretty='%h' verified^2      # the upstream parent = new "last upstream merge"
grep -m1 '^version' Cargo.toml           # the new version string
```

1. **§0 — update the "Last upstream merge" row** to that sha and version.
2. **Appendix — add any new pitfall** this sync cost you, with the symptom that revealed it.
3. **§1 — record any newly purged or newly kept subsystem**, and update all three scripts
   together if the pattern set changed: `purge-guard.sh`, `classify-upstream.sh`,
   `upstream-features.sh` (both its prose *and* identifier patterns).
4. **Skim the `KEEP` inventory from §3.2b one last time.** Gate 10 only checks the
   mechanically verifiable items — new crates, subcommands, config fields. Prose highlights
   like "streams handle transient failures more reliably" cannot be asserted by a script; a
   human read is the only check they get.

Commit the doc update as part of the sync, before building — §4 requires a clean tree.

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

**Always kill the two long-running helpers** — they are separate processes holding the old
binary and no reload reaches them:
```bash
pkill -f 'jcode menubar'; pkill -f 'jcode setup-hotkey'
```

**If a `jcode serve` daemon is running** (`pgrep -f 'jcode( .*)? serve'`), adopt the new binary:
```bash
jcode server reload        # exec()s into the new binary on the same socket
python3 - <<'PY'
import json, os
p = os.path.expanduser('~/.jcode/servers.json')
d = json.load(open(p)) if os.path.exists(p) else {}
if not d:
    print("no registered servers — nothing to adopt (expected when no daemon runs)")
else:
    for name, s in d.items():
        print(f"{name}: {s.get('git_hash', '<absent>')}")
PY
# ^ every git_hash must change — that is the proof the daemon adopted the new binary
```

> `servers.json` is `{}` whenever no daemon is registered. The previous one-liner here indexed
> `list(d)[0]` unconditionally and died with `IndexError` in exactly that case, which is the
> normal state after a clean shutdown. The block above reports instead of crashing.

**Rollback:** repoint `current`/`stable` at the previous `versions/<hash>/` and reload. No
rebuild. (After a `--purge` there is no previous version, so rollback becomes
`git reset --hard pre-merge-<date>` + rebuild.)

**Keep at least one previous `versions/<hash>/`.** That directory *is* the rollback; the store
is append-only by design. Prune older ones only when more than two are present.

---

## 6. Verification gates

Gates are **not all run at the same point.** Two of them can only pass at one specific moment;
running them in this table's position instead makes them vacuous. The "When" column is binding.

| # | Gate | When | Command | Pass |
|---|---|---|---|---|
| 1 | Purge invariant | §3.4, tmp | `./scripts/purge-guard.sh` | exit 0 |
| 2 | Module decls resolve | §3.4, tmp | `python3 scripts/check_module_files.py` | exit 0 |
| 3 | Lockfile coherent | §3.4, tmp | `cargo metadata --locked --format-version 1 >/dev/null` | exit 0 |
| 4 | Secret scan of incoming | **§3.5, staged, pre-commit** | `gitleaks git --staged --redact --no-banner -v` | reviewed |
| 5 | Merge recorded | **§3.5, post-commit** | `git log -1 --pretty=%p` | **two** hashes |
| 6 | Build | §4, `$MAIN` | `cargo build --profile release-lto; echo $?` | 0 |
| 7 | Smoke, isolated | §4, `$MAIN` | `./target/release-lto/jcode --no-update --socket /tmp/verify.sock run 'hi'` | see below |
| 8 | Removed CLI absent | §5, post-install | `jcode --help \| grep -E '^\s+(pair\|permissions)\b'` | no match |
| 9 | Version reproducible | §5, post-install | `jcode --version` | matches HEAD, no `-dirty` |
| 10 | **No upstream work dropped** | §3.5, post-transfer | `./scripts/upstream-features.sh verify "$BASE" upstream/master` | exit 0 |

- **Gate 4 must precede the commit.** `--staged` scans the index; after `git commit` the index
  is empty and the scan passes having examined nothing.
- **Gate 5 must follow the commit** — it inspects `HEAD`'s parents.
- **Gates 8–9 must follow the symlink flip.** They invoke `jcode` from `PATH`, which resolves
  through `~/.local/bin/jcode` → `builds/current`. Before §5 they measure the *old* binary.
- **Gate 10 is the only gate that can fail on absence.** Gates 1–9 all check that something
  wrong is not there; gate 10 checks that something right *is*. Do not skip it because the
  build is green — a dropped feature compiles perfectly.

Gate 7 uses an **isolated socket** deliberately (`AGENTS.md`): it proves the *new* binary works
rather than an old daemon answering. Two distinct passes, depending on whether `~/.jcode/auth.json`
holds credentials:

| State | Expected output | Verdict |
|---|---|---|
| Credentials present | a real model reply, then a `[Tokens] upload: … download: …` line | **pass** (stronger — the full turn loop ran) |
| No credentials | stops at `No credentials configured for provider auto-detection` | **pass** (reached the provider layer) |

Anything else — a panic, a hang, a socket error, a missing-tool error — is a failure. Remove
`/tmp/verify.sock` afterwards so the next run does not adopt a stale socket.

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

13. **`gitleaks --staged` after committing scans nothing and passes.** The gate table used to
    list it as step 4 of a post-merge block, by which point §3.5 had already committed.
    *Symptom:* a "reviewed" secret gate on a sync that imported 133 upstream files without ever
    examining one. *Fix:* gate 4 now runs inside §3.5 while the index is populated.

14. **`git push` does not push tags.** The rollback tag is the entire recovery story for §3, and
    `git push origin master` leaves it local. *Symptom:* `git ls-remote --tags origin` empty
    while `git tag` listed `pre-merge-…`. *Fix:* §3.1 pushes the tag explicitly.

15. **GitHub orders commit history by author date, not push time.** After pushing a merge whose
    commits were authored days earlier, nothing appears at today's date and the contribution
    graph stays blank. *Symptom:* "the push does not appear on the repo commit history" when
    `git ls-remote` and the GitHub API both confirmed it had landed. Not a fault — verify with
    `gh api repos/<owner>/<repo>/commits?sha=master` before debugging a push.

16. **`servers.json` is `{}` when no daemon is registered**, which is the normal post-shutdown
    state — not an error. The old §5 proof one-liner indexed `list(d)[0]` and raised
    `IndexError` there. *Fix:* iterate and report (§5).

17. **A classifier that drifts from the guard is worse than none.** `classify-upstream.sh` and
    `purge-guard.sh` duplicate the pattern set by necessity (one walks commits, one walks the
    tree). If only one is updated, the classifier reports work as safe that the guard later
    rejects — after you have already resolved it. Change both together.

18. **Every gate was defensive; none checked that upstream work arrived.** A feature dropped in
    resolution produces no conflict, no compile error and no guard hit — a clean green sync that
    is silently a downgrade. *Symptom:* none, which is the point; it was found by reasoning about
    what the gates could not see, not by an incident. *Fix:* §3.2b + gate 10.

19. **A crate-name pattern does not match a type name.** `jcode-gateway-types` in the identifier
    list left `GatewayConfig` unmatched, so three purged config fields were reported as dropped
    upstream work. *Symptom:* `MISS config field bind_addr` on a sync that had correctly deleted
    it. *Fix:* bare stems (`gateway`, not `jcode-gateway-types`).

20. **BSD `sed` does not understand `\s`.** On macOS the extraction silently returns the entire
    diff line instead of the identifier, and every downstream check reports a false MISS.
    *Symptom:* `expected /^\s++    Pair {\s*[({,]/`. *Fix:* `[[:space:]]` in every `sed`
    expression. GNU-only regex shorthands are a recurring hazard in this repo's scripts —
    `grep -E` accepts `\s` here, `sed -E` does not.
