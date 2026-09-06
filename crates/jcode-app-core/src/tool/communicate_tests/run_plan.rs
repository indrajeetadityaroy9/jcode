// Run-plan driver tests: reporter output, driver-claim guard, concurrency and
// utilization accounting, await wake-ups, progress and terminal summaries.
#[tokio::test]
async fn run_plan_reporter_finalize_puts_summary_before_log() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let output_path = dir.path().join("tsk42.output");
    let reporter = super::RunPlanReporter::background(&output_path);
    assert_eq!(reporter.task_id.as_deref(), Some("tsk42"));

    reporter.log("assigned a -> session_fox").await;
    reporter.log("assigned b -> session_wolf").await;
    reporter
        .finalize("Swarm plan reached terminal state.")
        .await;

    let content = tokio::fs::read_to_string(&output_path)
        .await
        .expect("output file");
    let summary_idx = content
        .find("Swarm plan reached terminal state.")
        .expect("summary present");
    let log_idx = content.find("assigned a -> session_fox").expect("log kept");
    assert!(
        summary_idx < log_idx,
        "summary must lead the output file so completion previews are useful:\n{content}"
    );
}

#[tokio::test]
async fn run_plan_reporter_inline_is_a_no_op() {
    let reporter = super::RunPlanReporter::inline();
    assert!(reporter.task_id.is_none());
    // Must not panic or create files.
    reporter.log("ignored").await;
    reporter.progress(1, 2, "ignored".to_string()).await;
    reporter.finalize("ignored").await;
}

#[tokio::test]
async fn run_plan_driver_guard_blocks_while_driver_task_is_live() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let manager = crate::background::BackgroundTaskManager::with_output_dir(dir.path().into());
    let session = "session-guard-live";

    // Keep the fake driver alive long enough for the second claim to observe it.
    let info = manager
        .spawn_with_notify("swarm", None, session, false, false, |_| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(crate::background::TaskResult::completed(Some(0)))
        })
        .await;
    assert!(manager.is_live_task(&info.task_id));

    match super::try_claim_run_plan_driver(&manager, session) {
        super::RunPlanDriverClaimResult::Claimed(claim) => claim.record_task(&info.task_id),
        super::RunPlanDriverClaimResult::AlreadyRunning(_) => {
            panic!("first claim for a fresh session must succeed")
        }
    }

    match super::try_claim_run_plan_driver(&manager, session) {
        super::RunPlanDriverClaimResult::AlreadyRunning(Some(task_id)) => {
            assert_eq!(task_id, info.task_id);
        }
        super::RunPlanDriverClaimResult::AlreadyRunning(None) => {
            panic!("claim was recorded with a task id, so the blocker should carry it")
        }
        super::RunPlanDriverClaimResult::Claimed(_) => {
            panic!("second claim must be blocked while the driver task is live")
        }
    }

    manager.cancel(&info.task_id).await.expect("cancel driver");
}

#[tokio::test]
async fn run_plan_driver_guard_allows_restart_after_stale_driver() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let manager = crate::background::BackgroundTaskManager::with_output_dir(dir.path().into());
    let session = "session-guard-stale";

    // Simulate the pre-reload world: a status file on disk still says a swarm
    // driver is Running for this session, and the per-process claim map still
    // holds its task id, but no such task is live in this process (the map is
    // fresh after reload / the task was pruned on completion).
    let stale_task_id = "stalezzzz1";
    let stale_status = serde_json::json!({
        "task_id": stale_task_id,
        "tool_name": "swarm",
        "session_id": session,
        "status": "running",
        "exit_code": null,
        "error": null,
        "started_at": chrono::Utc::now().to_rfc3339(),
        "completed_at": null,
        "duration_secs": null,
        "detached": false,
        "notify": false,
        "wake": false
    });
    tokio::fs::write(
        manager.status_path_for(stale_task_id),
        serde_json::to_string_pretty(&stale_status).expect("serialize stale status"),
    )
    .await
    .expect("write stale status file");

    match super::try_claim_run_plan_driver(&manager, session) {
        super::RunPlanDriverClaimResult::Claimed(claim) => claim.record_task(stale_task_id),
        super::RunPlanDriverClaimResult::AlreadyRunning(_) => {
            panic!("fresh session must be claimable")
        }
    }
    assert!(
        !manager.is_live_task(stale_task_id),
        "stale task must not be live in this process"
    );

    // The stale Running status file and stale claim must not block restarting
    // the driver.
    match super::try_claim_run_plan_driver(&manager, session) {
        super::RunPlanDriverClaimResult::Claimed(_claim) => {}
        super::RunPlanDriverClaimResult::AlreadyRunning(_) => {
            panic!("stale (non-live) driver must not block a restart")
        }
    }
}

