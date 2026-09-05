# Lifecycle Hooks

jcode can run external commands at well-defined lifecycle points so other
programs can observe or gate agent behavior without forking jcode. Hooks
complement the [spawn hook](SPAWN_HOOK.md) (which controls *where headed
sessions appear*); lifecycle hooks tell you *what is happening inside them*.

## Configuration

```toml
# ~/.jcode/config.toml
[hooks]
turn_start    = "~/bin/jcode-turn-start"      # observer
turn_end      = "~/bin/jcode-turn-notify"     # observer
session_start = ""                            # observer
session_end   = ""                            # observer
pre_tool      = "~/bin/jcode-tool-policy"     # gate
post_tool     = ""                            # observer
pre_tool_timeout_ms = 5000
```

Every hook key accepts either a single command string or an array of command
lines. All configured commands for an event run, in declaration order:

```toml
[hooks]
turn_end = ["~/bin/jcode-event-log", "~/bin/jcode-turn-notify"]
```

Env overrides (always win; empty value disables a config hook):
`JCODE_HOOK_TURN_START`, `JCODE_HOOK_TURN_END`, `JCODE_HOOK_SESSION_START`,
`JCODE_HOOK_SESSION_END`, `JCODE_HOOK_PRE_TOOL`, `JCODE_HOOK_POST_TOOL`,
`JCODE_HOOK_PRE_TOOL_TIMEOUT_MS`. A value starting with `[` is parsed as a
TOML array, so the multi-command form works from the environment too; if it
fails to parse it is treated as one literal command line.

## Common contract

- The hook command line is parsed shell-style (quotes and backslash escapes
  work) but executed **directly**, not through a shell. A leading `~/` in the
  program path is expanded.
- The hook runs in the session working directory when known.
- Every hook receives:

| Variable | Meaning |
| --- | --- |
| `JCODE_HOOK_EVENT` | `turn_start`, `turn_end`, `session_start`, `session_end`, `pre_tool`, `post_tool` |
| `JCODE_HOOK_SESSION_ID` | Session the event belongs to |
| `JCODE_HOOK_CWD` | Session working directory |
| `JCODE_HOOK_PAYLOAD` | JSON object mirroring all fields (capped at 16 KB) |
| `JCODE_HOOKS_DISABLED` | Always `1` in the hook's environment; any jcode invoked from a hook sees it and resolves *every* hook to "not configured" (recursion guard) |

## Observer hooks

`turn_start`, `turn_end`, `session_start`, `session_end`, and `post_tool` are
**observers**: spawned detached, fire-and-forget. They can never block or slow
the agent; failures are only logged.

### `turn_start`

Fires when an agent turn begins — after the user message is added and before
the model starts generating, so it lands before the first `pre_tool`. This is
the only way to observe the otherwise-invisible window between prompt
submission and the first tool call (the agent is thinking or streaming text).

Extra fields: `JCODE_HOOK_MODEL`, `JCODE_HOOK_SOURCE`. `SOURCE` is currently
always `chat`: the streaming turn path is the only dispatch site.

### `turn_end`

Fires when the same turn completes (streaming turn path: TUI, swarm workers,
headless sessions, and ambient cycles all run through it).

Extra fields: `JCODE_HOOK_STATUS` (`ok`/`error`), `JCODE_HOOK_DURATION_MS`,
`JCODE_HOOK_MODEL`, `JCODE_HOOK_LAST_ASSISTANT_TEXT` (first 4000 chars),
`JCODE_HOOK_ERROR` (on failure, first 1000 chars).

### `session_start` / `session_end`

`session_start` fires when an agent session becomes active, with
`JCODE_HOOK_SOURCE` = `create` (brand new), `attach` (existing session object
attached), or `resume` (restored by id). `session_end` fires on normal close
(`JCODE_HOOK_SOURCE=close`). Both also export `JCODE_HOOK_MODEL`.

A crashed session does **not** fire `session_end`; only `mark_closed` does.

### `post_tool`

