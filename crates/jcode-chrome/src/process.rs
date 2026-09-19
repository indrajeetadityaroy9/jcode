//! Launching and supervising the headless Chrome child.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};

/// Written inside the profile directory so a later run can reclaim an orphan.
const PID_FILE: &str = "jcode-chrome.pid";

/// Profile directories are named `chrome-profile-<owning daemon pid>`; the
/// sweep parses that suffix to decide whether a directory is abandoned.
const PROFILE_PREFIX: &str = "chrome-profile-";

/// Installed Chrome builds, in the order they are preferred. macOS-only fork,
/// so these are the only locations worth probing.
const CHROME_PATHS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
];

pub(crate) fn resolve_binary(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        bail!(
            "configured Chrome binary does not exist: {}",
            path.display()
        );
    }
    for candidate in CHROME_PATHS {
        let path = Path::new(candidate);
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }
    bail!("no Chrome installation found; install Google Chrome or set [websearch] chrome_binary")
}

/// Secrets a Chrome child must not receive merely because jcode has them.
/// Mirrors `is_sensitive_inherited_env_key` in `jcode-base`'s MCP client; the
/// predicate is duplicated rather than shared because this is a leaf crate.
fn is_sensitive_inherited_env_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.ends_with("_API_KEY")
        || key.ends_with("_ACCESS_TOKEN")
        || key.ends_with("_AUTH_TOKEN")
        || key.ends_with("_SECRET")
        || key.starts_with("AWS_")
        || key.starts_with("AZURE_")
        || key == "GOOGLE_APPLICATION_CREDENTIALS"
}

fn child_env() -> HashMap<String, String> {
    std::env::vars()
        .filter(|(key, _)| !is_sensitive_inherited_env_key(key))
        .collect()
}

fn pid_is_alive(pid: i32) -> bool {
    // Signal 0 performs the permission and existence checks without delivering
    // anything, which is exactly the liveness probe wanted here.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Remove profile directories whose owning daemon is gone, killing any Chrome
/// still holding them.
///
/// Required because the daemon's SIGTERM handler calls `std::process::exit(0)`
/// and therefore runs no destructor: without this sweep, orphaned Chromes and
/// their profiles accumulate.
pub(crate) fn sweep_abandoned_profiles(parent_dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(parent_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || !path.is_dir() {
            continue;
        }
        let Some(owner) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(PROFILE_PREFIX))
            .and_then(|pid| pid.parse::<i32>().ok())
        else {
            continue;
        };
        if pid_is_alive(owner) {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(path.join(PID_FILE))
            && let Ok(chrome_pid) = raw.trim().parse::<i32>()
            && pid_is_alive(chrome_pid)
        {
            unsafe {
                libc::kill(chrome_pid, libc::SIGKILL);
            }
        }
        let _ = std::fs::remove_dir_all(&path);
    }
}

pub(crate) fn spawn(binary: &Path, user_data_dir: &Path) -> Result<Child> {
    let child = Command::new(binary)
        .arg("--headless")
        // Ephemeral port: Chrome picks a free one and reports it in
        // DevToolsActivePort, so this can never collide with the user's Chrome.
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        // Both flags come from Chromium's own macOS build documentation
        // ("Avoiding system permissions dialogs"): they suppress the keychain
        // and incoming-connection prompts.
        .arg("--use-mock-keychain")
        .arg("--disable-features=DialMediaRouteProvider")
        .arg("about:blank")
        .env_clear()
        .envs(child_env())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to spawn Chrome at {}", binary.display()))?;

    // The sweep in a later run can only reclaim this Chrome if its pid is on
    // disk, so a failure here is worth surfacing rather than ignoring.
    if let Some(pid) = child.id() {
        let pid_path = user_data_dir.join(PID_FILE);
        std::fs::write(&pid_path, pid.to_string())
            .with_context(|| format!("failed to record Chrome pid at {}", pid_path.display()))?;
    }
    Ok(child)
}

/// Read the debugging port Chrome chose.
///
/// Chrome writes `DevToolsActivePort` with the port on line 1 and the browser
/// target path on line 2 (no trailing newline). Only line 1 is needed; the
/// WebSocket URL is taken from `/json/version` instead.
pub(crate) async fn await_debug_port(user_data_dir: &Path, timeout: Duration) -> Result<u16> {
    let port_file = user_data_dir.join("DevToolsActivePort");
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_error = String::from("file never appeared");
    while tokio::time::Instant::now() < deadline {
        match std::fs::read_to_string(&port_file) {
            Ok(raw) => match raw.lines().next().unwrap_or("").trim().parse::<u16>() {
                Ok(port) if port != 0 => return Ok(port),
                Ok(_) => last_error = "port was 0".to_string(),
                Err(err) => last_error = err.to_string(),
            },
            Err(err) => last_error = err.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!(
        "Chrome did not report a debugging port in {}: {last_error}",
        port_file.display()
    )
}
