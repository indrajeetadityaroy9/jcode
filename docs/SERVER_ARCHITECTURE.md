# Server Architecture

See also:

- [`plans/SERVER_SERVICE_SPLIT_PLAN.md`](./plans/SERVER_SERVICE_SPLIT_PLAN.md)
- [`SWARM_ARCHITECTURE.md`](./SWARM_ARCHITECTURE.md)
- [`plans/MULTI_SESSION_CLIENT_ARCHITECTURE.md`](./plans/MULTI_SESSION_CLIENT_ARCHITECTURE.md)

## Overview

jcode uses a **single-server, multi-client** architecture. One server process
manages all sessions and state; TUI clients connect over a Unix socket and
can reconnect transparently after disconnects or server reloads.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              SERVER (🔥 blazing)                              │
│                                                                             │
│  jcode serve                                                                │
│  ├── Unix socket:  $TMPDIR/jcode.sock  (macOS default)                      │
│  ├── Debug socket: $TMPDIR/jcode-debug.sock                                 │
│  ├── Registry:     ~/.jcode/servers.json                                    │
│  ├── Provider (Claude/OpenAI/OpenRouter)                                    │
│  ├── MCP pool (shared across sessions)                                      │
│  └── Sessions:                                                              │
│        ├── 🦊 fox   (active)  → "🔥 blazing 🦊 fox"                         │
│        ├── 🐻 bear  (active)  → "🔥 blazing 🐻 bear"                        │
│        └── 🦉 owl   (idle)    → "🔥 blazing 🦉 owl"                         │
└─────────────────────────────────────────────────────────────────────────────┘
         │              │              │
         ▼              ▼              ▼
    ┌─────────┐   ┌─────────┐   ┌─────────┐
    │ Client 1│   │ Client 2│   │ Client 3│
    │ 🦊 fox  │   │ 🐻 bear │   │ 🦉 owl  │
    └─────────┘   └─────────┘   └─────────┘
```

## Naming

```
SERVER = Adjective/Verb modifier          SESSIONS = Animal nouns
────────────────────────────              ────────────────────────
🔥 blazing   ❄️ frozen   ⚡ swift          🦊 fox    🐻 bear   🦉 owl
🌀 rising    🍂 falling  🌊 rushing        🌙 moon   ⭐ star   🔥 fire
✨ bright    🌑 dark     💫 spinning       🐺 wolf   🦁 lion   🐋 whale

Combined: "🔥 blazing 🦊 fox" = server + session
```

The server gets a random adjective/verb name on startup (e.g., "blazing").
Each session gets an animal noun (e.g., "fox"). Together they form a natural
phrase displayed in the UI: "🔥 blazing 🦊 fox".

The server name persists across reloads via the registry (`~/.jcode/servers.json`).
When the server execs into a new binary on `/reload`, the new process registers
with a fresh name. Stale entries are cleaned up automatically.

## Lifecycle

```
  START                          CONNECT                     RELOAD
  ─────                          ───────                     ──────
  jcode (first run)              jcode (subsequent)          /reload
       │                              │                          │
       ├─▶ No server? Spawn daemon    ├─▶ Server exists?         ├─▶ Server execs into
       ├─▶ Wait for socket            │   Connect directly       │   new binary (same PID)
       ├─▶ Connect as client          │                          ├─▶ All clients disconnect
       └─▶ Create session             └─▶ Create/resume session  └─▶ Clients auto-reconnect
