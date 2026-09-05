# Soft Interrupt: Seamless Message Injection

> **Status:** Shipped, except the `urgent` user-facing path (see
> [Urgent mode](#urgent-mode-not-wired-into-the-tui)).

## Overview

Soft interrupt lets a message be injected into an ongoing AI conversation
without cancelling the current generation. Instead of the disruptive
cancel-and-restart flow, messages are queued and incorporated at safe points
where the model provider connection is idle.

## Hard interrupt (the old flow, still available as explicit cancel)

```
User cancels during AI processing
         │
         ▼
    remote.cancel()  ← Cancels current generation
         │
         ▼
    Wait for Done event
         │
         ▼
    Send user message as new request
         │
         ▼
    AI restarts fresh
```

**Problems:**
- Loses any partial work the AI was doing
- Delay while cancellation completes
- Full context re-send on new API call
- Jarring user experience

## Soft interrupt (the default for typing during a turn)

```
User types message during AI processing
         │
         ▼
    Message stored in the session's soft interrupt queue
         │
         ▼
    AI continues processing...
         │
         ▼
    Safe injection point reached
         │
         ▼
    Message appended to conversation history
         │
         ▼
    AI naturally sees it on next loop iteration
```

**Benefits:**
- No cancellation, no lost work
- No delay
- AI naturally incorporates user input
- Smooth user experience

## Safe Injection Points

The key constraint is: **we can only inject when not actively streaming from
the model provider**. The agent loop (`run_turn_streaming_mpsc` in
`crates/jcode-app-core/src/agent/turn_streaming_mpsc.rs`) has three such pause
points, named B, C and D in the code and in the wire protocol.

There is no point "A". Injecting right after the assistant message that
carries `tool_use` blocks would strand them without `tool_result`s, so that
position was never made an injection point.

```rust
loop {
    // 1. Build messages and call provider.stream()
    // === PROVIDER OWNS THE CONNECTION HERE ===
    // Stream events: TextDelta, ToolStart, ToolInput, ToolUseEnd

    // 2. Stream ends

    // 3. Add assistant message to history
    // (MUST happen before injection to preserve cache and conversation order)

    // 4. Check if tool calls exist
    if tool_calls.is_empty() {
        // ═══════════════════════════════════════════════
        // ✅ INJECTION POINT B: No tools, turn complete
        // ═══════════════════════════════════════════════
        break;
    }

    // 5. Execute tools and add tool_results
    for tool_index in 0..tool_count {
        // ═══════════════════════════════════════════════
        // ✅ INJECTION POINT C: Before tool N (N > 0),
        //    urgent aborts only — skipped tool_results
        //    are written first
        // ═══════════════════════════════════════════════

        // Execute single tool, add result to history...
    }

    // ═══════════════════════════════════════════════
    // ✅ INJECTION POINT D: All tools done, before next API call
    // ═══════════════════════════════════════════════

    // Loop continues → next provider.stream() call
}
```

### Critical API Constraint

**The Anthropic API requires that every `tool_use` block must be immediately
followed by its corresponding `tool_result` block.** No user text messages can
be injected between a `tool_use` and its `tool_result`.

This means we CANNOT inject messages:
- After the assistant message with tool_use blocks
- Before all tool_results have been added

### Injection Point Details

| Point | Location | Timing | Use Case |
|-------|----------|--------|----------|
| **B** | Turn complete | No tools requested | Safe: no tool_use blocks to pair |
| **C** | Inside tool loop, before tool N (N > 0) | Urgent abort only | Writes stub tool_results for every remaining tool first |
| **D** | After all tools | Before next API call | **Default**: safest point for injection |

**Important**: We do NOT inject between tools for non-urgent interrupts. Doing
so would place user text between tool_results, which could violate API
constraints. All non-urgent injection is deferred to Point D.

### Point B: Turn Complete (No Tools)

```
Timeline:
  Provider: TextDelta... [stream ends, no tool calls]
  Agent: ──► INJECT HERE ◄──
  Agent: Would exit loop, but instead continues with user message

AI sees: "I finished my response, user has follow-up"
```

Point B is reached only after the loop has decided the turn is genuinely
over — the "incomplete response" and "stranded tool_use" continuations are
checked first, and either of those wins over injection for that iteration.

**Best for:** Quick follow-ups when AI is just responding with text.

### Point C: Between Tools (urgent only)

```
Timeline:
  Agent: Execute tool 1 → result 1
  Agent: ──► urgent interrupt seen before tool 2 ◄──
  Agent: Write "[Skipped: user interrupted]" results for tools 2..N
  Agent: Inject user message
  Agent: Append "[User interrupted: N remaining tool(s) skipped]"
  Agent: Next API call

AI sees: tool 1 result, skipped-tool results, the user message, and a note
saying how many tools were dropped
```

The guard is `tool_index > 0`: the tool already in flight is never aborted,
so a single-tool turn can never be cut short here. The event carries
`tools_skipped`, which the TUI surfaces as `⚡ N tool(s) skipped`.

**Best for:**
- Urgent abort: "wait, don't do the other tools"

### Point D: After All Tools

```
Timeline:
  Agent: Execute all tools → all results collected
  Agent: ──► INJECT HERE ◄──
  Agent: Next API call includes: [all tool results] + [user message]

AI sees: "All my tools completed, and user added context"
```

There is a second D site for providers that run tools internally: when every
returned tool call is filtered out as provider-handled, the loop injects at D
and continues rather than ending the turn.

**Best for:** Default behavior. Cleanest, most predictable.

## Implementation

### The queue

`crates/jcode-agent-runtime/src/lib.rs` owns the shared types:

```rust
pub struct SoftInterruptMessage {
    pub content: String,
    pub images: Vec<(String, String)>,
    /// If true, can skip remaining tools when injected at point C.
    pub urgent: bool,
    pub source: SoftInterruptSource,
}

pub enum SoftInterruptSource {
    User,
    System,
    BackgroundTask,
}

pub type SoftInterruptQueue = Arc<std::sync::Mutex<Vec<SoftInterruptMessage>>>;
```

The queue is an `Arc<Mutex<..>>` rather than an agent field precisely so the
server can push into it *while* the agent lock is held by the running turn.
The server also keeps a per-session queue registry, so a message can be queued
for a session whose agent is currently busy or not yet resumed.

Queued-but-uninjected messages survive a restart: they are written to the
soft-interrupt store on session close/crash and restored on resume
(`persist_soft_interrupt_snapshot` / `restore_persisted_soft_interrupts` in
`crates/jcode-app-core/src/agent/interrupts.rs`, backed by
`crates/jcode-base/src/soft_interrupt_store.rs`).

### Injection

`Agent::inject_soft_interrupts` (same file) drains the queue and appends the
messages to the conversation. It does not produce one blob: consecutive
messages are grouped by `source`, each group joined with `\n\n`, and each
group added as its own user message with the display role that matches its
source (`System` → system, `BackgroundTask` → background task, `User` →
normal). Images ride along as `ContentBlock::Image` blocks ahead of the text.

The two entry points the loop calls are `handle_streaming_no_tool_calls`
(point B) and `take_post_tool_soft_interrupt` (point D); point C is inlined in
the tool loop because it must write the skipped `tool_result`s first.

### Protocol

`crates/jcode-protocol/src/wire.rs`:

```rust
#[serde(rename = "soft_interrupt")]
SoftInterrupt {
    id: u64,
    content: String,
    images: Vec<(String, String)>,
    /// If true, can abort remaining tools at point C
    urgent: bool,
},

#[serde(rename = "cancel_soft_interrupts")]
CancelSoftInterrupts { id: u64 },
```

```rust
#[serde(rename = "soft_interrupt_injected")]
SoftInterruptInjected {
    content: String,
    display_role: Option<String>,
    point: String,  // "B", "C", or "D"
    tools_skipped: Option<usize>,
}
```

`CancelSoftInterrupts` exists because a message can sit in the queue for a
long time; the TUI uses it to retract everything it has queued but not yet
seen injected.

### TUI

`RemoteBackend::soft_interrupt(content, images, urgent)` in
`crates/jcode-tui/src/tui/backend.rs` sends the request; the app tracks the
request id until the matching `Ack` arrives, so a dropped message can be
recovered into the composer. On send the status line shows
`⏭ Interleave sent`.

## User Experience

Which key does what during processing is governed by `display.queue_mode`
(default `false`), toggled at runtime with `Ctrl+T` / `Ctrl+Tab`:

| `queue_mode` | Enter | Ctrl+Enter / Cmd+Enter |
|---|---|---|
| `false` (default) | interleave — soft interrupt now | queue until the turn ends |
| `true` | queue until the turn ends | interleave — soft interrupt now |

```
User presses Enter during processing (queue_mode = false):
  → Message sent as a soft interrupt and tracked as pending
  → Status shows: "⏭ Interleave sent"
  → AI continues working...
  → Injected at the next safe point (usually D)
  → soft_interrupt_injected event echoes the content back into the transcript
```

### Urgent mode (not wired into the TUI)

The `urgent` flag is plumbed end to end — TUI backend, protocol, server, and
the point-C abort — but **no TUI keybinding sets it**. Every TUI call site
passes `urgent = false`. There is no Shift+Enter urgent submit, and no
"⚡ Will inject ASAP" status.

Today the only ways to reach point C are the debug command
`queue_interrupt_urgent:<content>` on the debug socket, and internal callers
that queue with `urgent = true` (e.g. system-sourced interrupts). If the
urgent UX is wanted, the missing piece is a keybinding that calls
`soft_interrupt(.., urgent = true)`.

## Comparison

| Aspect | Hard interrupt (explicit cancel) | Soft interrupt (default) |
|--------|-------------------------|---------------------|
| Cancels generation | Yes | No |
| Loses partial work | Yes | No |
| Delay | Yes (wait for cancel) | No |
| API calls | Wastes partial call | Efficient |
| User experience | Jarring | Smooth |
| Complexity | Simple | Moderate |

## Edge Cases

1. **Multiple soft interrupts**: combined per source group with `\n\n`
2. **Soft interrupt during text-only response**: injected at Point B, loop continues
3. **Provider handles tools internally**: injected at the provider-handled Point D site
4. **Urgent interrupt with no tools**: nothing to skip; it injects like a normal one
5. **Urgent interrupt on the first tool**: not honoured — point C requires `tool_index > 0`
6. **Graceful shutdown mid-turn**: remaining tools get `[Skipped - server reloading]` results and the turn ends without injection
7. **Session ends with messages still queued**: snapshotted to disk and restored on resume

## Testing

1. Send message while AI is streaming text (no tools) → should inject at Point B
2. Send message while AI is executing tools → should inject at Point D (after all tools)
3. Send `queue_interrupt_urgent:` over the debug socket with multiple tools queued → should skip remaining tools at Point C
4. Send multiple messages rapidly → should combine into one injection per source
5. Verify no API errors about tool_use/tool_result pairing
