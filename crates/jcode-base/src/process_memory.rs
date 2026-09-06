use crate::logging;
use anyhow::{Result, anyhow};
#[cfg(feature = "jemalloc")]
use libc::c_char;
use serde::Serialize;
use std::collections::VecDeque;
#[cfg(feature = "jemalloc")]
use std::ffi::CString;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_HISTORY_SAMPLES: usize = 512;

#[cfg(feature = "jemalloc")]
struct JemallocStatsMibs {
    epoch: tikv_jemalloc_ctl::epoch_mib,
    allocated: tikv_jemalloc_ctl::stats::allocated_mib,
    active: tikv_jemalloc_ctl::stats::active_mib,
    metadata: tikv_jemalloc_ctl::stats::metadata_mib,
    resident: tikv_jemalloc_ctl::stats::resident_mib,
    mapped: tikv_jemalloc_ctl::stats::mapped_mib,
    retained: tikv_jemalloc_ctl::stats::retained_mib,
}

#[cfg(feature = "jemalloc-prof")]
struct JemallocProfilingMibs {
    enabled: tikv_jemalloc_ctl::profiling::prof_mib,
}

/// Process-level memory numbers, read from the macOS kernel by
/// [`snapshot_with_source`].
///
/// Every field is `Option` because each has its own reader and any of them can
/// fail; a failed read stays `None` rather than being reported as zero.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProcessMemorySnapshot {
    /// Resident set size, from `proc_pidinfo(PROC_PIDTASKINFO)`'s
    /// `pti_resident_size`.
    pub rss_bytes: Option<u64>,
    /// Kernel-tracked high-water resident size since process start, from
    /// `task_info(TASK_VM_INFO)`'s `resident_size_peak`. This is a real
    /// kernel counter — macOS's analogue of Linux's `VmHWM` — not a maximum
    /// accumulated over the samples in [`history`].
    pub peak_rss_bytes: Option<u64>,
    /// Virtual size, from `pti_virtual_size`. On macOS this is dominated by
    /// large reserved-but-unbacked regions (hundreds of GB is normal) and says
    /// nothing about memory pressure; `rss_bytes` is the number to watch.
    pub virtual_bytes: Option<u64>,
    /// Number of OS threads in the task, from `pti_threadnum`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_count: Option<u64>,
    /// Main thread stack size, from `getrlimit(RLIMIT_STACK)`: macOS sizes
    /// that mapping at exec from the soft limit. Auxiliary thread stacks live
    /// in separate anonymous mappings and are not included here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<OsProcessMemoryInfo>,
    pub allocator: AllocatorInfo,
}

