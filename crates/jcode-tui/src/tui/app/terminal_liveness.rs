//! Detection of an abandoned controlling terminal.
//!
//! A TUI client can outlive its terminal: if the terminal emulator dies
//! without delivering SIGHUP (or the signal arrives while the runtime is
//! wedged), the client keeps running headless forever, holding its full
//! transcript and ~80-150 MB of heap. Dozens of such orphans were observed
//! stacking up from spawned swarm windows. crossterm's `EventStream` returns
//! `None` after input EOF, and the run loops used to just sleep and retry,
//! so nothing ever exited.
//!
//! This module answers one question cheaply: "did the controlling terminal
//! this process started with go away?" There is no cheap probe for that on
//! macOS, so it conservatively reports `false` and orphan exit relies on
//! SIGHUP alone.

use std::sync::OnceLock;

/// tty_nr captured on first call. `None` until initialized, `Some(0)` when
/// the process never had a controlling terminal (piped/headless usage), in
/// which case abandonment is never reported.
static INITIAL_TTY_NR: OnceLock<u64> = OnceLock::new();

/// Record the startup controlling terminal. Called implicitly by
/// [`terminal_abandoned`], but callers may invoke it early (before any chance
/// of the terminal dying) for a more faithful baseline.
pub(crate) fn capture_initial_tty() {
    let _ = INITIAL_TTY_NR.get_or_init(|| current_tty_nr().unwrap_or(0));
}

/// True when this process started with a controlling terminal that has since
/// disappeared. Cheap, safe to call from tick loops.
pub(crate) fn terminal_abandoned() -> bool {
    capture_initial_tty();
    let initial = INITIAL_TTY_NR.get().copied().unwrap_or(0);
    if initial == 0 {
        // Never had a controlling terminal: nothing to lose.
        return false;
    }
    match current_tty_nr() {
        Some(0) => true,
        // No tty number available or a live tty: assume alive.
        _ => false,
    }
}

fn current_tty_nr() -> Option<u64> {
    None
}
