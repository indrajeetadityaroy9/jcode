//! `swarm run_plan` driver internals: concurrency policy, the single-driver
//! claim guard, utilization/progress reporting and the background launch.
//!
//! Split out of `communicate.rs`, which keeps the tool dispatch surface. These
//! items form one unit because they all exist to keep exactly one driver
//! running a given plan and to report what that driver did; they were already
//! adjacent in the parent.
use super::*;

/// Decide how many swarm workers `run_plan` keeps active at once.
///
/// Policy:
///   * an explicit `requested` limit always wins (clamped to >= 1);
///   * deep mode with no explicit limit fans out wide: use `deep_cap`, where
///     `0` means "no extra cap" (`usize::MAX`) so the whole ready set is
///     dispatched, bounded only by the swarm member cap;
///   * light mode with no explicit limit keeps the small, cheap fan-out default.
///
/// Pure and side-effect free so the concurrency contract is unit-testable
/// without a live swarm.
pub(super) fn resolve_run_plan_concurrency(
    requested: Option<usize>,
    is_deep: bool,
    deep_cap: usize,
) -> usize {
    match requested {
        Some(explicit) => explicit.max(1),
        None if is_deep => {
            if deep_cap == 0 {
                usize::MAX
            } else {
                deep_cap
            }
        }
        None => LIGHT_MODE_DEFAULT_CONCURRENCY,
    }
}

/// Running tally of how well a `run_plan` drive used its concurrency budget.
///
/// Deep mode's promise is comprehensiveness through parallel fan-out, so a run
/// that finishes with peak parallelism ~1 despite a 32+ slot budget means the
/// graph was decomposed serially and the budget was wasted. Tracking this per
/// loop (max in-flight, plus how often open slots sat idle with no ready work)
/// turns "did we actually use the budget?" into a measured, reportable number
/// instead of a hope.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct RunPlanUtilization {
    /// Highest number of simultaneously in-flight tasks observed.
    pub(super) peak_in_flight: usize,
    /// Coordination loops observed.
    pub(super) loops: usize,
    /// Loops where open worker slots existed but the plan had nothing ready to
    /// dispatch into them (budget idle due to graph narrowness, not the cap).
    pub(super) starved_loops: usize,
}

impl RunPlanUtilization {
    /// Record one coordination loop. `open_slots` is `None` when the budget is
    /// unbounded (`concurrency_limit == usize::MAX`): an infinite budget has no
    /// meaningful starvation denominator, so only peak parallelism is tracked.
    pub(super) fn record_loop(
        &mut self,
        in_flight: usize,
        open_slots: Option<usize>,
        dispatched: usize,
    ) {
        self.loops += 1;
        self.peak_in_flight = self.peak_in_flight.max(in_flight + dispatched);
        if let Some(open_slots) = open_slots
            && open_slots > 0
            && dispatched < open_slots
        {
            self.starved_loops += 1;
        }
    }

    /// Render the utilization line for the terminal report. In deep mode a
    /// starved run also gets an actionable hint, because the fix (wider
    /// decomposition) belongs to the model reading this output.
    pub(super) fn report(&self, concurrency_limit: usize, is_deep: bool) -> String {
        let limit_label = if concurrency_limit == usize::MAX {
            "unbounded".to_string()
        } else {
            concurrency_limit.to_string()
        };
        let mut line = format!(
            "Budget utilization: peak {} of {} concurrent worker slot(s); {} of {} loop(s) had idle capacity with nothing ready.",
            self.peak_in_flight, limit_label, self.starved_loops, self.loops
        );
        let mostly_starved = self.loops > 0 && self.starved_loops * 2 >= self.loops;
        let ran_narrow = self.loops >= 3 && self.peak_in_flight <= 2;
        if is_deep && (mostly_starved || ran_narrow) {
            line.push_str(
                "\nDeep-mode hint: the graph ran much narrower than the agent budget. If coverage \
                 matters, expand remaining or follow-up work into MANY independent sibling nodes \
                 (depends_on only for real data dependencies) so the ready set fills the budget.",
            );
        }
        line
    }
}

