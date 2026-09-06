//! Platform setup hints shown on startup.
//!
//! - macOS: if the user is on the default built-in Terminal.app, show a one-time
//!   notice that it renders jcode poorly and suggest a modern terminal (Ghostty),
//!   and install the Spotlight/Dock launcher app.
//! - Linux: create a .desktop launcher file.
//!
//! Each nudge can be dismissed permanently with "Don't ask again".
//! State is persisted in `~/.jcode/setup_hints.json`.
#![cfg_attr(all(test, not(target_os = "macos")), allow(dead_code))]

#[cfg(any(target_os = "macos", target_os = "linux"))]
use anyhow::Context;
use anyhow::Result;
use jcode_storage as storage;
use serde::{Deserialize, Serialize};
use std::io::{self, IsTerminal};
use std::path::PathBuf;

pub mod keymap;

#[cfg(any(test, target_os = "macos"))]
mod macos_launcher;
#[cfg(any(test, target_os = "macos"))]
mod macos_terminal;
#[cfg(any(test, target_os = "macos"))]
use macos_launcher::{install_macos_app_launcher, should_refresh_macos_app_launcher};
#[cfg(any(test, target_os = "macos"))]
use macos_terminal::{
    MacTerminalKind, effective_macos_terminal, escape_applescript_text, escape_shell_single_quotes,
    launch_command_for_macos_terminal, paused_jcode_shell_command, save_preferred_macos_terminal,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SetupHintsState {
    pub launch_count: u64,
    #[serde(alias = "wezterm_configured")]
    pub alacritty_configured: bool,
    #[serde(alias = "wezterm_dismissed")]
    pub alacritty_dismissed: bool,
    #[serde(default)]
    pub desktop_shortcut_created: bool,
    pub mac_ghostty_guided: bool,
    pub mac_ghostty_dismissed: bool,
    /// Number of times we have shown the terminal/setup nudge prompt to the user
    /// (across all platforms). Used to cap the total number of nudges so we never
    /// pester someone forever if they keep choosing "Not now".
    #[serde(default)]
    pub terminal_nudge_count: u64,
    /// Canonical signature of the keybinding conflicts we last warned the user
    /// about (sorted, joined chord+field pairs). Empty means "no conflicts known
    /// / never warned". We only re-show the startup conflict notice when this
    /// signature changes, so users are warned once per distinct conflict set and
    /// never nagged about the same conflicts on every launch.
    #[serde(default)]
    pub keymap_conflict_signature: String,
    /// Whether we've shown the one-time "glyph-safe mode is active" disclosure
    /// for fragile-glyph terminals (macOS VS Code integrated terminal / Apple
    /// Terminal). We surface the tradeoff once per install so the user knows
    /// colors are quantized to 256 to avoid the terminal's glyph corruption.
    #[serde(default)]
    pub glyph_safe_notice_shown: bool,
}

impl Default for SetupHintsState {
    fn default() -> Self {
        Self {
            launch_count: 0,
            alacritty_configured: false,
            alacritty_dismissed: false,
            desktop_shortcut_created: false,
            mac_ghostty_guided: false,
            mac_ghostty_dismissed: false,
            terminal_nudge_count: 0,
            keymap_conflict_signature: String::new(),
            glyph_safe_notice_shown: false,
        }
    }
}

/// Maximum number of times we will ever show the terminal/setup nudge prompt
/// to a user (across all launches and platforms). After this many nudges we stop
/// asking, even if the user never explicitly picked "Don't ask again".
pub const MAX_TERMINAL_NUDGES: u64 = 5;

#[derive(Debug, Clone, Default)]
pub struct StartupHints {
    pub auto_send_message: Option<String>,
    pub status_notice: Option<String>,
    pub display_message: Option<(String, String)>,
}

impl StartupHints {
    fn with_status_and_display(
        status_notice: String,
        title: impl Into<String>,
        display_message: String,
    ) -> Self {
        Self {
            auto_send_message: None,
            status_notice: Some(status_notice),
            display_message: Some((title.into(), display_message)),
        }
    }
}

impl SetupHintsState {
    fn path() -> Result<PathBuf> {
        Ok(storage::jcode_dir()?.join("setup_hints.json"))
    }

    pub fn load() -> Self {
        let Ok(path) = Self::path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Load state from `path`, falling back to its `.bak` sibling.
    ///
    /// The atomic writer keeps the previous version at `.bak`. If the primary
    /// file is missing or unreadable (deleted, interrupted swap), fall back to
    /// it instead of silently resetting state like `launch_count`, which the
    /// hint cadence relies on.
    fn load_from(path: &std::path::Path) -> Self {
        if let Ok(state) = storage::read_json(path) {
            return state;
        }
        let bak = path.with_extension("bak");
        storage::read_json(&bak).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        // Best-effort UI state (launch counter + one-time hint/nudge flags).
        // This is written on every interactive launch and is not durability
        // critical: losing the most recent update on a power cut just re-shows a
        // hint or under-counts a launch. Use the non-fsync fast write so we do
        // not pay macOS's `F_FULLFSYNC` (full disk-platter flush, ~8ms here)
        // twice on the startup critical path. The atomic rename still protects
        // against torn/partial writes, and load() falls back to `.bak`.
        storage::write_json_fast(&path, self)
    }

    /// Whether we are still allowed to show a terminal/setup nudge. Once we have
    /// shown the prompt `MAX_TERMINAL_NUDGES` times we stop asking entirely.
    #[cfg(any(test, target_os = "macos"))]
    fn nudge_budget_remaining(&self) -> bool {
        self.terminal_nudge_count < MAX_TERMINAL_NUDGES
    }

    /// Record that a nudge prompt was shown to the user and persist the count.
    /// Only invoked on the macOS nudge path; under `cfg(test)` on other
    /// platforms it compiles but has no caller.
    #[cfg(any(test, target_os = "macos"))]
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only called on the macOS nudge path")
    )]
    fn record_nudge_shown(&mut self) {
        self.terminal_nudge_count = self.terminal_nudge_count.saturating_add(1);
        let _ = self.save();
    }
}

