#[test]
fn test_provider_for_model_claude() {
    assert_eq!(provider_for_model("claude-opus-4-6"), Some("claude"));
    assert_eq!(provider_for_model("claude-opus-4-6[1m]"), Some("claude"));
    assert_eq!(provider_for_model("claude-sonnet-4-6"), Some("claude"));
}

#[test]
fn test_provider_for_model_openai() {
    assert_eq!(provider_for_model("gpt-5.2-codex"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.5"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.4"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.4[1m]"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.4-pro"), Some("openai"));
}

#[test]
fn test_provider_for_model_gemini() {
    assert_eq!(provider_for_model("gemini-2.5-pro"), Some("gemini"));
    assert_eq!(provider_for_model("gemini-2.5-flash"), Some("gemini"));
    assert_eq!(provider_for_model("gemini-3-pro-preview"), Some("gemini"));
}

#[test]
fn test_available_models_display_uses_route_models() {
    // Hermetic env: this reads the process-global model catalog, so without a
    // clean scope it observes whatever catalog a sibling test installed and
    // fails depending on test ordering.
    with_clean_provider_test_env(|| {
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

        let models = provider.available_models_display();
        assert!(
            models
                .iter()
                .any(|model| known_openai_model_ids().contains(model)),
            "route-backed display models should include OpenAI picker rows: {:?}",
            models
        );
        assert!(
            models
                .iter()
                .any(|model| known_anthropic_model_ids().contains(model)),
            "route-backed display models should include Anthropic picker rows: {:?}",
            models
        );
    });
}

#[test]
fn test_session_route_restore_request_matrix_preserves_runtime_identity() {
    let cases = [
        (
            "claude-sonnet-4-6",
            Some("claude"),
            Some("claude-oauth"),
            "claude-oauth:claude-sonnet-4-6",
        ),
        (
            "claude-sonnet-4-6",
            Some("claude"),
            Some("anthropic-api-key"),
            "claude-api:claude-sonnet-4-6",
        ),
        (
            "gpt-5.4",
            Some("openai"),
            Some("openai-oauth"),
            "openai-oauth:gpt-5.4",
        ),
        (
            "gpt-5.4",
            Some("openai"),
            Some("openai-api-key"),
            "openai-api:gpt-5.4",
        ),
        (
            "default",
            Some("antigravity"),
            Some("antigravity-https"),
            "antigravity:default",
        ),
    ];

    for (model, provider_key, api_method, expected) in cases {
        assert_eq!(
            MultiProvider::model_switch_request_for_session_route(model, provider_key, api_method),
            expected,
            "restore request should preserve route identity for {provider_key:?}/{api_method:?}"
        );
    }
}

#[test]
fn test_openai_auth_mode_prefixed_model_switch_changes_credentials() {
    with_clean_provider_test_env(|| {
        let prev_runtime = std::env::var_os("JCODE_RUNTIME_PROVIDER");
        crate::env::remove_var("JCODE_RUNTIME_PROVIDER");
        crate::env::set_var("OPENAI_API_KEY", "sk-test-openai-api-key");
        crate::auth::codex::upsert_account_from_tokens(
            "openai-1",
            "oauth-access-token",
            "oauth-refresh-token",
            None,
            None,
        )
        .expect("save OAuth account");

        let openai = test_openai_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(Some(Arc::clone(&openai) as Arc<dyn Provider>)),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenAI),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();

        // Route pinning is MultiProvider's job; per-pin token resolution is
        // covered by jcode-provider-openai-runtime's tests.
        assert_eq!(
            openai.credential_mode(),
            jcode_provider_core::CredentialMode::Auto,
            "default OpenAI credentials stay on the OAuth-first Auto pin"
        );

        provider
            .set_model("openai-api:gpt-5.5")
            .expect("API-key route should select the OpenAI API credentials");
        assert_eq!(
            openai.credential_mode(),
            jcode_provider_core::CredentialMode::ApiKey
        );

        provider
            .set_model("openai-oauth:gpt-5.5")
            .expect("OAuth route should switch back to Codex OAuth credentials");
        assert_eq!(
            openai.credential_mode(),
            jcode_provider_core::CredentialMode::OAuth
        );

        if let Some(prev_runtime) = prev_runtime {
            crate::env::set_var("JCODE_RUNTIME_PROVIDER", prev_runtime);
        } else {
            crate::env::remove_var("JCODE_RUNTIME_PROVIDER");
        }
    });
}