/// Extract the background task id from its output file path
/// (`<task_id>.output`), mirroring the bash tool's convention so progress
/// updates can be routed back to the background task manager.
pub(super) fn task_id_from_output_path(path: &std::path::Path) -> Option<&str> {
    path.file_name()?.to_str()?.strip_suffix(".output")
}

/// Progress/log sink for a `run_plan` execution.
///
/// In background mode this appends human-readable lines to the background
/// task's output file and pushes determinate progress (terminal/total plan
/// nodes) into the background task manager, so the UI renders a live swarm
/// progress card and `bg status` stays meaningful. In inline (blocking) mode
/// every method is a no-op.
pub(super) struct RunPlanReporter {
    pub(super) task_id: Option<String>,
    output_path: Option<std::path::PathBuf>,
}

impl RunPlanReporter {
    pub(super) fn inline() -> Self {
        Self {
            task_id: None,
            output_path: None,
        }
    }

    pub(super) fn background(output_path: &std::path::Path) -> Self {
        Self {
            task_id: task_id_from_output_path(output_path).map(str::to_string),
            output_path: Some(output_path.to_path_buf()),
        }
    }

    /// Whether this reporter feeds a live background progress card (inline
    /// reporters are no-ops, so refresh polling would be wasted requests).
    pub(super) fn is_background(&self) -> bool {
        self.task_id.is_some()
    }

    pub(super) async fn log(&self, line: &str) {
        let Some(path) = &self.output_path else {
            return;
        };
        use tokio::io::AsyncWriteExt;
        if let Ok(mut file) = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
        {
            let _ = file.write_all(format!("{}\n", line).as_bytes()).await;
        }
    }

    pub(super) async fn progress(&self, terminal: usize, total: usize, message: String) {
        let Some(task_id) = &self.task_id else {
            return;
        };
        let progress = crate::bus::BackgroundTaskProgress {
            kind: crate::bus::BackgroundTaskProgressKind::Determinate,
            percent: None,
            message: Some(message),
            current: Some(terminal as u64),
            total: Some(total as u64),
            unit: Some("nodes".to_string()),
            eta_seconds: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
            source: crate::bus::BackgroundTaskProgressSource::Reported,
        }
        .normalize();
        let _ = crate::background::global()
            .update_progress(task_id, progress)
            .await;
    }

    /// Record an explicit checkpoint (a JCODE_CHECKPOINT-style milestone) on
    /// the background task, so pause/alert moments surface as checkpoint events
    /// in the UI instead of only trailing the output log. No-op inline.
    pub(super) async fn checkpoint(&self, message: &str) {
        self.log(message).await;
        let Some(task_id) = &self.task_id else {
            return;
        };
        let progress = crate::bus::BackgroundTaskProgress {
            kind: crate::bus::BackgroundTaskProgressKind::Indeterminate,
            percent: None,
            message: Some(message.to_string()),
            current: None,
            total: None,
            unit: None,
            eta_seconds: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
            source: crate::bus::BackgroundTaskProgressSource::Reported,
        }
        .normalize();
        let _ = crate::background::global()
            .update_checkpoint(task_id, progress)
            .await;
    }

    /// Rewrite the output file so `summary` leads and the progressive log
    /// trails it. Background completion previews take the first ~500 chars of
    /// the output file, so the terminal summary must come first for the
    /// agent's wake notification to be useful.
    pub(super) async fn finalize(&self, summary: &str) {
        let Some(path) = &self.output_path else {
            return;
        };
        let log = tokio::fs::read_to_string(path).await.unwrap_or_default();
        let content = if log.trim().is_empty() {
            format!("{}\n", summary)
        } else {
            format!("{}\n\n--- run log ---\n{}", summary, log)
        };
        let _ = tokio::fs::write(path, content).await;
    }
}