/// Launch a new jcode window in the user's preferred macOS terminal, passing
/// `extra_args` (e.g. `["--resume", "<session-id>"]`) to the jcode invocation.
///
/// This reuses the same terminal detection as the launcher app, but
/// deliberately avoids AppleScript automation: callers like the menu bar
/// helper run as background processes that cannot present the "control
/// Terminal" TCC prompt, so `osascript` would fail. Terminals that support
/// `open -na <App> --args ...` are launched directly; for the rest we write
/// the launch command to an executable `.command` file and `open` it, which
/// Terminal/iTerm run in a new window without any automation permission.
#[cfg(target_os = "macos")]
pub fn launch_jcode_in_macos_terminal(extra_args: &[String]) -> Result<()> {
    let terminal = effective_macos_terminal();
    let exe = std::env::current_exe()?;
    let exe_path = exe.to_string_lossy().into_owned();
    let shell_command = macos_terminal::paused_jcode_shell_command_with_args(&exe_path, extra_args);

    let command = match macos_terminal::no_automation_launch(terminal, &shell_command) {
        macos_terminal::NoAutomationLaunch::Shell(command) => command,
        macos_terminal::NoAutomationLaunch::CommandFile { app } => {
            let dir = storage::jcode_dir()?.join("launcher");
            std::fs::create_dir_all(&dir)?;
            let script_path = dir.join("open_session.command");
            std::fs::write(
                &script_path,
                format!("#!/bin/bash\nclear\n{shell_command}\n"),
            )?;
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
            }
            let target =
                macos_terminal::escape_shell_single_quotes(script_path.to_string_lossy().as_ref());
            match app {
                Some(app) => format!("/usr/bin/open -a {app} '{target}'"),
                None => format!("/usr/bin/open '{target}'"),
            }
        }
    };

    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .status()
        .context("failed to launch terminal for jcode")?;
    if !status.success() {
        anyhow::bail!(
            "terminal launch command exited with status {:?}",
            status.code()
        );
    }
    Ok(())
}

