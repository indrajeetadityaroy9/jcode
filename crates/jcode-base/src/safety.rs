use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::storage;



// ---------------------------------------------------------------------------
// Action log / transcript
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLog {
    pub action_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptStatus {
    Complete,
    Interrupted,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbientTranscript {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub status: TranscriptStatus,
    pub provider: String,
    pub model: String,
    pub actions: Vec<ActionLog>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub compactions: u32,
    pub memories_modified: u32,
    /// Full conversation transcript (markdown) for email notifications
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
}

// ---------------------------------------------------------------------------
// SafetySystem
// ---------------------------------------------------------------------------

pub struct SafetySystem {
    actions: Mutex<Vec<ActionLog>>,
}

impl SafetySystem {
    /// Create a new action log for one ambient cycle.
    pub fn new() -> Self {
        SafetySystem {
            actions: Mutex::new(Vec::new()),
        }
    }

    /// Append an action to the in-memory log.
    pub fn log_action(&self, log: ActionLog) {
        if let Ok(mut actions) = self.actions.lock() {
            actions.push(log);
        }
    }

    /// Generate a human-readable summary of logged actions.
    pub fn generate_summary(&self) -> String {
        let actions = self.actions.lock().map(|a| a.clone()).unwrap_or_default();

        if actions.is_empty() {
            return "No actions recorded.".to_string();
        }

        let mut lines: Vec<String> = vec!["Done:".to_string()];
        for a in &actions {
            lines.push(format!("- {} — {}", a.action_type, a.description));
        }
        lines.join("\n")
    }

    /// Persist a transcript to ~/.jcode/ambient/transcripts/{timestamp}.json
    pub fn save_transcript(&self, transcript: &AmbientTranscript) -> Result<()> {
        let dir = storage::jcode_dir()?.join("ambient").join("transcripts");
        storage::ensure_dir(&dir)?;

        let filename = transcript.started_at.format("%Y-%m-%d-%H%M%S").to_string();
        let path = dir.join(format!("{}.json", filename));
        storage::write_json(&path, transcript)
    }
}

impl Default for SafetySystem {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ID generation helper
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_home<F, T>(f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let _guard = crate::storage::lock_test_env();
        let prev_home = std::env::var_os("JCODE_HOME");
        let temp = tempfile::TempDir::new().expect("create temp dir");
        crate::env::set_var("JCODE_HOME", temp.path());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

        match prev_home {
            Some(value) => crate::env::set_var("JCODE_HOME", value),
            None => crate::env::remove_var("JCODE_HOME"),
        }

        result.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }

    #[test]
    fn test_log_action_and_summary() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            sys.log_action(ActionLog {
                action_type: "memory_consolidation".to_string(),
                description: "Merged 2 duplicate memories".to_string(),
                details: None,
                timestamp: Utc::now(),
            });
            sys.log_action(ActionLog {
                action_type: "edit".to_string(),
                description: "Fixed typo in README".to_string(),
                details: None,
                timestamp: Utc::now(),
            });

            let summary = sys.generate_summary();
            assert!(summary.contains("memory_consolidation"));
            assert!(summary.contains("edit"));
            assert!(summary.contains("Done:"));
        });
    }

    #[test]
    fn test_empty_summary() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            let summary = sys.generate_summary();
            assert_eq!(summary, "No actions recorded.");
        });
    }

}
