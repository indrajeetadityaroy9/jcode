use super::*;

fn with_clean_provider_test_env<T>(f: impl FnOnce() -> T) -> T {
    let _guard = crate::storage::lock_test_env();
    // Concrete provider runtimes live downstream (jcode-provider-*-runtime),
    // so base tests register shared stubs through the same composition-root
    // registry the binary uses. Registration is idempotent (last write wins),
    // and per-test overrides can re-register a different stub.
    register_test_external_runtimes();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    let profile_env_keys = vec![
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "JCODE_RUNTIME_PROVIDER",
        "JCODE_ACTIVE_PROVIDER",
        "JCODE_INITIAL_PROVIDER_EXPLICIT",
        "JCODE_OPENAI_MODEL",
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_PROVIDER_PROFILE_ACTIVE",
        "JCODE_PROVIDER_PROFILE_NAME",
    ];
    let saved_profile_env = profile_env_keys
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect::<Vec<_>>();
    crate::env::set_var("JCODE_HOME", temp.path());
    for (key, _) in &saved_profile_env {
        crate::env::remove_var(key);
    }
    crate::auth::claude::set_active_account_override(None);
    crate::auth::codex::set_active_account_override(None);
    // The in-memory model catalog services are process-global; earlier tests
    // may have hydrated scopes (fixture models) that would corrupt this test's
    // known_*_model_ids() validation, and vice versa. Reset on entry and exit
    // so neither direction leaks.
    crate::provider::models::reset_model_catalog_services_for_tests();

    let result = f();

    crate::provider::models::reset_model_catalog_services_for_tests();
    crate::auth::claude::set_active_account_override(None);
    crate::auth::codex::set_active_account_override(None);
    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    for (key, value) in saved_profile_env {
        if let Some(value) = value {
            crate::env::set_var(key, value);
        } else {
            crate::env::remove_var(key);
        }
    }
    result
}

fn enter_test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime")
}

fn save_test_openai_oauth_credentials() {
    crate::auth::codex::upsert_account_from_tokens(
        &crate::auth::codex::primary_account_label(),
        "test-oauth-access-token",
        "test-oauth-refresh-token",
        None,
        Some(chrono::Utc::now().timestamp_millis() + 86_400_000),
    )
    .expect("save test OpenAI OAuth credentials");
}

fn test_multi_provider_with_openai() -> MultiProvider {
    save_test_openai_oauth_credentials();
    crate::env::set_var("OPENAI_API_KEY", "sk-test-openai-api-key");
    MultiProvider {
        claude: RwLock::new(None),
        anthropic: RwLock::new(None),
        openai: RwLock::new(Some(test_openai_runtime() as Arc<dyn Provider>)),
        antigravity: RwLock::new(None),
        gemini: RwLock::new(None),
        active: RwLock::new(ActiveProvider::OpenAI),
        use_claude_cli: false,
        startup_notices: RwLock::new(Vec::new()),
        initial_provider: None,
        routes_memo: std::sync::Mutex::new(None),
        post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    }
}

#[test]
fn openai_model_switch_prefixes_preserve_oauth_vs_api_state_space() {
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        let provider = test_multi_provider_with_openai();
        let models = known_openai_model_ids();
        let primary = models.first().expect("at least one OpenAI model").as_str();
        let alternate = models.get(1).map(String::as_str).unwrap_or(primary);

        let cases = [
            vec![
                (
                    format!("openai-api:{primary}"),
                    openai::OpenAICredentialMode::ApiKey,
                    primary,
                ),
                (
                    format!("openai-oauth:{alternate}"),
                    openai::OpenAICredentialMode::OAuth,
                    alternate,
                ),
            ],
            vec![
                (
                    format!("openai-oauth:{primary}"),
                    openai::OpenAICredentialMode::OAuth,
                    primary,
                ),
                (
                    format!("openai-api:{alternate}"),
                    openai::OpenAICredentialMode::ApiKey,
                    alternate,
                ),
                (
                    format!("openai-oauth:{primary}"),
                    openai::OpenAICredentialMode::OAuth,
                    primary,
                ),
            ],
        ];

        for sequence in cases {
            for (request, expected_mode, expected_model) in sequence {
                provider
                    .set_model(&request)
                    .unwrap_or_else(|err| panic!("switch {request} should succeed: {err}"));
                assert_eq!(
                    provider.active_provider(),
                    ActiveProvider::OpenAI,
                    "{request}"
                );
                assert_eq!(provider.model(), expected_model, "{request}");
                assert_eq!(
                    provider
                        .openai_provider()
                        .expect("OpenAI provider")
                        .credential_mode(),
                    expected_mode,
                    "{request}"
                );
            }
        }
    });
}