/// Finer-grained OS accounting for the process.
///
/// Filled from `task_info(TASK_VM_INFO)`'s ledgers, which supply the resident
/// anonymous/file-backed split and the compressor footprint. The proportional
/// (`pss_*`) figures, the clean/dirty splits, `rss_shmem_bytes` and
/// `anon_huge_pages_bytes` are Linux `smaps_rollup` concepts with no macOS
/// per-task equivalent and stay `None`; their docs below describe the intended
/// meaning for the consumers that fall back to the fields that are populated.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OsProcessMemoryInfo {
    pub pss_bytes: Option<u64>,
    /// Proportional set size of anonymous mappings: heap + thread stacks +
    /// other private anon memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pss_anon_bytes: Option<u64>,
    /// Proportional set size of file-backed mappings (`Pss_File:`): mostly
    /// the executable text/rodata and shared libraries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pss_file_bytes: Option<u64>,
    /// Proportional set size of shmem mappings (`Pss_Shmem:`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pss_shmem_bytes: Option<u64>,
    /// Bytes backed by transparent huge pages (`AnonHugePages:`); a subset of
    /// anon memory that amplifies allocator retention (one live allocation
    /// pins a whole 2MB page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anon_huge_pages_bytes: Option<u64>,
    /// Resident anonymous bytes, from `TASK_VM_INFO`'s `internal + reusable`
    /// ledgers: live heap, thread stacks, private mappings, plus heap pages
    /// libmalloc has freed that are still mapped and resident. Together with
    /// `rss_file_bytes` this sums exactly to
    /// `ProcessMemorySnapshot::rss_bytes`.
    pub rss_anon_bytes: Option<u64>,
    /// Resident file-backed bytes, from `TASK_VM_INFO`'s `external` ledger:
    /// executable text, dylibs and mapped files.
    pub rss_file_bytes: Option<u64>,
    /// Always `None` on macOS: the kernel folds shared memory into the
    /// `external` ledger rather than accounting for it separately.
    pub rss_shmem_bytes: Option<u64>,
    pub private_clean_bytes: Option<u64>,
    pub private_dirty_bytes: Option<u64>,
    pub shared_clean_bytes: Option<u64>,
    pub shared_dirty_bytes: Option<u64>,
    /// Bytes of this task's anonymous memory held by the VM compressor, from
    /// `TASK_VM_INFO`'s `compressed` ledger. macOS compresses rather than
    /// swapping, so this is the closest analogue to Linux's `VmSwap`; it
    /// counts compressor-resident bytes, not bytes written to a swap file.
    pub swap_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllocatorInfo {
    pub name: &'static str,
    pub stats_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<AllocatorStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tuning: Option<AllocatorTuningInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profiling: Option<AllocatorProfilingInfo>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AllocatorStats {
    pub allocated_bytes: Option<u64>,
    pub active_bytes: Option<u64>,
    pub metadata_bytes: Option<u64>,
    pub resident_bytes: Option<u64>,
    pub mapped_bytes: Option<u64>,
    pub retained_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AllocatorProfilingInfo {
    pub available: bool,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AllocatorTuningInfo {
    pub available: bool,
    pub background_thread: Option<bool>,
    pub max_background_threads: Option<u64>,
    pub arena_count: Option<u64>,
    pub initialized_arenas: Option<u64>,
    pub dirty_decay_ms: Option<i64>,
    pub muzzy_decay_ms: Option<i64>,
    pub retain: Option<bool>,
    pub tcache_enabled: Option<bool>,
    pub tcache_max_bytes: Option<u64>,
}

impl Default for AllocatorInfo {
    fn default() -> Self {
        allocator_info()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessMemoryHistoryEntry {
    pub timestamp_ms: u128,
    pub source: String,
    pub snapshot: ProcessMemorySnapshot,
}

static MEMORY_HISTORY: OnceLock<Mutex<VecDeque<ProcessMemoryHistoryEntry>>> = OnceLock::new();

fn memory_history() -> &'static Mutex<VecDeque<ProcessMemoryHistoryEntry>> {
    MEMORY_HISTORY.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_HISTORY_SAMPLES)))
}

/// Sample this process's memory usage from the kernel.
pub fn snapshot() -> ProcessMemorySnapshot {
    snapshot_with_source("snapshot")
}

/// Sample this process's memory usage, tagging the history entry with `source`.
///
/// Two kernel calls back this: `proc_pidinfo(PROC_PIDTASKINFO)` for resident
/// size, virtual size and thread count, and `task_info(TASK_VM_INFO)` for the
/// kernel-tracked resident high-water mark and the anonymous/file-backed/
/// compressed breakdown. A field whose reader fails stays `None`; nothing here
/// substitutes an estimate.
pub fn snapshot_with_source(source: impl Into<String>) -> ProcessMemorySnapshot {
    let source = source.into();
    let task = proc_task_info();
    let vm = task_vm_info();
    let snapshot = ProcessMemorySnapshot {
        rss_bytes: task.map(|task| task.pti_resident_size),
        peak_rss_bytes: vm.as_ref().map(|vm| vm.resident_size_peak),
        virtual_bytes: task.map(|task| task.pti_virtual_size),
        thread_count: match task {
            // The kernel reports this as a signed count; a non-positive value
            // would mean the read did not land, not that the task has no
            // threads, so it is dropped rather than reported as a number.
            Some(task) if task.pti_threadnum > 0 => Some(task.pti_threadnum as u64),
            _ => None,
        },
        main_stack_bytes: main_thread_stack_bytes(),
        os: vm.as_ref().map(os_memory_info),
        allocator: allocator_info(),
    };
    record_snapshot(source, snapshot.clone());
    snapshot
}

/// This process's resident set size in bytes: `pti_resident_size` from
/// `proc_pidinfo(PROC_PIDTASKINFO)`, the same kernel read that fills
/// [`ProcessMemorySnapshot::rss_bytes`].
///
/// Side-effect-free and allocation-free, which is what separates it from
/// [`snapshot`]: that also reads the `TASK_VM_INFO` ledgers and the allocator
/// statistics, and records an entry in the process-global history ring.
/// Callers running on a frame cadence — the TUI's per-frame resource
/// attribution — want this one number and none of that per-frame cost.
pub fn resident_bytes() -> Option<u64> {
    proc_task_info().map(|task| task.pti_resident_size)
}

// Note for future callers: `proc_taskinfo` also carries `pti_total_user` and
// `pti_total_system`, but those two accumulate only the CPU time of *exited*
// threads on macOS — measured here, they advanced 3.6 ms across a 150 ms
// single-thread busy loop. They are therefore not a process CPU-time reader,
// and nothing in this repo uses them; `getrusage(RUSAGE_SELF)` is
// (`jcode-tui`'s per-frame resource attribution).

// ---------------------------------------------------------------------------
// macOS kernel readers.
//
// `libc` exposes `proc_pidinfo`, `proc_taskinfo` and `PROC_PIDTASKINFO`, so
// those come from the crate. It does not expose the `TASK_VM_INFO` flavor or
// its payload struct, so those are hand-declared below in the same style
// `jcode-core`'s `stdin_detect` uses for `proc_fdinfo`/`PROC_PIDLISTFDS`.
// ---------------------------------------------------------------------------

/// `TASK_VM_INFO` from `<mach/task_info.h>`.
const TASK_VM_INFO: libc::task_flavor_t = 22;

/// `TASK_VM_INFO_REV0_COUNT`: [`TaskVmInfoRev0`]'s size in 32-bit words, and
/// the smallest count `task_info` accepts for this flavor.
const TASK_VM_INFO_REV0_COUNT: libc::mach_msg_type_number_t =
    (std::mem::size_of::<TaskVmInfoRev0>() / std::mem::size_of::<libc::integer_t>())
        as libc::mach_msg_type_number_t;

/// `struct task_vm_info` from `<mach/task_info.h>`, truncated to its
/// `TASK_VM_INFO_REV0` prefix: the fields present since OS X 10.9 and the
/// minimum the kernel will fill for this flavor. Later revisions append
/// `phys_footprint`, the task address range and a long tail of ledger
/// counters; none are requested here, so this crate is not exposed to their
/// churn.
///
/// `#[repr(C)]` layout: `mach_vm_size_t` is `u64` and `integer_t` is `i32`, so
/// the two `i32` fields share the 8-byte slot after `virtual_size` and the
/// struct is 144 bytes = 36 32-bit words. Verified against the macOS 15 SDK
/// header with `offsetof`: `resident_size` 16, `resident_size_peak` 24,
/// `internal` 48, `external` 64, `compressed` 120.
///
/// Only five fields are read; the rest are part of the kernel's ABI for this
/// flavor and must stay declared for the layout to line up, hence the
/// `dead_code` allowance.
#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct TaskVmInfoRev0 {
    virtual_size: u64,
    region_count: i32,
    page_size: i32,
    resident_size: u64,
    /// High-water resident size since task creation, tracked by the kernel.
    /// This is macOS's analogue of Linux's `VmHWM`.
    resident_size_peak: u64,
    device: u64,
    device_peak: u64,
    /// Resident bytes on the task's "internal" ledger: anonymous private
    /// memory that is still in use — live heap, thread stacks and private
    /// mappings. Freed-but-resident heap pages move to `reusable`, so this is
    /// not the whole anonymous resident set on its own.
    internal: u64,
    internal_peak: u64,
    /// Resident bytes on the "external" ledger: file-backed and shared pages,
    /// i.e. executable text, dylibs and mapped files.
    external: u64,
    external_peak: u64,
    /// Resident anonymous bytes libmalloc has released with
    /// `MADV_FREE_REUSABLE`: still mapped and still resident, reclaimed by
    /// the kernel on demand or by a pressure-relief call. The macOS analogue
    /// of jemalloc's dirty pages.
    reusable: u64,
    reusable_peak: u64,
    purgeable_volatile_pmap: u64,
    purgeable_volatile_resident: u64,
    purgeable_volatile_virtual: u64,
    /// Bytes of this task's anonymous memory currently held by the VM
    /// compressor: macOS compresses instead of swapping, so this is the
    /// closest thing to Linux's `VmSwap`.
    compressed: u64,
    compressed_peak: u64,
    compressed_lifetime: u64,
}

/// Read `proc_taskinfo` for this process.
fn proc_task_info() -> Option<libc::proc_taskinfo> {
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // Safety: `proc_taskinfo` is a plain-data `#[repr(C)]` struct of integers,
    // so an all-zero bit pattern is a valid inhabitant.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    // Safety: the buffer pointer addresses a live local of exactly `size`
    // bytes, which is the length passed to the kernel; `proc_pidinfo` writes
    // only through it and retains nothing after returning.
    let written = unsafe {
        libc::proc_pidinfo(
            std::process::id() as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut libc::proc_taskinfo as *mut libc::c_void,
            size,
        )
    };
    // A short write means the kernel filled a different struct than the one
    // declared here; reporting nothing beats reporting half a struct.
    (written == size).then_some(info)
}

unsafe extern "C" {
    /// This process's own task port. `libc`'s `mach_task_self()` accessor is
    /// deprecated in favour of the `mach2` crate, and the C macro of the same
    /// name expands to exactly this global, so declaring the symbol keeps the
    /// reader on `libc` without adding a dependency.
    static mach_task_self_: libc::mach_port_t;
}