#[test]
fn test_initial_openai_provider_can_switch_to_anthropic_auth_routes() {
    with_clean_provider_test_env(|| {
        crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
        crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
            label: "claude-1".to_string(),
            access: "oauth-access-token".to_string(),
            refresh: "oauth-refresh-token".to_string(),
            expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
            email: None,
            subscription_type: Some("max".to_string()),
            scopes: vec!["user:inference".to_string()],
        })
        .expect("save Claude OAuth account");

        let anthropic = test_anthropic_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(Some(Arc::clone(&anthropic) as Arc<dyn Provider>)),
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
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();

        // Route pinning is MultiProvider's job; the concrete token resolution
        // for each pin is covered by jcode-provider-anthropic-runtime's
        // credential-mode tests.
        assert_eq!(
            anthropic.credential_mode(),
            jcode_provider_core::CredentialMode::Auto,
            "default (Auto) leaves the credential pin unset"
        );

        provider
            .set_model("claude-oauth:claude-opus-4-6")
            .expect("OAuth route should select Claude OAuth credentials");
        assert_eq!(
            anthropic.credential_mode(),
            jcode_provider_core::CredentialMode::OAuth
        );

        provider
            .set_model("claude-api:claude-opus-4-6")
            .expect("API route should select Anthropic API-key credentials");
        assert_eq!(
            anthropic.credential_mode(),
            jcode_provider_core::CredentialMode::ApiKey
        );
    });
}

#[test]
fn test_config_default_provider_anthropic_api_pins_api_credential() {
    use jcode_provider_core::{Provider, ResolvedCredential};
    // A config `default_provider = "anthropic-api"` is a routing decision that
    // also pins the OAuth-vs-API credential. Applying the default at startup
    // must leave the provider on the API-key route so the header auth tag and
    // model picker report "API Key", not the Auto/OAuth fallback.
    for (default_provider, expected, expect_oauth) in [
        ("anthropic-api", ResolvedCredential::ApiKey, false),
        ("claude-api", ResolvedCredential::ApiKey, false),
        ("claude", ResolvedCredential::Oauth, true),
        ("anthropic", ResolvedCredential::Oauth, true),
    ] {
        with_clean_provider_test_env(|| {
            crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
            crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
                label: "claude-1".to_string(),
                access: "oauth-access-token".to_string(),
                refresh: "oauth-refresh-token".to_string(),
                expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
                email: None,
                subscription_type: Some("max".to_string()),
                scopes: vec!["user:inference".to_string()],
            })
            .expect("save Claude OAuth account");

            let anthropic = test_anthropic_runtime();
            let provider = MultiProvider {
                claude: RwLock::new(None),
                anthropic: RwLock::new(Some(Arc::clone(&anthropic) as Arc<dyn Provider>)),
                openai: RwLock::new(None),
                antigravity: RwLock::new(None),
                gemini: RwLock::new(None),
                active: RwLock::new(ActiveProvider::Claude),
                use_claude_cli: false,
                startup_notices: RwLock::new(Vec::new()),
                initial_provider: None,
                routes_memo: std::sync::Mutex::new(None),
                post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };
            let rt = enter_test_runtime();
            let _runtime_guard = rt.enter();

            provider
                .set_config_default_model("claude-opus-4-6", Some(default_provider))
                .unwrap_or_else(|e| {
                    panic!("default_provider '{default_provider}' should apply: {e}")
                });

            assert_eq!(
                provider.active_provider(),
                ActiveProvider::Claude,
                "default_provider '{default_provider}' routes to Claude",
            );
            assert_eq!(
                provider.active_explicit_credential(),
                (!expect_oauth).then_some(ResolvedCredential::ApiKey),
                "default_provider '{default_provider}' explicit-pin visibility",
            );
            assert_eq!(
                anthropic.credential_mode(),
                if expect_oauth {
                    // "claude"/"anthropic" leave Auto (OAuth-first) rather than
                    // pinning OAuth explicitly.
                    jcode_provider_core::CredentialMode::Auto
                } else {
                    jcode_provider_core::CredentialMode::ApiKey
                },
                "default_provider '{default_provider}' should resolve {expected:?}",
            );
        });
    }
}

