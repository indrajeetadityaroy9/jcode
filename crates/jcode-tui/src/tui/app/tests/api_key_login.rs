// API-key login through the real key path.
//
// Both tests here were rescued from the deleted first-run onboarding test
// module when that subsystem was removed (`docs/FORK_WORKFLOW.md` §1,
// *Onboarding* row). Neither asserts anything about onboarding: they cover the
// prompt an API-key provider shows and what typing a key into it actually
// does, which is live behavior on every launch.

/// The empty screen's only remaining suggestion is the login affordance, and it
/// appears exactly when no provider is authenticated.
///
/// Deleting onboarding collapsed `suggestion_prompts` down to this one pair;
/// the new-user cards it used to return were gated on a new-install
/// predicate that required a launch count of five or fewer.
/// Nothing else asserts the surviving arm, so a later cleanup could delete it
/// and leave an unauthenticated user staring at an empty screen with no hint
/// that `/login` exists — the compiler would not object.
#[test]
fn empty_state_offers_login_exactly_when_unauthenticated() {
    let app = create_test_app();
    let prompts = app.suggestion_prompts();

    if crate::auth::AuthStatus::check_fast().has_any_available() {
        assert!(
            prompts.is_empty(),
            "an authenticated session must not show suggestion cards, got {prompts:?}"
        );
    } else {
        assert_eq!(
            prompts,
            vec![("Log in to get started".to_string(), "/login".to_string())],
            "an unauthenticated session must offer exactly the login affordance"
        );
    }
}

/// The API-key prompt must state the endpoint it will talk to and must NOT
/// invent a "Suggested default model:" line. A static suggestion here goes
/// stale silently and sends users to a model their key cannot reach.
#[test]
fn direct_api_key_login_does_not_advertise_a_static_model_default() {
    let mut app = create_test_app();

    app.start_login_provider(
        crate::provider_catalog::resolve_login_provider("openai-api")
            .expect("OpenAI API provider descriptor"),
    );
    let openai_prompt = &app
        .display_messages()
        .last()
        .expect("missing OpenAI API key prompt")
        .content;
    assert!(openai_prompt.contains("Endpoint: https://api.openai.com/v1"));
    assert!(!openai_prompt.contains("Suggested default model:"));

    app.start_login_provider(
        crate::provider_catalog::resolve_login_provider("anthropic-api")
            .expect("Anthropic API provider descriptor"),
    );
    let anthropic_prompt = &app
        .display_messages()
        .last()
        .expect("missing Anthropic API key prompt")
        .content;
    assert!(anthropic_prompt.contains("Endpoint: https://api.anthropic.com"));
    assert!(!anthropic_prompt.contains("Suggested default model:"));
}

/// End-to-end: a typed API key reaches the input buffer, Enter submits it to
/// the pending-login handler without re-opening the provider picker, and the
/// key is actually persisted and exported.
///
/// Driven through the production key dispatch (`handle_key`), not by calling a
/// login helper directly, because the bug this guards was a *handler* that
/// intercepted characters before the input buffer ever saw them. The key
/// deliberately contains `k`, `n`, `l` and `y` — characters that a competing
/// key handler is most likely to claim as navigation.
#[test]
fn openrouter_key_typed_through_the_full_key_path_is_saved_and_exported() {
    use crossterm::event::{KeyCode, KeyModifiers};

    with_temp_jcode_home(|| {
        let mut app = create_test_app();

        // Simulate having chosen OpenRouter from the picker: the picker is
        // closed and a pending API-key login prompt is active.
        app.inline_interactive_state = None;
        app.start_login_provider(
            crate::provider_catalog::resolve_login_provider("openrouter").unwrap(),
        );
        assert!(app.pending_login.is_some());
        assert!(app.inline_interactive_state.is_none());

        let key = "sk-or-key-no-loop";
        for ch in key.chars() {
            app.handle_key(KeyCode::Char(ch), KeyModifiers::NONE)
                .unwrap();
            assert!(
                app.inline_interactive_state.is_none(),
                "picker re-opened while typing '{ch}'"
            );
        }
        assert_eq!(
            app.input, key,
            "every typed character must reach the input buffer"
        );

        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(
            app.pending_login.is_none(),
            "Enter must consume the pending login, not bounce back to the picker"
        );
        assert!(
            app.inline_interactive_state.is_none(),
            "Enter must not re-open the provider picker"
        );
        assert!(
            app.input.is_empty(),
            "input buffer should clear after submit"
        );

        // The key must actually be persisted, not merely accepted: it is
        // written to $JCODE_HOME/config/jcode/openrouter.env and exported to
        // OPENROUTER_API_KEY so the provider can authenticate immediately.
        let env_file = crate::storage::app_config_dir()
            .unwrap()
            .join("openrouter.env");
        let contents = std::fs::read_to_string(&env_file)
            .unwrap_or_else(|e| panic!("openrouter.env should exist at {env_file:?}: {e}"));
        assert!(
            contents.contains(&format!("OPENROUTER_API_KEY={key}")),
            "saved env file must contain the typed key, got:\n{contents}"
        );
        assert_eq!(
            std::env::var("OPENROUTER_API_KEY").ok().as_deref(),
            Some(key),
            "key must be exported to the process env for immediate use"
        );
    });
}