#[tokio::test]
async fn run_plan_driver_guard_is_atomic_for_racing_claims() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let manager: &'static crate::background::BackgroundTaskManager = Box::leak(Box::new(
        crate::background::BackgroundTaskManager::with_output_dir(dir.path().into()),
    ));
    let session = "session-guard-race";

    // Two run_plan calls in one batch race the claim; exactly one may win.
    let mut join_set = tokio::task::JoinSet::new();
    for _ in 0..2 {
        join_set.spawn(async move {
            match super::try_claim_run_plan_driver(manager, session) {
                // Keep the claim held (as the winner does while spawning).
                super::RunPlanDriverClaimResult::Claimed(claim) => Some(claim),
                super::RunPlanDriverClaimResult::AlreadyRunning(_) => None,
            }
        });
    }
    let mut held_claims = Vec::new();
    while let Some(result) = join_set.join_next().await {
        if let Some(claim) = result.expect("claim task should not panic") {
            held_claims.push(claim);
        }
    }
    assert_eq!(held_claims.len(), 1, "exactly one racing claim may win");
}

#[tokio::test]
async fn run_plan_driver_guard_releases_claim_on_drop_without_task() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let manager = crate::background::BackgroundTaskManager::with_output_dir(dir.path().into());
    let session = "session-guard-drop";

    match super::try_claim_run_plan_driver(&manager, session) {
        super::RunPlanDriverClaimResult::Claimed(claim) => drop(claim),
        super::RunPlanDriverClaimResult::AlreadyRunning(_) => {
            panic!("fresh session must be claimable")
        }
    }

    // A failed/cancelled startup path must not permanently block the session.
    match super::try_claim_run_plan_driver(&manager, session) {
        super::RunPlanDriverClaimResult::Claimed(_claim) => {}
        super::RunPlanDriverClaimResult::AlreadyRunning(_) => {
            panic!("dropped Starting claim must be released")
        }
    }
}

#[test]
fn run_plan_concurrency_is_mode_aware() {
    // Light mode (no explicit limit) keeps the small cheap fan-out default.
    assert_eq!(
        resolve_run_plan_concurrency(None, false, 32),
        super::LIGHT_MODE_DEFAULT_CONCURRENCY
    );

    // Deep mode (no explicit limit) fans out wide using the configured cap,
    // NOT the old hardcoded 3 and NOT the light default.
    assert_eq!(resolve_run_plan_concurrency(None, true, 32), 32);
    assert_eq!(resolve_run_plan_concurrency(None, true, 64), 64);

    // Deep mode with the cap set to 0 means "no extra cap": dispatch the whole
    // ready set, bounded only by the swarm member cap.
    assert_eq!(resolve_run_plan_concurrency(None, true, 0), usize::MAX);

    // An explicit request always wins over the mode default, in both modes,
    // and is clamped to at least 1.
    assert_eq!(resolve_run_plan_concurrency(Some(5), true, 32), 5);
    assert_eq!(resolve_run_plan_concurrency(Some(5), false, 32), 5);
    assert_eq!(resolve_run_plan_concurrency(Some(0), true, 32), 1);
}

#[test]
fn run_plan_utilization_tracks_peak_and_starvation() {
    let mut util = super::RunPlanUtilization::default();

    // Loop 1: 0 in flight, 8 open slots, dispatched 8 -> budget fully used.
    util.record_loop(0, Some(8), 8);
    // Loop 2: 8 in flight, 0 open slots, dispatched 0 -> saturated, not starved.
    util.record_loop(8, Some(0), 0);
    // Loop 3: 2 in flight, 6 open slots, dispatched 1 -> starved (5 idle slots).
    util.record_loop(2, Some(6), 1);

    assert_eq!(util.peak_in_flight, 8);
    assert_eq!(util.loops, 3);
    assert_eq!(util.starved_loops, 1);

    let report = util.report(8, true);
    assert!(report.contains("peak 8 of 8"));
    assert!(report.contains("1 of 3 loop(s)"));
    // 1/3 starved and peak 8: healthy run, no hint.
    assert!(!report.contains("Deep-mode hint"));
}

