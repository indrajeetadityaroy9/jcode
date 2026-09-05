# System Prompt Assembly

How jcode turns a 33-line markdown file into the bytes a provider actually receives.

This is the architecture doc: what the layers are, what order they compose in, which
half is cacheable, and what each provider prepends on top. For the user-facing "how do
I change it" guide, see [`SYSTEM_PROMPT_CONFIG.md`](./SYSTEM_PROMPT_CONFIG.md).

Everything below was verified against code at `ea836fb4f`. Line cites are load-bearing:
if one drifts, trust the code. An unqualified `prompt.rs:NNN` means
`crates/jcode-base/src/prompt.rs`; every other file is cited by full path.

## 1. The built-in prompt is small

`crates/jcode-base/src/prompt/system_prompt.md` — 33 lines, ~1.5 KB, compiled in with
`include_str!` as `DEFAULT_SYSTEM_PROMPT` (`prompt.rs:7`). Four
sections:

| Section | What it establishes |
|---|---|
| **Identity** | "Your name is Jcode. You are a maximally proactive coding agent and assistant." |
| **Autonomy and persistence** | Persist to completion; fix over surface; infer intent; treat asking the user as a blocking action to use sparingly; hesitate on destructive/irreversible actions (payment, dropping a database, sending mail); never reset a password. |
| **Coding** | Commit as you go by default, even in a dirty repo; other jcode agents may be in the same checkout and "the harness handles this natively without git worktrees"; no interactive commands; keep iterating inside a closed feedback loop. |
| **User interaction** | Concise by default (under 5 lines); no em dashes and no semicolons substituting for them; markdown + LaTeX render; use the `todo` tool extensively; `open` to show the user something. |

That is the whole thing. Every other behavior the model exhibits comes from the layers
below, from tool descriptions, or from the model itself.

Because it is embedded with `include_str!`, editing this file requires a rebuild.

## 2. Layer composition

`build_system_prompt_split_with_capabilities` (`prompt.rs:493`) is the single assembly
point. It returns a `SplitSystemPrompt { static_part, dynamic_part }` plus a
`ContextInfo` accounting struct. Parts are joined with `\n\n` within each half.

### Static half — stable across turns, so it can be cached

| # | Layer | Source | Framing |
|---|---|---|---|
| 1 | Base prompt | `./.jcode/system-prompt.md` → `~/.jcode/system-prompt.md` → built-in (`prompt.rs:15`) | raw |
| 2 | Capability modules | `MERMAID_PROMPT` (`prompt.rs:35`), only if `features.mermaid` | `# Mermaid` |
| 3 | Project instructions | `./AGENTS.md` | `# Project Instructions (AGENTS.md)` |
| 4 | Global instructions | `~/AGENTS.md` | `# Global Instructions (~/AGENTS.md)` |
| 5 | Project overlay | `./.jcode/prompt-overlay.md` | `# Project Prompt Overlay (...)` |
| 6 | Global overlay | `~/.jcode/prompt-overlay.md` | `# Global Prompt Overlay (...)` |
| 7 | Project tool guidance | `./.jcode/preferred-tools.md` | `# Project Preferred Tools (...)` |
| 8 | Global tool guidance | `~/.jcode/preferred-tools.md` | `# Global Preferred Tools (...)` |
| 9 | Skills index | live `SkillRegistry` | `# Available Skills` |

Notes that matter in practice:

- **Project beats global by position, not by exclusion.** Both files load, project first
  (`prompt.rs:787-816` for AGENTS.md; `prompt.rs:849-866` for overlays; `prompt.rs:892-909` for preferred
  tools). Later text does not replace earlier text; the model just reads both.
- **`~/AGENTS.md` is skipped when it *is* the project file.** The loader canonicalizes
  both paths and drops the global copy on a match (`prompt.rs:795-805`), so working in
  `$HOME` or through a symlinked alias does not duplicate the instructions.
- **Capability modules are gated by config**, not by build features: `PromptCapabilities::current()`
  reads `config().features.mermaid` (`prompt.rs:50-54`).
- **The skills index is a one-liner per skill**: `- /name - description`, with the
  description flattened to a single line and clipped to `SKILL_DESC_MAX_CHARS`
  (`clip_skill_description`, `prompt.rs:233`). It ends with an instruction to mention
  these skills when the user asks about capabilities (`prompt.rs:260`).

