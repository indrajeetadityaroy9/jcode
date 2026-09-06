use super::*;

#[test]
fn paused_jcode_shell_command_keeps_failures_visible() {
    let command = paused_jcode_shell_command("/tmp/jcode");
    assert!(command.contains("Press Enter to close"));
    assert!(command.contains("Jcode exited with status"));
    assert!(command.contains("jcode executable not found"));
}

#[test]
fn macos_terminal_notice_only_fires_for_default_terminal_app() {
    let mut state = SetupHintsState::default();
    let hints = macos_terminal_notice(&mut state, MacTerminalKind::AppleTerminal)
        .expect("Terminal.app should produce a notice");

    assert_eq!(
        hints.status_notice.as_deref(),
        Some("Tip: Terminal.app renders jcode poorly. Try Ghostty, iTerm2, or Alacritty.")
    );
    let (title, message) = hints.display_message.expect("expected display message");
    assert_eq!(title, "Terminal");
    assert!(message.contains("Terminal.app renders jcode poorly"));
    assert!(message.contains("Ghostty"));
    // It is a plain notice, not an AI handoff prompt.
    assert!(hints.auto_send_message.is_none());
    // The nudge is marked handled so it only ever shows once.
    assert!(state.mac_ghostty_guided);
    assert!(state.mac_ghostty_dismissed);
}

#[test]
fn macos_terminal_notice_silent_for_modern_terminals() {
    for terminal in [
        MacTerminalKind::Ghostty,
        MacTerminalKind::Iterm2,
        MacTerminalKind::WezTerm,
        MacTerminalKind::Warp,
        MacTerminalKind::Alacritty,
        MacTerminalKind::Vscode,
        MacTerminalKind::Unknown,
    ] {
        let mut state = SetupHintsState::default();
        assert!(
            macos_terminal_notice(&mut state, terminal).is_none(),
            "{terminal:?} should not be nudged"
        );
        // Even when silent, the nudge is marked handled so we never re-check it.
        assert!(state.mac_ghostty_guided);
        assert!(state.mac_ghostty_dismissed);
    }
}

#[test]
fn nudge_budget_caps_at_max_and_persists() {
    let mut state = SetupHintsState::default();
    assert_eq!(state.terminal_nudge_count, 0);

    for shown in 1..=MAX_TERMINAL_NUDGES {
        assert!(
            state.nudge_budget_remaining(),
            "should still allow nudge before #{shown}"
        );
        state.terminal_nudge_count = shown;
    }

    // After MAX_TERMINAL_NUDGES, we stop asking even without an explicit dismiss.
    assert_eq!(state.terminal_nudge_count, MAX_TERMINAL_NUDGES);
    assert!(!state.nudge_budget_remaining());
}

#[test]
fn load_from_falls_back_to_bak_when_primary_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("setup_hints.json");
    let bak = dir.path().join("setup_hints.bak");

    std::fs::write(&bak, r#"{"launch_count":42}"#).unwrap();

    // Primary file missing: must recover launch_count from the .bak instead of
    // resetting to default (which would restart the hint cadence).
    let loaded = SetupHintsState::load_from(&path);
    assert_eq!(loaded.launch_count, 42);
}

#[test]
fn load_from_falls_back_to_bak_when_primary_corrupt_without_inline_recovery() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("setup_hints.json");
    let bak = dir.path().join("setup_hints.bak");

    std::fs::write(&path, b"{not json").unwrap();
    std::fs::write(&bak, r#"{"launch_count":7}"#).unwrap();

    let loaded = SetupHintsState::load_from(&path);
    assert_eq!(loaded.launch_count, 7);
}

#[test]
fn load_from_defaults_when_both_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("setup_hints.json");
    let loaded = SetupHintsState::load_from(&path);
    assert_eq!(loaded.launch_count, 0);
}

