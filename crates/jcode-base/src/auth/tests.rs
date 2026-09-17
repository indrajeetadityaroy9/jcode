use super::*;
use std::ffi::OsString;

fn restore_env_var(key: &str, previous: Option<OsString>) {
    if let Some(previous) = previous {
        crate::env::set_var(key, previous);
    } else {
        crate::env::remove_var(key);
    }
}

#[test]
fn auth_state_default_is_not_configured() {
    let state = AuthState::default();
    assert_eq!(state, AuthState::NotConfigured);
}

#[test]
fn auth_status_default_all_not_configured() {
    let status = AuthStatus::default();
    assert_eq!(status.anthropic.state, AuthState::NotConfigured);
    assert_eq!(status.openrouter, AuthState::NotConfigured);
    assert_eq!(status.openai, AuthState::NotConfigured);
    assert_eq!(status.antigravity, AuthState::NotConfigured);
    assert!(!status.openai_has_oauth);
    assert!(!status.openai_has_api_key);
    assert!(!status.anthropic.has_oauth);
    assert!(!status.anthropic.has_api_key);
}

#[test]
fn full_and_fast_auth_status_match_for_shared_probe_fields() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp dir");
    let home = temp.path().join("home");
    let xdg = temp.path().join("xdg");
    std::fs::create_dir_all(&home).expect("create temp home");
    std::fs::create_dir_all(&xdg).expect("create temp xdg config");
    let saved = [
        "JCODE_HOME",
        "XDG_CONFIG_HOME",
        "HOME",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_ENV_FILE",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_TRANSPORT_STATE",
        "JCODE_OPENROUTER_ALLOW_NO_AUTH",
        "JCODE_OPENROUTER_MODEL_CATALOG",
        "JCODE_OPENROUTER_STATIC_MODELS",
        "JCODE_OPENROUTER_MODEL",
    ]
    .into_iter()
    .map(|key| (key, std::env::var_os(key)))
    .collect::<Vec<_>>();

    crate::env::set_var("JCODE_HOME", temp.path().join("jcode-home"));
    crate::env::set_var("XDG_CONFIG_HOME", &xdg);
    crate::env::set_var("HOME", &home);
    crate::env::set_var("ANTHROPIC_API_KEY", "anthropic-test-key");
    crate::env::set_var("OPENAI_API_KEY", "openai-test-key");
    crate::env::set_var("OPENROUTER_API_KEY", "openrouter-test-key");
    for key in [
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_ENV_FILE",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_TRANSPORT_STATE",
        "JCODE_OPENROUTER_ALLOW_NO_AUTH",
        "JCODE_OPENROUTER_MODEL_CATALOG",
        "JCODE_OPENROUTER_STATIC_MODELS",
        "JCODE_OPENROUTER_MODEL",
    ] {
        crate::env::remove_var(key);
    }
    AuthStatus::invalidate_cache();

    let (full, _) = build_auth_status_uncached(AuthProbeMode::Full);
    let (fast, _) = build_auth_status_uncached(AuthProbeMode::Fast);

    assert_auth_status_shared_fields_match(&full, &fast);
    assert_eq!(full.anthropic.state, AuthState::Available);
    assert_eq!(full.openai, AuthState::Available);
    assert_eq!(full.openrouter, AuthState::Available);

    for (key, value) in saved {
        restore_env_var(key, value);
    }
    AuthStatus::invalidate_cache();
}

fn assert_auth_status_shared_fields_match(full: &AuthStatus, fast: &AuthStatus) {
    assert_eq!(
        full.anthropic.state, fast.anthropic.state,
        "anthropic.state"
    );
    assert_eq!(
        full.anthropic.has_oauth, fast.anthropic.has_oauth,
        "anthropic.has_oauth"
    );
    assert_eq!(
        full.anthropic.has_api_key, fast.anthropic.has_api_key,
        "anthropic.has_api_key"
    );
    assert_eq!(full.openrouter, fast.openrouter, "openrouter");
    assert_eq!(full.openai, fast.openai, "openai");
    assert_eq!(full.openai_has_oauth, fast.openai_has_oauth, "openai oauth");
    assert_eq!(
        full.openai_has_api_key, fast.openai_has_api_key,
        "openai api key"
    );
    assert_eq!(full.antigravity, fast.antigravity, "antigravity");
    assert_eq!(full.gemini, fast.gemini, "gemini");
}