/// Pure decision for the macOS terminal notice, given the detected terminal.
///
/// We deliberately only nudge for the default built-in Terminal.app: other
/// terminals (iTerm2, WezTerm, Alacritty, Ghostty, etc.) are fine, so we leave
/// them alone. Regardless of the result the nudge is marked handled so it is
/// only ever shown once. The notice is informational (no prompt, no AI handoff).
///
/// This mutates `state`'s nudge flags but does not persist; the caller is
/// responsible for saving.
#[cfg(any(test, target_os = "macos"))]
fn macos_terminal_notice(
    state: &mut SetupHintsState,
    terminal: MacTerminalKind,
) -> Option<StartupHints> {
    state.mac_ghostty_guided = true;
    state.mac_ghostty_dismissed = true;

    if terminal != MacTerminalKind::AppleTerminal {
        return None;
    }

    let message = "The built-in macOS Terminal.app renders jcode poorly (slow, limited colors, no inline images). Consider a modern terminal such as Ghostty, iTerm2, or Alacritty for a much better experience.".to_string();

    Some(StartupHints::with_status_and_display(
        "Tip: Terminal.app renders jcode poorly. Try Ghostty, iTerm2, or Alacritty.".to_string(),
        "Terminal",
        message,
    ))
}

/// macOS entry point: show the one-time Terminal.app notice for the effective
/// terminal.
#[cfg(target_os = "macos")]
fn nudge_macos_ghostty(state: &mut SetupHintsState) -> Option<StartupHints> {
    let hints = macos_terminal_notice(state, effective_macos_terminal());
    let _ = state.save();
    hints
}

/// Main entry point: check if we should show setup hints.
///
/// Called early in startup, before the TUI is initialized.
/// Returns optional structured startup hints for the TUI.
///
/// - macOS: refreshes the Spotlight/Dock launcher app, and on every 3rd launch
///   can surface the one-time Terminal.app notice.
/// - Other platforms: creates a desktop launcher entry once.
pub fn maybe_show_setup_hints() -> Option<StartupHints> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return None;
    }

    let mut state = SetupHintsState::load();
    state.launch_count += 1;
    let _ = state.save();

    #[cfg(any(test, target_os = "macos"))]
    {
        if should_refresh_macos_app_launcher(&state) {
            let _ = create_desktop_shortcut(&mut state);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if !state.desktop_shortcut_created {
            let _ = create_desktop_shortcut(&mut state);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if state.launch_count % 3 != 0 {
            return None;
        }

        if !state.mac_ghostty_guided
            && !state.mac_ghostty_dismissed
            && state.nudge_budget_remaining()
        {
            state.record_nudge_shown();
            return nudge_macos_ghostty(&mut state);
        }

        return None;
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Pure debounce decision for the keybinding-conflict notice.
///
/// Given the freshly-computed conflict `signature` and the `previous` signature
/// we last stored, decide what to do. Separated from I/O so the
/// warn-once-per-change policy can be unit-tested without touching the machine
/// or the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConflictHintDecision {
    /// Nothing changed since last time; stay silent and leave state untouched.
    Unchanged,
    /// The conflict set changed but is now empty (resolved); update the stored
    /// signature but show nothing.
    ResolvedSilently,
    /// New or changed conflicts; update the stored signature and show a notice.
    Warn,
}

pub(crate) fn conflict_hint_decision(signature: &str, previous: &str) -> ConflictHintDecision {
    if signature == previous {
        ConflictHintDecision::Unchanged
    } else if signature.is_empty() {
        ConflictHintDecision::ResolvedSilently
    } else {
        ConflictHintDecision::Warn
    }
}

/// Check whether jcode's keybindings conflict with shortcuts owned by the
/// terminal or the OS, and return a one-time startup notice when the set of
/// conflicts has changed since we last warned.
///
/// This is config-aware (the caller passes the user's live keybindings) and
/// debounced via a stored signature: a user is warned once per distinct
/// conflict set and never nagged about the same conflicts on subsequent
/// launches. Returns `None` when there are no conflicts, when nothing changed,
/// or when input is not a real TTY.
///
/// The actual diagnostics are always available on demand via the `/keys`
/// command; this only surfaces the proactive heads-up.
pub fn maybe_show_keymap_conflict_hint(
    keybindings: &jcode_config_types::KeybindingsConfig,
) -> Option<StartupHints> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return None;
    }

    let snapshot = keymap::snapshot_cached_or_refresh();
    let mut state = SetupHintsState::load();
    let (hint, changed) = keymap_conflict_hint_for(keybindings, &snapshot, &mut state);
    if changed {
        let _ = state.save();
    }
    hint
}

