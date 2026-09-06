#![cfg_attr(test, allow(clippy::items_after_test_module))]

pub use jcode_storage::*;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use std::path::Path;

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    jcode_storage::read_json_with_recovery_handler(path, |event| match event {
        jcode_storage::StorageRecoveryEvent::CorruptPrimary { path, error } => {
            crate::logging::warn(&format!(
                "Corrupt JSON at {}, trying backup: {}",
                path.display(),
                error
            ));
        }
        jcode_storage::StorageRecoveryEvent::RecoveredFromBackup { backup_path } => {
            crate::logging::info(&format!("Recovered from backup: {}", backup_path.display()));
        }
    })
}

/// Cache of the last value written by [`remember_last_focused_session`], so a
/// TUI that re-reports the same focused session on every frame does not rewrite
/// the file each time.
static LAST_FOCUSED_SESSION_WRITE_CACHE: std::sync::LazyLock<std::sync::Mutex<Option<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

fn last_focused_session_path() -> Result<std::path::PathBuf> {
    Ok(jcode_dir()?.join("last_focused_client_session"))
}

/// Record `session_id` as the most recently focused client session, so external
/// text injection (`jcode transcript`) can target it when no session is named.
pub fn remember_last_focused_session(session_id: &str) -> Result<()> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(());
    }

    if let Ok(cache) = LAST_FOCUSED_SESSION_WRITE_CACHE.lock()
        && cache.as_deref() == Some(session_id)
    {
        return Ok(());
    }

    let path = last_focused_session_path()?;
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    std::fs::write(&path, session_id).context("failed to persist last focused jcode session")?;

    if let Ok(mut cache) = LAST_FOCUSED_SESSION_WRITE_CACHE.lock() {
        *cache = Some(session_id.to_string());
    }

    Ok(())
}

/// The last focused client session, if it is still running. A recorded session
/// that has since exited is reported as `None` rather than as a stale target.
pub fn last_focused_session() -> Result<Option<String>> {
    let path = last_focused_session_path()?;
    let session_id = match std::fs::read_to_string(path) {
        Ok(text) => text.trim().to_string(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("failed to read last focused jcode session"),
    };
    if session_id.is_empty() {
        return Ok(None);
    }

    if active_session_ids().iter().any(|id| id == &session_id) {
        Ok(Some(session_id))
    } else {
        Ok(None)
    }
}

#[cfg(any(test, feature = "test-support"))]
use std::sync::{LazyLock, Mutex, MutexGuard};

#[cfg(any(test, feature = "test-support"))]
static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[cfg(any(test, feature = "test-support"))]
pub fn test_env_lock() -> &'static Mutex<()> {
    &ENV_LOCK
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// How many live guards this thread holds. Nested acquisitions must not
    /// re-lock: process-global test state is reached through several layers
    /// (a temp-`JCODE_HOME` helper that then builds a TUI app, for example),
    /// and a plain `Mutex` would self-deadlock on the inner call.
    static ENV_LOCK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Serializes every test that touches process-global state: environment
/// variables, `JCODE_HOME`-derived paths, and the render-state globals the TUI
/// keeps between frames.
///
/// One lock, taken reentrantly, is deliberate. Two locks in this role deadlocked
/// the `jcode-tui` suite: tests that locked the environment and then built an
/// app took them in the opposite order from tests that rendered first and then
/// swapped `JCODE_HOME`, so a parallel run stalled forever at the first
/// interleaving. A single reentrant lock cannot be acquired out of order.
#[cfg(any(test, feature = "test-support"))]
pub fn lock_test_env() -> TestEnvGuard {
    let depth = ENV_LOCK_DEPTH.with(std::cell::Cell::get);
    let guard = if depth == 0 {
        Some(
            test_env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    } else {
        None
    };
    ENV_LOCK_DEPTH.with(|cell| cell.set(depth + 1));
    TestEnvGuard { _guard: guard }
}

/// RAII guard for [`lock_test_env`]. Holds the mutex only for the outermost
/// acquisition on a thread; nested guards just keep the depth count.
#[cfg(any(test, feature = "test-support"))]
pub struct TestEnvGuard {
    _guard: Option<MutexGuard<'static, ()>>,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        ENV_LOCK_DEPTH.with(|cell| cell.set(cell.get().saturating_sub(1)));
    }
}

/// True when this thread already holds the lock.
#[cfg(any(test, feature = "test-support"))]
pub fn test_env_lock_held() -> bool {
    ENV_LOCK_DEPTH.with(std::cell::Cell::get) > 0
}

#[cfg(test)]
mod tests;
