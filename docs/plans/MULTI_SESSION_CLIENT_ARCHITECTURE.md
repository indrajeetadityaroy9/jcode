# Multi-Session Client Architecture

Status: **partially shipped.** Phase 2 (the Niri-style workspace row model plus
the workspace-map widget) is built and reachable today; everything else —
`ClientShell` / `SessionSurface` multi-surface hosting, pop-out/dock, and
session-multiplexed protocol — is still a proposal. See "What Shipped" below
before trusting any later section.

This document describes a proposed evolution of jcode's UI architecture from the
current **single-session-per-client** model to a **multi-session-capable client**
model with built-in session workspace management.

The goal is to support a built-in spatial/multi-session UI for users on all
platforms, while preserving the current external-window workflow used with tools
like Niri.

See also:

- [`SERVER_ARCHITECTURE.md`](../SERVER_ARCHITECTURE.md)
- [`SWARM_ARCHITECTURE.md`](../SWARM_ARCHITECTURE.md)

## What Shipped

A workspace model and its map widget exist and are wired into the TUI. What is
built:

- **Workspace row model.** `crates/jcode-tui-workspace/src/workspace_map.rs`
  holds `WorkspaceMapModel { rows: BTreeMap<i32, WorkspaceRow>, current_workspace }`
  (`workspace_map.rs:107-110`). Each `WorkspaceRow` is a horizontal strip of
  session tiles plus a remembered `last_focused` index
  (`workspace_map.rs:42-43`), so vertical movement restores per-row focus
  exactly as the "Niri-Style Workspace UX" section describes. New sessions land
  via `insert_right_of_focus` (`workspace_map.rs:69-77`), and horizontal
  movement is `move_focus_left` / `move_focus_right`
  (`workspace_map.rs:79-99`). Only real sessions occupy cells — there is no
  fixed matrix of empty slots.
- **Workspace map widget.** `workspace_map_widget.rs::render_workspace_map`
  (`:88`) draws a vertical stack of horizontal tile strips, shape- and
  colour-first, with per-state colours and an animated running state
  (`tile_symbol` `:128`, `tile_color` `:141`, covering `Running`, `Error`,
  `Waiting`, `Completed`, `Detached`, `Idle`). The TUI feeds it the current
  visible rows from `App` (`crates/jcode-tui/src/tui/app/tui_state.rs:1532-1540`).
- **Client-side state is instance-owned.** `WorkspaceClientState` lives on
  `App` (`crates/jcode-tui/src/tui/app.rs:1628`), not in a process global.
- **`/workspace` command.**
  `crates/jcode-tui/src/tui/app/remote/workspace.rs:45-116` implements
  `/workspace` (status), `/workspace on` / `/workspace import`,
  `/workspace off`, and `/workspace add [right|up|down]`.
- **Alt+hjkl navigation.** Defaults are `Alt+H/J/K/L`
  (`crates/jcode-tui/src/tui/keybind.rs:92-130`), remappable through
  `[keybindings]` `workspace_left` / `workspace_down` / `workspace_up` /
  `workspace_right` — which is exactly the "configurable remapping" the
  Keybindings section asks for. Keys are dispatched from
  `app/remote/key_handling.rs:389` into
  `app/remote/workspace.rs::handle_workspace_navigation_key`.

What did **not** ship, and how the shipped feature differs:

- **One surface, not many.** Navigation does not move a camera across several
  hosted surfaces. It calls `remote.resume_session(&target_session_id)` on the
  single existing connection (`app/remote/workspace.rs:37`), i.e. the one client
  surface re-attaches to the neighbouring session. Phase 3's "one client process
  hosts multiple session surfaces" is unbuilt.
- **No `ClientShell` / `SessionSurface` / `SessionController` / `SurfaceId`.**
  None of the types in "Suggested Internal Model" exist anywhere in the tree.
  `App` is still the single monolithic client state root.
- **No pop-out or dock.** No commands, no interop surface
  (`focus_session`, `dock_session`, `undock_session`, …).
- **No protocol multiplexing.** `Request` / `ServerEvent` in
  `crates/jcode-protocol` are still single-session per connection.
- **Remote mode only.** Workspace commands and navigation live under
  `tui/app/remote/`, so workspace mode applies to the normal client/server path
  and is not wired into the local (in-process) loop.

Everything below this section is the original proposal, unchanged except where
a phase is explicitly marked.

## Summary

Today, jcode is effectively organized like this:

- **Server** owns many sessions.
- **Each client** usually attaches to one session.
- **Each terminal window/process** usually hosts one client.

That gives a practical mapping of:

- `session ≈ client ≈ process`

The proposed architecture changes the client model to:

