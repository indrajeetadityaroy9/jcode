use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitStatus};

use crate::build;
use crate::bus::{Bus, BusEvent, ClientMaintenanceAction, SessionUpdateStatus};

pub fn hot_rebuild(session_id: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_dir =
        build::get_repo_dir().ok_or_else(|| anyhow::anyhow!("Could not find jcode repository"))?;

    eprintln!("Rebuilding jcode with session {}...", session_id);
    pull_latest_changes_for_rebuild(&repo_dir);
    run_release_build(&repo_dir)?;
    run_release_tests(&repo_dir)?;
    install_local_release_with_warning(&repo_dir);

    let exe = rebuild_reload_candidate(&repo_dir);
    if !exe.exists() {
        anyhow::bail!("Binary not found at {:?}", exe);
    }

    print_centered(&format!("Restarting with session {}...", session_id));
    exec_rebuilt_session(&exe, session_id, &cwd)
}

pub fn spawn_background_session_rebuild(session_id: String) {
    std::thread::spawn(move || run_background_session_rebuild(session_id));
}

fn pull_latest_changes_for_rebuild(repo_dir: &Path) {
    eprintln!("Pulling latest changes...");
    if let Err(e) = run_git_pull_ff_only(repo_dir, true) {
        eprintln!("Warning: {}. Continuing with current version.", e);
    }
}

fn run_release_build(repo_dir: &Path) -> Result<()> {
    eprintln!("Building...");
    let status = run_cargo_release_step(repo_dir, &["build", "--release"])?;
    if !status.success() {
        anyhow::bail!("Build failed - staying on current version");
    }
    Ok(())
}

fn run_release_tests(repo_dir: &Path) -> Result<()> {
    eprintln!("Running tests...");
    let status =
        run_cargo_release_step(repo_dir, &["test", "--release", "--", "--test-threads=1"])?;
    if !status.success() {
        crate::terminal_eprintln!("\n⚠️  Tests failed! Aborting reload to protect your session.");
        eprintln!("Fix the failing tests and try /rebuild again.");
        anyhow::bail!("Tests failed - staying on current version");
    }
    eprintln!("✓ All tests passed");
    Ok(())
}

fn run_cargo_release_step(repo_dir: &Path, args: &[&str]) -> Result<ExitStatus> {
    Ok(ProcessCommand::new("cargo")
        .args(args)
        .current_dir(repo_dir)
        .status()?)
}

fn install_local_release_with_warning(repo_dir: &Path) {
    if let Err(e) = build::install_local_release(repo_dir) {
        eprintln!("Warning: install failed: {}", e);
    }
}

fn rebuild_reload_candidate(repo_dir: &Path) -> PathBuf {
    build::client_update_candidate()
        .map(|(path, _)| path)
        .unwrap_or_else(|| build::release_binary_path(repo_dir))
}

fn exec_rebuilt_session(exe: &Path, session_id: &str, cwd: &Path) -> Result<()> {
    crate::env::set_var("JCODE_RESUMING", "1");

    let mut cmd = ProcessCommand::new(exe);
    cmd.arg("--resume").arg(session_id).current_dir(cwd);
    let err = crate::platform::replace_process(&mut cmd);

    Err(anyhow::anyhow!("Failed to exec {:?}: {}", exe, err))
}

fn run_background_session_rebuild(session_id: String) {
    let publisher = BackgroundRebuildPublisher::new(session_id);
    let Some(repo_dir) = build::get_repo_dir() else {
        publisher.error("Rebuild failed: could not find the jcode repository.");
        return;
    };

    background_pull_latest_changes(&publisher, &repo_dir);
    if !background_release_build(&publisher, &repo_dir) {
        return;
    }
    if !background_release_tests(&publisher, &repo_dir) {
        return;
    }
    background_install_local_release(&publisher, &repo_dir);
    publish_rebuild_ready_or_error(publisher, &repo_dir);
}

#[derive(Clone)]
struct BackgroundRebuildPublisher {
    session_id: String,
    action: ClientMaintenanceAction,
}

impl BackgroundRebuildPublisher {
    fn new(session_id: String) -> Self {
        Self {
            session_id,
            action: ClientMaintenanceAction::Rebuild,
        }
    }

    fn status(&self, message: impl Into<String>) {
        self.publish(SessionUpdateStatus::Status {
            session_id: self.session_id.clone(),
            action: self.action,
            message: message.into(),
        });
    }

