#[test]
fn test_alignment_command_persists_and_applies_immediately() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.set_centered(false);
        app.input = "/alignment centered".to_string();

        app.submit_input();

        let cfg = crate::config::Config::load();
        assert!(cfg.display.centered);
        assert!(app.centered_mode());
        assert_eq!(app.status_notice(), Some("Layout: Centered".to_string()));

        let last = app.display_messages().last().expect("missing response");
        assert_eq!(last.role, "system");
        assert!(last.content.contains("Saved default alignment: centered"));
    });
}

#[test]
fn test_alignment_status_shows_current_and_saved_defaults() {
    with_temp_jcode_home(|| {
        crate::config::Config::set_display_centered(false).expect("save alignment default");

        let mut app = create_test_app();
        app.set_centered(true);
        app.input = "/alignment".to_string();

        app.submit_input();

        let last = app.display_messages().last().expect("missing response");
        assert_eq!(last.role, "system");
        assert!(last.content.contains("Alignment is currently centered."));
        assert!(last.content.contains("Saved default: left-aligned."));
        assert!(last.content.contains("/alignment centered"));
        assert!(
            last.content
                .contains(&jcode_tui_core::keybind::alt_chord("C"))
        );
    });
}

#[test]
fn test_alignment_invalid_usage_shows_error() {
    let mut app = create_test_app();
    app.input = "/alignment diagonal".to_string();

    app.submit_input();

    let last = app.display_messages().last().expect("missing response");
    assert_eq!(last.role, "error");
    assert!(last.content.contains("Usage: /alignment"));
}

#[test]
fn test_compact_notifications_command_persists_and_applies_immediately() {
    with_temp_jcode_home(|| {
        crate::config::Config::set_compact_notifications(false).expect("save default");

        let mut app = create_test_app();
        app.input = "/compact-notifications on".to_string();

        app.submit_input();

        let cfg = crate::config::Config::load();
        assert!(cfg.display.compact_notifications);
        assert_eq!(
            app.status_notice(),
            Some("Compact notifications: on".to_string())
        );

        let last = app.display_messages().last().expect("missing response");
        assert_eq!(last.role, "system");
        assert!(last.content.contains("Saved compact notifications: on"));
    });
}

#[test]
fn test_compact_notifications_status_reports_current_value() {
    with_temp_jcode_home(|| {
        crate::config::Config::set_compact_notifications(true).expect("save default");

        let mut app = create_test_app();
        app.input = "/compact-notifications".to_string();

        app.submit_input();

        let last = app.display_messages().last().expect("missing response");
        assert_eq!(last.role, "system");
        assert!(
            last.content
                .contains("Compact notifications are currently on.")
        );
    });
}

#[test]
fn test_compact_notifications_invalid_usage_shows_error() {
    let mut app = create_test_app();
    app.input = "/compact-notifications maybe".to_string();

    app.submit_input();

    let last = app.display_messages().last().expect("missing response");
    assert_eq!(last.role, "error");
    assert!(last.content.contains("Usage: /compact-notifications"));
}

#[test]
fn test_mask_email_censors_local_part() {
    assert_eq!(mask_email("jeremyh1@uw.edu"), "j***1@uw.edu");
}

#[test]
fn test_usage_report_shows_no_connected_providers_when_results_empty() {
    let mut app = create_test_app();
    app.handle_usage_report(Vec::new());

    let msg = app.display_messages().last().expect("missing usage card");
    assert_eq!(msg.role, "usage");
    assert!(msg.content.contains("No connected providers"));
    assert!(msg.content.contains("/login claude"));
    assert!(msg.content.contains("/login openai"));
}

#[test]
fn test_usage_command_requests_usage_report_with_inline_view() {
    let mut app = create_test_app();

    assert!(super::commands::handle_usage_command(&mut app, "/usage"));

    assert!(app.inline_interactive_state.is_none());
    assert!(app.usage_overlay.is_none());
    assert!(app.inline_view_state.is_none());
    assert_eq!(
        app.display_messages().last().map(|m| m.role.as_str()),
        Some("usage")
    );
    assert!(app.usage_report_refreshing);
}

#[test]
fn test_usage_submit_input_requests_usage_report_with_inline_view() {
    let mut app = create_test_app();
    app.input = "/usage".to_string();

    app.submit_input();

    assert!(app.inline_interactive_state.is_none());
    assert!(app.usage_overlay.is_none());
    assert!(app.inline_view_state.is_none());
    assert_eq!(
        app.display_messages().last().map(|m| m.role.as_str()),
        Some("usage")
    );
    assert!(app.usage_report_refreshing);
}

#[test]
fn test_usage_typing_does_not_open_picker_preview() {
    let mut app = create_test_app();

    for c in "/usage".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .expect("type /usage");
    }

    assert!(app.inline_interactive_state.is_none());
    assert_eq!(app.input(), "/usage");
    assert!(!app.usage_report_refreshing);
}

#[test]
fn test_usage_enter_requests_report_with_inline_view() {
    let mut app = create_test_app();

    for c in "/usage".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .expect("type /usage");
    }

    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .expect("submit /usage");

    assert!(app.inline_interactive_state.is_none());
    assert!(app.usage_overlay.is_none());
    assert!(app.inline_view_state.is_none());
    assert_eq!(app.input(), "");
    assert_eq!(
        app.display_messages().last().map(|m| m.role.as_str()),
        Some("usage")
    );
    assert!(app.usage_report_refreshing);
}