- **Server** still owns many sessions.
- **Many clients** may still exist at once.
- **Each client may host one or many session surfaces**.

That changes the mapping to:

- `session = server-owned runtime`
- `surface = client-side attachment/view of a session`
- `client = container for one or many surfaces`

This preserves independent windows while enabling a built-in multi-session
workspace.

## Goals

- Add a built-in multi-session workspace UI.
- Preserve the current independent-client workflow.
- Preserve interoperability with external window managers like Niri.
- Make macOS and other platforms first-class for spatial multi-session use.
- Avoid duplicating the entire TUI stack into separate "independent" and
  "workspace" apps.
- Keep the server as the source of truth for sessions.

## Non-Goals

- Replacing OS-level window managers.
- Building a general-purpose terminal multiplexer for arbitrary applications.
- Requiring all users to adopt workspace mode.
- Supporting fully concurrent editing from multiple interactive attachments to the
  same session in the first version.

## Current Architecture

Current high-level model:

```text
Server
  ├── Session A
  ├── Session B
  └── Session C

Client 1 -> Session A
Client 2 -> Session B
Client 3 -> Session C
```

In practice, each client is typically its own terminal window/process, so users
who want a spatial layout today rely on an external window manager.

This works well on Linux with tools like Niri, but is not portable enough for a
cross-platform built-in workspace experience.

## Proposed Architecture

### Core idea

The server continues to own sessions, but the client evolves from a
single-session UI into a **multi-session shell**.

```text
Server
  ├── Session A
  ├── Session B
  ├── Session C
  └── Session D

Client 1 (workspace)
  ├── Surface A -> Session A
  ├── Surface B -> Session B
  └── Surface C -> Session C

Client 2 (independent)
  └── Surface D -> Session D
```

A independent window becomes just a client hosting one surface. A workspace
becomes a client hosting many surfaces.

## Terminology

### Session

A server-owned runtime containing:

- conversation history
- provider/model state
- tool execution state
- session persistence
- background task state
- memory extraction state

A session is **not** fundamentally a window or process.

### Surface (or Attachment)

A client-side interactive or passive view of a session.

Examples:

- a session shown inside the built-in workspace
- a independent jcode window attached to one session

A surface is the UI representation of a session in a specific client.

### Client

A TUI process that hosts one or many surfaces.

Examples:

- current independent jcode window
- future multi-session workspace client

## Key Design Rule

The architecture must separate:

### Shared session state

Owned by the server:

- messages
- streaming/tool events
- model/provider selection
- persisted metadata
- background execution state
- server-side session lifecycle

### Surface-local UI state

Owned by a specific client surface:

- input draft
- cursor position
- scroll position
- selection/copy state
- local pane focus
- pane zoom/fullscreen state
- local viewport and layout placement

This separation is required to support:

- one session shown in different places over time
- popping a session out into a independent window
- docking a independent session back into a workspace
- different local view state for the same underlying session

## Client Modes

The same client binary should support two primary modes.

### Single-surface mode

Equivalent to today's independent client:

- one client
- one surface
- one session attached

This should remain the default/simple mental model for many users.

### Multi-surface mode

Workspace mode:

- one client
- many surfaces
- spatial navigation and session management built in

This mode provides the in-app session manager and workspace UI.

## Interoperability with External Window Managers

Preserving interop with Niri and similar tools is a core requirement.

The built-in workspace must not replace independent clients. Instead, both should
remain first-class.

### Required workflow support

- attach a session inside the in-app workspace
- pop a session out into its own independent client/window
- optionally dock a independent session back into a workspace
- allow multiple independent clients to coexist with a workspace client

### Resulting model

- many clients may exist at once
- each client may host one or many session surfaces
- the server still owns the underlying sessions

## Interaction Ownership

For an initial implementation, a session should have **one active interactive
surface** at a time.

That means:

- if a workspace surface is popped out into a independent window, the independent
  surface becomes the active interactive owner
- the workspace surface should either disappear or become passive
- docking reverses that ownership

This avoids synchronization problems with:

- multiple input drafts
- racing submissions
- cursor/focus conflicts
- duplicate interactive ownership of the same session

A future design may allow richer mirroring or passive previews, but v1 should
prefer a single active controller per session.

## Niri-Style Workspace UX (row model built, camera navigation not)

The preferred first version is **not** a tiled multi-pane dashboard where many
sessions are all visible at once.

Instead, the built-in workspace should behave like a Niri-style spatial session
manager:

- the main viewport shows **one full-size session at a time**
- each session occupies a full-screen logical cell in the workspace
- moving left/right/up/down moves the **camera** through the workspace
- each workspace row behaves like a Niri horizontal strip of sessions
- moving up/down switches workspace rows and restores that row's remembered focus
- new sessions are inserted to the **right of the focused session** in the
  current workspace row