#[test]
fn test_config_default_model_with_credential_prefix_applies_model_and_pin() {
    use jcode_provider_core::{Provider, ResolvedCredential};
    // The model picker saves default_model as a full spec like
    // `claude-api:claude-opus-4-6`. Startup must parse the prefix (routing +
    // credential pin) instead of handing the raw spec to the Anthropic
    // provider, which would reject it and silently keep the fallback default.
    for (spec, expect_oauth) in [
        ("claude-api:claude-opus-4-6", false),
        ("claude-oauth:claude-opus-4-6", true),
        ("claude:claude-opus-4-6", true),
    ] {
        with_clean_provider_test_env(|| {
            crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
            crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
                label: "claude-1".to_string(),
                access: "oauth-access-token".to_string(),
                refresh: "oauth-refresh-token".to_string(),
                expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
                email: None,
                subscription_type: Some("max".to_string()),
                scopes: vec!["user:inference".to_string()],
            })
            .expect("save Claude OAuth account");

            let anthropic = test_anthropic_runtime();
            let provider = MultiProvider {
                claude: RwLock::new(None),
                anthropic: RwLock::new(Some(Arc::clone(&anthropic) as Arc<dyn Provider>)),
                openai: RwLock::new(None),
                antigravity: RwLock::new(None),
                gemini: RwLock::new(None),
                active: RwLock::new(ActiveProvider::Claude),
                use_claude_cli: false,
                startup_notices: RwLock::new(Vec::new()),
                initial_provider: None,
                routes_memo: std::sync::Mutex::new(None),
                post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };
            let rt = enter_test_runtime();
            let _runtime_guard = rt.enter();

            provider
                .set_config_default_model(spec, Some("anthropic-api"))
                .unwrap_or_else(|e| panic!("default_model '{spec}' should apply: {e}"));

            assert_eq!(
                provider.active_provider(),
                ActiveProvider::Claude,
                "default_model '{spec}' routes to Claude",
            );
            assert_eq!(
                provider.model(),
                "claude-opus-4-6",
                "default_model '{spec}' should set the bare model id",
            );
            // `claude-api:` must pin the API key; `claude:`/`claude-oauth:`
            // resolve OAuth-first (Auto or explicit OAuth respectively), so the
            // pin must not be ApiKey. Concrete token resolution per pin is
            // covered by jcode-provider-anthropic-runtime's tests.
            if expect_oauth {
                assert_ne!(
                    anthropic.credential_mode(),
                    jcode_provider_core::CredentialMode::ApiKey,
                    "default_model '{spec}' must not pin the API key (expected {:?})",
                    ResolvedCredential::Oauth,
                );
            } else {
                assert_eq!(
                    anthropic.credential_mode(),
                    jcode_provider_core::CredentialMode::ApiKey,
                    "default_model '{spec}' should resolve {:?}",
                    ResolvedCredential::ApiKey,
                );
            }
        });
    }
}