/// Read the `TASK_VM_INFO` ledgers for this task.
fn task_vm_info() -> Option<TaskVmInfoRev0> {
    let mut info = TaskVmInfoRev0::default();
    let mut count = TASK_VM_INFO_REV0_COUNT;
    // Safety: `mach_task_self_` is written by dyld before `main` and never
    // mutated afterwards, so reading it is a plain load of an initialised
    // `mach_port_t`. `task_info` writes at most `count` 32-bit words through
    // the info pointer, which addresses a live `TaskVmInfoRev0` of exactly
    // that many words, and updates `count` in place; both pointers outlive
    // the call and neither is retained.
    let status = unsafe {
        libc::task_info(
            mach_task_self_,
            TASK_VM_INFO,
            &mut info as *mut TaskVmInfoRev0 as libc::task_info_t,
            &mut count,
        )
    };
    (status == libc::KERN_SUCCESS && count >= TASK_VM_INFO_REV0_COUNT).then_some(info)
}

/// Map the `TASK_VM_INFO` ledgers onto the fields macOS can actually supply.
///
/// Resident anonymous memory is `internal + reusable`, not `internal` alone.
/// When libmalloc frees a large block it marks the pages `MADV_FREE_REUSABLE`,
/// which moves them off the internal ledger while they stay mapped and
/// resident, so `internal` alone undercounts by exactly the freed-but-resident
/// heap this module exists to watch. The kernel keeps
/// `internal + external + reusable == resident_size` exactly (verified across
/// live/freed/purged states on macOS 15), so the two reported halves add up to
/// [`ProcessMemorySnapshot::rss_bytes`].
///
/// The proportional figures (`pss_*`), the clean/dirty splits, `rss_shmem` and
/// `anon_huge_pages` stay `None`: they come from Linux's `smaps_rollup`, and
/// macOS has no per-task equivalent. Approximating them would need a
/// `mach_vm_region_recurse` walk of every mapping, which is neither cheap nor
/// a proportional accounting, and transparent huge pages do not exist here at
/// all.
fn os_memory_info(vm: &TaskVmInfoRev0) -> OsProcessMemoryInfo {
    OsProcessMemoryInfo {
        rss_anon_bytes: Some(vm.internal.saturating_add(vm.reusable)),
        rss_file_bytes: Some(vm.external),
        swap_bytes: Some(vm.compressed),
        ..OsProcessMemoryInfo::default()
    }
}

/// The main thread's stack reservation, from `getrlimit(RLIMIT_STACK)`.
///
/// macOS sizes the main thread's stack mapping at exec from `RLIMIT_STACK`'s
/// soft limit, so this is that mapping's size: on the main thread
/// `pthread_get_stacksize_np` returns exactly `rlim_cur` (8372224 by default,
/// verified on macOS 15). It is a reservation, not a high-water usage figure,
/// and auxiliary thread stacks are not included — `thread_stack_estimate` in
/// `runtime_memory_log` extrapolates those from `thread_count`.
///
/// An unlimited or zero soft limit yields `None` rather than a nonsense size.
fn main_thread_stack_bytes() -> Option<u64> {
    // Safety: `rlimit` is two integers, so zero is a valid inhabitant.
    let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
    // Safety: `getrlimit` writes one `rlimit` through the pointer, which
    // addresses a live, correctly sized local and is not retained.
    let status = unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit as *mut libc::rlimit) };
    if status != 0 || limit.rlim_cur == 0 || limit.rlim_cur == libc::RLIM_INFINITY {
        return None;
    }
    Some(limit.rlim_cur)
}

pub fn history(limit: usize) -> Vec<ProcessMemoryHistoryEntry> {
    let Ok(history) = memory_history().lock() else {
        logging::error("process memory history lock poisoned; returning empty history");
        return Vec::new();
    };
    history.iter().rev().take(limit).cloned().collect()
}

pub fn allocator_info() -> AllocatorInfo {
    #[cfg(feature = "jemalloc")]
    {
        let stats = jemalloc_stats();
        let profiling = jemalloc_profiling_info();
        AllocatorInfo {
            name: "jemalloc",
            stats_available: stats.is_some(),
            stats,
            tuning: jemalloc_tuning_info(),
            profiling,
        }
    }

    #[cfg(not(feature = "jemalloc"))]
    {
        let stats = system_malloc_stats();
        AllocatorInfo {
            name: "system",
            stats_available: stats.is_some(),
            stats,
            tuning: None,
            profiling: None,
        }
    }
}

/// `struct malloc_statistics_t` from `<malloc/malloc.h>`, hand-declared for
/// the same reason as [`TaskVmInfoRev0`]: `libc` does not bind libmalloc's
/// introspection API.
///
/// `#[repr(C)]` layout: `unsigned` followed by three `size_t`, so on 64-bit
/// targets four bytes of padding follow `blocks_in_use` and the struct is 32
/// bytes. Verified against the macOS 15 SDK header with `offsetof`:
/// `size_in_use` 8, `max_size_in_use` 16, `size_allocated` 24.
#[cfg(not(feature = "jemalloc"))]
#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct MallocStatistics {
    blocks_in_use: libc::c_uint,
    /// Bytes of live (allocated, not yet freed) blocks.
    size_in_use: libc::size_t,
    /// High-water mark of `size_in_use`.
    max_size_in_use: libc::size_t,
    /// Bytes the zones have mapped from the kernel to back those blocks.
    size_allocated: libc::size_t,
}

#[cfg(not(feature = "jemalloc"))]
unsafe extern "C" {
    /// libmalloc introspection. A null `zone` aggregates every registered
    /// malloc zone, which is what a whole-process figure needs: macOS runs
    /// several (nano, scalable, and any zone a dylib creates).
    fn malloc_zone_statistics(zone: *mut libc::c_void, stats: *mut MallocStatistics);
    /// Ask libmalloc to hand freed pages back to the kernel. A null `zone`
    /// covers every zone and `goal` 0 means "as much as possible". The return
    /// value is only meaningful for a nonzero `goal`, so it is ignored here;
    /// the effect is visible in `TASK_VM_INFO`'s `reusable` ledger dropping
    /// to zero.
    fn malloc_zone_pressure_relief(zone: *mut libc::c_void, goal: libc::size_t) -> libc::size_t;
}

/// Live and mapped byte counts for the macOS system allocator.
///
/// `active_bytes`, `resident_bytes` and `retained_bytes` have no libmalloc
/// equivalent (they are jemalloc arena concepts) and stay `None`; the
/// jemalloc build is the one that fills them.
#[cfg(not(feature = "jemalloc"))]
fn system_malloc_stats() -> Option<AllocatorStats> {
    let mut stats = MallocStatistics::default();
    // Safety: a null zone pointer is libmalloc's documented "all zones"
    // selector, and the out-pointer addresses a live, correctly sized
    // `MallocStatistics` that the call writes through and does not retain.
    unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut stats as *mut MallocStatistics) };
    // libmalloc reports through a void function, so an all-zero result is the
    // only failure signal there is: a live process always has zones with
    // mapped bytes, so zero means the read produced nothing.
    if stats.size_allocated == 0 {
        return None;
    }
    Some(AllocatorStats {
        allocated_bytes: Some(stats.size_in_use as u64),
        mapped_bytes: Some(stats.size_allocated as u64),
        ..AllocatorStats::default()
    })
}

