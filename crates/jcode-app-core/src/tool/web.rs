//! Shared plumbing for the tools that reach the web.
//!
//! Both `websearch` and `webfetch` drive one headless Chrome, so the browser
//! handle and the request deadline live here rather than being duplicated (or,
//! worse, invented twice with different values).

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

/// Default per-request budget, in seconds.
pub(super) const DEFAULT_TIMEOUT: u64 = 30;

/// Ceiling a caller may raise the per-request budget to.
pub(super) const MAX_TIMEOUT: u64 = 120;

/// Process-wide browser. Launching it costs a process and a profile directory,
/// so it is shared by every tool call and every session in this daemon.
static CHROME: OnceCell<Arc<jcode_chrome::ChromeEngine>> = OnceCell::const_new();

/// Launch the browser on first use.
///
/// Tool constructors run synchronously and infallibly inside a `OnceLock`
/// (`Registry::base_tools`), so the browser cannot be started there. Uses
/// `get_or_try_init` so a failed launch is retried on the next call rather
/// than cached as a permanent failure.
pub(super) async fn chrome_engine() -> Result<Arc<jcode_chrome::ChromeEngine>> {
    CHROME
        .get_or_try_init(|| async {
            let config = &crate::config::config().websearch;
            // Chrome refuses to share a profile directory: a second instance
            // aborts rather than attach. Keying on the daemon pid lets two
            // daemons run against one JCODE_HOME, and lets a later run
            // identify and reclaim profiles whose owner is gone.
            let user_data_dir = crate::storage::jcode_dir()
                .context("failed to resolve the jcode home directory")?
                .join(format!("chrome-profile-{}", std::process::id()));
            let binary = Some(std::path::PathBuf::from(&config.chrome_binary))
                .filter(|path| !path.as_os_str().is_empty());

            let engine = jcode_chrome::ChromeEngine::launch(jcode_chrome::ChromeOptions {
                binary,
                user_data_dir,
                startup_timeout: Duration::from_secs(DEFAULT_TIMEOUT),
            })
            .await?;
            Ok(Arc::new(engine))
        })
        .await
        .cloned()
}