    fn error(&self, message: impl Into<String>) {
        self.publish(SessionUpdateStatus::Error {
            session_id: self.session_id.clone(),
            action: self.action,
            message: message.into(),
        });
    }

    fn ready(self, repo_dir: &Path) {
        Bus::global().publish(BusEvent::SessionUpdateStatus(
            SessionUpdateStatus::ReadyToReload {
                session_id: self.session_id,
                action: self.action,
                version: rebuild_version_label(repo_dir),
            },
        ));
    }

    fn publish(&self, status: SessionUpdateStatus) {
        Bus::global().publish(BusEvent::SessionUpdateStatus(status));
    }
}

fn background_pull_latest_changes(publisher: &BackgroundRebuildPublisher, repo_dir: &Path) {
    publisher.status("Pulling latest changes in the background...");
    if let Err(error) = run_git_pull_ff_only(repo_dir, true) {
        publisher.status(format!(
            "Git pull skipped: {}. Continuing with the current checkout.",
            error
        ));
    }
}

fn background_release_build(publisher: &BackgroundRebuildPublisher, repo_dir: &Path) -> bool {
    publisher.status("Building release binary in the background...");
    let status = match run_cargo_release_step(repo_dir, &["build", "--release"]) {
        Ok(status) => status,
        Err(error) => {
            publisher.error(format!(
                "Rebuild failed while starting cargo build: {}",
                error
            ));
            return false;
        }
    };

    if !status.success() {
        publisher.error("Build failed — staying on the current binary.");
        return false;
    }
    true
}

fn background_release_tests(publisher: &BackgroundRebuildPublisher, repo_dir: &Path) -> bool {
    publisher.status("Running release tests in the background...");
    let status =
        match run_cargo_release_step(repo_dir, &["test", "--release", "--", "--test-threads=1"]) {
            Ok(status) => status,
            Err(error) => {
                publisher.error(format!("Rebuild failed while starting tests: {}", error));
                return false;
            }
        };

    if !status.success() {
        publisher.error(
            "Tests failed — staying on the current binary. Fix the failing tests and try /rebuild again.",
        );
        return false;
    }
    true
}

fn background_install_local_release(publisher: &BackgroundRebuildPublisher, repo_dir: &Path) {
    if let Err(error) = build::install_local_release(repo_dir) {
        publisher.status(format!(
            "Install warning: {}. Will reload from the repo build if needed.",
            error
        ));
    }
}

fn publish_rebuild_ready_or_error(publisher: BackgroundRebuildPublisher, repo_dir: &Path) {
    let exe = build::preferred_reload_candidate()
        .map(|(path, _)| path)
        .unwrap_or_else(|| build::release_binary_path(repo_dir));
    if !exe.exists() {
        publisher.error(format!(
            "Rebuild finished but no reloadable binary was found at {:?}.",
            exe
        ));
        return;
    }

    publisher.ready(repo_dir);
}

fn rebuild_version_label(repo_dir: &Path) -> String {
    build::current_build_info(repo_dir)
        .map(|info| {
            if info.dirty {
                format!("{}-dirty", info.hash)
            } else {
                info.hash
            }
        })
        .unwrap_or_else(|_| "local source build".to_string())
}

/// Summary emitted when `git pull` cannot reconcile the local and upstream
/// histories on its own (diverged branches, non-fast-forward, unrelated
/// histories).
const GIT_PULL_DIVERGED_SUMMARY: &str =
    "Local and upstream have diverged, so the update could not fast-forward.";

/// Longest single-line rebuild summary we hand to the UI.
const REBUILD_ERROR_SUMMARY_MAX_CHARS: usize = 72;

pub fn run_git_pull_ff_only(repo_dir: &Path, quiet: bool) -> Result<()> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("pull").arg("--ff-only");
    if quiet {
        cmd.arg("-q");
    }
    let output = cmd
        .current_dir(repo_dir)
        .output()
        .context("Failed to run git pull")?;

    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!("{}", summarize_git_pull_failure(&output.stderr));
    }
}