#[test]
fn provider_auth_default() {
    let auth = ProviderAuth::default();
    assert_eq!(auth.state, AuthState::NotConfigured);
    assert!(!auth.has_oauth);
    assert!(!auth.has_api_key);
}

#[test]
fn provider_auth_assessment_predicates_reflect_state() {
    fn assessment_with_state(state: AuthState) -> ProviderAuthAssessment {
        ProviderAuthAssessment {
            state,
            readiness: AuthReadinessLevel::None,
            method_detail: "test".to_string(),
            credential_source: AuthCredentialSource::None,
            credential_source_detail: "not configured".to_string(),
            expiry_confidence: AuthExpiryConfidence::Unknown,
            refresh_support: AuthRefreshSupport::Unknown,
            validation_method: AuthValidationMethod::Unknown,
            last_validation: None,
            last_refresh: None,
        }
    }

    let not_configured = assessment_with_state(AuthState::NotConfigured);
    assert!(!not_configured.is_configured());
    assert!(!not_configured.is_available());

    let expired = assessment_with_state(AuthState::Expired);
    assert!(expired.is_configured());
    assert!(!expired.is_available());

    let available = assessment_with_state(AuthState::Available);
    assert!(available.is_configured());
    assert!(available.is_available());
}

#[test]
fn command_exists_for_known_binary() {
    assert!(command_exists("ls"));
}

#[test]
fn command_exists_empty_string() {
    assert!(!command_exists(""));
    assert!(!command_exists("   "));
}

#[test]
fn command_exists_nonexistent() {
    assert!(!command_exists("surely_this_binary_does_not_exist_xyz"));
}

#[test]
fn command_exists_absolute_path() {
    assert!(command_exists("/bin/ls") || command_exists("/usr/bin/ls"));
}

#[test]
fn command_exists_absolute_nonexistent() {
    assert!(!command_exists("/nonexistent/path/to/binary"));
}

#[test]
fn contains_path_separator_detection() {
    assert!(contains_path_separator("/usr/bin/test"));
    assert!(contains_path_separator("./test"));
    assert!(!contains_path_separator("test"));
}

#[test]
fn has_extension_detection() {
    assert!(has_extension(std::path::Path::new("test.exe")));
    assert!(!has_extension(std::path::Path::new("test")));
    assert!(has_extension(std::path::Path::new("test.sh")));
}

#[test]
fn dedup_preserves_order() {
    let input = vec![
        std::ffi::OsString::from("a"),
        std::ffi::OsString::from("b"),
        std::ffi::OsString::from("a"),
        std::ffi::OsString::from("c"),
    ];
    let result = dedup_preserve_order(input);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], "a");
    assert_eq!(result[1], "b");
    assert_eq!(result[2], "c");
}

#[test]
fn auth_state_equality() {
    assert_eq!(AuthState::Available, AuthState::Available);
    assert_eq!(AuthState::Expired, AuthState::Expired);
    assert_eq!(AuthState::NotConfigured, AuthState::NotConfigured);
    assert_ne!(AuthState::Available, AuthState::Expired);
    assert_ne!(AuthState::Available, AuthState::NotConfigured);
}

#[test]
fn command_exists_cached_on_second_call() {
    // Clear cache first to isolate this test
    if let Ok(mut cache) = COMMAND_EXISTS_CACHE.lock() {
        cache.remove("surely_this_binary_does_not_exist_xyz_cache_test");
    }
    // First call populates the cache
    let result1 = command_exists("surely_this_binary_does_not_exist_xyz_cache_test");
    assert!(!result1);
    // Second call must return same result (from cache)
    let result2 = command_exists("surely_this_binary_does_not_exist_xyz_cache_test");
    assert_eq!(result1, result2);
}

#[test]
fn auth_status_check_returns_valid_struct() {
    let status = AuthStatus::check_fast();
    // Just verify it runs without panicking and has coherent state
    match status.anthropic.state {
        AuthState::Available | AuthState::Expired | AuthState::NotConfigured => {}
    }
    match status.openai {
        AuthState::Available | AuthState::Expired | AuthState::NotConfigured => {}
    }
}