Conceptually:

```text
workspace +1: [session C]
workspace  0: [session A] [session B]
workspace -1: [session D] [session E] [session F]
```

This is intentionally **not** a fixed matrix with fake empty cells.

## Workspace Map / Info Widget (built)

The built-in info widget should act as a **workspace map**, not a text-heavy
status list.

### Role

The widget should let the user understand at a glance:

- which workspace row is current
- which session is focused in the current row
- what sessions exist to the left/right
- what sessions exist in nearby rows above/below
- which session was last focused in each non-current row
- which sessions are running, completed, errored, waiting, or detached

### Layout model

The widget should render a **vertical stack of horizontal strips**.

- each row represents one workspace
- each rectangle in a row represents one session
- only sessions that actually exist are shown
- non-current workspaces still remember their last-focused session

This preserves the Niri mental model much better than a synthetic grid.

### Visual language

The widget should be shape-first and text-light.

Each session is represented as a rectangle.

Suggested encoding:

- **idle** → dim outlined rectangle
- **focused** → bright or double-outlined rectangle
- **running** → animated rectangle border / spinner-like perimeter motion
- **completed** → green rectangle
- **waiting** → yellow rectangle
- **error** → red rectangle
- **detached** → distinct outline style (for example dashed or external marker)

The widget should avoid verbose labels inside the map itself. Session names and
full details belong in the main header/status area, not in the map.

### Example shape progression

One session:

```text
╔══════╗
╚══════╝
```

Add one to the right:

```text
┌──────┐  ╔══════╗
└──────┘  ╚══════╝
```

Move up and add one there:

```text
        ╔══════╗
        ╚══════╝

┌──────┐  ┌──────┐
└──────┘  └──────┘
```

The real TUI version should use color and animation rather than text markers.

## Client-Side Architecture (proposed — none of this exists)

The current single `App` object is too monolithic to scale cleanly to many
sessions. The client should be split into layers.

### `ClientShell`

Global process/UI state:

- terminal event loop
- workspace layout
- camera/viewport position for workspace movement
- focus management
- keyboard mode (normal/insert/command)
- surface management
- pop-out / dock orchestration
- global commands and notifications

### `SessionController`

Per-session live controller:

- subscribe/resume session
- submit message
- cancel current turn
- apply model/session commands
- receive and apply server events
- reconnect logic

### `SessionSurfaceState`

Per-surface local UI state:

- input buffer
- cursor position
- scroll state
- selection/copy state
- side pane local viewport
- local focus and zoom state

### Shared session renderer

A reusable rendering layer that can render a session surface into an arbitrary
rect. This is the key step for making both independent and workspace modes reuse
one UI stack.

## Suggested Internal Model (proposed — none of these types exist)

```rust
struct ClientShell {
    surfaces: Vec<SessionSurface>,
    focused_surface: Option<SurfaceId>,
    mode: ClientMode,
    layout: LayoutState,
}

struct SessionSurface {
    surface_id: SurfaceId,
    session_id: SessionId,
    controller: SessionController,
    ui: SessionSurfaceState,
}

struct SessionController {
    // v1: dedicated remote connection per surface
    // v2: multiplexed session handle
}

struct SessionSurfaceState {
    input: String,
    cursor_pos: usize,
    scroll_offset: usize,
    side_pane_focus: bool,
    zoomed: bool,
}
```

This enables:

- independent mode = one-surface client
- workspace mode = many-surface client

## Transport / Protocol Strategy

### Phase 1: dedicated connection per active surface — NOT BUILT

Fastest practical path:

- one client process
- one remote connection per live session surface

Pros:

- minimal protocol changes
- reuses the current session-oriented client behavior
- easiest way to prove out workspace UX

Cons:

- more overhead per hosted surface
- duplicate connection/reconnect machinery inside one process
- not the cleanest long-term abstraction

### Phase 2: multiplexed client protocol — NOT BUILT

Longer-term architecture:

- one client connection can subscribe to many sessions
- requests and events are explicitly tagged by `session_id`

Examples:

```rust
Request::SendMessage { session_id, ... }
Request::Cancel { session_id, ... }
ServerEvent::TextDelta { session_id, text }
ServerEvent::Done { session_id, ... }
```

Pros:

- cleaner workspace-native design
- lower connection overhead
- clearer event routing for multi-session clients

Cons:

- larger protocol and server refactor

Recommendation: do not block v1 on protocol multiplexing.

## Keybindings and Navigation (built)

The default workspace binding set, as shipped
(`crates/jcode-tui/src/tui/keybind.rs:92-130`):