#[test]
fn openai_model_route_roundtrip_preserves_auth_method_for_model_switches() {
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        let provider = test_multi_provider_with_openai();
        let models = known_openai_model_ids();
        let primary = models.first().expect("at least one OpenAI model").as_str();
        let alternate = models.get(1).map(String::as_str).unwrap_or(primary);

        // This mirrors the /model picker path: the selected ModelRoute becomes a
        // default/session model + provider key, then a future /model switch uses
        // that persisted provider key to reconstruct the provider-prefixed
        // request. The important invariant is that OpenAI OAuth and API key are
        // distinct states even though both execute in ActiveProvider::OpenAI.
        let route_cases = [
            (
                primary,
                "openai-oauth",
                "openai-oauth",
                "openai-oauth",
                openai::OpenAICredentialMode::OAuth,
            ),
            (
                alternate,
                "openai-api-key",
                "openai-api",
                "openai-api",
                openai::OpenAICredentialMode::ApiKey,
            ),
            (
                primary,
                "openai-api",
                "openai-api",
                "openai-api",
                openai::OpenAICredentialMode::ApiKey,
            ),
        ];

        for (bare_model, api_method, expected_provider_key, expected_prefix, expected_mode) in
            route_cases
        {
            let selection =
                MultiProvider::default_model_selection_from_route(bare_model, api_method, "OpenAI");
            assert_eq!(
                selection.model_spec,
                format!("{expected_prefix}:{bare_model}")
            );
            assert_eq!(
                selection.provider_key.as_deref(),
                Some(expected_provider_key)
            );

            let request = MultiProvider::model_switch_request_for_session_model(
                bare_model,
                selection.provider_key.as_deref(),
            );
            assert_eq!(request, format!("{expected_prefix}:{bare_model}"));

            provider
                .set_model(&request)
                .unwrap_or_else(|err| panic!("/model switch {request} should succeed: {err}"));
            assert_eq!(
                provider.active_provider(),
                ActiveProvider::OpenAI,
                "{request}"
            );
            assert_eq!(provider.model(), bare_model, "{request}");
            assert_eq!(
                provider
                    .openai_provider()
                    .expect("OpenAI provider")
                    .credential_mode(),
                expected_mode,
                "{request}"
            );
        }
    });
}

#[test]
fn active_explicit_credential_reflects_openai_switch_immediately_and_none_for_auto() {
    use jcode_provider_core::{Provider, ResolvedCredential};
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        let provider = test_multi_provider_with_openai();
        let model = known_openai_model_ids()
            .first()
            .expect("at least one OpenAI model")
            .clone();

        // Fresh provider with both credentials present defaults to auto, which
        // has no explicit pin: the info widget must fall back to its cached
        // heuristic instead of asserting an OAuth-vs-API choice the user never
        // made.
        assert_eq!(
            provider.active_explicit_credential(),
            None,
            "auto mode must not report an explicit pin"
        );

        // Switching to the API-key route pins the credential in memory, so the
        // widget must report API key on the very next read with no cache delay.
        provider
            .set_model(&format!("openai-api:{model}"))
            .expect("switch to OpenAI API key");
        assert_eq!(
            provider.active_explicit_credential(),
            Some(ResolvedCredential::ApiKey),
            "explicit API-key switch must be visible immediately"
        );

        // Switching back to OAuth flips it back just as immediately.
        provider
            .set_model(&format!("openai-oauth:{model}"))
            .expect("switch to OpenAI OAuth");
        assert_eq!(
            provider.active_explicit_credential(),
            Some(ResolvedCredential::Oauth),
            "explicit OAuth switch must be visible immediately"
        );
    });
}

#[test]
fn openai_model_routes_cover_oauth_api_and_no_auth_state_space() {
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        let model = known_openai_model_ids()
            .first()
            .expect("at least one OpenAI model")
            .clone();

        let provider = test_multi_provider_with_openai();
        let routes = provider.model_routes();
        let methods = routes
            .iter()
            .filter(|route| route.provider == "OpenAI" && route.model == model)
            .map(|route| (route.api_method.as_str(), route.available))
            .collect::<Vec<_>>();
        assert!(
            methods.contains(&("openai-oauth", true)),
            "routes: {methods:?}"
        );
        assert!(
            methods.contains(&("openai-api-key", true)),
            "routes: {methods:?}"
        );

        crate::env::remove_var("OPENAI_API_KEY");
        crate::auth::AuthStatus::invalidate_cache();
        let oauth_only = provider.model_routes();
        let oauth_only_methods = oauth_only
            .iter()
            .filter(|route| route.provider == "OpenAI" && route.model == model)
            .map(|route| route.api_method.as_str())
            .collect::<Vec<_>>();
        assert_eq!(oauth_only_methods, vec!["openai-oauth"]);

        crate::env::set_var("OPENAI_API_KEY", "sk-test-openai-api-key");
        std::fs::remove_file(
            crate::storage::jcode_dir()
                .unwrap()
                .join("openai-auth.json"),
        )
        .expect("remove oauth credentials");
        crate::auth::AuthStatus::invalidate_cache();
        let api_only = provider.model_routes();
        let api_only_methods = api_only
            .iter()
            .filter(|route| route.provider == "OpenAI" && route.model == model)
            .map(|route| route.api_method.as_str())
            .collect::<Vec<_>>();
        assert_eq!(api_only_methods, vec!["openai-api-key"]);
    });
}