fn summarize_git_pull_failure(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let text = stderr.trim();
    if text.is_empty() {
        return "git pull failed".to_string();
    }

    if git_pull_failure_is_divergence(text) {
        return GIT_PULL_DIVERGED_SUMMARY.to_string();
    }

    if text.contains("There is no tracking information for the current branch") {
        return "git pull failed: current branch has no upstream tracking branch".to_string();
    }

    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("hint:"))
        .unwrap_or("git pull failed");
    let line = line.strip_prefix("fatal: ").unwrap_or(line);
    if line.eq_ignore_ascii_case("git pull failed") {
        "git pull failed".to_string()
    } else {
        format!("git pull failed: {}", line)
    }
}

/// Whether `git pull` stderr indicates the local and upstream branches have
/// diverged (and therefore need a manual merge/rebase, not a fast-forward).
fn git_pull_failure_is_divergence(stderr: &str) -> bool {
    stderr.contains("Need to specify how to reconcile divergent branches")
        || stderr.contains("Not possible to fast-forward")
        || stderr.contains("refusing to merge unrelated histories")
        || stderr.contains("have diverged")
}

/// Condense a rebuild-pipeline error into a single short line fit for a status
/// notice or a one-line card.
///
/// Rebuild errors reach the UI from several layers (git, cargo, the test run,
/// the local install), so raw text is often multi-line and long enough to wrap
/// several times. Users only need the first clause; the full text stays in the
/// log.
pub fn summarize_rebuild_error(error: &str) -> String {
    let first_line = error
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("unknown error");

    // Keep one clause: drop any trailing context sentence and punctuation.
    let clause = first_line
        .split_once(". ")
        .map(|(head, _)| head)
        .unwrap_or(first_line)
        .trim_end_matches(['.', ':'])
        .trim();
    let clause = if clause.is_empty() {
        first_line
    } else {
        clause
    };

    if clause.chars().count() <= REBUILD_ERROR_SUMMARY_MAX_CHARS {
        return clause.to_string();
    }
    let truncated: String = clause
        .chars()
        .take(REBUILD_ERROR_SUMMARY_MAX_CHARS - 1)
        .collect();
    format!("{}…", truncated.trim_end())
}

fn print_centered(msg: &str) {
    let msg = crate::output_style::terminal_text(msg);
    let width = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80);
    for line in msg.lines() {
        let visible_len = unicode_display_width(line);
        if visible_len >= width {
            println!("{}", line);
        } else {
            let pad = (width - visible_len) / 2;
            println!("{:>pad$}{}", "", line, pad = pad);
        }
    }
}

fn unicode_display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    let mut w = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        w += UnicodeWidthChar::width(c).unwrap_or(0);
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_pull_failure_summaries_are_stable() {
        assert_eq!(
            summarize_git_pull_failure(
                b"fatal: Need to specify how to reconcile divergent branches\n"
            ),
            GIT_PULL_DIVERGED_SUMMARY
        );
        assert_eq!(
            summarize_git_pull_failure(b"hint: ignore me\nfatal: no upstream\n"),
            "git pull failed: no upstream"
        );
        assert_eq!(summarize_git_pull_failure(b"   \n"), "git pull failed");
    }

    /// Every UI surface renders these on one line, so the summary must stay
    /// short and never contain a newline.
    #[test]
    fn summarize_rebuild_error_is_always_one_short_line() {
        let inputs = [
            "Tests failed — staying on the current binary. Fix the failing tests and try /rebuild again.",
            "Rebuild failed while starting cargo build: No such file or directory (os error 2)\n  caused by: cargo",
            "a very long single clause with no recognizable cause that just keeps going and going well past any sensible terminal width",
            "",
        ];
        for input in inputs {
            let summary = summarize_rebuild_error(input);
            assert!(!summary.contains('\n'), "multi-line summary for {input:?}");
            assert!(!summary.is_empty(), "empty summary for {input:?}");
            assert!(
                summary.chars().count() <= REBUILD_ERROR_SUMMARY_MAX_CHARS,
                "summary too long ({}) for {input:?}: {summary}",
                summary.chars().count()
            );
        }
    }

    #[test]
    fn summarize_rebuild_error_keeps_the_first_clause() {
        assert_eq!(
            summarize_rebuild_error(
                "Tests failed — staying on the current binary. Fix the failing tests and try /rebuild again."
            ),
            "Tests failed — staying on the current binary"
        );
        assert_eq!(
            summarize_rebuild_error("Build failed — staying on the current binary."),
            "Build failed — staying on the current binary"
        );
    }
}
