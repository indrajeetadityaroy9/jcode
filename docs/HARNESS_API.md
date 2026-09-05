# Harness as an API

Status: built. `crates/jcode-harness-api` (schema + client),
`crates/jcode-harness-api-server` (bridge), `jcode api-bridge`, and the Rust SDK
in `crates/jcode-sdk` all exist and ship in the released binary. The bridge is a
separate process on its own socket, not a handshake sniffed on the daemon
socket — see "Transport" below, which supersedes the original plan.

## Motivation

- The agent runtime ("harness") is already client/server: NDJSON over a Unix
  socket in the runtime directory (`$TMPDIR/jcode.sock` on this macOS-only
  fork), with `Request` / `ServerEvent` in `crates/jcode-protocol`. But it is an
  *internal* wire format: unversioned, 148 variants (77 `Request`,
  71 `ServerEvent`), TUI-shaped, and coupled to client rendering assumptions.
- Third-party and non-TUI clients need a boundary that does not move whenever
  the TUI's rendering assumptions change.

## The Harness API

Goal: one stable, versioned boundary. Every UI is a client. No UI-specific
logic in the runtime.

### Approach

`crates/jcode-harness-api`:

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
  - State: session status snapshot, model info/catalog, runtime info,
    connection phase.
  - Session admin: archive/restore, retention policy, rename, compact.
  - Workspace reads: read file, find files, search text, file status.
  Everything else (swarm internals, debug) stays on the internal
  protocol until promoted deliberately.
- **Transport.** NDJSON over a Unix socket. The API crate defines
  transport-agnostic types plus a small client (`HarnessClient`), so a
  WebSocket/TCP transport can be added later without touching the schema.
  The API does **not** share the daemon socket: it is served by a separate
  bridge process listening on its own path (see below).
- **Relationship to `jcode-protocol`.** Short term the server adapter maps
  API requests onto existing internal handling. Long term the internal
  protocol shrinks toward the API. Do not fork semantics: the API is a
  facade, the runtime remains the source of truth.

### What shipped

1. `crates/jcode-harness-api`: `ApiRequest` (32 variants) and `ApiEvent`
   (32 variants), each with an `Unknown` catch-all so a client can skip
   anything it does not recognize; `API_VERSION_MAJOR = 1` /
   `API_VERSION_MINOR = 0`; the `hello` / `hello_ok` handshake; and
   `HarnessClient` over any `BufRead` + `Write` pair (Unix socket, TCP, or an
   in-memory pipe for tests).
2. `crates/jcode-harness-api-server`: `run_bridge(api_socket, legacy_socket)`,
   which accepts API clients, performs the handshake, and translates each
   connection onto its own dial of the internal daemon socket. Advertised
   capabilities today: `sessions`, `streaming`, `persisted_session_discovery`,
   `runtime_info`, `api_key_provisioning`, `session_archive`,
   `session_retention`, `session_files`.
3. Reference client example: `crates/jcode-harness-api/examples/harness_repl.rs`.
4. Schema snapshot tests in
   `crates/jcode-harness-api/src/harness_api_tests/schema_snapshot.rs`, so an
   accidental wire-shape change fails `cargo test`. (This fork has no CI; the
   snapshot is only as good as the local test run.)
5. Rust SDK: `crates/jcode-sdk`. The upstream TypeScript SDK was removed from
   this fork, but the socket protocol is unchanged.

### Transport: a separate bridge process

The original plan was to sniff the first line on the existing daemon socket and
treat `hello` as an API client. That is not what exists. The API runs as its own
process on its own socket:

- **API socket**: `JCODE_API_SOCKET` if set, else
  `<runtime dir>/jcode-api.sock` (`crates/jcode-harness-api/src/sockets.rs:65-71`).
- **Daemon socket the bridge translates onto**: `JCODE_SOCKET` if set, else
  `<runtime dir>/jcode.sock` (`sockets.rs:73-80`).
- The runtime directory is resolved by the same rules as
  `jcode-storage::runtime_dir` (`JCODE_RUNTIME_DIR`, then `XDG_RUNTIME_DIR`,
  then `TMPDIR` on macOS, then `<temp>/jcode-<uid>`), duplicated deliberately in
  `sockets.rs` so client and bridge can never disagree about the path.

Two ways to start it:

- `jcode api-bridge [--api-socket <path>]` — the supported entry point, shipped
  inside the released binary. It best-effort starts the daemon first (a user
  trying the SDK usually has none running) and continues even if that spawn
  fails, since an already-running daemon is still usable. Note that the global
  `--socket` selects the *daemon* socket; the API socket flag is deliberately
  named `--api-socket`.
- `jcode-harness-api-bridge [api_socket] [legacy_socket]` — the standalone
  binary from `crates/jcode-harness-api-server`, which the SDK's launch path
  spawns directly.

Only one bridge may own an API socket: `run_bridge` takes an exclusive `flock`
on the sibling `.lock` path (`jcode-api.sock` → `jcode-api.lock`) and exits
cleanly if another live bridge holds it, which makes on-demand spawning
idempotent (`crates/jcode-harness-api-server/src/lib.rs:84-122`).

> The original version of this document also specified a pure-Rust desktop
> rewrite (`jcode-desktop2`, winit + wgpu + Vello + Parley). That app has been
> removed from this fork; the harness API remains, and the TUI is the only
> first-party client. See `docs/FORK_WORKFLOW.md` §1.
