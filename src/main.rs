#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Tune jemalloc for a long-running server with bursty allocations (e.g. loading
// and unloading an ~87 MB ONNX embedding model). The defaults (muzzy_decay_ms:0,
// retain:true, narenas:8*ncpu) caused 1.4 GB RSS in previous testing.
//
// dirty_decay_ms:1000  — return dirty pages to OS after 1 s idle
// muzzy_decay_ms:1000  — release muzzy pages after 1 s
// narenas:4            — limit arena count (17 threads don't need 64 arenas)
// prof:true            — enable profiling support in jemalloc-prof builds
// prof_active:false    — keep sampling disabled until explicitly enabled at runtime
#[cfg(all(feature = "jemalloc", not(feature = "jemalloc-prof")))]
// jemalloc reads this exact exported symbol name at startup.
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static malloc_conf: Option<&'static [u8; 50]> =
    Some(b"dirty_decay_ms:1000,muzzy_decay_ms:1000,narenas:4\0");

#[cfg(feature = "jemalloc-prof")]
// jemalloc reads this exact exported symbol name at startup.
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static malloc_conf: Option<&'static [u8; 78]> =
    Some(b"dirty_decay_ms:1000,muzzy_decay_ms:1000,narenas:4,prof:true,prof_active:false\0");

use anyhow::Result;

fn configure_system_allocator() {}

fn main() -> Result<()> {
    run_main()
}

fn run_main() -> Result<()> {
    configure_system_allocator();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // The generated LSUIElement helper hard-links this universal binary under
    // a dedicated executable name. Intercept that multicall entry point before
    // Tokio/CLI startup so AppKit and Notification Center stay on the real main
    // thread and the helper never initializes an agent session.
    if jcode::cli::macos_notification_broker::is_invocation() {
        return jcode::cli::macos_notification_broker::run();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async { jcode::run().await })
}