/// Core of [`maybe_show_keymap_conflict_hint`], separated from TTY detection and
/// disk I/O so the full decision + state-update path is unit-testable.
///
/// Returns the optional notice and whether `state` was mutated (and therefore
/// should be persisted by the caller).
pub(crate) fn keymap_conflict_hint_for(
    keybindings: &jcode_config_types::KeybindingsConfig,
    snapshot: &keymap::KeymapSnapshot,
    state: &mut SetupHintsState,
) -> (Option<StartupHints>, bool) {
    let conflicts = keymap::detect_conflicts(keybindings, snapshot);
    let signature = keymap::conflict_signature(&conflicts);

    match conflict_hint_decision(&signature, &state.keymap_conflict_signature) {
        ConflictHintDecision::Unchanged => (None, false),
        ConflictHintDecision::ResolvedSilently => {
            state.keymap_conflict_signature = signature;
            (None, true)
        }
        ConflictHintDecision::Warn => {
            state.keymap_conflict_signature = signature;
            let hint = keymap::render_status_line(keybindings, snapshot).map(|status| {
                let display = keymap::render_report(keybindings, snapshot);
                StartupHints::with_status_and_display(status, "Keybindings", display)
            });
            (hint, true)
        }
    }
}

/// Whether the current terminal triggers jcode's glyph-safe color quantization
/// (macOS VS Code integrated terminal / Apple Terminal). Mirrors the detection
/// in `jcode-tui-style`'s color module and `jcode-app-core::perf` so the
/// disclosure fires exactly when the behavior is active. Overridable with
/// `JCODE_GLYPH_SAFE_MODE=on|off`.
fn glyph_safe_mode_active() -> bool {
    if let Ok(raw) = std::env::var("JCODE_GLYPH_SAFE_MODE") {
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => return true,
            "0" | "false" | "no" | "off" => return false,
            _ => {}
        }
    }
    if !cfg!(target_os = "macos") {
        return false;
    }
    match std::env::var("TERM_PROGRAM") {
        Ok(tp) => {
            let tp = tp.to_ascii_lowercase();
            tp == "vscode" || tp == "apple_terminal"
        }
        Err(_) => false,
    }
}

/// One-time disclosure that glyph-safe mode (256-color quantization) is active,
/// shown the first time jcode launches in a fragile-glyph terminal. Discloses
/// the tradeoff (slightly reduced color fidelity) and how to opt out.
pub fn maybe_show_glyph_safe_notice() -> Option<StartupHints> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return None;
    }
    let mut state = SetupHintsState::load();
    let (hint, changed) = glyph_safe_notice_for(glyph_safe_mode_active(), &mut state);
    if changed {
        let _ = state.save();
    }
    hint
}

/// Core of [`maybe_show_glyph_safe_notice`], split out for unit testing.
/// Returns the optional notice and whether `state` was mutated.
pub(crate) fn glyph_safe_notice_for(
    active: bool,
    state: &mut SetupHintsState,
) -> (Option<StartupHints>, bool) {
    if !active || state.glyph_safe_notice_shown {
        return (None, false);
    }
    state.glyph_safe_notice_shown = true;
    let status =
        "Glyph-safe mode: colors quantized to 256 to avoid this terminal's glyph corruption."
            .to_string();
    let display = "This terminal (VS Code integrated terminal / Apple Terminal on macOS) corrupts \
its glyph cache under jcode's full-color animations, rendering letters as boxes. \
jcode automatically quantizes colors to the 256-palette here to keep text readable; \
the only tradeoff is slightly reduced color fidelity. Animations still run. \
For full color, use Ghostty, iTerm2, kitty, or WezTerm, or set JCODE_GLYPH_SAFE_MODE=off."
        .to_string();
    (
        Some(StartupHints::with_status_and_display(
            status, "Display", display,
        )),
        true,
    )
}

/// Create a desktop shortcut/launcher for jcode.
///
/// - macOS: creates a jcode.app bundle in ~/Applications/
fn create_desktop_shortcut(state: &mut SetupHintsState) -> Result<()> {
    #[cfg(any(test, target_os = "macos"))]
    {
        let (app_dir, _terminal) = install_macos_app_launcher()?;

        state.desktop_shortcut_created = true;
        let _ = state.save();

        jcode_logging::info(&format!("Created macOS app bundle: {}", app_dir.display()));
    }

    #[cfg(not(any(test, target_os = "macos")))]
    {
        state.desktop_shortcut_created = true;
        let _ = state.save();
    }

    Ok(())
}

#[cfg(test)]
#[path = "setup_hints_tests.rs"]
mod setup_hints_tests;