/// Per-process registry of sessions with a `run_plan` driver claimed or
/// running. The duplicate-driver guard does its check-and-insert under this
/// one lock, so two `run_plan` calls racing in the same batch cannot both
/// pass. Deliberately per-process: a stale `Running` status file left on disk
/// by a previous (reloaded/crashed) server process must never block
/// restarting the driver.
pub(super) fn run_plan_driver_claims()
-> &'static std::sync::Mutex<HashMap<String, RunPlanDriverClaim>> {
    static CLAIMS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, RunPlanDriverClaim>>> =
        std::sync::OnceLock::new();
    CLAIMS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub(super) enum RunPlanDriverClaim {
    /// Claimed, background task not spawned yet.
    Starting,
    /// Driver spawned as this background task.
    Running(String),
}

pub(super) enum RunPlanDriverClaimResult {
    Claimed(RunPlanClaimGuard),
    /// A driver already holds the claim. Carries its task id when known
    /// (None while the winner is still between claim and spawn).
    AlreadyRunning(Option<String>),
}

/// RAII holder for a `Starting` claim. Dropping it without
/// [`RunPlanClaimGuard::record_task`] releases the claim, so a cancelled or
/// failed startup path cannot permanently block `run_plan` for the session.
pub(super) struct RunPlanClaimGuard {
    session_id: String,
    defused: bool,
}

impl RunPlanClaimGuard {
    /// Upgrade the claim to `Running(task_id)`. From here staleness is
    /// resolved via `BackgroundTaskManager::is_live_task`: once the driver
    /// task finishes (and is pruned from the live map), the next claim
    /// replaces this entry.
    pub(super) fn record_task(mut self, task_id: &str) {
        let mut claims = run_plan_driver_claims()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        claims.insert(
            self.session_id.clone(),
            RunPlanDriverClaim::Running(task_id.to_string()),
        );
        self.defused = true;
    }
}

impl Drop for RunPlanClaimGuard {
    fn drop(&mut self) {
        if self.defused {
            return;
        }
        let mut claims = run_plan_driver_claims()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Only release a claim this guard still owns.
        if matches!(
            claims.get(&self.session_id),
            Some(RunPlanDriverClaim::Starting)
        ) {
            claims.remove(&self.session_id);
        }
    }
}

/// Atomically claim the `run_plan` driver slot for `session_id`.
///
/// Check-and-insert happens under one lock. An existing `Running` claim only
/// blocks while its background task is still live in this process; a claim
/// left by a finished (pruned) or pre-reload driver is replaced.
pub(super) fn try_claim_run_plan_driver(
    manager: &crate::background::BackgroundTaskManager,
    session_id: &str,
) -> RunPlanDriverClaimResult {
    let mut claims = run_plan_driver_claims()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match claims.get(session_id) {
        Some(RunPlanDriverClaim::Starting) => {
            return RunPlanDriverClaimResult::AlreadyRunning(None);
        }
        Some(RunPlanDriverClaim::Running(task_id)) => {
            if manager.is_live_task(task_id) {
                return RunPlanDriverClaimResult::AlreadyRunning(Some(task_id.clone()));
            }
            // Stale claim: the driver task already finished or belonged to a
            // previous process image. Fall through and take over.
        }
        None => {}
    }
    claims.insert(session_id.to_string(), RunPlanDriverClaim::Starting);
    RunPlanDriverClaimResult::Claimed(RunPlanClaimGuard {
        session_id: session_id.to_string(),
        defused: false,
    })
}