#[test]
fn run_plan_utilization_flags_serial_deep_runs() {
    // A deep run that trickles one task at a time despite a 32-slot budget.
    let mut util = super::RunPlanUtilization::default();
    for _ in 0..4 {
        util.record_loop(0, Some(32), 1);
    }
    assert_eq!(util.peak_in_flight, 1);
    assert_eq!(util.starved_loops, 4);

    let deep_report = util.report(32, true);
    assert!(deep_report.contains("peak 1 of 32"));
    assert!(deep_report.contains("Deep-mode hint"));
    assert!(deep_report.contains("expand"));

    // The same shape in light mode is by design; no nagging.
    let light_report = util.report(32, false);
    assert!(!light_report.contains("Deep-mode hint"));
}

#[test]
fn run_plan_utilization_handles_unbounded_budget() {
    let mut util = super::RunPlanUtilization::default();
    // Unbounded budget (deep_cap=0): open slots are not meaningful, so no
    // starvation accounting, but peak parallelism still records.
    util.record_loop(10, None, 5);
    assert_eq!(util.peak_in_flight, 15);
    assert_eq!(util.starved_loops, 0);
    let report = util.report(usize::MAX, true);
    assert!(report.contains("peak 15 of unbounded"));
}

#[test]
fn await_wakes_only_for_ready_items_beyond_the_wave_baseline() {
    let baseline: std::collections::HashSet<String> =
        ["stuck".to_string(), "assigned".to_string()].into();
    let mut summary = crate::protocol::PlanGraphStatus {
        swarm_id: None,
        version: 3,
        item_count: 6,
        ready_ids: vec!["stuck".to_string()],
        blocked_ids: Vec::new(),
        active_ids: vec!["a1".to_string()],
        completed_ids: Vec::new(),
        failed_ids: Vec::new(),
        failed_reasons: Default::default(),
        cycle_ids: Vec::new(),
        unresolved_dependency_ids: Vec::new(),
        next_ready_ids: Vec::new(),
        newly_ready_ids: Vec::new(),
        low_confidence_ids: Vec::new(),
        mode: "deep".to_string(),
        seeded_count: 0,
        grown_count: 0,
    };

    // Items already ready at wave start (even permanently-undispatchable
    // ones) must not wake the driver: that would busy-spin the await.
    assert!(!super::await_should_wake_for_new_ready(&baseline, &summary));

    // No ready items at all: keep waiting on members.
    summary.ready_ids.clear();
    assert!(!super::await_should_wake_for_new_ready(&baseline, &summary));

    // A retried failed node re-enters ready as a NEW id -> wake and dispatch.
    summary.ready_ids = vec!["stuck".to_string(), "retried-node".to_string()];
    assert!(super::await_should_wake_for_new_ready(&baseline, &summary));
}

#[test]
fn run_plan_progress_counts_only_completed_toward_percent_and_shows_live_active() {
    // Regression: a plan with 33 completed / 116 failed of 152 used to report
    // terminal/total = 149/152 (~98%) with active 0 while four externally
    // assigned workers were still running.
    let summary = crate::protocol::PlanGraphStatus {
        swarm_id: Some("swarm-a".to_string()),
        version: 9,
        item_count: 152,
        ready_ids: Vec::new(),
        blocked_ids: Vec::new(),
        active_ids: Vec::new(),
        completed_ids: (0..33).map(|i| format!("c{i}")).collect(),
        failed_ids: (0..116).map(|i| format!("f{i}")).collect(),
        failed_reasons: Default::default(),
        cycle_ids: Vec::new(),
        unresolved_dependency_ids: Vec::new(),
        next_ready_ids: Vec::new(),
        newly_ready_ids: Vec::new(),
        low_confidence_ids: Vec::new(),
        mode: "deep".to_string(),
        seeded_count: 0,
        grown_count: 0,
    };

    let (completed, total, message) = super::run_plan_progress_snapshot(&summary, 4, 137);
    // Percent driver is completed/total: 33/152 (~22%), never ~98%.
    assert_eq!(completed, 33);
    assert_eq!(total, 152);
    // Failed nodes are surfaced separately, and live in-flight workers show as
    // active even when the plan's own active_ids is empty (external
    // assign_task dispatches).
    assert_eq!(
        message,
        "completed 33 · failed 116 · blocked 0 · active 4 · assignments 137"
    );

    // The normalized background progress percent derived from (current,total)
    // must match completed/total, not terminal/total.
    let progress = crate::bus::BackgroundTaskProgress {
        kind: crate::bus::BackgroundTaskProgressKind::Determinate,
        percent: None,
        message: Some(message),
        current: Some(completed as u64),
        total: Some(total as u64),
        unit: Some("nodes".to_string()),
        eta_seconds: None,
        updated_at: chrono::Utc::now().to_rfc3339(),
        source: crate::bus::BackgroundTaskProgressSource::Reported,
    }
    .normalize();
    let percent = progress.percent.expect("determinate percent");
    assert!(
        (percent - 21.71).abs() < 0.1,
        "33/152 must normalize to ~21.7%, got {percent}"
    );
}