### Dynamic half — changes per turn, never cached

| # | Layer | Source |
|---|---|---|
| 10 | Recalled memories | memory prompt passed in by the caller |
| 11 | Active skill body | `# Active Skill\n\n<prompt>` (`prompt.rs:551`) |
| 12 | Per-turn reminder | `append_current_turn_system_reminder` (`crates/jcode-app-core/src/agent/prompting.rs:58`) |
| 13 | Swarm effort directive | `append_swarm_effort_directive` (`prompt.rs:179`) |

Layer 13 fires only when the provider's reasoning effort is `swarm` or `swarm-deep`
(`prompt.rs:101-129`). `SWARM_EFFORT_DIRECTIVE` (`prompt.rs:114`) tells the model to
decompose and spawn parallel agents; `SWARM_DEEP_EFFORT_DIRECTIVE` (`prompt.rs:119`)
replaces that with the full task-DAG workflow (seed a graph, expand oversized nodes,
expect a plan-wide adversarial root gate).

## 3. Two ways to bypass the layers

**Replace layer 1 only.** `load_base_system_prompt` (`prompt.rs:15`) returns the first
non-empty of `./.jcode/system-prompt.md`, `~/.jcode/system-prompt.md`, then the built-in.
Whitespace-only files fall through, so you cannot accidentally ship an empty prompt.
Layers 2-13 still apply. The function's own doc comment recommends `prompt-overlay.md`
for additions instead.

**Replace everything.** `Agent::set_system_prompt` (`crates/jcode-app-core/src/agent/turn_execution.rs:307`) sets
`system_prompt_override`, and `build_system_prompt_split` short-circuits on it before
reading any file (`crates/jcode-app-core/src/agent/prompting.rs:81-86`), returning the override as `static_part`
with an empty `dynamic_part`. Ambient mode is the in-tree user: it builds its own prompt
in `crates/jcode-app-core/src/ambient/prompt.rs` and installs it this way.

## 4. Session context is a message, not a system block

`build_session_context` (`prompt.rs:568`) emits a `# Session Context` block with UTC date
and time, OS, architecture, jcode version + git hash, a hardware summary, the working
directory, and git branch/status.

It does **not** go into either half of the split. `Session` wraps it in
`<system-reminder>` tags and appends it as a **user-role message** displayed as system
(`crates/jcode-base/src/session.rs:900-910`). Two consequences:

- It lives in the transcript, so it survives resume and shows up in replay.
- It sits *before* the cached system prefix in the request, so the volatile parts (time,
  git status) never invalidate the prompt cache.

`refresh_initial_session_context_message` (`crates/jcode-base/src/session.rs:917`) rewrites that message if the
session has not started a real conversation yet — this is how a remote client's actual
terminal working directory replaces the daemon's cwd without leaking the directory that
launched the server.

## 5. How the split reaches the wire

The `Provider` trait carries both shapes (`crates/jcode-provider-core/src/lib.rs:75-98`):

- `complete(messages, tools, system, resume_session_id)` — one flat system string.
- `complete_split(..., system_static, system_dynamic, ...)` — the cache-aware form.

`complete_split` has a **default implementation** that degrades gracefully: it calls
`messages_with_dynamic_system_context` (`crates/jcode-message-types/src/lib.rs:507`) to
wrap the dynamic half in `<system-reminder>` tags as a user message, inserts it right
after the most recent fresh user prompt, and passes only the static half as `system`.
So a provider that never overrides `complete_split` still gets a stable cacheable prefix.

Only Anthropic overrides it (`crates/jcode-provider-anthropic-runtime/src/lib.rs:1552`); the
base `MultiProvider`/`jcode` router forward it (`crates/jcode-base/src/provider/mod.rs:1696`,
`crates/jcode-base/src/provider/jcode.rs:97`, dispatched through `crates/jcode-base/src/provider/dispatch.rs:168`).

## 6. What Anthropic prepends

`build_system_param` (`crates/jcode-provider-anthropic/src/lib.rs`) emits a block array, and the
OAuth path is not just your prompt.

**OAuth / subscription** (`crates/jcode-provider-anthropic/src/lib.rs:677-702`), in order:

1. `x-anthropic-billing-header: cc_version=2.1.123; cc_entrypoint=sdk-cli; cch=33f85;`
   (`OAUTH_BILLING_HEADER`, `crates/jcode-provider-anthropic/src/lib.rs:10`)
2. `"You are a Claude agent, built on Anthropic's Claude Agent SDK."`
   (`CLAUDE_CODE_IDENTITY`, `crates/jcode-provider-anthropic/src/lib.rs:12`)
3. static half — `cache_control: ephemeral`, 1h TTL when enabled
4. dynamic half — no cache control

**API key** (`crates/jcode-provider-anthropic/src/lib.rs:705-728`): static (cached) then dynamic (uncached). No billing
header, no identity block.

Caching budget is deliberate and tight: system (1) + tools (1) + up to 2 message
breakpoints = 4, which is Anthropic's limit. The message markers use a sliding
two-marker window so turn N+1 reads turn N's snapshot (`add_message_cache_breakpoint`,
`crates/jcode-provider-anthropic/src/lib.rs:753`).

### The sidecar uses a different identity

The memory sidecar (`crates/jcode-base/src/sidecar.rs:44-45`) prepends *two* blocks:

```
You are Claude Code, Anthropic's official CLI for Claude.
You are jcode, powered by Claude Code. You are a third-party CLI, not the official Claude Code CLI.
```

This pair is valid **only** on the OAuth path. `build_claude_api_key_system_param` omits
it, and `test_build_claude_api_key_system_param_omits_identity_spoof` (`crates/jcode-base/src/sidecar.rs:1401`)
pins that distinction so the API-key path never grows the spoof by accident.

## 7. Sibling prompts

| Prompt | File | Role |
|---|---|---|
| Swarm routing | `prompt/swarm_prompt.md` (29 lines) | Model-routing policy expressed as a prompt, not config, because swarms are dynamic. Overridable at `~/.jcode/swarm-prompt.md` or `./.jcode/swarm-prompt.md` (`prompt.rs:77`). Default worker `claude-api:claude-fable-5`; implementation → `gpt-5.5` at `effort: low`; bulk reading → `gpt-5.5` at `effort: none`; only a root session may spawn, recursion reserved for `swarm-deep`. |
| Mission continuation | `prompt/mission_continuation.md` (58 lines) | `MISSION_CONTINUATION_TEMPLATE` (`prompt.rs:193`), consumed by `crates/jcode-app-core/src/mission.rs:154`. |
| Ambient cycle | `crates/jcode-app-core/src/ambient/prompt.rs` | Built independently and installed via `set_system_prompt`; see [`AMBIENT_MODE.md`](./AMBIENT_MODE.md). |

## 8. Accounting and inspection

`ContextInfo` (`prompt.rs:266`) records per-layer char counts as the prompt is built:
`system_prompt_chars`, `session_context_chars`, project/global AGENTS.md, `skills_chars`,
`memory_chars`, `prompt_overlay_chars`, `preferred_tools_chars`, plus the conversation-side
tallies (tool definitions, user/assistant messages, tool calls and results).

Token figures are a `chars / 4` approximation (`estimated_tokens`, `prompt_prefix_tokens`,
`tool_definition_tokens` — `prompt.rs:318-340`).

`breakdown()` (`prompt.rs:343`) returns the labelled rows the UI shows, emitting a row
only when that layer is non-empty:

```
sys ⚙ · session 🌍 · agents 📋 · ~agents 📋 · skills 🔧 · mem 🧠 · overlay 🧩 · tools 🧰
```

Run `/context` in the TUI (`crates/jcode-tui/src/tui/app/state_ui.rs:1933`) for the live
snapshot: this breakdown alongside cwd, terminal size, session id, todos, and the active
provider/model/effort/tier/transport and token total.

## 9. Reload semantics

- File-based layers are read **per prompt build**, so a new session picks up edits with
  no restart.
- A running session keeps the prompt captured at start; that is what keeps the tool
  definitions and the cached prefix stable mid-conversation.
- The built-in `system_prompt.md` needs a rebuild (`include_str!`).
- `swarm-prompt.md` is read when each new agent is created, so already-running agents keep
  the copy they captured.
