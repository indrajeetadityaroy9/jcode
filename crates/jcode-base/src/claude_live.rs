//! Discovery and explicit handoff support for live Claude Code sessions.
//!
//! Claude Code 2.1.x publishes one small registry record per interactive
//! process at `~/.claude/sessions/<pid>.json`. The record's `procStart` value
//! is a process-start token that guards against PID reuse before presenting
//! or signaling a process.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ClaudeSessionRegistryRecord {
    pid: u32,
    session_id: String,
    cwd: String,
    proc_start: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    started_at: Option<i64>,
    #[serde(default)]
    version: Option<String>,
}

/// A Claude Code process whose registry record and OS process identity agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveClaudeSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub proc_start: String,
    pub name: Option<String>,
    pub started_at: Option<i64>,
    pub version: Option<String>,
    registry_path: PathBuf,
}

/// Result of asking the identity-verified Claude process to exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopLiveClaudeOutcome {
    /// The stable process handle reported that Claude exited.
    Exited,
    /// SIGTERM was delivered, but exit was not observed before the deadline.
    ExitUnconfirmed,
}

impl LiveClaudeSession {
    fn from_record(record: ClaudeSessionRegistryRecord, registry_path: PathBuf) -> Self {
        Self {
            pid: record.pid,
            session_id: record.session_id,
            cwd: record.cwd,
            proc_start: record.proc_start,
            name: record.name,
            started_at: record.started_at,
            version: record.version,
            registry_path,
        }
    }
}

/// Return live, identity-verified interactive Claude Code CLI sessions.
///
/// On platforms where Claude's process-start token cannot yet be verified, the
/// safe behavior is to return no takeover candidates rather than trust a PID.
pub fn live_claude_sessions() -> Result<Vec<LiveClaudeSession>> {
    let root = crate::storage::user_home_path(".claude/sessions")?;
    live_claude_sessions_in(&root)
}

pub fn find_live_claude_session(session_id: &str) -> Result<Option<LiveClaudeSession>> {
    Ok(live_claude_sessions()?
        .into_iter()
        .find(|session| session.session_id == session_id))
}

fn live_claude_sessions_in(root: &Path) -> Result<Vec<LiveClaudeSession>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| {
        format!(
            "failed to read Claude live-session registry {}",
            root.display()
        )
    })? {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_slice::<ClaudeSessionRegistryRecord>(&bytes) else {
            continue;
        };
        if !registry_record_is_takeover_candidate(&path, &record) {
            continue;
        }
        if process_identity_matches(record.pid, &record.proc_start) {
            sessions.push(LiveClaudeSession::from_record(record, path));
        }
    }
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(sessions)
}

fn registry_record_is_takeover_candidate(
    path: &Path,
    record: &ClaudeSessionRegistryRecord,
) -> bool {
    if record.session_id.trim().is_empty()
        || record.cwd.trim().is_empty()
        || record.proc_start.trim().is_empty()
    {
        return false;
    }
    if record
        .kind
        .as_deref()
        .is_some_and(|kind| kind != "interactive")
        || record
            .entrypoint
            .as_deref()
            .is_some_and(|entrypoint| entrypoint != "cli")
    {
        return false;
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.parse::<u32>().ok())
        == Some(record.pid)
}

fn process_identity_matches(_pid: u32, _expected_start: &str) -> bool {
    false
}

fn registry_still_matches(session: &LiveClaudeSession) -> bool {
    let Ok(bytes) = std::fs::read(&session.registry_path) else {
        return false;
    };
    serde_json::from_slice::<ClaudeSessionRegistryRecord>(&bytes).is_ok_and(|record| {
        record.pid == session.pid
            && record.session_id == session.session_id
            && record.proc_start == session.proc_start
    })
}

/// Gracefully stop the exact Claude Code process represented by `session`.
///
/// The registry record and process identity are re-verified first, so a PID
/// that has exited or been reused is never acted on. No mechanism to stop the
/// process exists on this platform yet, so even a verified session returns an
/// error instead of being signaled.
pub fn stop_live_claude_session(
    session: &LiveClaudeSession,
    timeout: Duration,
) -> Result<StopLiveClaudeOutcome> {
    if !registry_still_matches(session) {
        bail!(
            "Claude Code session {} changed or closed before takeover",
            session.session_id
        );
    }

    if !process_identity_matches(session.pid, &session.proc_start) {
        bail!(
            "Claude Code session {} is no longer owned by the recorded process",
            session.session_id
        );
    }
    if !registry_still_matches(session) {
        bail!(
            "Claude Code session {} changed or closed before takeover",
            session.session_id
        );
    }

    {
        let _ = timeout;
        Err(anyhow!(
            "live Claude Code takeover is not yet supported on this platform"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    fn spawn_sleep() -> Child {
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn write_record(root: &Path, child: &Child, session_id: &str, start: &str) -> PathBuf {
        std::fs::create_dir_all(root).unwrap();
        let path = root.join(format!("{}.json", child.id()));
        std::fs::write(
            &path,
            serde_json::json!({
                "pid": child.id(),
                "sessionId": session_id,
                "cwd": "/tmp/project",
                "procStart": start,
                "kind": "interactive",
                "entrypoint": "cli",
                "startedAt": 123,
                "name": "probe",
                "version": "2.1.212"
            })
            .to_string(),
        )
        .unwrap();
        path
    }

    #[test]
    fn mismatched_identity_is_never_signaled() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut child = spawn_sleep();
        let path = write_record(temp.path(), &child, "mismatch", "wrong");
        let session = LiveClaudeSession {
            pid: child.id(),
            session_id: "mismatch".to_string(),
            cwd: "/tmp/project".to_string(),
            proc_start: "wrong".to_string(),
            name: None,
            started_at: None,
            version: None,
            registry_path: path,
        };

        assert!(stop_live_claude_session(&session, Duration::from_millis(50)).is_err());
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