/// Drive `run_plan` as a managed background task and return immediately.
///
/// The coordinating agent stays responsive: the plan loop runs inside the
/// shared `BackgroundTaskManager` (task id, progress card, `bg` tool
/// integration), and completion is delivered through the standard notify/wake
/// path like any other background task.
pub(super) async fn run_swarm_plan_in_background(
    ctx: &ToolContext,
    params: CommunicateInput,
) -> Result<ToolOutput> {
    // Validate the plan inline so an empty/broken plan errors immediately
    // instead of as a delayed background failure.
    let initial_summary = fetch_plan_status(&ctx.session_id).await?;
    if initial_summary.item_count == 0 {
        return Ok(ToolOutput::new("No swarm plan items to run."));
    }

    // Refuse to start a second driver for the same session: two concurrent
    // run_plan loops would race on assignments and double-spawn workers. The
    // claim is check-and-insert under one lock, so two run_plan calls in the
    // same batch cannot both pass. Only drivers live in this process count; a
    // stale "running" status file left by a server reload must not block
    // restarting the driver (the claim map is per-process and dead task ids
    // fail the is_live_task check).
    let manager = crate::background::global();
    let claim = match try_claim_run_plan_driver(manager, &ctx.session_id) {
        RunPlanDriverClaimResult::Claimed(claim) => claim,
        RunPlanDriverClaimResult::AlreadyRunning(existing) => {
            return Ok(ToolOutput::new(match existing {
                Some(task_id) => format!(
                    "A swarm run_plan driver is already running for this session (task {}). \
                     Check it with `bg action=\"status\" task_id=\"{}\"` or `swarm plan_status` instead of starting another.",
                    task_id, task_id
                ),
                None => "A swarm run_plan driver is already starting for this session. \
                         Check it with `swarm plan_status` instead of starting another."
                    .to_string(),
            }));
        }
    };

    let notify = params.notify.unwrap_or(true);
    let wake = params.wake.unwrap_or(true);
    // Keep the display name free of the "·" separator used by the background
    // notification markdown header, or downstream parsing mis-splits the label.
    let display_name = format!(
        "run_plan ({} nodes, {} mode)",
        initial_summary.item_count, initial_summary.mode
    );

    let bg_ctx = ctx.clone();
    let info = crate::background::global()
        .spawn_with_notify(
            "swarm",
            Some(display_name.clone()),
            &ctx.session_id,
            notify,
            wake,
            move |output_path| async move {
                let reporter = RunPlanReporter::background(&output_path);
                match run_swarm_plan_to_terminal(&bg_ctx, &params, &reporter).await {
                    Ok(output) => {
                        reporter.finalize(&output.output).await;
                        Ok(TaskResult::completed(Some(0)))
                    }
                    Err(error) => {
                        let message = format!("run_plan failed: {}", error);
                        reporter.finalize(&message).await;
                        Ok(TaskResult::failed(None, message))
                    }
                }
            },
        )
        .await;
    claim.record_task(&info.task_id);

    let delivery_note = if wake {
        "You'll be woken with the result when the plan reaches a terminal state."
    } else if notify {
        "A notification will appear when the plan reaches a terminal state."
    } else {
        "Notifications disabled. Use the `bg` tool to check status."
    };
    let output = format!(
        "🐝 Swarm plan running in background.\n\n\
         Task ID: {}\n\
         Plan: {} node(s), {} mode\n\
         Output file: {}\n\n\
         {}\n\
         Check progress: use the `bg` tool with action=\"status\" and task_id=\"{}\", or `swarm plan_status`.\n\
         Note: a server reload stops this driver (workers keep running); rerun `swarm run_plan` to resume driving the same plan.",
        info.task_id,
        initial_summary.item_count,
        initial_summary.mode,
        info.output_file.display(),
        delivery_note,
        info.task_id,
    );

    Ok(ToolOutput::new(output)
        .with_title(format!("Swarm run_plan in background: {}", info.task_id))
        .with_metadata(json!({
            "background": true,
            "swarm": true,
            "task_id": info.task_id,
            "display_name": display_name,
            "output_file": info.output_file.to_string_lossy(),
            "status_file": info.status_file.to_string_lossy(),
        })))
}

/// Hint appended to every `run_plan` driver failure: the driver exits without
/// the end-of-run cleanup, so spawned workers keep running even when
/// `retain_agents=false`, and the caller must know how to stop or resume them.
pub(super) const RUN_PLAN_WORKER_RETENTION_HINT: &str = "\nSpawned workers were retained; run `swarm cleanup` to stop them, rerun `swarm run_plan` to resume driving the same plan, or `swarm plan_status` to inspect.";

/// Append the worker-retention hint to a driver failure message, idempotently
/// so wrappers that re-report an already-hinted error do not duplicate it.
pub(super) fn with_worker_retention_hint(message: String) -> String {
    if message.contains("swarm cleanup") {
        message
    } else {
        format!("{message}{RUN_PLAN_WORKER_RETENTION_HINT}")
    }
}