/// The route-catalog memo must serve repeated `model_routes()` calls without
/// rebuilding (a shared server fans one ModelsUpdated event out to every
/// connection, each of which snapshots the catalog), while auth invalidation
/// and model switches must bypass it immediately.
#[test]
fn model_routes_memo_serves_repeats_and_invalidates_on_auth_and_model_changes() {
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        let provider = test_multi_provider_with_openai();

        let first = provider.model_routes();
        let second = provider.model_routes();
        assert_eq!(
            first, second,
            "memoized catalog must be identical across back-to-back reads"
        );
        assert!(
            provider
                .routes_memo
                .lock()
                .expect("routes memo lock")
                .is_some(),
            "memo should be populated after a build"
        );

        // Auth invalidation bumps the pricing generation, which must make the
        // memo stale even within the TTL window (verified behaviorally by the
        // oauth/api state-space test above; here we check the memo mechanism).
        let generation_before = crate::provider::pricing::auth_pricing_generation();
        crate::auth::AuthStatus::invalidate_cache();
        assert!(
            crate::provider::pricing::auth_pricing_generation() > generation_before,
            "auth invalidation must advance the pricing generation"
        );

        // A model switch drops the memo outright.
        let model = known_openai_model_ids()
            .first()
            .expect("at least one OpenAI model")
            .clone();
        provider.model_routes();
        let _ = provider.set_model(&format!("openai-api:{model}"));
        assert!(
            provider
                .routes_memo
                .lock()
                .expect("routes memo lock")
                .is_none(),
            "set_model must invalidate the routes memo"
        );

        // A second instance with identical catalog-relevant state must reuse
        // the shared process-wide memo (this is what collapses shared-server
        // connect bursts down to one build), and instances with different
        // state must not share a key.
        let first_instance = test_multi_provider_with_openai();
        let second_instance = test_multi_provider_with_openai();
        assert_eq!(
            first_instance.routes_memo_key(),
            second_instance.routes_memo_key(),
            "identical forks must share a memo key"
        );
        let baseline = first_instance.model_routes();
        assert!(
            second_instance
                .routes_memo
                .lock()
                .expect("routes memo lock")
                .is_none(),
            "second instance has not built anything yet"
        );
        let shared = second_instance.model_routes();
        assert_eq!(baseline, shared, "shared memo must serve identical routes");
        assert!(
            second_instance
                .routes_memo
                .lock()
                .expect("routes memo lock")
                .is_some(),
            "shared hit should hydrate the instance memo"
        );
        let _ = second_instance.set_model(&format!("openai-oauth:{model}"));
        // Credential-mode switches don't change catalog content (routes come
        // from global auth status), so the key may legitimately stay equal.
        // A different *model* must change it: the active model gets special
        // treatment (endpoint refresh priority) during the build.
        if let Some(alternate) = known_openai_model_ids().get(1) {
            let _ = second_instance.set_model(&format!("openai-oauth:{alternate}"));
            assert_ne!(
                first_instance.routes_memo_key(),
                second_instance.routes_memo_key(),
                "a different active model must change the memo key"
            );
        }
    });
}

/// Parameterized test stand-in for provider runtimes that live downstream
/// (jcode-provider-{gemini,antigravity}-runtime) and therefore cannot
/// be constructed from base tests. Mirrors each runtime's catalog surface
/// (static model list plus `ModelRoute`s) so routing/fallback tests stay
/// meaningful.
struct StubExternalRuntime {
    name: &'static str,
    provider_label: &'static str,
    api_method: &'static str,
    models: &'static [&'static str],
    model: std::sync::RwLock<String>,
    credential_mode: std::sync::RwLock<jcode_provider_core::CredentialMode>,
}