```

### Server Startup

When you run `jcode`, it checks if a server is already running:

1. **Server exists**: connect directly as a client
2. **No server**: spawn `jcode serve` as a detached daemon (with `setsid`),
   wait for the socket, then connect

The server is fully detached from the spawning client via `setsid()`, so killing
any client never affects the server or other clients.

Long-lived deployments can give the daemon a stable client-visible identity with
`jcode serve --server-name <name>` or the `JCODE_SERVER_NAME` environment
variable. The optional `JCODE_SERVER_DISPLAY_NAME` environment variable is also
accepted for service managers that prefer a display-oriented name. CLI input wins
over environment input. Names are normalized to registry-safe lowercase labels,
so `mount-cloud/fabian` displays as `mount-cloud-fabian`.

### Server Shutdown

The server shuts down when:
- **Idle timeout**: no clients connected for 5 minutes (configurable)
- **Manual**: server process is killed
- **Reload**: server execs into a new binary (same socket path)

### Remote Client Working Directory

By default, a client sends its current working directory to the server when it
subscribes, and the server uses that as the session working directory. Socket
forwarding wrappers for remote daemons can keep the client and server paths
separate with `--remote-working-dir`:

```bash
jcode --socket /tmp/jcode.sock -C /local/checkout --remote-working-dir /remote/checkout
```

`-C` must exist on the client. `--remote-working-dir` must be an absolute path
that exists on the server.

### Client Reconnection

Clients have a built-in reconnect loop. When the connection drops (server
reload, network issue, etc.):

1. Client shows "Connection lost - reconnecting..."
2. Retries with exponential backoff (1s, 2s, 4s... up to 30s)
3. On reconnect, resumes the same session (session state persists on disk)
4. If server was reloaded, client may also re-exec itself if a newer
   client binary is available

### Hot Reload (`/reload`)

1. Client sends `Request::Reload` to server
2. Server sends `Reloading` event to the requesting client
3. Server calls `exec()` into the new binary with `serve` args
4. New server process starts on the same socket
5. All clients auto-reconnect
6. The initiating client also re-execs if its binary is outdated

## Socket Paths

Both sockets live side by side in the *runtime directory*. This is a macOS-only
fork, so in practice that is the per-user `$TMPDIR` (something like
`/var/folders/xx/…/T/`):

```
$TMPDIR/
├── jcode.sock          # Main client/server socket
├── jcode-debug.sock    # Debug/introspection listener
└── jcode-daemon.lock   # Exclusive flock held for the daemon's lifetime
```

The runtime directory is resolved in this order
(`crates/jcode-storage/src/lib.rs:97-118`):

1. `JCODE_RUNTIME_DIR`, if set.
2. `XDG_RUNTIME_DIR`, if set. Nothing on macOS sets this by default, but a
   wrapper or service manager may.
3. `TMPDIR` — the normal macOS case.
4. Fallback: `<system temp dir>/jcode-<euid>`, created `0700`.

The socket paths themselves are then
(`crates/jcode-app-core/src/server/socket.rs:7-24`):

- **Main socket**: `JCODE_SOCKET` verbatim if set, else
  `<runtime dir>/jcode.sock`.
- **Debug socket**: always derived from the main socket path by replacing the
  trailing `.sock` with `-debug.sock`. Overriding `JCODE_SOCKET` therefore moves
  both sockets together, and `--socket <path>` (which exports `JCODE_SOCKET`)
  behaves the same way.

The daemon lock lives at `<runtime dir>/jcode-daemon.lock`
(`socket.rs:160-162`) and is what makes stale-socket reaping safe: a socket with
no live listener whose lock can be acquired is provably orphaned.

### Debug socket listener

The debug socket is a second accept loop in the server process
(`crates/jcode-app-core/src/server.rs:2238`, `2298`), routed by
`server/debug.rs::handle_debug_client`. It exists for testing and
introspection: snapshots of server/swarm state, session admin, debug jobs.

This is a *server-side listener*, not an agent tool. The old `debug_socket`
agent tool was removed in this fork; nothing in `crates/jcode-app-core/src/tool`
references it. Only out-of-band tooling talks to the debug socket.

## Key Behaviors

| Scenario | Behavior |
|----------|----------|
| First `jcode` run | Spawns server daemon, connects |
| Subsequent `jcode` | Connects to existing server |
| Kill a client | Server + other clients unaffected |
| `/reload` | Server execs new binary, clients reconnect |
| All clients close | Server idle-timeout after 5 min |
| Resume session | `jcode --resume fox` reconnects to existing session |