#[test]
fn auth_status_check_fast_ignores_expired_full_cache() {
    let _lock = crate::storage::lock_test_env();
    AuthStatus::invalidate_cache();

    let stale_status = AuthStatus {
        openrouter: AuthState::Expired,
        ..Default::default()
    };
    let stale_when = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(
            AUTH_STATUS_CACHE_TTL_SECS + 1,
        ))
        .expect("stale cache timestamp");

    *AUTH_STATUS_CACHE.write().expect("auth cache lock") =
        Some((stale_status, stale_when, auth_cache_home_key()));
    *AUTH_STATUS_FAST_CACHE
        .write()
        .expect("fast auth cache lock") = None;

    let status = AuthStatus::check_fast();
    assert_ne!(
        status.openrouter,
        AuthState::Expired,
        "check_fast must not reuse an expired full auth cache forever"
    );

    AuthStatus::invalidate_cache();
}

#[test]
fn openrouter_like_status_is_provider_specific() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp dir");
    let prev_home = std::env::var_os("JCODE_HOME");
    let prev_chutes = std::env::var_os("CHUTES_API_KEY");
    let prev_opencode = std::env::var_os("OPENCODE_API_KEY");

    crate::env::set_var("JCODE_HOME", temp.path());
    crate::env::set_var("CHUTES_API_KEY", "chutes-test-key");
    crate::env::remove_var("OPENCODE_API_KEY");
    AuthStatus::invalidate_cache();

    let status = AuthStatus::check_fast();
    let chutes_assessment =
        status.assessment_for_provider(crate::provider_catalog::CHUTES_LOGIN_PROVIDER);
    let opencode_assessment =
        status.assessment_for_provider(crate::provider_catalog::OPENCODE_LOGIN_PROVIDER);
    assert!(chutes_assessment.is_available());
    assert_eq!(opencode_assessment.state, AuthState::NotConfigured);
    assert_eq!(
        chutes_assessment.method_detail,
        "API key (`CHUTES_API_KEY`)".to_string()
    );

    restore_env_var("JCODE_HOME", prev_home);
    restore_env_var("CHUTES_API_KEY", prev_chutes);
    restore_env_var("OPENCODE_API_KEY", prev_opencode);
    AuthStatus::invalidate_cache();
}

#[test]
fn openrouter_status_excludes_shared_compatible_transport() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp dir");
    let keys = [
        "JCODE_HOME",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_ENV_FILE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_ALLOW_NO_AUTH",
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_OPENROUTER_TRANSPORT_STATE",
    ];
    let saved = keys
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect::<Vec<_>>();

    for key in keys {
        crate::env::remove_var(key);
    }
    crate::env::set_var("JCODE_HOME", temp.path());

    crate::env::set_var("OPENAI_API_KEY", "openai-test-key");
    assert!(!crate::provider::openrouter::has_openrouter_credentials());
    AuthStatus::invalidate_cache();
    assert_eq!(
        AuthStatus::check_fast().openrouter,
        AuthState::NotConfigured
    );

    crate::env::set_var("JCODE_OPENROUTER_API_BASE", "https://example.test/v1");
    crate::env::set_var("JCODE_OPENROUTER_API_KEY_NAME", "OPENAI_API_KEY");
    assert!(crate::provider::openrouter::has_credentials());
    assert!(!crate::provider::openrouter::has_openrouter_credentials());

    crate::env::remove_var("JCODE_OPENROUTER_API_BASE");
    crate::env::remove_var("JCODE_OPENROUTER_API_KEY_NAME");
    crate::env::remove_var("OPENAI_API_KEY");
    crate::env::set_var("OPENROUTER_API_KEY", "openrouter-test-key");
    assert!(crate::provider::openrouter::has_openrouter_credentials());
    AuthStatus::invalidate_cache();
    assert_eq!(AuthStatus::check_fast().openrouter, AuthState::Available);

    for (key, value) in saved {
        restore_env_var(key, value);
    }
    AuthStatus::invalidate_cache();
}

#[test]
fn configured_api_key_source_uses_valid_overrides() {
    let _lock = crate::storage::lock_test_env();
    let key_var = "JCODE_OPENAI_COMPAT_API_KEY_NAME";
    let file_var = "JCODE_OPENAI_COMPAT_ENV_FILE";
    let prev_key = std::env::var(key_var).ok();
    let prev_file = std::env::var(file_var).ok();

    crate::env::set_var(key_var, "GROQ_API_KEY");
    crate::env::set_var(file_var, "groq.env");

    let source = crate::provider_catalog::configured_api_key_source(
        key_var,
        file_var,
        "OPENAI_COMPAT_API_KEY",
        "compat.env",
    );
    assert_eq!(
        source,
        Some(("GROQ_API_KEY".to_string(), "groq.env".to_string()))
    );

    if let Some(v) = prev_key {
        crate::env::set_var(key_var, v);
    } else {
        crate::env::remove_var(key_var);
    }
    if let Some(v) = prev_file {
        crate::env::set_var(file_var, v);
    } else {
        crate::env::remove_var(file_var);
    }
}

