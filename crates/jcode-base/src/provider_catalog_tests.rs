use super::*;

struct EnvGuard {
    vars: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn save(keys: &[&str]) -> Self {
        let vars = keys
            .iter()
            .map(|key| (key.to_string(), std::env::var(key).ok()))
            .collect();
        Self { vars }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.vars {
            if let Some(value) = value {
                crate::env::set_var(key, value);
            } else {
                crate::env::remove_var(key);
            }
        }
    }
}

#[test]
fn named_anthropic_compatible_profile_maps_endpoint_auth_headers_and_model() {
    let _lock = crate::storage::lock_test_env();
    let _guard = EnvGuard::save(&[
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_ANTHROPIC_API_BASE",
        "JCODE_ANTHROPIC_API_KEY_NAME",
        "JCODE_ANTHROPIC_AUTH",
        "JCODE_ANTHROPIC_AUTH_HEADER",
        "JCODE_ANTHROPIC_HEADERS",
        "JCODE_ANTHROPIC_MODEL",
        "JCODE_RUNTIME_PROVIDER",
    ]);
    let previous_home = std::env::var_os("JCODE_HOME");
    let temp = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::config::Config::invalidate_cache();

    let config_path = crate::config::Config::path().expect("config path");
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config dir");
    std::fs::write(
        &config_path,
        r#"
        [providers.corporate-claude]
        type = "anthropic-compatible"
        base_url = "https://gateway.example.com/anthropic/v1/"
        auth = "bearer"
        api_key_env = "CORPORATE_CLAUDE_TOKEN"
        default_model = "claude-custom"

        [providers.corporate-claude.headers]
        x-tenant-id = "tenant-42"

        [[providers.corporate-claude.models]]
        id = "claude-custom"
        context_window = 128000
        "#,
    )
    .expect("write config");

    apply_named_provider_profile_env("corporate-claude").expect("apply Anthropic profile");
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_API_BASE").ok().as_deref(),
        Some("https://gateway.example.com/anthropic/v1")
    );
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_API_KEY_NAME")
            .ok()
            .as_deref(),
        Some("CORPORATE_CLAUDE_TOKEN")
    );
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_AUTH").ok().as_deref(),
        Some("bearer")
    );
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_MODEL").ok().as_deref(),
        Some("claude-custom")
    );
    let headers: std::collections::BTreeMap<String, String> = serde_json::from_str(
        &std::env::var("JCODE_ANTHROPIC_HEADERS").expect("custom headers env"),
    )
    .expect("headers JSON");
    assert_eq!(
        headers.get("x-tenant-id").map(String::as_str),
        Some("tenant-42")
    );
    assert_eq!(
        std::env::var("JCODE_RUNTIME_PROVIDER").ok().as_deref(),
        Some("anthropic-api")
    );

    if let Some(previous_home) = previous_home {
        crate::env::set_var("JCODE_HOME", previous_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::Config::invalidate_cache();
}

#[test]
fn named_provider_inline_api_key_is_private_runtime_fallback() {
    let _lock = crate::storage::lock_test_env();
    let _guard = EnvGuard::save(&[
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_ANTHROPIC_API_BASE",
        "JCODE_ANTHROPIC_API_KEY_NAME",
        "JCODE_ANTHROPIC_ENV_FILE",
        "JCODE_ANTHROPIC_AUTH",
        "JCODE_ANTHROPIC_AUTH_HEADER",
        "JCODE_ANTHROPIC_HEADERS",
        "JCODE_ANTHROPIC_MODEL",
        "JCODE_RUNTIME_PROVIDER",
        "JCODE_PROVIDER_MY_GATEWAY_API_KEY",
    ]);

    let cfg: crate::config::Config = toml::from_str(
        r#"
        [providers.my-gateway]
        type = "anthropic-compatible"
        base_url = "https://llm.example.com/v1"
        api_key = "inline-secret"
        "#,
    )
    .expect("config should parse");

    apply_named_provider_profile_env_from_config("my-gateway", &cfg).expect("apply profile");

    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_API_KEY_NAME")
            .ok()
            .as_deref(),
        Some("JCODE_PROVIDER_MY_GATEWAY_API_KEY")
    );
    assert_eq!(
        std::env::var("JCODE_PROVIDER_MY_GATEWAY_API_KEY")
            .ok()
            .as_deref(),
        Some("inline-secret")
    );
}

#[test]
fn matrix_load_api_key_from_env_or_config_prefers_env() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config").join("jcode");
    std::fs::create_dir_all(&config_root).expect("config dir");

    let _guard = EnvGuard::save(&["JCODE_HOME", "OPENCODE_API_KEY"]);
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::env::set_var("OPENCODE_API_KEY", "env-secret");
    std::fs::write(
        config_root.join("opencode.env"),
        "OPENCODE_API_KEY=file-secret\n",
    )
    .expect("env file");

    assert_eq!(
        load_api_key_from_env_or_config("OPENCODE_API_KEY", "opencode.env").as_deref(),
        Some("env-secret")
    );
}

#[test]
fn matrix_load_api_key_from_env_or_config_reads_config_file() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config").join("jcode");
    std::fs::create_dir_all(&config_root).expect("config dir");

    let _guard = EnvGuard::save(&["JCODE_HOME", "OPENCODE_API_KEY"]);
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::env::remove_var("OPENCODE_API_KEY");
    std::fs::write(
        config_root.join("opencode.env"),
        "OPENCODE_API_KEY=file-secret\n",
    )
    .expect("env file");

    assert_eq!(
        load_api_key_from_env_or_config("OPENCODE_API_KEY", "opencode.env").as_deref(),
        Some("file-secret")
    );
}

#[test]
fn open_weight_family_context_limits_match_published_windows() {
    use jcode_provider_core::models::open_weight_family_context_limit as f;

    // GLM family spelling variants across gateways.
    assert_eq!(f("glm-4.5"), Some(128_000));
    assert_eq!(f("glm-4.7"), Some(200_000));
    assert_eq!(f("zai-org/glm-4.7"), Some(200_000));
    assert_eq!(f("accounts/fireworks/models/glm-4p7"), Some(200_000));
    assert_eq!(f("glm-5"), Some(200_000));
    assert_eq!(f("glm-5.1"), Some(200_000));
    assert_eq!(f("zai-glm-5-1"), Some(200_000));
    assert_eq!(f("glm-5.2"), Some(1_000_000));

    // Other open-weight families.
    assert_eq!(f("kimi-k2.5"), Some(262_144));
    assert_eq!(f("minimax-m2.7"), Some(204_800));
    assert_eq!(f("mimo-v2.5"), Some(262_144));
    assert_eq!(f("muse-spark-1.2"), Some(1_048_576));
    assert_eq!(f("deepseek-v3.2"), Some(163_840));
    assert_eq!(f("deepseek-v4-pro"), Some(1_000_000));
    assert_eq!(f("qwen3-235b-a22b-instruct-2507"), Some(262_144));
    assert_eq!(f("gpt-oss-120b"), Some(131_072));
    assert_eq!(f("llama-3.3-70b-instruct"), Some(131_072));
    assert_eq!(f("sonar-pro"), Some(128_000));

    // Unknown families stay unresolved so the dynamic cache/default can act.
    assert_eq!(f("some-unknown-model"), None);
}
