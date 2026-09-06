use anyhow::Result;
use std::process::Command as ProcessCommand;

pub use crate::session_rebuild::{hot_rebuild, spawn_background_session_rebuild};

use crate::{build, tui::RunResult};

pub fn has_requested_action(run_result: &RunResult) -> bool {
    run_result.reload_session.is_some()
        || run_result.rebuild_session.is_some()
        || run_result.restart_session.is_some()
}

pub fn execute_requested_action(run_result: &RunResult) -> Result<()> {
    if let Some(ref reload_session_id) = run_result.reload_session {
        hot_reload(reload_session_id)?;
    }

    if let Some(ref rebuild_session_id) = run_result.rebuild_session {
        hot_rebuild(rebuild_session_id)?;
    }

    if let Some(restart_session_id) = &run_result.restart_session {
        hot_restart(restart_session_id)?;
    }

    Ok(())
}

pub fn hot_restart(session_id: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let exe = std::env::current_exe()?;

    crate::logging::info(&format!("Restarting with current binary: {:?}", exe));

    crate::env::set_var("JCODE_RESUMING", "1");

    let mut cmd = ProcessCommand::new(&exe);
    cmd.arg("--resume").arg(session_id).current_dir(&cwd);
    let err = crate::platform::replace_process(&mut cmd);

    Err(anyhow::anyhow!("Failed to exec {:?}: {}", exe, err))
}

pub fn hot_reload(session_id: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;

    crate::env::set_var("JCODE_RESUMING", "1");

    if let Ok(migrate_binary) = std::env::var("JCODE_MIGRATE_BINARY") {
        let binary_path = std::path::PathBuf::from(&migrate_binary);
        if binary_path.exists() {
            crate::logging::info("Migrating to stable binary...");
            let mut cmd = ProcessCommand::new(&binary_path);
            cmd.arg("--resume")
                .arg(session_id)
                .arg("--no-update")
                .env_remove("JCODE_MIGRATE_BINARY")
                .current_dir(cwd);
            let err = crate::platform::replace_process(&mut cmd);
            return Err(anyhow::anyhow!("Failed to exec {:?}: {}", binary_path, err));
        } else {
            crate::logging::warn(&format!(
                "Migration binary not found at {:?}, falling back to local binary",
                binary_path
            ));
        }
    }

    let (exe, _label) = build::preferred_reload_candidate()
        .ok_or_else(|| anyhow::anyhow!("No reloadable binary found"))?;

    if let Ok(metadata) = std::fs::metadata(&exe) {
        let age = metadata
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .map(|d| {
                let secs = d.as_secs();
                if secs < 60 {
                    format!("{} seconds ago", secs)
                } else if secs < 3600 {
                    format!("{} minutes ago", secs / 60)
                } else {
                    format!("{} hours ago", secs / 3600)
                }
            })
            .unwrap_or_else(|| "unknown".to_string());
        crate::logging::info(&format!("Reloading with binary built {}...", age));
    }

    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if !exe.exists() {
                continue;
            }
        }
        let mut cmd = ProcessCommand::new(&exe);
        cmd.arg("--resume")
            .arg(session_id)
            // The server has already completed its handoff before the client
            // re-execs. Let the replacement client paint and accept input from
            // its startup stub immediately; the authoritative History payload
            // will repopulate the transcript after reconnect.
            .env("JCODE_RELOAD_FAST_START", "1")
            .current_dir(&cwd);
        let err = crate::platform::replace_process(&mut cmd);

        if err.kind() == std::io::ErrorKind::NotFound && attempt < 2 {
            crate::logging::warn(&format!(
                "exec attempt {} failed (ENOENT) for {:?}, retrying...",
                attempt + 1,
                exe
            ));
            continue;
        }
        return Err(anyhow::anyhow!("Failed to exec {:?}: {}", exe, err));
    }
    Err(anyhow::anyhow!(
        "Failed to exec {:?}: binary not found after retries",
        exe
    ))
}
