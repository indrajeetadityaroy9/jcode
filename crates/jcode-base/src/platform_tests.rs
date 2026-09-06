use super::*;

#[test]
fn desired_nofile_soft_limit_only_raises_when_possible() {
    assert_eq!(desired_nofile_soft_limit(1024, 524_288, 8192), Some(8192));
    assert_eq!(desired_nofile_soft_limit(8192, 524_288, 8192), None);
    assert_eq!(desired_nofile_soft_limit(1024, 4096, 8192), Some(4096));
}

#[cfg(unix)]
#[test]
fn spawn_detached_creates_new_session() {
    // Ask the kernel directly rather than shelling out to `ps -o sid=`: that
    // keyword is Linux-only (macOS `ps` exposes `sess`, which prints a session
    // pointer, not a usable sid), so the old probe could never pass here.
    let parent_sid = unsafe { libc::getsid(0) };

    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg("sleep 2")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut child = super::spawn_detached(&mut cmd).expect("spawn detached child");
    let child_pid = child.id() as i32;

    // Read the session while the child is still alive, then reap it.
    let child_sid = unsafe { libc::getsid(child_pid) };
    let _ = child.kill();
    let _ = child.wait();

    assert_ne!(
        child_sid, -1,
        "getsid on the live detached child should work"
    );
    assert_eq!(
        child_sid, child_pid,
        "detached child should lead its own session"
    );
    assert_ne!(
        child_sid, parent_sid,
        "detached child should not share parent session"
    );
}
