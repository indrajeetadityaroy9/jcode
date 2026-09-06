use anyhow::{Context, Result};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use tokio::time::{Duration, timeout};

#[derive(Debug, Clone)]
pub struct DictationRun {
    pub text: String,
    pub mode: crate::protocol::TranscriptMode,
}

pub async fn run_configured() -> Result<DictationRun> {
    let cfg = crate::config::config().dictation.clone();
    let command = cfg.command.trim();
    if command.is_empty() {
        anyhow::bail!(
            "Dictation is not configured. Set `[dictation].command` in `~/.jcode/config.toml`."
        );
    }

    let text = run_command(command, cfg.timeout_secs).await?;
    Ok(DictationRun {
        text,
        mode: cfg.mode,
    })
}

pub async fn run_command(command: &str, timeout_secs: u64) -> Result<String> {
    let mut child = shell_command(command);
    child.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = child
        .spawn()
        .with_context(|| format!("failed to start `{}`", command))?;

    let output = if timeout_secs == 0 {
        child
            .wait_with_output()
            .await
            .context("failed to wait for dictation command")?
    } else {
        timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
            .await
            .with_context(|| format!("dictation command timed out after {}s", timeout_secs))?
            .context("failed to wait for dictation command")?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!("dictation command exited with {}", output.status);
        }
        anyhow::bail!(stderr);
    }

    let transcript = String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .trim()
        .to_string();
    if transcript.is_empty() {
        anyhow::bail!("dictation command returned an empty transcript");
    }

    Ok(transcript)
}

fn shell_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-lc").arg(command);
    cmd
}

fn last_focused_session_write_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

pub fn remember_last_focused_session(session_id: &str) -> Result<()> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(());
    }

    if let Ok(cache) = last_focused_session_write_cache().lock()
        && cache.as_deref() == Some(session_id)
    {
        return Ok(());
    }

    let path = last_focused_session_path()?;
    if let Some(parent) = path.parent() {
        crate::storage::ensure_dir(parent)?;
    }
    std::fs::write(&path, session_id).context("failed to persist last focused jcode session")?;

    if let Ok(mut cache) = last_focused_session_write_cache().lock() {
        *cache = Some(session_id.to_string());
    }

    Ok(())
}

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

    if crate::storage::active_session_ids()
        .iter()
        .any(|id| id == &session_id)
    {
        Ok(Some(session_id))
    } else {
        Ok(None)
    }
}

fn last_focused_session_path() -> Result<std::path::PathBuf> {
    Ok(crate::storage::jcode_dir()?.join("last_focused_client_session"))
}

#[cfg(test)]
#[path = "dictation_tests.rs"]
mod dictation_tests;