pub fn purge_allocator() -> Result<AllocatorTuningInfo> {
    #[cfg(feature = "jemalloc")]
    {
        logging::info("purging jemalloc allocator arenas");
        let _ = jemalloc_void_ctl("thread.idle");
        let arena_count = tikv_jemalloc_ctl::arenas::narenas::read()
            .map_err(|e| anyhow!("failed to read jemalloc arena count: {}", e))?;
        let mut initialized_arenas = 0u64;
        for arena_idx in 0..arena_count {
            if jemalloc_read_dynamic::<bool>(&format!("arena.{arena_idx}.initialized"))
                .unwrap_or(false)
            {
                initialized_arenas += 1;
                jemalloc_void_ctl(&format!("arena.{arena_idx}.purge"))?;
            }
        }

        Ok(jemalloc_tuning_info().unwrap_or(AllocatorTuningInfo {
            available: true,
            initialized_arenas: Some(initialized_arenas),
            ..AllocatorTuningInfo::default()
        }))
    }

    #[cfg(not(feature = "jemalloc"))]
    {
        // libmalloc has no arenas or decay tunables to report, but it does
        // expose a pressure valve, and on macOS that is where the recoverable
        // memory sits: freeing a large block leaves its pages mapped and
        // resident on the `reusable` ledger until something reclaims them.
        // Measured on macOS 15: 128 MiB freed then relieved dropped resident
        // size from 135 MB to 1.3 MB.
        logging::info("relieving libmalloc memory pressure across all zones");
        // Safety: a null zone pointer is libmalloc's documented "all zones"
        // selector and the call takes no out-pointers.
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        Ok(AllocatorTuningInfo {
            // libmalloc exposes no tunables, so there is nothing to report
            // beyond the purge having run.
            available: false,
            ..AllocatorTuningInfo::default()
        })
    }
}

pub fn set_allocator_decay_ms(dirty_ms: isize, muzzy_ms: isize) -> Result<AllocatorTuningInfo> {
    logging::info(&format!(
        "setting allocator decay dirty_ms={dirty_ms} muzzy_ms={muzzy_ms}"
    ));
    #[cfg(feature = "jemalloc")]
    {
        unsafe {
            tikv_jemalloc_ctl::raw::write(b"arenas.dirty_decay_ms\0", dirty_ms)
                .map_err(|e| anyhow!("failed to update arenas.dirty_decay_ms: {}", e))?;
            tikv_jemalloc_ctl::raw::write(b"arenas.muzzy_decay_ms\0", muzzy_ms)
                .map_err(|e| anyhow!("failed to update arenas.muzzy_decay_ms: {}", e))?;
        }

        let arena_count = tikv_jemalloc_ctl::arenas::narenas::read()
            .map_err(|e| anyhow!("failed to read jemalloc arena count: {}", e))?;
        for arena_idx in 0..arena_count {
            if jemalloc_read_dynamic::<bool>(&format!("arena.{arena_idx}.initialized"))
                .unwrap_or(false)
            {
                jemalloc_write_dynamic(&format!("arena.{arena_idx}.dirty_decay_ms"), dirty_ms)?;
                jemalloc_write_dynamic(&format!("arena.{arena_idx}.muzzy_decay_ms"), muzzy_ms)?;
            }
        }

        Ok(jemalloc_tuning_info().unwrap_or(AllocatorTuningInfo {
            available: true,
            dirty_decay_ms: Some(dirty_ms as i64),
            muzzy_decay_ms: Some(muzzy_ms as i64),
            ..AllocatorTuningInfo::default()
        }))
    }

    #[cfg(not(feature = "jemalloc"))]
    {
        let _ = (dirty_ms, muzzy_ms);
        logging::warn("allocator decay update requested but jemalloc feature is disabled");
        Err(anyhow!(
            "allocator decay controls unavailable: rebuild with --features jemalloc"
        ))
    }
}

pub fn set_allocator_profiling_active(active: bool) -> Result<()> {
    #[cfg(feature = "jemalloc-prof")]
    {
        unsafe {
            tikv_jemalloc_ctl::raw::write(b"prof.active\0", active)
                .map_err(|e| anyhow!("failed to update jemalloc prof.active: {}", e))
        }
    }

    #[cfg(not(feature = "jemalloc-prof"))]
    {
        let _ = active;
        Err(anyhow!(
            "jemalloc profiling controls unavailable: rebuild with --features jemalloc-prof"
        ))
    }
}

pub fn dump_allocator_profile(path: Option<&Path>) -> Result<PathBuf> {
    #[cfg(feature = "jemalloc-prof")]
    {
        let output_path = match path {
            Some(path) => path.to_path_buf(),
            None => default_heap_profile_path()?,
        };

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let c_path = CString::new(output_path.to_string_lossy().as_bytes())
            .map_err(|_| anyhow!("heap profile path contains NUL byte"))?;

        unsafe {
            tikv_jemalloc_ctl::raw::write(b"prof.dump\0", c_path.as_ptr())
                .map_err(|e| anyhow!("failed to dump jemalloc heap profile: {}", e))?;
        }

        Ok(output_path)
    }

    #[cfg(not(feature = "jemalloc-prof"))]
    {
        let _ = path;
        Err(anyhow!(
            "jemalloc heap dumps unavailable: rebuild with --features jemalloc-prof"
        ))
    }
}

pub fn set_allocator_profile_prefix(prefix: &str) -> Result<()> {
    #[cfg(feature = "jemalloc-prof")]
    {
        let c_prefix =
            CString::new(prefix).map_err(|_| anyhow!("heap profile prefix contains NUL byte"))?;
        unsafe {
            tikv_jemalloc_ctl::raw::write(b"prof.prefix\0", c_prefix.as_ptr())
                .map_err(|e| anyhow!("failed to update jemalloc prof.prefix: {}", e))
        }
    }

    #[cfg(not(feature = "jemalloc-prof"))]
    {
        let _ = prefix;
        Err(anyhow!(
            "jemalloc heap profiling unavailable: rebuild with --features jemalloc-prof"
        ))
    }
}

pub fn estimate_json_bytes<T: Serialize>(value: &T) -> usize {
    #[derive(Default)]
    struct ByteCounter {
        bytes: usize,
    }

    impl std::io::Write for ByteCounter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(buffer.len());
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, value)
        .map(|()| counter.bytes)
        .unwrap_or(0)
}