#[test]
fn test_multi_provider_fork_switch_request_preserves_route_identity_state_space() {
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        crate::env::set_var("OPENAI_API_KEY", "sk-test-openai-api-key");
        crate::auth::codex::upsert_account_from_tokens(
            "openai-1",
            "oauth-access-token",
            "oauth-refresh-token",
            None,
            None,
        )
        .expect("save OpenAI OAuth account");
        let openai = test_openai_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(Some(openai)),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenAI),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        let generation_before = crate::provider::pricing::auth_pricing_generation();
        provider
            .set_model("openai-api:gpt-5.5")
            .expect("API-key route should be selectable");
        let api_route_memo_key = provider.routes_memo_key();
        assert_eq!(
            crate::provider::pricing::auth_pricing_generation(),
            generation_before,
            "restoring an in-memory credential route must not invalidate global catalogs"
        );
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "openai-api:gpt-5.5"
        );
        let _fork = provider.fork();
        assert_eq!(
            crate::provider::pricing::auth_pricing_generation(),
            generation_before,
            "forking a provider must not invalidate global catalogs"
        );
        provider
            .set_model("openai-oauth:gpt-5.5")
            .expect("OAuth route should be selectable");
        assert_ne!(
            provider.routes_memo_key(),
            api_route_memo_key,
            "OAuth and API-key catalogs need distinct shared memo keys"
        );
        assert_eq!(
            crate::provider::pricing::auth_pricing_generation(),
            generation_before
        );
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "openai-oauth:gpt-5.5"
        );
    });

    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
        crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
            label: "claude-1".to_string(),
            access: "oauth-access-token".to_string(),
            refresh: "oauth-refresh-token".to_string(),
            expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
            email: None,
            subscription_type: Some("max".to_string()),
            scopes: vec!["user:inference".to_string()],
        })
        .expect("save Claude OAuth account");
        let anthropic = test_anthropic_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(Some(anthropic)),
            openai: RwLock::new(None),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            active: RwLock::new(ActiveProvider::Claude),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        provider
            .set_model("claude-oauth:claude-opus-4-6")
            .expect("OAuth route should be selectable");
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "claude-oauth:claude-opus-4-6"
        );
        provider
            .set_model("claude-api:claude-opus-4-6")
            .expect("API-key route should be selectable");
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "claude-api:claude-opus-4-6"
        );
    });
}

#[test]
fn test_provider_for_model_unknown() {
    assert_eq!(provider_for_model("unknown-model"), None);
}

#[test]
fn test_context_limit_spark_vs_codex() {
    // Static tables only - see `test_resolve_model_capabilities_uses_provider_hint`.
    crate::provider::clear_context_limit_cache();
    assert_eq!(
        context_limit_for_model("gpt-5.3-codex-spark"),
        Some(128_000)
    );
    assert_eq!(context_limit_for_model("gpt-5.5"), Some(272_000));
    assert_eq!(context_limit_for_model("gpt-5.3-codex"), Some(272_000));
    assert_eq!(context_limit_for_model("gpt-5.2-codex"), Some(272_000));
    assert_eq!(context_limit_for_model("gpt-5-codex"), Some(272_000));
}

#[test]
fn test_context_limit_gpt_5_4() {
    crate::provider::clear_context_limit_cache();
    assert_eq!(context_limit_for_model("gpt-5.4"), Some(1_000_000));
    assert_eq!(context_limit_for_model("gpt-5.4-pro"), Some(1_000_000));
    assert_eq!(context_limit_for_model("gpt-5.4[1m]"), Some(1_000_000));
}

#[test]
fn test_context_limit_respects_provider_hint() {
    crate::provider::clear_context_limit_cache();
    assert_eq!(
        context_limit_for_model_with_provider("gpt-5.4", Some("openai")),
        Some(1_000_000)
    );
    assert_eq!(
        context_limit_for_model_with_provider("claude-sonnet-4-6[1m]", Some("claude")),
        Some(1_048_576)
    );
}

#[test]
fn test_resolve_model_capabilities_uses_provider_hint() {
    // These assertions describe the *static* classification tables. The dynamic
    // context-limit cache outranks them and is process-global, seeded as a side
    // effect of any test that builds provider routes (the Antigravity catalog
    // publishes gpt-5.x at 272k), so state the precondition rather than
    // depending on test order.
    crate::provider::clear_context_limit_cache();

    let openai = resolve_model_capabilities("gpt-5.4", Some("openai"));
    assert_eq!(openai.provider.as_deref(), Some("openai"));
    assert_eq!(openai.context_window, Some(1_000_000));

    let gemini = resolve_model_capabilities("gemini-2.5-pro", Some("gemini"));
    assert_eq!(gemini.provider.as_deref(), Some("gemini"));
    assert_eq!(gemini.context_window, Some(1_000_000));
}