#[test]
fn run_plan_progress_active_prefers_plan_execution_state_when_larger() {
    let summary = crate::protocol::PlanGraphStatus {
        swarm_id: None,
        version: 1,
        item_count: 10,
        ready_ids: Vec::new(),
        blocked_ids: vec!["b1".to_string()],
        active_ids: vec!["a1".to_string(), "a2".to_string(), "a3".to_string()],
        completed_ids: vec!["c1".to_string(), "c2".to_string()],
        failed_ids: vec!["f1".to_string()],
        failed_reasons: Default::default(),
        cycle_ids: Vec::new(),
        unresolved_dependency_ids: Vec::new(),
        next_ready_ids: Vec::new(),
        newly_ready_ids: Vec::new(),
        low_confidence_ids: Vec::new(),
        mode: "light".to_string(),
        seeded_count: 0,
        grown_count: 0,
    };

    // Plan says 3 active but only 1 live member is observable (e.g. status
    // propagation lag): keep the larger plan-state number.
    let (completed, total, message) = super::run_plan_progress_snapshot(&summary, 1, 5);
    assert_eq!((completed, total), (2, 10));
    assert_eq!(
        message,
        "completed 2 · failed 1 · blocked 1 · active 3 · assignments 5"
    );
}

#[test]
fn plan_status_budget_line_is_deep_only_and_nudges_serialized_graphs() {
    let base = crate::protocol::PlanGraphStatus {
        swarm_id: Some("swarm-a".to_string()),
        version: 1,
        item_count: 10,
        ready_ids: vec!["a".to_string()],
        blocked_ids: Vec::new(),
        active_ids: vec!["b".to_string()],
        completed_ids: vec!["c".to_string()],
        failed_ids: Vec::new(),
        failed_reasons: Default::default(),
        cycle_ids: Vec::new(),
        unresolved_dependency_ids: Vec::new(),
        next_ready_ids: Vec::new(),
        newly_ready_ids: Vec::new(),
        low_confidence_ids: Vec::new(),
        mode: "deep".to_string(),
        seeded_count: 0,
        grown_count: 0,
    };

    // Light plans get no budget line at all.
    let light = crate::protocol::PlanGraphStatus {
        mode: "light".to_string(),
        ..base.clone()
    };
    assert_eq!(super::plan_status_budget_line(&light, 32), None);

    // Deep + narrow frontier (2 of 32) with 7 more items serialized behind
    // edges -> budget line plus the widen nudge.
    let narrow = super::plan_status_budget_line(&base, 32).expect("deep plans get a budget line");
    assert!(narrow.contains("Parallel budget: 32"));
    assert!(narrow.contains("ready set is 1 wide (1 active)"));
    assert!(narrow.contains("expand_node"));

    // Deep + the frontier is all that remains -> line but no nudge.
    let almost_done = crate::protocol::PlanGraphStatus {
        item_count: 3,
        ..base.clone()
    };
    let line = super::plan_status_budget_line(&almost_done, 32).unwrap();
    assert!(line.contains("Parallel budget: 32"));
    assert!(!line.contains("expand_node"));

    // deep_cap=0 (unbounded) surfaces the member cap as the budget.
    let unbounded = super::plan_status_budget_line(&base, 0).unwrap();
    assert!(unbounded.contains("1000 (member cap)"));
}

#[test]
fn assign_error_classification_recovers_on_member_cap_instead_of_failing() {
    use super::AssignErrorAction;

    // Graceful exhaustion of work or workers ends the assignment burst.
    assert_eq!(
        super::classify_assign_error(
            "No runnable unassigned tasks are available in the swarm plan"
        ),
        AssignErrorAction::BreakGracefully
    );
    assert_eq!(
        super::classify_assign_error(
            "No ready or completed swarm agents are available for automatic task assignment."
        ),
        AssignErrorAction::BreakGracefully
    );

    // The member cap must trigger recovery (cleanup + reuse fallback), not a
    // run-aborting failure. The server wraps the cap message in a spawn-failure
    // prefix, so classification must match on the substring.
    assert_eq!(
        super::classify_assign_error(
            "Failed to spawn preferred worker: Swarm member limit reached (max 1000). \
             This swarm already has 1000 agents; it cannot spawn more."
        ),
        AssignErrorAction::RecoverCapacity
    );

    // Anything else is still a real failure.
    assert_eq!(
        super::classify_assign_error("Not in a swarm."),
        AssignErrorAction::Fail
    );
}

