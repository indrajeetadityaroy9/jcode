# Compile-time crate splitting plan

Status: partly landed. `jcode-storage`, `jcode-provider-anthropic`, and
`jcode-provider-openai` all exist. `jcode-session-core`,
`jcode-tui-app-state`, and `jcode-server-protocol-runtime` were never built and
are not currently planned. The root `src/` module layer this doc was written
against is gone: the root package holds only `main.rs`, `lib.rs`, `cli/`, and
`bin/`, and everything else lives under `crates/`.

## Goal

Minimize the amount of code that must be rechecked or rebuilt when iterating on
Jcode. The three spine crates (`jcode-base` -> `jcode-app-core` -> `jcode-tui`)
are still the integration shell, but stable leaf code should live in small
crates with one-way dependencies.

## Principles

1. Extract stable leaves first: filesystem/storage, protocol/types, parsers,
   provider request/stream codecs, and TUI render primitives.
2. Avoid cyclic domain crates. Spine crates may depend on leaf crates, but leaf
   crates must not call back into spine logging/config/runtime directly. Use data
   types, callbacks, or explicit events at boundaries.
3. Split by recompilation volatility, not by directory names. Code edited often
   should not force heavy provider/TUI/server modules to rebuild unless needed.
4. Keep heavy optional dependencies behind crates/features. Embeddings and PDF
   should remain isolated and feature-gated.
5. Preserve compatibility facades during migration. `crate::storage::*` re-exports
   `jcode_storage::*` while callers move gradually.

## Landed: `jcode-storage`

`jcode-storage` is a leaf crate for app paths, permission hardening, atomic
JSON writes, append-only JSONL helpers, and the active-pid registry. The
compatibility facade is no longer the root `src/storage.rs` this doc originally
described — it moved with the rest of the monolith and is now
`crates/jcode-base/src/storage.rs`, whose first statement is
`pub use jcode_storage::*;` (`:3`) and which keeps the backup-recovery logging
behavior by wrapping `read_json_with_recovery_handler`.

Measured at extraction time (not re-measured since the spine split):

- `cargo check -p jcode-storage`: ~0.9s after initial dependencies were built.
- `cargo check -p jcode --lib`: ~14s in the then-current warm-cache state.

## Extraction status

1. `jcode-provider-anthropic` — **landed.** Anthropic request/stream
   translation lives in `crates/jcode-provider-anthropic`, depending on
   `jcode-provider-core`, `jcode-message-types`, `jcode-schema-dialect`, and
   `jcode-logging`. The provider *runtime* went further out, into
   `jcode-provider-anthropic-runtime`; what stayed in
   `crates/jcode-base/src/provider/anthropic.rs` is a documented compatibility
   shim (`:1-14`) holding the OAuth headers, API-key resolution, cache-TTL
   toggle, and static model list that base's own auth/usage code shares.
2. `jcode-provider-openai` — **landed**, same shape:
   `crates/jcode-provider-openai` owns `request`/`stream`/`websocket_health`,
   `jcode-provider-openai-runtime` owns the runtime, and
   `crates/jcode-base/src/provider/openai.rs` is a shim (`:1-12`).
3. `jcode-session-core` — **not built.** Session storage paths, journal
   metadata, and memory-profile transforms were split into modules instead, under
   `crates/jcode-base/src/session/` (`storage_paths.rs`, `journal.rs`,
   `memory_profile.rs`, `persistence.rs`, `model.rs`). Extracting a crate still
   requires cutting the dependencies on spine prompt/logging behind callbacks.
4. `jcode-tui-app-state` — **not built.** Key/input/navigation state still lives
   with the rest of the app in `crates/jcode-tui/src/tui/app/`, whose
   `input.rs`, `inline_interactive.rs`, and `commands.rs` are three of the six
   largest tracked files (`scripts/code_size_budget.json`). Rendering primitives
   did move out to `jcode-tui-render`, `jcode-tui-style`, and
   `jcode-tui-markdown`.
5. `jcode-server-protocol-runtime` — **not built.** Client event fanout and
   agent execution both still live in `crates/jcode-app-core/src/server/`. The
   wire protocol itself did move, into `jcode-protocol`, and the Unix-socket
   transport into `jcode-transport`.

## Anti-patterns to avoid

- Extracting crates that depend on the root `jcode` package. No workspace peer
  does today. Note that eight `jcode-provider-*-runtime` crates depend on
  `jcode-base` instead, which keeps them off the spine's rebuild path but is not
  a contract-only dependency.
- Tiny crates for every file. Too many crates increase metadata overhead and make
  refactors painful.
- Moving only type aliases while leaving implementations in root. The expensive
  compile units remain expensive.