#[test]
fn configured_api_key_source_rejects_invalid_values() {
    let _lock = crate::storage::lock_test_env();
    let key_var = "JCODE_OPENAI_COMPAT_API_KEY_NAME";
    let file_var = "JCODE_OPENAI_COMPAT_ENV_FILE";
    let prev_key = std::env::var(key_var).ok();
    let prev_file = std::env::var(file_var).ok();

    crate::env::set_var(key_var, "bad-key");
    crate::env::set_var(file_var, "../bad.env");

    let source = crate::provider_catalog::configured_api_key_source(
        key_var,
        file_var,
        "OPENAI_COMPAT_API_KEY",
        "compat.env",
    );
    assert!(source.is_none());

    if let Some(v) = prev_key {
        crate::env::set_var(key_var, v);
    } else {
        crate::env::remove_var(key_var);
    }
    if let Some(v) = prev_file {
        crate::env::set_var(file_var, v);
    } else {
        crate::env::remove_var(file_var);
    }
}

#[test]
fn anthropic_api_provider_reports_api_key_independently_of_oauth() {
    // Regression: the `anthropic-api` (API-key) login provider used to share the
    // OAuth/subscription credential's availability via `auth_state_key::Anthropic`.
    // That made it claim "available / OAuth + API key" even with zero API key
    // configured, then fail at request time (API-key mode never falls back to
    // OAuth). It must report purely on the presence of an Anthropic API key.
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp dir");
    let home = temp.path().join("home");
    let xdg = temp.path().join("xdg");
    std::fs::create_dir_all(&home).expect("create temp home");
    std::fs::create_dir_all(&xdg).expect("create temp xdg config");
    let saved = ["JCODE_HOME", "XDG_CONFIG_HOME", "HOME", "ANTHROPIC_API_KEY"]
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect::<Vec<_>>();

    crate::env::set_var("JCODE_HOME", temp.path().join("jcode-home"));
    crate::env::set_var("XDG_CONFIG_HOME", &xdg);
    crate::env::set_var("HOME", &home);
    crate::env::remove_var("ANTHROPIC_API_KEY");
    AuthStatus::invalidate_cache();

    // No API key anywhere: the API-key provider must be NotConfigured, even if
    // OAuth credentials happen to exist for the separate `claude` provider.
    let status = AuthStatus::check_fast();
    let api = status.assessment_for_provider(crate::provider_catalog::ANTHROPIC_API_LOGIN_PROVIDER);
    assert_eq!(
        api.state,
        AuthState::NotConfigured,
        "anthropic-api must not borrow OAuth availability"
    );
    assert_eq!(api.method_detail, "not configured");

    // With an API key present (env here; config-file path is covered separately),
    // the API-key provider becomes available and names ANTHROPIC_API_KEY honestly.
    crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-api-test-key");
    AuthStatus::invalidate_cache();
    let status = AuthStatus::check_fast();
    let api = status.assessment_for_provider(crate::provider_catalog::ANTHROPIC_API_LOGIN_PROVIDER);
    assert_eq!(api.state, AuthState::Available);
    assert!(
        api.method_detail.contains("ANTHROPIC_API_KEY"),
        "method detail should name the API key env: {}",
        api.method_detail
    );

    for (key, value) in saved {
        restore_env_var(key, value);
    }
    AuthStatus::invalidate_cache();
}

