# Harness as an API

Status: draft, approved direction (2026-07-24)

## Motivation

- The agent runtime ("harness") is already client/server: NDJSON over Unix
  socket (`~/.jcode/jcode.sock`), with `Request` / `ServerEvent` in
  `crates/jcode-protocol`. But it is an *internal* wire format: unversioned,
  ~147 variants, TUI-shaped, and coupled to client rendering assumptions.
- Third-party and non-TUI clients need a boundary that does not move whenever
  the TUI's rendering assumptions change.

## The Harness API

Goal: one stable, versioned boundary. Every UI is a client. No UI-specific
logic in the runtime.

### Approach

Introduce `crates/jcode-harness-api`:

- **Versioned envelope.** Every frame carries `v` (protocol major) at the top
  level. Handshake: client sends `hello { min_version, max_version, client }`,
  server replies `hello_ok { version, server, capabilities }`. Unknown fields
  are ignored; unknown event types are skippable (tagged enums with a
  catch-all `Unknown` on the client side).
- **Curated surface, not a dump.** Start with a small stable core and grow:
  - Session lifecycle: create/attach/detach/list sessions, working dir.
  - Conversation: send message (text + images), cancel, soft interrupt,
    clear, rewind, history fetch.
  - Streaming events: text/reasoning deltas, tool start/input/exec/done,
    token usage, turn done, errors.
  - Permissions: permission request event + client response.
  - State: agent status snapshot, todos, plan/task-graph summaries.
  Everything else (swarm internals, debug) stays on the internal
  protocol until promoted deliberately.
- **Transport.** NDJSON over Unix socket stays the primary transport.
  The API crate defines transport-agnostic types + a small client
  (`HarnessClient`) and server adapter, so a WebSocket/TCP transport can be
  added later without touching the schema.
- **Relationship to `jcode-protocol`.** Short term the server adapter maps
  API requests onto existing internal handling. Long term the internal
  protocol shrinks toward the API. Do not fork semantics: the API is a
  facade, the runtime remains the source of truth.

### Deliverables

1. `crates/jcode-harness-api`: types, version constants, handshake,
   `HarnessClient` (blocking + async-friendly framing).
2. Server: accept API handshake on the existing socket (sniff first line:
   `hello` = API client, else legacy).
3. Reference client example (`examples/harness_repl.rs`): connect, create
   session, send a message, print streamed events. This is the acceptance
   test for the API.
4. Schema snapshot test so accidental breaking changes fail CI.

> The original version of this document also specified a pure-Rust desktop
> rewrite (`jcode-desktop2`, winit + wgpu + Vello + Parley). That app has been
> removed from this fork; the harness API remains, and the TUI is the only
> first-party client. See `docs/FORK_WORKFLOW.md` §1.
