//! Host-wide metrics read straight from the macOS kernel: physical RAM,
//! reclaimable memory and the load average.
//!
//! These are machine-level numbers, and every consumer that wants to know how
//! loaded the box is reads them from here: the overnight resource card
//! (`jcode-app-core::overnight`), performance-tier detection
//! (`jcode-app-core::perf`) and the TUI's per-frame resource attribution
//! (`jcode-tui::tui::ui_frame_metrics`). One implementation is the point —
//! "available memory" on macOS is a choice of page classes rather than a
//! kernel-provided figure, so three private copies would eventually disagree
//! about the same number.
//!
//! Process-level counters (this process's resident size, its CPU time) belong
//! to [`crate::process_memory`], not here.
//!
//! Every reader returns `None` when its syscall fails; nothing here
//! substitutes a zero or an estimate, because a fabricated 0 reads as
//! "machine is out of memory" or "machine is idle".
//!
//! No dependency beyond `libc`: the two Mach entry points `libc` does not
//! expose are hand-declared below, in the style of
//! `jcode-core::stdin_detect`'s `proc_pidinfo` block.

use std::ffi::CStr;

/// Bytes in a megabyte. The MB-valued readers here report whole MB.
pub const BYTES_PER_MB: u64 = 1024 * 1024;

// `libc` already declares `host_statistics64`, `vm_statistics64` and
// `HOST_VM_INFO64`, but its `mach_host_self` is deprecated in favor of the
// `mach2` crate and it has no `mach_port_deallocate` at all, so those two are
// declared here instead of taking a new dependency.
unsafe extern "C" {
    /// Returns a send right to the host port. The caller owns a reference and
    /// must release it with `mach_port_deallocate`.
    fn mach_host_self() -> libc::mach_port_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
    /// `mach_task_self()` is a macro over this global in `<mach/mach_init.h>`.
    static mach_task_self_: libc::mach_port_t;
}

/// Installed physical RAM in bytes, from `sysctlbyname("hw.memsize")`.
pub fn total_physical_memory_bytes() -> Option<u64> {
    sysctl_u64(c"hw.memsize")
}

/// Reclaimable memory in bytes: `host_statistics64(HOST_VM_INFO64)`'s
/// `free_count + inactive_count + purgeable_count`, times the host page size.
///
/// macOS has no single `MemAvailable`, so that choice of page classes *is* the
/// meaning of the number: `free_count` (which already includes speculative
/// read-ahead pages), `inactive_count` (evictable, mostly file-backed) and
/// `purgeable_count` (discardable on demand). Active, wired and
/// compressor-held pages are excluded because reclaiming them needs a
/// swap-out or is impossible. `free_count` alone is deliberately not reported
/// as available: macOS drives free memory to near zero by design, so it reads
/// as exhaustion on a perfectly healthy machine.
pub fn available_memory_bytes() -> Option<u64> {
    let page_size = host_page_size()?;
    // Safety: `mach_host_self` takes no arguments and returns a port name by
    // value. The send right it hands back is released below.
    let host = unsafe { mach_host_self() };
    if host == 0 {
        // MACH_PORT_NULL
        return None;
    }
    // Safety: `vm_statistics64` is a `#[repr(C)]` struct of integers, so an
    // all-zero bit pattern is a valid value.
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // Safety: `host` is a live host port; the out-buffer is a live
    // `vm_statistics64` and `count` is its size in `integer_t` words, which is
    // exactly what `HOST_VM_INFO64` fills.
    let rc = unsafe {
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            &mut stats as *mut libc::vm_statistics64 as *mut libc::integer_t,
            &mut count,
        )
    };
    // Safety: `mach_task_self_` names this task and `host` is the send right
    // just acquired from `mach_host_self`; deallocating it exactly once
    // balances that acquisition and leaks no port reference per sample.
    unsafe { mach_port_deallocate(mach_task_self_, host) };
    if rc != libc::KERN_SUCCESS {
        return None;
    }
    let pages = u64::from(stats.free_count)
        + u64::from(stats.inactive_count)
        + u64::from(stats.purgeable_count);
    Some(pages * page_size)
}

/// `(total_mb, available_mb)`: physical RAM and reclaimable memory as whole
/// megabytes.
///
/// The two halves come from [`total_physical_memory_bytes`] and
/// [`available_memory_bytes`], and either can be `None` on its own. Because
/// the Mach page counters are sampled independently of `hw.memsize`, the sum
/// is clamped to the total: callers derive a used-percent from
/// `total - available` and must never see a wider available than total.
pub fn memory_mb() -> (Option<u64>, Option<u64>) {
    let total_bytes = total_physical_memory_bytes();
    let available_bytes = available_memory_bytes();
    let available_mb = available_bytes.map(|bytes| match total_bytes {
        Some(total) => bytes.min(total) / BYTES_PER_MB,
        None => bytes / BYTES_PER_MB,
    });
    (total_bytes.map(|bytes| bytes / BYTES_PER_MB), available_mb)
}