/// Return freed-but-retained heap pages to the OS.
///
/// Both allocators have a real mechanism: jemalloc purges every initialised
/// arena, libmalloc relieves pressure across every zone. See
/// [`purge_allocator`].
pub fn release_retained_heap(reason: &str) {
    if let Err(err) = purge_allocator() {
        logging::info(&format!("allocator purge ({reason}) failed: {err}"));
    } else {
        logging::debug(&format!("allocator purge ({reason}) completed"));
    }

    // Whatever apparent retention remains after the release is the
    // unrecoverable floor (fragmentation residual); measure future growth
    // from it so retention-triggered callers stay quiet at steady state.
    record_post_trim_retention_baseline();
}

static LAST_HEAP_RELEASE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Debounced [`release_retained_heap`]: skips the release when one already ran
/// within `min_interval`. Returns true when a release was performed.
pub fn release_retained_heap_debounced(reason: &str, min_interval: std::time::Duration) -> bool {
    use std::sync::atomic::Ordering;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let last_ms = LAST_HEAP_RELEASE_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last_ms) < min_interval.as_millis() as u64 {
        return false;
    }
    if LAST_HEAP_RELEASE_MS
        .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return false;
    }
    release_retained_heap(reason);
    true
}

/// Default apparent-retention growth threshold that triggers a background trim.
pub const DEFAULT_RETENTION_TRIM_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;

/// Post-trim apparent-retention baseline (bytes). Updated after every
/// [`release_retained_heap`] and ratcheted down when current apparent
/// retention falls below it, so growth is always measured from the floor.
static POST_TRIM_APPARENT_RETENTION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Resident anonymous memory not accounted for by live allocator bytes:
/// freed-but-still-resident heap pages plus fragmentation overhead. This is
/// the memory a trim/purge could plausibly return to the OS, measured from the
/// OS side (resident anonymous bytes) minus the allocator's live bytes.
///
/// The OS side is `TASK_VM_INFO`'s `internal + reusable` ledgers, i.e. every
/// anonymous resident page. `reusable` is where libmalloc parks large freed
/// blocks — mapped and resident until something reclaims them — so it is the
/// recoverable part; `internal` also covers thread stacks and private
/// mappings, so this figure carries a non-heap floor. That is why callers
/// compare *growth above a post-trim baseline* rather than the absolute
/// value.
///
/// `None` when either half is unreadable: no `task_info` result, or an
/// allocator that reports no live-bytes figure.
fn apparent_heap_retention_bytes() -> Option<u64> {
    let vm = task_vm_info()?;
    let resident_anon = vm.internal.saturating_add(vm.reusable);
    let live_bytes = allocator_info().stats?.allocated_bytes?;
    Some(resident_anon.saturating_sub(live_bytes))
}

