use super::*;
use std::process::{Command, Stdio};

/// `check` reports a process blocked in `read(0)` on a pipe.
///
/// This replaces a test that asked whether *the test process itself* was
/// reading stdin. That answer is a property of the harness, not of this code:
/// `macos::check` reports `Reading` when fd 0 is a pipe or vnode and any thread
/// sits in `TH_STATE_WAITING`, and a libtest process launched from a shell
/// pipeline satisfies both whenever a worker thread happens to be parked. It
/// passed or failed depending on how the suite was invoked and how many threads
/// were live, which is exactly the kind of assertion that cannot hold.
#[cfg(target_os = "macos")]
#[test]
fn blocked_child_on_a_pipe_is_reported_as_reading() {
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("failed to spawn cat");

    // `cat` has to reach its first `read(0)` before the probe can see it.
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut state = is_waiting_for_stdin(pid);
    while state != StdinState::Reading && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(25));
        state = is_waiting_for_stdin(pid);
    }

    child.kill().ok();
    child.wait().ok();

    assert_eq!(
        state,
        StdinState::Reading,
        "a child blocked reading a piped stdin must be detected"
    );
}

#[test]
fn test_nonexistent_pid() {
    let state = is_waiting_for_stdin(u32::MAX);
    assert_ne!(state, StdinState::Reading);
}