impl StubExternalRuntime {
    fn new(
        name: &'static str,
        provider_label: &'static str,
        api_method: &'static str,
        models: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            provider_label,
            api_method,
            models,
            model: std::sync::RwLock::new(models[0].to_string()),
            credential_mode: std::sync::RwLock::new(jcode_provider_core::CredentialMode::Auto),
        }
    }

    fn antigravity() -> Self {
        Self::new(
            "antigravity",
            "Antigravity",
            "https",
            antigravity::AVAILABLE_MODELS,
        )
    }

    fn anthropic() -> Self {
        Self::new(
            "anthropic",
            "Anthropic",
            "https",
            anthropic::AVAILABLE_MODELS,
        )
    }

    fn openai() -> Self {
        Self::new("openai", "OpenAI", "https", ALL_OPENAI_MODELS)
    }
}

#[async_trait::async_trait]
impl Provider for StubExternalRuntime {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> anyhow::Result<EventStream> {
        anyhow::bail!("stub {} runtime does not stream", self.name)
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn model(&self) -> String {
        self.model
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    fn set_model(&self, model: &str) -> anyhow::Result<()> {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            anyhow::bail!("{} model cannot be empty", self.provider_label);
        }
        // Mirror the real runtimes' family validation: the registry is
        // process-global, so hot-init can hand this stub to tests that expect
        // cross-provider models to be rejected (e.g. a Claude model under a
        // forced-OpenAI selection).
        if !self.models.contains(&trimmed) {
            anyhow::bail!(
                "Unsupported {} model '{}'. Use /model to choose from the models available to your account.",
                self.provider_label,
                trimmed,
            );
        }
        *self
            .model
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = trimmed.to_string();
        Ok(())
    }
    fn available_models(&self) -> Vec<&'static str> {
        self.models.to_vec()
    }
    fn available_models_display(&self) -> Vec<String> {
        self.models.iter().map(|model| model.to_string()).collect()
    }
    fn available_models_for_switching(&self) -> Vec<String> {
        self.available_models_display()
    }
    fn model_routes(&self) -> Vec<ModelRoute> {
        self.available_models_display()
            .into_iter()
            .map(|model| ModelRoute {
                model,
                provider: self.provider_label.to_string(),
                api_method: self.api_method.to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            })
            .collect()
    }
    fn credential_mode(&self) -> jcode_provider_core::CredentialMode {
        *self
            .credential_mode
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn set_credential_mode(&self, mode: jcode_provider_core::CredentialMode) -> anyhow::Result<()> {
        *self
            .credential_mode
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = mode;
        Ok(())
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(StubExternalRuntime::new(
            self.name,
            self.provider_label,
            self.api_method,
            self.models,
        ))
    }
}

fn test_antigravity_runtime() -> Arc<dyn Provider> {
    Arc::new(StubExternalRuntime::antigravity())
}

fn test_anthropic_runtime() -> Arc<StubExternalRuntime> {
    Arc::new(StubExternalRuntime::anthropic())
}

fn test_openai_runtime() -> Arc<StubExternalRuntime> {
    Arc::new(StubExternalRuntime::openai())
}

/// Register the shared external-runtime stubs for every downstream provider
/// slot base can hot-initialize. Called by `with_clean_provider_test_env` so
/// hot-init/startup tests find a runtime the way the real binary does.
fn register_test_external_runtimes() {
    external::register_external_provider(external::ANTHROPIC_RUNTIME, || {
        test_anthropic_runtime() as Arc<dyn Provider>
    });
    external::register_external_provider(external::OPENAI_RUNTIME, || {
        test_openai_runtime() as Arc<dyn Provider>
    });
    external::register_external_provider(external::ANTIGRAVITY_RUNTIME, test_antigravity_runtime);
}

#[test]
fn new_session_fork_reloads_changed_config_provider_and_model() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        runtime.block_on(async {
            crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test");
            crate::env::set_var("OPENAI_API_KEY", "sk-openai-test");

            crate::config::Config::set_default_model(Some("claude-fable-5"), Some("anthropic-api"))
                .expect("save initial Claude default");
            let template = MultiProvider::new();
            assert_eq!(template.name(), "Claude");
            assert_eq!(template.model(), "claude-fable-5");

            crate::config::Config::set_default_model(Some("gpt-5.5"), Some("openai-api"))
                .expect("save changed OpenAI default");

            let fresh = template.fork_for_new_session();
            assert_eq!(fresh.name(), "OpenAI");
            assert_eq!(fresh.model(), "gpt-5.5");
            assert_eq!(
                fresh.active_resolved_credential(),
                Some(jcode_provider_core::ResolvedCredential::ApiKey)
            );

            // Ordinary forks still preserve the existing session's selection.
            let preserved = template.fork();
            assert_eq!(preserved.name(), "Claude");
            assert_eq!(preserved.model(), "claude-fable-5");
        });
    });
}

include!("tests/auth_refresh.rs");
include!("tests/model_resolution.rs");
include!("tests/fallback_failover.rs");
include!("tests/catalog_accounts.rs");