/// Refresh the post-trim baseline from the current apparent retention.
fn record_post_trim_retention_baseline() {
    if let Some(apparent) = apparent_heap_retention_bytes() {
        POST_TRIM_APPARENT_RETENTION.store(apparent, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Pure trigger decision for retention-based trimming: has apparent retention
/// grown at least `threshold` bytes above the post-trim `baseline`?
fn retention_growth_exceeds(apparent: u64, baseline: u64, threshold: u64) -> bool {
    apparent.saturating_sub(baseline) >= threshold
}

/// Release retained heap when apparent retention (resident anon minus live
/// allocator bytes) has grown at least `threshold_bytes` above the post-trim
/// baseline. Intended for periodic (heartbeat) callers: cheap when below
/// threshold (one allocator stats read), debounced against other release
/// paths when above it. Returns true when a release ran.
///
/// When [`apparent_heap_retention_bytes`] cannot read one of its two halves,
/// this falls back to an absolute threshold on the allocator's own retained
/// counter, which only the jemalloc build reports.
///
/// This closes the gap left by event-driven trims (turn completion, history
/// load): a server hosting many mostly-idle sessions can accumulate hundreds
/// of MB of freed-but-resident pages without ever hitting those event hooks.
/// Measuring *growth above the post-trim floor* keeps the watchdog quiet at
/// steady state: the unrecoverable fragmentation residual left after a trim
/// becomes the new baseline instead of re-triggering every cycle.
pub fn release_retained_heap_if_excessive(
    reason: &str,
    threshold_bytes: u64,
    min_interval: std::time::Duration,
) -> bool {
    use std::sync::atomic::Ordering;

    let Some(apparent) = apparent_heap_retention_bytes() else {
        // No OS-side metric available: fall back to the allocator-reported
        // retained counter as an absolute threshold. Coarse, but better than
        // never trimming.
        let retained = allocator_info()
            .stats
            .and_then(|stats| stats.retained_bytes)
            .unwrap_or(0);
        if retained < threshold_bytes {
            return false;
        }
        return release_retained_heap_debounced(reason, min_interval);
    };

    // Ratchet the baseline down so growth is measured from the true floor
    // (e.g. after freed pages get reused into live memory).
    let mut baseline = POST_TRIM_APPARENT_RETENTION.load(Ordering::Relaxed);
    if apparent < baseline {
        POST_TRIM_APPARENT_RETENTION.store(apparent, Ordering::Relaxed);
        baseline = apparent;
    }

    if !retention_growth_exceeds(apparent, baseline, threshold_bytes) {
        return false;
    }

    let released = release_retained_heap_debounced(reason, min_interval);
    if released {
        let after = apparent_heap_retention_bytes().unwrap_or(apparent);
        logging::info(&format!(
            "retained-heap trim ({reason}): apparent retention {} MB grew {} MB above post-trim baseline {} MB (threshold {} MB); recovered ~{} MB",
            apparent / (1024 * 1024),
            (apparent - baseline) / (1024 * 1024),
            baseline / (1024 * 1024),
            threshold_bytes / (1024 * 1024),
            apparent.saturating_sub(after) / (1024 * 1024),
        ));
    }
    released
}

/// Retention trim threshold in bytes, from `JCODE_HEAP_RETENTION_TRIM_MB`
/// (in MiB), falling back to [`DEFAULT_RETENTION_TRIM_THRESHOLD_BYTES`].
/// `0` disables retention-triggered trimming (returns `u64::MAX`).
pub fn retention_trim_threshold_bytes() -> u64 {
    parse_retention_trim_threshold(
        std::env::var("JCODE_HEAP_RETENTION_TRIM_MB")
            .ok()
            .as_deref(),
    )
}

fn parse_retention_trim_threshold(value: Option<&str>) -> u64 {
    match value.and_then(|value| value.trim().parse::<u64>().ok()) {
        Some(0) => u64::MAX,
        Some(mb) => mb.saturating_mul(1024 * 1024),
        None => DEFAULT_RETENTION_TRIM_THRESHOLD_BYTES,
    }
}

fn record_snapshot(source: String, snapshot: ProcessMemorySnapshot) {
    let Ok(mut history) = memory_history().lock() else {
        logging::error("process memory history lock poisoned; dropping snapshot");
        return;
    };
    if history.len() >= MAX_HISTORY_SAMPLES {
        logging::debug("process memory history full; dropping oldest snapshot");
        history.pop_front();
    }
    history.push_back(ProcessMemoryHistoryEntry {
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        source,
        snapshot,
    });
}

#[cfg(feature = "jemalloc-prof")]
fn default_heap_profile_path() -> Result<PathBuf> {
    let base = crate::storage::jcode_dir()?.join("profiles").join("heap");
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let pid = std::process::id();
    Ok(base.join(format!("jcode-{}-{}.heap", pid, timestamp)))
}

#[cfg(feature = "jemalloc")]
fn jemalloc_stats() -> Option<AllocatorStats> {
    let mibs = jemalloc_stats_mibs()?;
    mibs.epoch.advance().ok()?;

    Some(AllocatorStats {
        allocated_bytes: mibs.allocated.read().ok().map(|value| value as u64),
        active_bytes: mibs.active.read().ok().map(|value| value as u64),
        metadata_bytes: mibs.metadata.read().ok().map(|value| value as u64),
        resident_bytes: mibs.resident.read().ok().map(|value| value as u64),
        mapped_bytes: mibs.mapped.read().ok().map(|value| value as u64),
        retained_bytes: mibs.retained.read().ok().map(|value| value as u64),
    })
}

#[cfg(feature = "jemalloc")]
fn jemalloc_tuning_info() -> Option<AllocatorTuningInfo> {
    let arena_count = tikv_jemalloc_ctl::arenas::narenas::read().ok()?;
    let mut initialized_arenas = 0u64;
    for arena_idx in 0..arena_count {
        if jemalloc_read_dynamic::<bool>(&format!("arena.{arena_idx}.initialized")).unwrap_or(false)
        {
            initialized_arenas += 1;
        }
    }

    Some(AllocatorTuningInfo {
        available: true,
        background_thread: tikv_jemalloc_ctl::background_thread::read().ok(),
        max_background_threads: tikv_jemalloc_ctl::max_background_threads::read()
            .ok()
            .map(|value| value as u64),
        arena_count: Some(arena_count as u64),
        initialized_arenas: Some(initialized_arenas),
        dirty_decay_ms: unsafe {
            tikv_jemalloc_ctl::raw::read::<isize>(b"arenas.dirty_decay_ms\0")
        }
        .ok()
        .map(|value| value as i64),
        muzzy_decay_ms: unsafe {
            tikv_jemalloc_ctl::raw::read::<isize>(b"arenas.muzzy_decay_ms\0")
        }
        .ok()
        .map(|value| value as i64),
        retain: unsafe { tikv_jemalloc_ctl::raw::read::<bool>(b"opt.retain\0") }.ok(),
        tcache_enabled: unsafe { tikv_jemalloc_ctl::raw::read::<bool>(b"opt.tcache\0") }.ok(),
        tcache_max_bytes: unsafe { tikv_jemalloc_ctl::raw::read::<usize>(b"arenas.tcache_max\0") }
            .ok()
            .map(|value| value as u64),
    })
}

#[cfg(feature = "jemalloc")]
fn jemalloc_read_dynamic<T: Copy>(name: &str) -> Result<T> {
    let c_name = CString::new(name).map_err(|_| anyhow!("mallctl name contains NUL byte"))?;
    unsafe {
        tikv_jemalloc_ctl::raw::read(c_name.as_bytes_with_nul())
            .map_err(|e| anyhow!("failed to read jemalloc mallctl {}: {}", name, e))
    }
}

#[cfg(feature = "jemalloc")]
fn jemalloc_write_dynamic<T>(name: &str, value: T) -> Result<()> {
    let c_name = CString::new(name).map_err(|_| anyhow!("mallctl name contains NUL byte"))?;
    unsafe {
        tikv_jemalloc_ctl::raw::write(c_name.as_bytes_with_nul(), value)
            .map_err(|e| anyhow!("failed to update jemalloc mallctl {}: {}", name, e))
    }
}

#[cfg(feature = "jemalloc")]
fn jemalloc_void_ctl(name: &str) -> Result<()> {
    let c_name = CString::new(name).map_err(|_| anyhow!("mallctl name contains NUL byte"))?;
    unsafe {
        let err = tikv_jemalloc_sys::mallctl(
            c_name.as_ptr() as *const c_char,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
        if err != 0 {
            return Err(anyhow!(
                "failed to invoke jemalloc mallctl {}: {}",
                name,
                err
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "jemalloc")]
fn jemalloc_stats_mibs() -> Option<&'static JemallocStatsMibs> {
    static MIBS: OnceLock<Option<JemallocStatsMibs>> = OnceLock::new();
    MIBS.get_or_init(|| {
        Some(JemallocStatsMibs {
            epoch: tikv_jemalloc_ctl::epoch::mib().ok()?,
            allocated: tikv_jemalloc_ctl::stats::allocated::mib().ok()?,
            active: tikv_jemalloc_ctl::stats::active::mib().ok()?,
            metadata: tikv_jemalloc_ctl::stats::metadata::mib().ok()?,
            resident: tikv_jemalloc_ctl::stats::resident::mib().ok()?,
            mapped: tikv_jemalloc_ctl::stats::mapped::mib().ok()?,
            retained: tikv_jemalloc_ctl::stats::retained::mib().ok()?,
        })
    })
    .as_ref()
}

#[cfg(feature = "jemalloc-prof")]
fn jemalloc_profiling_info() -> Option<AllocatorProfilingInfo> {
    let mibs = jemalloc_profiling_mibs()?;
    Some(AllocatorProfilingInfo {
        available: true,
        enabled: mibs.enabled.read().ok(),
    })
}

#[cfg(all(feature = "jemalloc", not(feature = "jemalloc-prof")))]
fn jemalloc_profiling_info() -> Option<AllocatorProfilingInfo> {
    Some(AllocatorProfilingInfo {
        available: false,
        enabled: None,
    })
}

#[cfg(feature = "jemalloc-prof")]
fn jemalloc_profiling_mibs() -> Option<&'static JemallocProfilingMibs> {
    static MIBS: OnceLock<Option<JemallocProfilingMibs>> = OnceLock::new();
    MIBS.get_or_init(|| {
        Some(JemallocProfilingMibs {
            enabled: tikv_jemalloc_ctl::profiling::prof::mib().ok()?,
        })
    })
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_json_bytes_matches_serialized_length_without_buffering_output() {
        let value = serde_json::json!({
            "escaped": "line one\nline two\t\"quoted\"",
            "unicode": "RAM profile 🦊",
            "nested": [null, true, -42, {"payload": "x".repeat(64 * 1024)}],
        });

        assert_eq!(
            estimate_json_bytes(&value),
            serde_json::to_vec(&value)
                .expect("serialize reference value")
                .len()
        );
    }

    #[test]
    fn estimate_json_bytes_returns_zero_when_serialization_fails() {
        struct FailingSerialize;

        impl serde::Serialize for FailingSerialize {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("intentional failure"))
            }
        }

        assert_eq!(estimate_json_bytes(&FailingSerialize), 0);
    }

    #[test]
    fn release_retained_heap_is_safe_to_call() {
        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        // Allocate and drop a large transient buffer, then release. This must
        // not crash on any allocator configuration.
        let buffer = vec![0u8; 8 * 1024 * 1024];
        drop(buffer);
        release_retained_heap("unit_test");
    }

    #[test]
    fn release_retained_heap_debounced_skips_within_interval() {
        // First call resets the shared debounce clock; the immediate second
        // call within a long interval must be skipped.
        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        release_retained_heap_debounced("unit_test_first", std::time::Duration::ZERO);
        let ran = release_retained_heap_debounced(
            "unit_test_second",
            std::time::Duration::from_secs(3600),
        );
        assert!(
            !ran,
            "second call within debounce interval should be skipped"
        );
    }

    #[test]
    fn parse_retention_trim_threshold_handles_default_disable_and_values() {
        assert_eq!(
            parse_retention_trim_threshold(None),
            DEFAULT_RETENTION_TRIM_THRESHOLD_BYTES
        );
        assert_eq!(
            parse_retention_trim_threshold(Some("garbage")),
            DEFAULT_RETENTION_TRIM_THRESHOLD_BYTES
        );
        // 0 disables retention trimming entirely.
        assert_eq!(parse_retention_trim_threshold(Some("0")), u64::MAX);
        assert_eq!(
            parse_retention_trim_threshold(Some(" 128 ")),
            128 * 1024 * 1024
        );
    }

    #[test]
    fn release_retained_heap_if_excessive_skips_below_threshold() {
        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        // u64::MAX growth threshold can never be exceeded, so no release
        // should run regardless of current allocator state.
        let ran = release_retained_heap_if_excessive(
            "unit_test_below_threshold",
            u64::MAX,
            std::time::Duration::ZERO,
        );
        assert!(!ran, "release should not run below threshold");
    }

    #[test]
    fn retention_growth_trigger_measures_growth_above_baseline() {
        let mb = 1024 * 1024;
        // At or below baseline: no growth.
        assert!(!retention_growth_exceeds(100 * mb, 100 * mb, 64 * mb));
        assert!(!retention_growth_exceeds(50 * mb, 100 * mb, 64 * mb));
        // Growth below threshold stays quiet (the post-trim residual case).
        assert!(!retention_growth_exceeds(163 * mb, 100 * mb, 64 * mb));
        // Growth at/above threshold fires.
        assert!(retention_growth_exceeds(164 * mb, 100 * mb, 64 * mb));
        assert!(retention_growth_exceeds(300 * mb, 100 * mb, 64 * mb));
        // Threshold 0 always fires.
        assert!(retention_growth_exceeds(0, 0, 0));
    }

    #[test]
    fn allocator_info_matches_enabled_allocator_features() {
        let info = allocator_info();
        if cfg!(feature = "jemalloc") {
            assert_eq!(info.name, "jemalloc");
            assert_eq!(info.stats_available, info.stats.is_some());
            assert!(info.profiling.is_some());
        } else {
            assert_eq!(info.name, "system");
            assert_eq!(info.stats_available, info.stats.is_some());
            assert!(info.profiling.is_none());
        }
    }

    /// Serialises the tests that read or perturb process-global memory state:
    /// the retention baseline, the heap-release debounce clock, and the
    /// resident-size measurements a concurrent 128 MiB allocation would skew.
    static PROCESS_MEMORY_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Installed RAM (`hw.memsize`): an upper bound this process's own
    /// resident size cannot cross. Read through the shared host-metrics
    /// primitive so this file holds no second `sysctlbyname` copy.
    fn total_physical_memory_bytes() -> Option<u64> {
        crate::host_metrics::total_physical_memory_bytes()
    }

    #[test]
    fn snapshot_reports_resident_memory_that_tracks_real_allocations() {
        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        const BALLAST_BYTES: usize = 128 * 1024 * 1024;

        let total_physical = total_physical_memory_bytes().expect("hw.memsize is readable");
        let before = snapshot_with_source("unit_test_rss_before");
        let rss_before = before
            .rss_bytes
            .expect("PROC_PIDTASKINFO must report a resident size for this process");
        assert!(
            rss_before > 1024 * 1024,
            "a live Rust test process is resident in more than 1 MiB: {rss_before}"
        );
        assert!(
            rss_before < total_physical,
            "resident size {rss_before} cannot exceed installed RAM {total_physical}"
        );

        // Fault in 128 MiB of anonymous pages. `vec!` alone only reserves
        // zero-fill pages, so each page is touched to force it resident. A
        // reader returning a fabricated or constant value cannot follow this.
        let mut ballast = vec![0u8; BALLAST_BYTES];
        for page in ballast.chunks_mut(4096) {
            page[0] = 1;
        }
        let during = snapshot_with_source("unit_test_rss_during");
        std::hint::black_box(&ballast);
        drop(ballast);

        let rss_during = during
            .rss_bytes
            .expect("resident size while the ballast is live");
        assert!(
            rss_during >= rss_before + (BALLAST_BYTES as u64) * 3 / 4,
            "resident size must follow the {BALLAST_BYTES}-byte ballast: before={rss_before} during={rss_during}"
        );
        assert!(
            rss_during < total_physical,
            "resident size {rss_during} cannot exceed installed RAM {total_physical}"
        );

        let peak = during
            .peak_rss_bytes
            .expect("TASK_VM_INFO must report resident_size_peak");
        assert!(
            peak >= rss_during,
            "the kernel's resident high-water mark {peak} cannot be below the current resident size {rss_during}"
        );

        let virtual_bytes = during.virtual_bytes.expect("pti_virtual_size");
        assert!(
            virtual_bytes >= rss_during,
            "virtual size {virtual_bytes} must cover the resident set {rss_during}"
        );
    }

    #[test]
    fn resident_bytes_agrees_with_the_snapshot_and_records_no_history() {
        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        let total_physical = total_physical_memory_bytes().expect("hw.memsize is readable");

        let standalone = resident_bytes().expect("PROC_PIDTASKINFO reports a resident size");
        assert!(
            standalone > 1024 * 1024 && standalone < total_physical,
            "resident size {standalone} is outside 1 MiB..{total_physical}"
        );

        // Same kernel field as the snapshot's, so the two readings can only
        // differ by whatever the process allocated in between.
        let snapshot_rss = snapshot_with_source("unit_test_resident_bytes")
            .rss_bytes
            .expect("snapshot resident size");
        let drift = snapshot_rss.abs_diff(standalone);
        assert!(
            drift < 32 * 1024 * 1024,
            "resident_bytes {standalone} and snapshot {snapshot_rss} disagree by {drift} bytes"
        );

        // The whole reason this accessor exists: it must not touch the global
        // history ring, which the TUI would otherwise write to on a frame
        // cadence. The explicit snapshot above left its own entry at the head;
        // repeated accessor calls must leave that head untouched.
        let marker = history(1);
        let marker = marker
            .first()
            .expect("the explicit snapshot recorded an entry");
        assert_eq!(marker.source, "unit_test_resident_bytes");
        for _ in 0..5 {
            resident_bytes().expect("resident size");
        }
        let newest = history(1);
        let newest = newest.first().expect("history still holds the marker");
        assert_eq!(
            (newest.source.as_str(), newest.timestamp_ms),
            (marker.source.as_str(), marker.timestamp_ms),
            "resident_bytes must record nothing"
        );
    }

    #[test]
    fn task_vm_info_ledgers_partition_the_resident_set() {
        let vm = task_vm_info().expect("TASK_VM_INFO is readable for this task");
        // One syscall, so this identity is exact rather than sampled: the
        // kernel splits every resident page into internal (live anon),
        // external (file-backed) or reusable (freed anon still mapped). It is
        // also the check that catches a wrong field offset in the
        // hand-declared `TaskVmInfoRev0` layout.
        assert_eq!(
            vm.internal + vm.external + vm.reusable,
            vm.resident_size,
            "internal={} external={} reusable={} must partition resident_size={}",
            vm.internal,
            vm.external,
            vm.reusable,
            vm.resident_size
        );
        assert!(
            vm.resident_size_peak >= vm.resident_size,
            "the resident high-water mark {} cannot be below the current resident size {}",
            vm.resident_size_peak,
            vm.resident_size
        );
        assert!(
            vm.page_size == 4096 || vm.page_size == 16384,
            "page_size read as {}, which is not a macOS page size — the struct layout is wrong",
            vm.page_size
        );
    }

    #[test]
    fn snapshot_reports_thread_count_and_the_resident_anon_file_split() {
        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        use std::sync::{Arc, Barrier};

        // Nine participants: eight parked threads plus this one, so all eight
        // are provably alive when the snapshot is taken.
        let barrier = Arc::new(Barrier::new(9));
        let parked: Vec<_> = (0..8)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    barrier.wait();
                })
            })
            .collect();
        barrier.wait();

        let snapshot = snapshot_with_source("unit_test_threads");

        barrier.wait();
        for handle in parked {
            handle.join().expect("parked thread joins");
        }

        let threads = snapshot
            .thread_count
            .expect("PROC_PIDTASKINFO must report pti_threadnum");
        assert!(
            threads >= 9,
            "eight parked threads plus the measuring thread were alive, so the task had at least nine: {threads}"
        );

        let rss = snapshot.rss_bytes.expect("resident size");
        let os = snapshot
            .os
            .expect("TASK_VM_INFO must populate the os breakdown");
        let anon = os.rss_anon_bytes.expect("internal ledger");
        let file = os.rss_file_bytes.expect("external ledger");
        assert!(
            anon > 0 && file > 0,
            "a running process has both anonymous and file-backed resident pages: anon={anon} file={file}"
        );
        // `rss_anon_bytes + rss_file_bytes` is the whole resident set, but it
        // comes from a different syscall than `rss_bytes`, so a concurrent
        // allocation in another test can skew the two samples apart; the exact
        // identity is asserted from a single read in
        // `task_vm_info_ledgers_partition_the_resident_set`.
        let ledger_sum = anon + file;
        assert!(
            ledger_sum.abs_diff(rss) < rss / 2,
            "rss_anon+rss_file={ledger_sum} must account for the resident size {rss}"
        );
        assert!(
            os.swap_bytes.is_some(),
            "the compressor ledger is part of the same read and is always available"
        );
    }

    #[test]
    fn main_stack_bytes_reports_a_page_aligned_stack_reservation() {
        let main_stack =
            main_thread_stack_bytes().expect("RLIMIT_STACK has a finite soft limit on macOS");
        // macOS's default is 8 MiB minus a 16 KiB guard (8372224). A value
        // outside one page .. 1 GiB would mean the read landed elsewhere.
        assert!(
            (4096..=1024 * 1024 * 1024).contains(&main_stack),
            "implausible main-thread stack reservation: {main_stack}"
        );
        assert_eq!(
            main_stack % 4096,
            0,
            "a stack mapping is page-aligned; {main_stack} is not"
        );
    }

    #[test]
    fn apparent_heap_retention_measures_resident_anon_minus_live_allocator_bytes() {
        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        let live_bytes = allocator_info()
            .stats
            .expect("the allocator reports stats")
            .allocated_bytes
            .expect("live allocator bytes are readable");
        let anon = snapshot_with_source("unit_test_retention")
            .os
            .expect("os breakdown")
            .rss_anon_bytes
            .expect("resident anonymous bytes");
        let apparent = apparent_heap_retention_bytes()
            .expect("both halves of apparent retention are readable");

        assert!(live_bytes > 0, "a running process has live heap bytes");
        assert!(
            anon > live_bytes,
            "resident anonymous memory {anon} must exceed live heap bytes {live_bytes}"
        );
        assert!(
            apparent > 0 && apparent < anon,
            "apparent retention {apparent} is resident anon {anon} minus live bytes {live_bytes}"
        );
    }

    #[test]
    fn record_post_trim_retention_baseline_stores_a_measurement() {
        use std::sync::atomic::Ordering;

        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        let total_physical = total_physical_memory_bytes().expect("hw.memsize is readable");
        // Poison the baseline with a value no measurement can produce, so a
        // reader that reports nothing leaves it in place and this fails.
        POST_TRIM_APPARENT_RETENTION.store(u64::MAX, Ordering::Relaxed);
        record_post_trim_retention_baseline();

        let baseline = POST_TRIM_APPARENT_RETENTION.load(Ordering::Relaxed);
        assert_ne!(
            baseline,
            u64::MAX,
            "the baseline must be overwritten with a real measurement"
        );
        assert!(
            baseline > 0 && baseline < total_physical,
            "baseline {baseline} must be a plausible byte count below installed RAM {total_physical}"
        );
    }

    #[test]
    fn retention_growth_branch_releases_and_rebaselines() {
        use std::sync::atomic::Ordering;

        let _guard = PROCESS_MEMORY_TEST_LOCK
            .lock()
            .expect("process memory test lock");
        let apparent = apparent_heap_retention_bytes().expect("apparent retention is measurable");
        assert!(
            apparent > 1,
            "the growth branch needs measurable retention: {apparent}"
        );

        POST_TRIM_APPARENT_RETENTION.store(0, Ordering::Relaxed);
        // A one-byte threshold separates the two branches: the no-OS-metric
        // fallback compares the allocator's `retained_bytes`, which libmalloc
        // does not report (so it reads as 0 and stays below the threshold),
        // meaning only the growth branch can release here. `Duration::ZERO`
        // disables the debounce without touching its shared clock.
        let released = release_retained_heap_if_excessive(
            "unit_test_retention_growth",
            1,
            std::time::Duration::ZERO,
        );
        assert!(
            released,
            "growth of {apparent} bytes above a zero baseline must trigger a release"
        );
        assert!(
            POST_TRIM_APPARENT_RETENTION.load(Ordering::Relaxed) > 0,
            "the release must re-baseline from a fresh measurement"
        );
    }
}
