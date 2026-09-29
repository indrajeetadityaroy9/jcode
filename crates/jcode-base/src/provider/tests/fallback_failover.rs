#[test]
fn test_fallback_sequence_includes_all_providers() {
    assert_eq!(
        MultiProvider::fallback_sequence(ActiveProvider::Claude),
        vec![
            ActiveProvider::Claude,
            ActiveProvider::OpenAI,
            ActiveProvider::Gemini,
        ]
    );
    assert_eq!(
        MultiProvider::fallback_sequence(ActiveProvider::OpenAI),
        vec![
            ActiveProvider::OpenAI,
            ActiveProvider::Claude,
            ActiveProvider::Gemini,
        ]
    );
    assert_eq!(
        MultiProvider::fallback_sequence(ActiveProvider::Gemini),
        vec![
            ActiveProvider::Gemini,
            ActiveProvider::Claude,
            ActiveProvider::OpenAI,
            ActiveProvider::Antigravity,
        ]
    );
}

#[test]
fn test_parse_provider_hint_supports_known_values() {
    assert_eq!(
        MultiProvider::parse_provider_hint("claude"),
        Some(ActiveProvider::Claude)
    );
    assert_eq!(
        MultiProvider::parse_provider_hint("Anthropic"),
        Some(ActiveProvider::Claude)
    );
    assert_eq!(
        MultiProvider::parse_provider_hint("openai"),
        Some(ActiveProvider::OpenAI)
    );
    assert_eq!(
        MultiProvider::parse_provider_hint("gemini"),
        Some(ActiveProvider::Gemini)
    );
}

#[test]
fn test_active_provider_env_only_seeds_sessions_when_explicitly_selected() {
    with_clean_provider_test_env(|| {
        crate::env::set_var("JCODE_ACTIVE_PROVIDER", "openai");
        assert_eq!(MultiProvider::initial_provider_from_env(), None);

        crate::provider::activation::select_initial_runtime_provider_key("openai");
        assert_eq!(
            MultiProvider::initial_provider_from_env(),
            Some(ActiveProvider::OpenAI)
        );

        crate::provider::activation::clear_initial_runtime_provider();
        assert_eq!(MultiProvider::initial_provider_from_env(), None);
    });
}

#[test]
fn test_initial_provider_allows_cross_provider_switch_and_reports_target_credentials() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _enter = runtime.enter();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(None),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenAI),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: Some(ActiveProvider::OpenAI),
            routes_memo: std::sync::Mutex::new(None),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        let err = provider
            .set_model("claude:claude-sonnet-4-6")
            .expect_err("the target provider should report its missing credentials");
        assert!(
            err.to_string().contains("Claude credentials not available"),
            "expected target-provider credential error, got: {}",
            err
        );
    });
}

#[test]
fn test_auto_default_prefers_claude_over_openai_when_both_available() {
    let active = MultiProvider::auto_default_provider(ProviderAvailability {
        openai: true,
        claude: true,
        antigravity: false,
        gemini: false,
    });
    assert_eq!(active, ActiveProvider::Claude);
}

#[test]
fn test_should_failover_on_403_forbidden() {
    let err = anyhow::anyhow!(
        "Provider token exchange failed (HTTP 403 Forbidden): not accessible by integration"
    );
    assert!(MultiProvider::classify_failover_error(&err).should_failover());
}

#[test]
fn test_should_failover_on_token_exchange_failed() {
    let msg = r#"Provider token exchange failed (HTTP 403 Forbidden): {"error_details":{"title":"Contact Support"}}"#;
    let err = anyhow::anyhow!("{}", msg);
    assert!(MultiProvider::classify_failover_error(&err).should_failover());
}

#[test]
fn test_should_failover_on_access_denied() {
    let err = anyhow::anyhow!("Access denied: account suspended");
    assert!(MultiProvider::classify_failover_error(&err).should_failover());
}

#[test]
fn test_should_failover_when_status_code_starts_message() {
    let err = anyhow::anyhow!("401 unauthorized");
    assert!(MultiProvider::classify_failover_error(&err).should_failover());
    assert_eq!(
        MultiProvider::classify_failover_error(&err),
        FailoverDecision::RetryAndMarkUnavailable
    );
}

#[test]
fn test_should_not_failover_on_non_independent_status_digits() {
    let err = anyhow::anyhow!("backend returned code 14290");
    assert!(!MultiProvider::classify_failover_error(&err).should_failover());
}

#[test]
fn test_context_limit_error_fails_over_without_marking_provider_unavailable() {
    let err = anyhow::anyhow!("Context length exceeded maximum context window");
    assert!(MultiProvider::classify_failover_error(&err).should_failover());
    assert_eq!(
        MultiProvider::classify_failover_error(&err),
        FailoverDecision::RetryNextProvider
    );
}

#[test]
fn test_should_not_failover_on_generic_error() {
    let err = anyhow::anyhow!("Connection timed out");
    assert!(!MultiProvider::classify_failover_error(&err).should_failover());
}

#[test]
fn test_no_provider_error_mentions_tokens_and_details() {
    let provider = MultiProvider {
        claude: RwLock::new(None),
        anthropic: RwLock::new(None),
        openai: RwLock::new(None),
        antigravity: RwLock::new(None),
        gemini: RwLock::new(None),
        active: RwLock::new(ActiveProvider::OpenAI),
        use_claude_cli: false,
        startup_notices: RwLock::new(Vec::new()),
        initial_provider: None,
        routes_memo: std::sync::Mutex::new(None),
        post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let err = provider.no_provider_available_error(&[
        "OpenAI: rate limited".to_string(),
        "Gemini: not configured".to_string(),
    ]);
    let text = err.to_string();
    assert!(text.contains("No tokens/providers left"));
    assert!(text.contains("OpenAI: rate limited"));
    assert!(text.contains("Gemini: not configured"));
}