#[test]
fn claude_oauth_provider_reports_oauth_independently_of_api_key() {
    // Mirror of the regression above: the `claude` (OAuth/subscription) login
    // provider must report on OAuth credentials alone. An ANTHROPIC_API_KEY
    // used to leak into `auth_state_key::Anthropic`, making the OAuth row claim
    // "available / OAuth + API key" with zero OAuth accounts -- contradicting
    // the separate `anthropic-api` row and the header's active-route tag.
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp dir");
    let home = temp.path().join("home");
    let xdg = temp.path().join("xdg");
    std::fs::create_dir_all(&home).expect("create temp home");
    std::fs::create_dir_all(&xdg).expect("create temp xdg config");
    let saved = ["JCODE_HOME", "XDG_CONFIG_HOME", "HOME", "ANTHROPIC_API_KEY"]
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect::<Vec<_>>();

    crate::env::set_var("JCODE_HOME", temp.path().join("jcode-home"));
    crate::env::set_var("XDG_CONFIG_HOME", &xdg);
    crate::env::set_var("HOME", &home);
    // API key present, no OAuth anywhere: the OAuth provider must stay
    // NotConfigured and must not describe the API key as its method.
    crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-api-test-key");
    AuthStatus::invalidate_cache();

    let status = AuthStatus::check_fast();
    let oauth = status.assessment_for_provider(crate::provider_catalog::CLAUDE_LOGIN_PROVIDER);
    assert_eq!(
        oauth.state,
        AuthState::NotConfigured,
        "claude (OAuth) must not borrow API-key availability"
    );
    assert_eq!(oauth.method_detail, "not configured");
    assert!(
        !oauth.credential_source_detail.contains("ANTHROPIC_API_KEY"),
        "OAuth row must not attribute the API key as its source: {}",
        oauth.credential_source_detail
    );

    // The API-key row still owns that credential.
    let api = status.assessment_for_provider(crate::provider_catalog::ANTHROPIC_API_LOGIN_PROVIDER);
    assert_eq!(api.state, AuthState::Available);

    for (key, value) in saved {
        restore_env_var(key, value);
    }
    AuthStatus::invalidate_cache();
}

/// Test binaries must never open real browser windows: login flows are
/// exercised heavily by unit tests, and each ungated `open::that` pops an
/// OAuth page on the developer's desktop. `running_in_test_harness` detects
/// the `target/**/deps/` test-binary path, and `browser_suppressed` must honor
/// it even without --no-browser or NO_BROWSER/JCODE_NO_BROWSER.
#[test]
fn browser_suppressed_inside_test_harness_without_env_overrides() {
    assert!(
        super::running_in_test_harness(),
        "test binary should be detected as a test harness (exe under target/**/deps/)"
    );
    assert!(
        super::browser_suppressed(false),
        "browser opens must be suppressed in test binaries even without --no-browser/env vars"
    );
}

/// Antigravity/Gemini access tokens live about an hour and are refreshed
/// transparently on the next request. Reporting `Expired` just because the
/// cached access token aged out made a fully working provider render as broken
/// in `/login`, the header, and `jcode auth status`, which is what
/// the "antigravity is not working" reports actually were. Only a missing or
/// permanently rejected refresh token means the user must log in again.
#[test]
fn refreshable_token_state_covers_the_full_expiry_state_space() {
    let never_rejected = |_: &str| false;
    let always_rejected = |_: &str| true;

    // (case, expired access token, refresh token, refresh token rejected) -> state
    let cases: [(&str, bool, &str, bool, AuthState); 6] = [
        (
            "hourly access token expired but refresh works",
            true,
            "1//live-refresh-token",
            false,
            AuthState::Available,
        ),
        (
            "fresh access token",
            false,
            "1//live-refresh-token",
            false,
            AuthState::Available,
        ),
        (
            "fresh access token, no refresh token",
            false,
            "",
            false,
            AuthState::Available,
        ),
        (
            "expired with no refresh token needs re-login",
            true,
            "   ",
            false,
            AuthState::Expired,
        ),
        (
            "expired with revoked refresh token needs re-login",
            true,
            "1//revoked",
            true,
            AuthState::Expired,
        ),
        (
            "fresh access token is trusted even if an old refresh token was rejected",
            false,
            "1//revoked",
            true,
            AuthState::Available,
        ),
    ];

    for (case, expired, refresh_token, rejected, expected) in cases {
        let observed = if rejected {
            super::refreshable_token_state_with(
                Ok((expired, refresh_token.to_string())),
                always_rejected,
            )
        } else {
            super::refreshable_token_state_with(
                Ok((expired, refresh_token.to_string())),
                never_rejected,
            )
        };
        assert_eq!(observed, expected, "{case}");
    }
}

#[test]
fn missing_refreshable_credentials_are_not_configured() {
    assert_eq!(
        super::refreshable_token_state_with(
            Err(anyhow::anyhow!("No Antigravity tokens found.")),
            |_| false
        ),
        AuthState::NotConfigured
    );
}