- `Alt+h/j/k/l` for workspace movement
- remappable via `[keybindings]` `workspace_left` / `workspace_down` /
  `workspace_up` / `workspace_right`, for users who already use those chords in
  an external WM (for example `Super+h/j/k/l`)

Not built: the proposed modal split

- **normal mode** → workspace navigation and layout actions
- **insert mode** → focused session receives typed input

There are no client modes. `Alt`-chorded keys are unambiguous against text
entry, so navigation is handled directly in the normal input path
(`app/remote/key_handling.rs:389`) without a mode switch.

## Pop-Out / Dock Workflows (proposed — not implemented)

### Pop out to independent window

1. User selects a workspace surface.
2. Client spawns a independent jcode client attached to the same session.
3. Independent surface becomes the active interactive owner.
4. Workspace surface is removed or downgraded to passive.

### Dock into workspace

1. User requests dock for a independent session.
2. Workspace client creates a surface for that session.
3. Workspace surface becomes active interactive owner.
4. Independent client exits or detaches.

## Interop API Surface (proposed — not implemented)

The architecture should expose a small control surface for external and internal
interop.

Potential operations:

- `list_sessions`
- `list_surfaces`
- `workspace_state`
- `focus_session(session_id)`
- `open_session_in_window(session_id)`
- `dock_session(session_id)`
- `undock_session(session_id)`
- `move_session_to_workspace(session_id, position)`

This can initially be provided through existing jcode control channels such as:

- CLI commands
- the main server protocol
- debug/control socket

The exact public API shape is less important than preserving a clean internal
model for these operations.

## Recommended UI Direction

For a first version, prefer a **full-screen, camera-style workspace** over a
true many-pane dashboard.

Reasons:

- much closer to the Niri mental model
- keeps each session full-size and fully readable
- makes smooth movement between sessions more feasible in a terminal UI
- simplifies rendering because only the current session needs full live focus
- still allows richer overview modes later

This can later grow into optional resizeable session surfaces or richer
multi-visible workspace views, but the first version should optimize for a
smooth Niri-like experience.

## Migration Plan

### Phase 0: renderer extraction — NOT BUILT

- Extract a reusable session rendering layer from the current TUI.
- Stop assuming one `App` owns the entire terminal surface.

### Phase 1: surface/controller split — NOT BUILT

- Split current monolithic client state into shell/controller/surface layers.
- Keep single-surface behavior unchanged.

### Phase 2: workspace model + map widget — SHIPPED

- Introduce a Niri-style workspace row model. *(Done:
  `crates/jcode-tui-workspace/src/workspace_map.rs`.)*
- Add the workspace-map info widget with rectangle-only state rendering.
  *(Done: `workspace_map_widget.rs::render_workspace_map`.)*
- Track remembered focus per workspace row. *(Done: `WorkspaceRow.last_focused`.)*

Driven by `/workspace` and `Alt+hjkl`; see "What Shipped".

### Phase 3: full-screen camera navigation — NOT BUILT

- Allow one client process to host multiple session surfaces.
- Show one full-size session at a time.
- Move the viewport between neighboring sessions/workspaces.

### Phase 4: pop-out support — NOT BUILT

- Add commands to open a hosted session in a independent client.
- Preserve current `jcode --resume <session>` workflow.

### Phase 5: dock support — NOT BUILT

- Allow a independent session to be reattached into a workspace client.
- Keep one interactive owner per session.

### Phase 6: protocol cleanup — NOT BUILT

- Evaluate session-multiplexed protocol support.
- Replace dedicated per-surface connections if and when it is clearly beneficial.

## Open Questions

- Should passive mirrored surfaces exist in v1, or should a session exist in only
  one visible place at a time?
- Which pieces of side-panel state are session-scoped vs surface-scoped?
- Should workspace mode be a new command (`jcode workspace`) or a runtime mode of
  the normal client?
- How should dock/undock be exposed: command palette, slash commands, CLI, debug
  socket, or all of the above?
- How much workspace layout state should be persisted across launches?
- How much offscreen session state should be pre-rendered for smooth animation?

## Recommendation

Adopt the following design direction:

1. **Expand the client to support multiple session surfaces.**
2. **Keep the server as the owner of sessions.**
3. **Preserve independent clients as first-class.**
4. **Treat workspace panes and independent windows as different surfaces for the
   same session model.**
5. **Start with one active interactive surface per session.**
6. **Use a Niri-style full-screen workspace with a rectangle-only workspace map
   widget as the primary UX.**
7. **Prototype with one connection per active surface before attempting protocol
   multiplexing.**

This gives jcode a portable built-in multi-session workspace without sacrificing
existing workflows or external window-manager interop.
