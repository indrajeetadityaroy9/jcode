//! Local IPC transport over Unix domain sockets.
//!
//! `Listener` and `Stream` wrap `tokio::net::Unix*`. This fork targets macOS
//! only, so there is no second backend behind the abstraction — see
//! `docs/FORK_WORKFLOW.md` §1 for the purge that removed the named-pipe one.
//!
//! It lives in its own crate so the harness API bridge can use it without
//! depending on `jcode-base`, which the bridge is deliberately free of.

#[cfg(not(unix))]
compile_error!("jcode-transport is Unix-only in this fork; see docs/FORK_WORKFLOW.md §1");

mod unix;
pub use unix::*;
