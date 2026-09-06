#[test]
fn test_openai_provider_unavailability_is_scoped_per_account() {
    let _guard = crate::storage::lock_test_env();

    crate::auth::codex::set_active_account_override(Some("work".to_string()));
    clear_all_provider_unavailability_for_account();
    record_provider_unavailable_for_account("openai", "work rate limit");
    assert!(
        provider_unavailability_detail_for_account("openai")
            .unwrap_or_default()
            .contains("work rate limit")
    );

    crate::auth::codex::set_active_account_override(Some("personal".to_string()));
    clear_all_provider_unavailability_for_account();
    assert!(provider_unavailability_detail_for_account("openai").is_none());

    crate::auth::codex::set_active_account_override(Some("work".to_string()));
    assert!(
        provider_unavailability_detail_for_account("openai")
            .unwrap_or_default()
            .contains("work rate limit")
    );

    clear_all_provider_unavailability_for_account();
    crate::auth::codex::set_active_account_override(None);
}

#[test]
fn test_openai_model_catalog_is_scoped_per_account() {
    let _guard = crate::storage::lock_test_env();
    let work_model = "scoped-work-model-123";
    let personal_model = "scoped-personal-model-456";

    crate::auth::codex::set_active_account_override(Some("work".to_string()));
    populate_account_models(vec![work_model.to_string()]);
    assert!(known_openai_model_ids().contains(&work_model.to_string()));
    assert!(!known_openai_model_ids().contains(&personal_model.to_string()));

    crate::auth::codex::set_active_account_override(Some("personal".to_string()));
    assert!(!known_openai_model_ids().contains(&work_model.to_string()));
    populate_account_models(vec![personal_model.to_string()]);
    assert!(known_openai_model_ids().contains(&personal_model.to_string()));
    assert!(!known_openai_model_ids().contains(&work_model.to_string()));

    crate::auth::codex::set_active_account_override(Some("work".to_string()));
    assert!(known_openai_model_ids().contains(&work_model.to_string()));
    assert!(!known_openai_model_ids().contains(&personal_model.to_string()));

    crate::auth::codex::set_active_account_override(None);
}

#[test]
fn test_openai_live_catalog_replaces_static_fallback_list() {
    let _guard = crate::storage::lock_test_env();
    crate::auth::codex::set_active_account_override(Some("work".to_string()));

    populate_account_models(vec!["gpt-5.4-live-only".to_string()]);
    let models = known_openai_model_ids();

    assert_eq!(models[..1], ["gpt-5.4-live-only".to_string()]);
    // The only entries allowed past the live catalog are the platform-API-only
    // GPT Pro models, appended when an OPENAI_API_KEY is configured on the
    // machine running the tests.
    for extra in &models[1..] {
        assert!(
            jcode_provider_core::is_openai_api_only_pro_model(extra),
            "unexpected non-pro extra model '{extra}' in live catalog list"
        );
    }

    crate::auth::codex::set_active_account_override(None);
}

#[test]
fn test_anthropic_live_catalog_replaces_static_fallback_list() {
    let _guard = crate::storage::lock_test_env();
    crate::env::remove_var("ANTHROPIC_API_KEY");
    crate::auth::claude::set_active_account_override(Some("work".to_string()));

    // Use a model the static classifier does not recognize so this exercises
    // the generic catalog-driven path (>=1M cached limit => synthesized [1m]
    // alias). The id must carry no parseable version, because any versioned
    // Claude id is now classified statically (>=5.0 => native 1M, which
    // deliberately gets no redundant [1m] alias).
    populate_context_limits(
        [("claude-nebula-preview".to_string(), 1_048_576)]
            .into_iter()
            .collect(),
    );
    populate_anthropic_models(vec!["claude-nebula-preview".to_string()]);
    let models = known_anthropic_model_ids();

    assert_eq!(
        models,
        vec![
            "claude-nebula-preview".to_string(),
            "claude-nebula-preview[1m]".to_string()
        ]
    );

    crate::auth::claude::set_active_account_override(None);
}

