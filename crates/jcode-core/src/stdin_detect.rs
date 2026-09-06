#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdinState {
    Reading,
    NotReading,
    Unknown,
}

pub fn is_waiting_for_stdin(pid: u32) -> StdinState {
    macos::check(pid)
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::mem;

    // libproc bindings
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: i32,
        ) -> i32;
    }

    const PROC_PIDLISTFDS: i32 = 1;

    #[repr(C)]
    struct proc_fdinfo {
        proc_fd: i32,
        proc_fdtype: u32,
    }

    // Thread info
    const PROC_PIDTHREADINFO: i32 = 5;
    const PROC_PIDLISTTHREADS: i32 = 6;

    #[repr(C)]
    struct proc_threadinfo {
        pth_user_time: u64,
        pth_system_time: u64,
        pth_cpu_usage: i32,
        pth_policy: i32,
        pth_run_state: i32,
        pth_flags: i32,
        pth_sleep_time: i32,
        pth_curpri: i32,
        pth_priority: i32,
        pth_maxpriority: i32,
        pth_name: [u8; 64],
    }

    // Take this from `libc` rather than hand-rolling it. It was previously
    // defined locally as 2, which is `TH_STATE_STOPPED`, so a thread blocked on
    // `read(0)` never registered and macOS stdin forwarding never fired
    // (issue #651). Apple's `mach/thread_info.h` has RUNNING=1, STOPPED=2,
    // WAITING=3, and `libc` already exposes it, so sourcing it removes the
    // opportunity for this constant to drift again.
    use libc::TH_STATE_WAITING;

    /// Report whether `pid` looks blocked reading stdin.
    ///
    /// Known limitation: fd 0 is classified from its *type* only, and a vnode
    /// covers both a pty and `/dev/null`, so a process whose stdin is
    /// `/dev/null` and which parks in any wait state is reported as `Reading`.
    /// Separating them needs the fd's vnode path (`PROC_PIDFDVNODEPATHINFO`,
    /// declared below but unused), so callers must treat `Reading` as advisory
    /// rather than proof that input is wanted.
    pub fn check(pid: u32) -> StdinState {
        // Check if fd 0 (stdin) is a pipe or pty
        if !stdin_is_interactive(pid as i32) {
            return StdinState::NotReading;
        }

        // Check thread states - if any thread is in WAITING state,
        // the process might be blocked on I/O
        if is_thread_waiting(pid as i32) {
            return StdinState::Reading;
        }

        StdinState::NotReading
    }

    fn stdin_is_interactive(pid: i32) -> bool {
        // Get list of file descriptors
        let fd_size = mem::size_of::<proc_fdinfo>() as i32;
        let buf_size = fd_size * 256; // up to 256 fds
        let mut buf = vec![0u8; buf_size as usize];

        let ret = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDLISTFDS,
                0,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf_size,
            )
        };

        if ret <= 0 {
            return false;
        }

        let num_fds = ret / fd_size;
        let fds = unsafe {
            std::slice::from_raw_parts(buf.as_ptr() as *const proc_fdinfo, num_fds as usize)
        };

        // Check if fd 0 exists and is a pipe or vnode (pty)
        for fd in fds {
            if fd.proc_fd == 0 {
                // fd type 1 = vnode (could be pty), 6 = pipe
                return fd.proc_fdtype == 1 || fd.proc_fdtype == 6;
            }
        }

        false
    }

    fn is_thread_waiting(pid: i32) -> bool {
        // Get thread list
        let mut thread_ids = vec![0u64; 64];
        let ret = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDLISTTHREADS,
                0,
                thread_ids.as_mut_ptr() as *mut libc::c_void,
                (thread_ids.len() * mem::size_of::<u64>()) as i32,
            )
        };

        if ret <= 0 {
            return false;
        }

        let num_threads = ret as usize / mem::size_of::<u64>();

        // Check each thread's state
        for i in 0..num_threads {
            let mut tinfo: proc_threadinfo = unsafe { mem::zeroed() };
            let ret = unsafe {
                proc_pidinfo(
                    pid,
                    PROC_PIDTHREADINFO,
                    thread_ids[i],
                    &mut tinfo as *mut _ as *mut libc::c_void,
                    mem::size_of::<proc_threadinfo>() as i32,
                )
            };

            if ret > 0 && tinfo.pth_run_state == TH_STATE_WAITING {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
#[path = "stdin_detect_tests.rs"]
mod stdin_detect_tests;