/// One-minute load average: the first sample of `getloadavg(3)`, the libc
/// wrapper over the kernel's `vm.loadavg`.
///
/// Fewer than one sample filled means the read failed, which reports `None`
/// rather than a zero that would look like an idle box.
pub fn load_average_1m() -> Option<f64> {
    let mut loadavg: [libc::c_double; 3] = [0.0; 3];
    // Safety: `getloadavg` writes at most `nelem` doubles, and the buffer holds
    // exactly the 3 requested.
    let filled = unsafe { libc::getloadavg(loadavg.as_mut_ptr(), 3) };
    if filled >= 1 { Some(loadavg[0]) } else { None }
}

/// `(load_one, cpu_count)`: the one-minute load average and the number of CPUs
/// available to this process, the pair every load consumer needs in order to
/// compute load-per-CPU.
pub fn load_and_cpu_count() -> (Option<f64>, Option<usize>) {
    // Explicit match rather than a swallowing combinator: an unreadable CPU
    // count is reported as absent, never as a substituted 1, which would
    // double a load-per-CPU ratio.
    let cpus = match std::thread::available_parallelism() {
        Ok(value) => Some(value.get()),
        Err(_) => None,
    };
    (load_average_1m(), cpus)
}

/// Read a `u64`-valued sysctl by name.
fn sysctl_u64(name: &CStr) -> Option<u64> {
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // Safety: `name` is NUL-terminated, the output buffer is a live `u64`, and
    // `len` states its exact size so `sysctlbyname` cannot write past it. It
    // retains neither pointer.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len != std::mem::size_of::<u64>() {
        return None;
    }
    Some(value)
}

/// Host page size in bytes; Mach page counters are in these units (16 KiB on
/// Apple silicon, 4 KiB on Intel).
fn host_page_size() -> Option<u64> {
    // Safety: `sysconf` takes an integer name and returns a long. No pointers.
    let value = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if value <= 0 {
        return None;
    }
    Some(value as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a `u64` sysctl through the `sysctl(8)` CLI, as an oracle
    /// independent of this module's `sysctlbyname` call.
    fn sysctl_cli_u64(name: &str) -> u64 {
        let output = std::process::Command::new("sysctl")
            .args(["-n", name])
            .output()
            .expect("sysctl ships with macOS");
        assert!(output.status.success(), "sysctl -n {name} failed");
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("sysctl value is an integer")
    }

    #[test]
    fn total_physical_memory_matches_hw_memsize() {
        let bytes = total_physical_memory_bytes().expect("hw.memsize is readable on macOS");
        assert_eq!(bytes, sysctl_cli_u64("hw.memsize"));
        let (total_mb, _) = memory_mb();
        assert_eq!(total_mb, Some(bytes / BYTES_PER_MB));
        assert!(
            total_mb.is_some_and(|mb| mb > 1024),
            "physical RAM under 1 GiB: {total_mb:?} MB"
        );
    }

    #[test]
    fn available_memory_is_a_real_fraction_of_physical_ram() {
        let (total_mb, available_mb) = memory_mb();
        let total_mb = total_mb.expect("hw.memsize is readable on macOS");
        let available_mb = available_mb.expect("host_statistics64 is readable on macOS");
        // A fabricated 0 or a total-sized guess both fail here: the machine
        // always has some reclaimable memory, and never all of it.
        assert!(
            available_mb > 0 && available_mb < total_mb,
            "available {available_mb} MB is outside 1..{total_mb} MB"
        );
        // Free pages alone would be a small fraction of this; the point of
        // counting inactive+purgeable is that the figure is not a near-zero
        // number on a healthy machine.
        let page_size = host_page_size().expect("page size");
        let available_bytes = available_memory_bytes().expect("host_statistics64");
        assert_eq!(
            available_bytes % page_size,
            0,
            "available {available_bytes} bytes is not a whole number of {page_size}-byte pages"
        );
    }

    #[test]
    fn load_average_tracks_the_kernel() {
        let (load, cpus) = load_and_cpu_count();
        let load = load.expect("getloadavg fills at least one sample");
        assert!(
            load.is_finite() && (0.0..200.0).contains(&load),
            "implausible load average {load}"
        );
        let cpus = cpus.expect("cpu count is readable");
        assert!(cpus >= 1, "cpu count {cpus}");

        // Oracle: `sysctl -n vm.loadavg` prints "{ 4.82 5.87 6.08 }".
        let output = std::process::Command::new("sysctl")
            .args(["-n", "vm.loadavg"])
            .output()
            .expect("sysctl ships with macOS");
        let raw = String::from_utf8_lossy(&output.stdout);
        let kernel: f64 = raw
            .split_whitespace()
            .nth(1)
            .expect("first loadavg sample")
            .parse()
            .expect("loadavg is a float");
        let tolerance = (kernel * 0.5).max(2.0);
        assert!(
            (load - kernel).abs() <= tolerance,
            "load {load} disagrees with vm.loadavg {kernel}"
        );
    }
}