#[test]
fn test_normalize_model_id_strips_1m_suffix() {
    assert_eq!(models::normalize_model_id("gpt-5.4[1m]"), "gpt-5.4");
    assert_eq!(models::normalize_model_id(" GPT-5.4[1M] "), "gpt-5.4");
}

#[test]
fn test_merge_openai_model_ids_appends_dynamic_oauth_models() {
    let models = models::merge_openai_model_ids(vec![
        "gpt-5.4".to_string(),
        "gpt-5.4-fast-preview".to_string(),
        "gpt-5.4-fast-preview".to_string(),
        " gpt-5.5-experimental ".to_string(),
    ]);

    assert!(models.iter().any(|model| model == "gpt-5.4"));
    assert!(models.iter().any(|model| model == "gpt-5.4-fast-preview"));
    assert!(models.iter().any(|model| model == "gpt-5.5-experimental"));
    assert_eq!(
        models
            .iter()
            .filter(|model| model.as_str() == "gpt-5.4-fast-preview")
            .count(),
        1
    );
}

#[test]
fn test_merge_anthropic_model_ids_appends_dynamic_models() {
    let models = models::merge_anthropic_model_ids(vec![
        "claude-opus-4-6".to_string(),
        "claude-sonnet-5-preview".to_string(),
        "claude-sonnet-5-preview".to_string(),
        " claude-haiku-5-beta ".to_string(),
    ]);

    assert!(models.iter().any(|model| model == "claude-opus-4-6"));
    assert!(models.iter().any(|model| model == "claude-opus-4-6[1m]"));
    assert!(
        models
            .iter()
            .any(|model| model == "claude-sonnet-5-preview")
    );
    assert!(models.iter().any(|model| model == "claude-haiku-5-beta"));
    assert_eq!(
        models
            .iter()
            .filter(|model| model.as_str() == "claude-sonnet-5-preview")
            .count(),
        1
    );
}

#[test]
fn test_parse_anthropic_model_catalog_reads_context_limits() {
    let data = serde_json::json!({
        "data": [
            {
                "id": "claude-opus-4-6",
                "max_input_tokens": 1_048_576
            },
            {
                "id": "claude-sonnet-5-preview",
                "max_input_tokens": 333_000
            }
        ]
    });

    let catalog = models::parse_anthropic_model_catalog(&data);
    assert!(
        catalog
            .available_models
            .contains(&"claude-opus-4-6".to_string())
    );
    assert!(
        catalog
            .available_models
            .contains(&"claude-sonnet-5-preview".to_string())
    );
    assert_eq!(
        catalog.context_limits.get("claude-opus-4-6"),
        Some(&1_048_576)
    );
    assert_eq!(
        catalog.context_limits.get("claude-sonnet-5-preview"),
        Some(&333_000)
    );
}

#[test]
fn test_context_limit_claude() {
    with_clean_provider_test_env(|| {
        assert_eq!(context_limit_for_model("claude-opus-4-6"), Some(200_000));
        assert_eq!(context_limit_for_model("claude-sonnet-4-6"), Some(200_000));
        assert_eq!(
            context_limit_for_model("claude-opus-4-6[1m]"),
            Some(1_048_576)
        );
        assert_eq!(
            context_limit_for_model("claude-sonnet-4-6[1m]"),
            Some(1_048_576)
        );
        // Opus 4.8 / 4.7 expose a 1M window natively (no `[1m]` opt-in needed),
        // matching the live Anthropic catalog's `max_input_tokens: 1000000`.
        assert_eq!(context_limit_for_model("claude-opus-4-8"), Some(1_000_000));
        assert_eq!(
            context_limit_for_model("claude-opus-4-8[1m]"),
            Some(1_000_000)
        );
        assert_eq!(context_limit_for_model("claude-opus-4-7"), Some(1_000_000));
    });
}

#[test]
fn test_context_limit_dynamic_cache() {
    populate_context_limits(
        [("test-model-xyz".to_string(), 64_000)]
            .into_iter()
            .collect(),
    );
    assert_eq!(context_limit_for_model("test-model-xyz"), Some(64_000));
}