Fires after every tool call. Extra fields: `JCODE_HOOK_TOOL_NAME`,
`JCODE_HOOK_STATUS`, `JCODE_HOOK_DURATION_MS`, `JCODE_HOOK_OUTPUT_BYTES` (on
success), `JCODE_HOOK_ERROR` (on failure, first 1000 chars).

## Gate hook: `pre_tool`

`pre_tool` runs **synchronously before every tool call** and can block it:

- The hook receives `JCODE_HOOK_TOOL_NAME` plus the full tool input JSON on
  **stdin** (and a 16 KB-truncated copy in `JCODE_HOOK_TOOL_INPUT`).
- **Exit 0**: allow the call.
- **Exit 2**: block the call. The hook's stderr (trimmed, capped at 2000
  chars) is returned to the model as the tool error, so the model can adapt.
- **Anything else fails open** with a logged warning: other exit codes, a
  timeout (`pre_tool_timeout_ms`, default 5000 ms), an unparseable command
  line, a missing binary or other spawn error, and any wait error. The tool
  call proceeds in every one of those cases.
- With several commands configured, each runs in order and *all* of them run;
  the call is blocked if any of them exits 2, and the first such block's
  stderr becomes the model-visible reason.

Fail-open is deliberate: a broken policy script should degrade to "no policy"
rather than brick every session. Note what this means for the timeout in
particular — a hook that hangs does **not** stop the tool, it just delays it
by `pre_tool_timeout_ms`. If you need fail-closed semantics, make the hook
itself robust (it is your trust boundary, not jcode).

### Example policy script

```bash
#!/usr/bin/env bash
# ~/bin/jcode-tool-policy
# stdin: tool input JSON. Env: JCODE_HOOK_TOOL_NAME, JCODE_HOOK_SESSION_ID...
input=$(cat)

case "$JCODE_HOOK_TOOL_NAME" in
  bash)
    if grep -qE 'rm -rf /([^a-zA-Z]|$)|mkfs|dd if=' <<<"$input"; then
      echo "blocked: destructive shell command" >&2
      exit 2
    fi
    ;;
  write|edit)
    if grep -q '"file_path":"/etc/' <<<"$input"; then
      echo "blocked: writes to /etc are not allowed" >&2
      exit 2
    fi
    ;;
esac
exit 0
```

## Example: tmux status + desktop notification on turn end

```bash
#!/usr/bin/env bash
# ~/bin/jcode-turn-notify
if [ "$JCODE_HOOK_STATUS" = ok ]; then icon=✅; else icon=❌; fi
tmux display-message "jcode $icon ${JCODE_HOOK_SESSION_ID:0:12}" 2>/dev/null
notify-send "jcode turn $JCODE_HOOK_STATUS" \
  "${JCODE_HOOK_LAST_ASSISTANT_TEXT:0:120}" 2>/dev/null
exit 0
```

## Example: JSON event log of all hook activity

Point several hooks at one script and fan out on `JCODE_HOOK_EVENT`:

```bash
#!/usr/bin/env bash
# ~/bin/jcode-event-log
echo "$JCODE_HOOK_PAYLOAD" >> ~/.local/state/jcode-events.jsonl
```

```toml
[hooks]
turn_start    = "~/bin/jcode-event-log"
turn_end      = "~/bin/jcode-event-log"
session_start = "~/bin/jcode-event-log"
session_end   = "~/bin/jcode-event-log"
post_tool     = "~/bin/jcode-event-log"
```

## Design notes

- Hook lookups are config-driven and re-read on config reload; you can add or
  change hooks without restarting jcode.
- Hot paths (`turn_start`/`turn_end`/`pre_tool`/`post_tool`) check whether a
  hook is configured before building any payload, so unconfigured hooks cost
  ~nothing.
- The recursion guard (`JCODE_HOOKS_DISABLED=1`) means a hook may safely call
  `jcode` CLI commands without re-triggering hooks in that nested process.
- `JCODE_HOOK_PAYLOAD` is truncated at a UTF-8 boundary, so an over-long
  payload yields invalid JSON rather than a panic. Treat a parse failure as
  "payload was too big" and read the individual env vars instead.
- Under the shared server, hooks inherit the *requesting client's* terminal
  identity rather than the daemon's environment, so multiplexer integrations
  address the right pane.
