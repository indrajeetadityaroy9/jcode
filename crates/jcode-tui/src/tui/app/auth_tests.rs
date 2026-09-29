use super::{App, antigravity_input_requires_state_validation};

fn with_temp_jcode_home<T>(f: impl FnOnce() -> T) -> T {
    let _env_guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let saved_env = ["JCODE_HOME", "OPENAI_API_KEY"].map(|key| (key, std::env::var_os(key)));

    crate::env::set_var("JCODE_HOME", temp.path());
    for (key, _) in saved_env.iter().skip(1) {
        crate::env::remove_var(key);
    }

    let result = f();

    for (key, value) in saved_env {
        if let Some(value) = value {
            crate::env::set_var(key, value);
        } else {
            crate::env::remove_var(key);
        }
    }
    result
}

#[test]
fn antigravity_auto_callback_code_skips_manual_callback_parser() {
    assert!(!antigravity_input_requires_state_validation(
        "raw_authorization_code",
        Some("expected_state")
    ));
}

#[test]
fn antigravity_manual_callback_url_keeps_state_validation() {
    assert!(antigravity_input_requires_state_validation(
        "http://127.0.0.1:51121/oauth-callback?code=abc&state=expected_state",
        Some("expected_state")
    ));
}

#[test]
fn oauth_preflight_mentions_browser_fallback_and_doctor() {
    let message = App::record_oauth_preflight("openai", false, Some("localhost:1455"), Some(true));
    assert!(message.contains("could not open a browser"));
    assert!(message.contains("auth doctor openai"));
}

#[test]
fn oauth_preflight_mentions_manual_safe_callback_mode() {
    let message = App::record_oauth_preflight(
        "gemini",
        true,
        Some("http://127.0.0.1:0/oauth2callback"),
        Some(false),
    );
    assert!(message.contains("manual-safe paste completion"));
    assert!(message.contains("oauth2callback"));
}

#[test]
fn tui_api_key_logout_clears_saved_key_and_process_env() -> anyhow::Result<()> {
    with_temp_jcode_home(|| {
        App::save_named_api_key("openai.env", "OPENAI_API_KEY", "sk-test-tui-login")?;

        assert_eq!(
            std::env::var("OPENAI_API_KEY").as_deref(),
            Ok("sk-test-tui-login")
        );

        App::clear_api_key_login("OPENAI_API_KEY", "openai.env")?;

        assert!(std::env::var_os("OPENAI_API_KEY").is_none());
        assert!(
            crate::provider_catalog::load_api_key_from_env_or_config(
                "OPENAI_API_KEY",
                "openai.env",
            )
            .is_none()
        );
        Ok(())
    })
}
