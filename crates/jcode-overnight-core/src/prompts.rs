use chrono::{DateTime, Utc};

use super::{
    OvernightManifest, OvernightRunStatus, format_minutes,
};

pub(crate) fn overnight_phase(manifest: &OvernightManifest, now: DateTime<Utc>) -> &'static str {
    match manifest.status {
        OvernightRunStatus::Completed => "completed",
        OvernightRunStatus::Failed => "failed",
        OvernightRunStatus::CancelRequested => "cancelling",
        OvernightRunStatus::Running => {
            if now < manifest.handoff_ready_at {
                "running"
            } else if now < manifest.target_wake_at {
                "wind-down"
            } else if manifest.morning_report_posted_at.is_none() {
                "morning report"
            } else if now < manifest.post_wake_grace_until {
                "post-wake"
            } else {
                "finalizing"
            }
        }
    }
}

pub(crate) fn time_relation_to_target(manifest: &OvernightManifest, now: DateTime<Utc>) -> String {
    let minutes = manifest
        .target_wake_at
        .signed_duration_since(now)
        .num_minutes();
    if minutes >= 0 {
        format!("target in {}", format_minutes(minutes as u32))
    } else {
        format!("target passed {} ago", format_minutes((-minutes) as u32))
    }
}

pub(crate) fn relative_time(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let minutes = now.signed_duration_since(then).num_minutes();
    if minutes >= 0 {
        format!("{} ago", format_minutes(minutes as u32))
    } else {
        format!("in {}", format_minutes((-minutes) as u32))
    }
}

pub(crate) fn next_prompt_label(manifest: &OvernightManifest, now: DateTime<Utc>) -> String {
    if !matches!(manifest.status, OvernightRunStatus::Running) {
        return "none".to_string();
    }
    if now < manifest.handoff_ready_at {
        return format!(
            "handoff mode in {} or after current turn",
            format_minutes(
                manifest
                    .handoff_ready_at
                    .signed_duration_since(now)
                    .num_minutes()
                    .max(0) as u32
            )
        );
    }
    if now < manifest.target_wake_at {
        return format!(
            "morning report in {} or after current turn",
            format_minutes(
                manifest
                    .target_wake_at
                    .signed_duration_since(now)
                    .num_minutes()
                    .max(0) as u32
            )
        );
    }
    if manifest.morning_report_posted_at.is_none() {
        return "morning report after current turn".to_string();
    }
    if now < manifest.post_wake_grace_until {
        return format!(
            "final wrap by {} or after current turn",
            manifest.post_wake_grace_until.format("%H:%M UTC")
        );
    }
    "final wrap after current turn".to_string()
}

pub fn build_visible_current_session_prompt(manifest: &OvernightManifest) -> String {
    let mission = manifest
        .mission
        .as_deref()
        .unwrap_or("Continue the current session's highest-value work, prioritizing verified, low-risk progress.");
    format!(
        r#"You are now the visible Overnight Coordinator for Jcode run `{run_id}`.

The user expects this current session to become the overnight session. Keep all work visible here: your normal tool calls, any spawned/swarm helper agents, their reports, and validation should be observable from this session like a normal interactive run.

Important: because this is the visible current-session mode, there is no separate hidden supervisor loop running additional turns for you. You must self-manage the overnight lifecycle from this visible turn: check the target wake time yourself, post a morning report when it is reached, avoid continuing past the grace window except for a bounded safe wrap-up, and check the manifest for cancellation before starting each major new task.

Target wake/report time: `{target_wake_at}`
Soft post-wake grace window ends: `{post_wake_grace_until}`

Mission:
{mission}

Operating contract:
- Do not wait for the user. If you need user judgment/credentials/taste, record it and switch to another useful task.
- Optimize for verified, low-risk progress. Prefer objective bugs, repros, regression tests, bounded quality fixes, and clear validation.
- Avoid broad rewrites, taste-based decisions, risky migrations, payments, sending email, pushing to remotes, deleting data, or external side effects unless explicitly allowed.
- Spawn helper/swarm agents only when valuable, and keep their work headed/visible from this session. Prefer read-only scouts/verifiers over many editors.
- Watch RAM/load/battery and avoid concurrent heavy builds or tests unless resources are clearly healthy.

Review/log requirements:
- Keep `{review_notes}` updated as you work.
- For each meaningful task, maintain one task-card JSON in `{task_cards}` using `{task_card_schema}`.
- Task cards should include Before/After, evidence, validation, files changed, risk, status, and outcome.
- Put useful command outputs in `{validation}`.
- The generated review page is `{review_html}`.
- Manifest path: `{manifest_path}`. If cancellation is requested or the run completes, update the manifest/status consistently when safe.

Initial steps:
1. Inspect current repo/session state, including git status and current todos.
2. Build a ranked queue of verifiable candidate tasks.
3. Pick the highest-confidence bounded task.
4. Prove/reproduce before fixing.
5. Validate, update review notes/task cards, and continue with the next bounded task until the target wake/report time.
"#,
        run_id = manifest.run_id,
        target_wake_at = manifest.target_wake_at.to_rfc3339(),
        post_wake_grace_until = manifest.post_wake_grace_until.to_rfc3339(),
        mission = mission,
        review_notes = manifest.review_notes_path.display(),
        task_cards = manifest.task_cards_dir.display(),
        task_card_schema = manifest
            .task_cards_dir
            .join("task-card-schema.md")
            .display(),
        validation = manifest.validation_dir.display(),
        review_html = manifest.review_path.display(),
        manifest_path = manifest.run_dir.join("manifest.json").display(),
    )
}

pub fn prompt_event_summary(prompt: &str) -> String {
    if prompt.starts_with("You are the Overnight Coordinator") {
        "Sending initial overnight coordinator mission".to_string()
    } else if prompt.starts_with("Handoff-ready") {
        "Sending handoff-ready poke".to_string()
    } else if prompt.starts_with("Target wake") {
        "Sending morning report poke".to_string()
    } else if prompt.starts_with("Post-wake continuation") {
        "Sending post-wake continuation poke".to_string()
    } else if prompt.starts_with("Final overnight wrap-up") {
        "Sending final wrap-up poke".to_string()
    } else {
        "Sending continuation poke".to_string()
    }
}
