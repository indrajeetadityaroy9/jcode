#[test]
fn test_agents_review_picker_saves_config_override() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        configure_test_remote_models(&mut app);
        app.open_agent_model_picker(crate::tui::AgentModelTarget::Review);

        let selected = app
            .inline_interactive_state
            .as_ref()
            .and_then(|picker| {
                picker.filtered.iter().position(|&idx| {
                    matches!(
                        picker.entries[idx].action,
                        crate::tui::PickerAction::AgentModelChoice {
                            target: crate::tui::AgentModelTarget::Review,
                            clear_override: false,
                        }
                    )
                })
            })
            .expect("review picker should include at least one model option");
        app.inline_interactive_state.as_mut().unwrap().selected = selected;
        let selected_model_idx = app.inline_interactive_state.as_ref().unwrap().filtered[selected];
        app.inline_interactive_state.as_mut().unwrap().entries[selected_model_idx].options[0]
            .available = true;

        let expected = {
            let picker = app.inline_interactive_state.as_ref().unwrap();
            let entry = &picker.entries[picker.filtered[selected]];
            let base = if entry.effort.is_some() {
                entry
                    .name
                    .rsplit_once(" (")
                    .map(|(base, _)| base.to_string())
                    .unwrap_or_else(|| entry.name.clone())
            } else {
                entry.name.clone()
            };
            let route = &entry.options[entry.selected_option];
            if route.api_method == "openai-oauth" {
                format!("openai-oauth:{}", base)
            } else if route.api_method == "openai-api" {
                format!("openai-api:{}", base)
            } else if route.api_method == "claude-oauth" {
                format!("claude-oauth:{}", base)
            } else if route.api_method == "claude-api" && route.provider == "Anthropic" {
                format!("claude-api:{}", base)
            } else {
                base
            }
        };

        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .expect("save agent model override");

        let cfg = crate::config::Config::load();
        assert_eq!(cfg.autoreview.model.as_deref(), Some(expected.as_str()));
        assert!(app.inline_interactive_state.is_none());
    });
}

#[test]
fn test_model_command_suggestions_include_matching_models() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    let suggestions = app.get_suggestions_for("/model g52c");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/model gpt-5.2-codex")
    );
}

#[test]
fn test_model_command_trailing_space_shows_model_suggestions() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    let suggestions = app.get_suggestions_for("/model ");
    assert!(
        suggestions
            .iter()
            .any(|(cmd, _)| cmd == "/model gpt-5.3-codex")
    );
}

#[test]
fn test_login_command_suggestions_follow_provider_catalog() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/login ");

    for provider in crate::provider_catalog::tui_login_providers() {
        assert!(
            suggestions
                .iter()
                .any(|(cmd, detail)| cmd == &format!("/login {}", provider.id)
                    && detail == &provider.menu_detail),
            "missing /login suggestion for provider {}",
            provider.id
        );
    }
}

#[test]
fn test_model_autocomplete_completes_unique_match() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);
    app.input = "/model g52c".to_string();
    app.cursor_pos = app.input.len();

    assert!(app.autocomplete());
    assert_eq!(app.input(), "/model gpt-5.2-codex");
}

#[test]
fn test_model_picker_preview_stays_open_and_updates_filter() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    for c in "/model g52c".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker preview should be open");
    assert!(picker.preview);
    assert_eq!(picker.filter, "g52c");
    assert!(
        picker
            .filtered
            .iter()
            .any(|&i| picker.entries[i].name.starts_with("gpt-5.2-codex ("))
    );
    assert_eq!(app.input(), "/model g52c");
}

#[test]
fn test_model_picker_preview_enter_selects_model() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    for c in "/model g52c".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    // Enter from preview mode selects the model and closes the picker
    assert!(app.inline_interactive_state.is_none());
    assert!(app.input().is_empty());
    assert_eq!(app.cursor_pos(), 0);
}