#[test]
fn conflict_hint_decision_warns_only_when_conflicts_change() {
    // No conflicts ever: empty == empty => stay silent.
    assert_eq!(
        conflict_hint_decision("", ""),
        ConflictHintDecision::Unchanged
    );

    // New conflicts where there were none: warn.
    assert_eq!(
        conflict_hint_decision("keybindings.model_switch_next|ctrl+tab|ctrl+tab", ""),
        ConflictHintDecision::Warn
    );

    // Same conflicts as last time: stay silent.
    let sig = "keybindings.model_switch_next|ctrl+tab|ctrl+tab";
    assert_eq!(
        conflict_hint_decision(sig, sig),
        ConflictHintDecision::Unchanged
    );

    // Conflicts resolved since last time (had some, now none): update silently.
    assert_eq!(
        conflict_hint_decision("", sig),
        ConflictHintDecision::ResolvedSilently
    );

    // Conflict set changed (different conflicts): warn again.
    assert_eq!(
        conflict_hint_decision("keybindings.scroll_up|ctrl+k|ctrl+k", sig),
        ConflictHintDecision::Warn
    );
}

#[test]
fn keymap_conflict_hint_full_path_debounces_and_persists_signature() {
    use crate::keymap::source::{DiscoveredBinding, KeySource};
    use crate::keymap::{KeyChord, KeymapSnapshot};
    use jcode_config_types::KeybindingsConfig;

    fn snapshot(bindings: Vec<DiscoveredBinding>) -> KeymapSnapshot {
        KeymapSnapshot {
            version: 1,
            captured_at: "0".to_string(),
            os: "macos".to_string(),
            terminal: "Ghostty".to_string(),
            terminal_version: "1.3.1".to_string(),
            bindings,
        }
    }
    fn term(keys: &str, action: &str) -> DiscoveredBinding {
        DiscoveredBinding {
            chord: KeyChord::parse(keys).unwrap(),
            source: KeySource::Terminal,
            action: action.to_string(),
            raw: format!("{keys}={action}"),
            tool: String::new(),
        }
    }

    let cfg = KeybindingsConfig::default();
    let mut state = SetupHintsState::default();

    // 1) First time with a real conflict: warn + state changes.
    let conflicting = snapshot(vec![term("ctrl+tab", "next_tab")]);
    let (hint, changed) = keymap_conflict_hint_for(&cfg, &conflicting, &mut state);
    assert!(hint.is_some(), "should warn on first conflict");
    assert!(changed, "state signature should be recorded");
    let (title, body) = hint.unwrap().display_message.unwrap();
    assert_eq!(title, "Keybindings");
    assert!(body.contains("keybindings.model_switch_next"));
    assert!(!state.keymap_conflict_signature.is_empty());

    // 2) Same conflict again: debounced, no state change.
    let (hint2, changed2) = keymap_conflict_hint_for(&cfg, &conflicting, &mut state);
    assert!(hint2.is_none(), "same conflict set must not re-warn");
    assert!(!changed2, "no state change when nothing changed");

    // 3) Conflict resolved (clean snapshot): silent, but signature cleared.
    let clean = snapshot(vec![term("cmd+t", "new_tab")]);
    let (hint3, changed3) = keymap_conflict_hint_for(&cfg, &clean, &mut state);
    assert!(hint3.is_none(), "resolved conflicts show nothing");
    assert!(changed3, "signature should be cleared");
    assert!(state.keymap_conflict_signature.is_empty());
}

#[test]
fn glyph_safe_notice_shows_once_then_debounces() {
    let mut state = SetupHintsState::default();

    // First launch in a fragile terminal: disclose the tradeoff and persist.
    let (hint, changed) = glyph_safe_notice_for(true, &mut state);
    assert!(
        hint.is_some(),
        "should disclose glyph-safe mode on first launch"
    );
    assert!(changed, "state should be marked shown");
    assert!(state.glyph_safe_notice_shown);
    let (title, body) = hint.unwrap().display_message.unwrap();
    assert_eq!(title, "Display");
    assert!(body.contains("quantizes colors"));
    assert!(body.contains("JCODE_GLYPH_SAFE_MODE=off"));

    // Subsequent launches: debounced, no repeat.
    let (hint2, changed2) = glyph_safe_notice_for(true, &mut state);
    assert!(hint2.is_none(), "must not re-disclose on later launches");
    assert!(!changed2);
}

#[test]
fn glyph_safe_notice_silent_on_robust_terminals() {
    let mut state = SetupHintsState::default();
    let (hint, changed) = glyph_safe_notice_for(false, &mut state);
    assert!(
        hint.is_none(),
        "no disclosure when glyph-safe mode is inactive"
    );
    assert!(!changed);
    assert!(!state.glyph_safe_notice_shown);
}