#[test]
fn cap_recovery_prefers_cleanup_then_reuse_then_gives_up() {
    use super::CapRecoveryStep;

    // First cap hit with freed capacity: retry keeping the fresh-spawn preference.
    assert_eq!(super::cap_recovery_step(1, 3), CapRecoveryStep::RetryFresh);
    // First cap hit but nothing could be freed: fall back to reusing ready workers.
    assert_eq!(super::cap_recovery_step(1, 0), CapRecoveryStep::RetryReuse);
    // Recovery already ran and the cap still refuses: continue with in-flight
    // work instead of aborting or spinning.
    assert_eq!(super::cap_recovery_step(2, 0), CapRecoveryStep::GiveUp);
    assert_eq!(super::cap_recovery_step(3, 5), CapRecoveryStep::GiveUp);
}

#[test]
fn run_plan_driver_failures_carry_worker_retention_hint() {
    // Every driver-failure path must tell the caller the spawned workers are
    // still running and how to stop them.
    let hinted = super::with_worker_retention_hint(
        "run_plan stalled after 3 loop(s): no ready tasks and no in-flight workers.".to_string(),
    );
    assert!(hinted.contains("Spawned workers were retained"));
    assert!(hinted.contains("swarm cleanup"));

    // Max-loops keeps its intentional retention-for-inspection wording but
    // still gains the actionable hint.
    let max_loops = super::with_worker_retention_hint(
        "run_plan exceeded 200 coordination loops; leaving workers untouched for inspection"
            .to_string(),
    );
    assert!(max_loops.contains("swarm cleanup"));

    // Idempotent: re-wrapping (e.g. the background wrapper re-reporting the
    // error) must not duplicate the hint.
    let twice = super::with_worker_retention_hint(hinted.clone());
    assert_eq!(twice.matches("Spawned workers were retained").count(), 1);
}

#[test]
fn run_plan_terminal_summary_reports_failed_nodes() {
    let base = crate::protocol::PlanGraphStatus {
        swarm_id: Some("swarm-a".to_string()),
        version: 1,
        item_count: 4,
        ready_ids: Vec::new(),
        blocked_ids: Vec::new(),
        active_ids: Vec::new(),
        completed_ids: vec!["a".to_string(), "b".to_string()],
        failed_ids: vec!["c".to_string(), "d".to_string()],
        failed_reasons: Default::default(),
        cycle_ids: Vec::new(),
        unresolved_dependency_ids: Vec::new(),
        next_ready_ids: Vec::new(),
        newly_ready_ids: Vec::new(),
        low_confidence_ids: Vec::new(),
        mode: "deep".to_string(),
        seeded_count: 0,
        grown_count: 0,
    };

    let with_failures = super::format_run_plan_terminal_summary(5, &base, 7);
    assert!(with_failures.contains("completed=2"));
    assert!(with_failures.contains("failed=2"));
    assert!(with_failures.contains("Failed nodes: c, d"));
    assert!(with_failures.contains("did NOT finish cleanly"));

    // A clean run reports failed=0 and no failure callout.
    let clean = crate::protocol::PlanGraphStatus {
        completed_ids: vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ],
        failed_ids: Vec::new(),
        failed_reasons: Default::default(),
        ..base
    };
    let clean_summary = super::format_run_plan_terminal_summary(5, &clean, 7);
    assert!(clean_summary.contains("failed=0"));
    assert!(!clean_summary.contains("Failed nodes"));
}

#[test]
fn plan_terminal_node_count_includes_failed_without_double_counting() {
    let summary = crate::protocol::PlanGraphStatus {
        swarm_id: Some("swarm-a".to_string()),
        version: 1,
        item_count: 4,
        ready_ids: Vec::new(),
        blocked_ids: vec!["x".to_string()],
        active_ids: Vec::new(),
        completed_ids: vec!["a".to_string()],
        failed_ids: vec!["c".to_string()],
        failed_reasons: Default::default(),
        // "x" is both blocked and cyclic; it must count once.
        cycle_ids: vec!["x".to_string()],
        unresolved_dependency_ids: Vec::new(),
        next_ready_ids: Vec::new(),
        newly_ready_ids: Vec::new(),
        low_confidence_ids: Vec::new(),
        mode: "light".to_string(),
        seeded_count: 0,
        grown_count: 0,
    };
    // a (completed) + c (failed) + x (blocked/cycle, deduped) = 3. Without
    // failed_ids in the terminal count a run with failed nodes would never
    // satisfy terminal_count >= item_count and run_plan could spin or stall.
    assert_eq!(super::plan_terminal_node_count(&summary), 3);
}