#[test]
fn test_openai_model_catalog_hydrates_from_disk_cache() {
    with_clean_provider_test_env(|| {
        crate::auth::codex::set_active_account_override(Some("disk-openai".to_string()));
        persist_openai_model_catalog(&OpenAIModelCatalog {
            available_models: vec!["openai-disk-only-model".to_string()],
            context_limits: [("openai-disk-only-model".to_string(), 424_242)]
                .into_iter()
                .collect(),
            reasoning_efforts: [(
                "openai-disk-only-model".to_string(),
                vec!["low".to_string(), "max".to_string()],
            )]
            .into_iter()
            .collect(),
        });

        assert_eq!(
            cached_openai_model_ids(),
            Some(vec!["openai-disk-only-model".to_string()])
        );
        assert_eq!(
            context_limit_for_model("openai-disk-only-model"),
            Some(424_242)
        );
        assert_eq!(
            cached_openai_reasoning_efforts()
                .and_then(|efforts| efforts.get("openai-disk-only-model").cloned()),
            Some(vec!["low".to_string(), "max".to_string()])
        );

        crate::auth::codex::set_active_account_override(None);
    });
}

#[test]
fn test_anthropic_model_catalog_hydrates_from_disk_cache() {
    with_clean_provider_test_env(|| {
        crate::env::remove_var("ANTHROPIC_API_KEY");
        crate::auth::claude::set_active_account_override(Some("disk-claude".to_string()));
        persist_anthropic_model_catalog(&AnthropicModelCatalog {
            available_models: vec!["claude-nebula-preview".to_string()],
            context_limits: [("claude-nebula-preview".to_string(), 1_048_576)]
                .into_iter()
                .collect(),
        });

        assert_eq!(
            cached_anthropic_model_ids(),
            Some(vec![
                "claude-nebula-preview".to_string(),
                "claude-nebula-preview[1m]".to_string()
            ])
        );
        assert_eq!(
            context_limit_for_model("claude-nebula-preview"),
            Some(1_048_576)
        );

        crate::auth::claude::set_active_account_override(None);
    });
}

#[test]
fn test_same_provider_account_candidates_include_other_openai_accounts() {
    with_clean_provider_test_env(|| {
        let now_ms = chrono::Utc::now().timestamp_millis() + 60_000;
        crate::auth::codex::upsert_account(crate::auth::codex::OpenAiAccount {
            label: "seed-a".to_string(),
            access_token: "acc-a".to_string(),
            refresh_token: "ref-a".to_string(),
            id_token: None,
            account_id: Some("acct-a".to_string()),
            expires_at: Some(now_ms),
            email: Some("a@example.com".to_string()),
        })
        .unwrap();
        crate::auth::codex::upsert_account(crate::auth::codex::OpenAiAccount {
            label: "seed-b".to_string(),
            access_token: "acc-b".to_string(),
            refresh_token: "ref-b".to_string(),
            id_token: None,
            account_id: Some("acct-b".to_string()),
            expires_at: Some(now_ms),
            email: Some("b@example.com".to_string()),
        })
        .unwrap();

        crate::auth::codex::set_active_account("openai-1").unwrap();
        let candidates = MultiProvider::same_provider_account_candidates(ActiveProvider::OpenAI);
        assert_eq!(candidates, vec!["openai-2".to_string()]);
    });
}

#[test]
fn test_normalize_copilot_model_name_claude() {
    assert_eq!(
        normalize_copilot_model_name("claude-opus-4.6"),
        Some("claude-opus-4-6")
    );
    assert_eq!(
        normalize_copilot_model_name("claude-sonnet-4.6"),
        Some("claude-sonnet-4-6")
    );
    assert_eq!(
        normalize_copilot_model_name("claude-sonnet-4.5"),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(
        normalize_copilot_model_name("claude-haiku-4.5"),
        Some("claude-haiku-4-5")
    );
}

#[test]
fn test_normalize_copilot_model_name_already_canonical() {
    assert_eq!(normalize_copilot_model_name("claude-opus-4-6"), None);
    assert_eq!(normalize_copilot_model_name("claude-sonnet-4-6"), None);
    assert_eq!(normalize_copilot_model_name("gpt-5.3-codex"), None);
}

#[test]
fn test_normalize_copilot_model_name_unknown() {
    assert_eq!(normalize_copilot_model_name("gemini-3-pro-preview"), None);
    assert_eq!(normalize_copilot_model_name("mistral-large-2411"), None);
}

#[test]
fn test_provider_for_model_copilot_dot_notation() {
    assert_eq!(provider_for_model("claude-opus-4.6"), Some("claude"));
    assert_eq!(provider_for_model("claude-sonnet-4.6"), Some("claude"));
    assert_eq!(provider_for_model("claude-haiku-4.5"), Some("claude"));
    assert_eq!(provider_for_model("gpt-4.1"), Some("openai"));
}
