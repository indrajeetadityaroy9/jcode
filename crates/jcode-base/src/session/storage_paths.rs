use anyhow::Result;
use std::path::{Path, PathBuf};

use super::PersistVectorMode;
use crate::storage;

pub(crate) fn session_path_in_dir(base: &std::path::Path, session_id: &str) -> PathBuf {
    base.join("sessions").join(format!("{}.json", session_id))
}

pub(super) use crate::process_memory::estimate_json_bytes;

pub(super) fn file_len_or_zero(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

pub(super) fn persist_vector_mode_label(mode: PersistVectorMode) -> &'static str {
    match mode {
        PersistVectorMode::Clean => "clean",
        PersistVectorMode::Append => "append",
        PersistVectorMode::Full => "full",
    }
}

pub fn session_path(session_id: &str) -> Result<PathBuf> {
    if let Some(base) = test_session_base() {
        return Ok(session_path_in_dir(&base, session_id));
    }
    let base = storage::jcode_dir()?;
    Ok(session_path_in_dir(&base, session_id))
}

/// Where a test build that never set `JCODE_HOME` keeps its sessions.
///
/// `Agent::new` persists a snapshot for every agent it builds (via
/// `log_env_snapshot("create")`), so a test that skips the temp-`JCODE_HOME`
/// setup writes mock sessions straight into the developer's real `~/.jcode`.
/// One `cargo test -p jcode-app-core --lib` run added 108 of them, and they
/// pass the resume picker's user-session filter, so real usage history becomes
/// unanalysable. Redirecting rather than refusing to write keeps tests that
/// legitimately persist and reload a session working; they just do it under a
/// per-process temp root.
#[cfg(any(test, feature = "test-support"))]
fn test_session_base() -> Option<PathBuf> {
    if std::env::var_os("JCODE_HOME").is_some() {
        return None;
    }
    static ROOT: std::sync::LazyLock<PathBuf> = std::sync::LazyLock::new(|| {
        std::env::temp_dir().join(format!("jcode-test-sessions-{}", std::process::id()))
    });
    Some(ROOT.clone())
}

#[cfg(not(any(test, feature = "test-support")))]
fn test_session_base() -> Option<PathBuf> {
    None
}

pub fn session_journal_path_from_snapshot(path: &Path) -> PathBuf {
    let mut name = path
        .file_stem()
        .map(|stem| stem.to_os_string())
        .unwrap_or_default();
    name.push(".journal.jsonl");
    path.with_file_name(name)
}

pub fn session_journal_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_journal_path_from_snapshot(&session_path(
        session_id,
    )?))
}

pub fn session_exists(session_id: &str) -> bool {
    session_path(session_id)
        .map(|path| path.exists())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pollution guard: a test build that never set `JCODE_HOME` must not
    /// write into the developer's real `~/.jcode/sessions`, because those
    /// snapshots pass the resume picker's user-session filter and make real
    /// usage history unanalysable.
    #[test]
    fn session_path_stays_out_of_the_real_home_when_jcode_home_is_unset() {
        let _lock = crate::storage::lock_test_env();
        let previous = std::env::var_os("JCODE_HOME");
        unsafe { std::env::remove_var("JCODE_HOME") };

        let path = session_path("session_probe_1_abcdef01").expect("path");
        let journal = session_journal_path("session_probe_1_abcdef01").expect("journal");
        let real_home = dirs::home_dir().expect("home").join(".jcode");

        if let Some(value) = previous {
            unsafe { std::env::set_var("JCODE_HOME", value) };
        }

        assert!(
            !path.starts_with(&real_home),
            "test snapshot landed in the real store: {}",
            path.display()
        );
        assert!(
            path.starts_with(std::env::temp_dir()),
            "expected a temp root, got {}",
            path.display()
        );
        // The journal derives from the snapshot, so one funnel covers both.
        assert_eq!(journal.parent(), path.parent());
    }

    /// Redirecting must not break the tests that isolate properly: an explicit
    /// `JCODE_HOME` still wins, so they keep reloading what they persisted.
    #[test]
    fn session_path_honours_an_explicit_jcode_home() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::tempdir().expect("tempdir");
        let previous = std::env::var_os("JCODE_HOME");
        unsafe { std::env::set_var("JCODE_HOME", temp.path()) };

        let path = session_path("session_probe_2_abcdef02").expect("path");

        match previous {
            Some(value) => unsafe { std::env::set_var("JCODE_HOME", value) },
            None => unsafe { std::env::remove_var("JCODE_HOME") },
        }

        assert_eq!(
            path,
            temp.path()
                .join("sessions")
                .join("session_probe_2_abcdef02.json")
        );
    }
}
