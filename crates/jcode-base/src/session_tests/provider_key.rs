use super::*;

/// Set all three runtime env vars this derivation reads, so a test never
/// depends on what an earlier activation left behind in the process.
fn runtime_env(
    runtime: Option<&'static str>,
    namespace: Option<&'static str>,
    active: Option<&'static str>,
) -> (EnvVarGuard, EnvVarGuard, EnvVarGuard) {
    let guard = |key: &'static str, value: Option<&'static str>| match value {
        Some(value) => EnvVarGuard::set(key, value),
        None => EnvVarGuard::remove(key),
    };
    (
        guard("JCODE_RUNTIME_PROVIDER", runtime),
        guard("JCODE_OPENROUTER_CACHE_NAMESPACE", namespace),
        guard("JCODE_ACTIVE_PROVIDER", active),
    )
}

/// The defect: an autodetected `gemini-api` OpenAI-compatible profile writes a
/// process-global cache namespace, and every later session - Claude ones
/// included - was stamped with it. That pair is unusable: resume and swarm
/// spawn both rebuild a route from it and reach Google's endpoint for an
/// Anthropic model.
#[test]
fn derive_session_provider_key_ignores_a_namespace_from_another_provider() {
    let _lock = lock_env();
    let _env = runtime_env(None, Some("gemini-api"), None);

    assert_eq!(
        derive_session_provider_key("Claude").as_deref(),
        Some("claude")
    );
}

#[test]
fn derive_session_provider_key_ignores_an_active_provider_from_another_provider() {
    let _lock = lock_env();
    let _env = runtime_env(None, None, Some("openai"));

    assert_eq!(
        derive_session_provider_key("Claude").as_deref(),
        Some("claude")
    );
}

/// The distinction that must survive: `claude` vs `claude-api` is the same
/// provider on a different credential, and only the env carries it. Dropping it
/// would collapse an API-key session onto the OAuth route on resume.
#[test]
fn derive_session_provider_key_keeps_the_anthropic_api_key_route() {
    let _lock = lock_env();
    let _env = runtime_env(Some("claude-api"), None, None);

    assert_eq!(
        derive_session_provider_key("Claude").as_deref(),
        Some("claude-api")
    );
}

/// An unclassifiable runtime value is no evidence against the env, so custom
/// and Azure-style runtimes keep naming their own sessions.
#[test]
fn derive_session_provider_key_keeps_an_unclassifiable_runtime_value() {
    let _lock = lock_env();
    let _env = runtime_env(Some("azure-openai"), None, None);

    assert_eq!(
        derive_session_provider_key("OpenAI").as_deref(),
        Some("azure-openai")
    );
}

/// An unclassifiable *provider name* is equally no evidence, so providers
/// outside the first-party table still take their key from the runtime.
#[test]
fn derive_session_provider_key_honours_the_env_for_an_unknown_provider_name() {
    let _lock = lock_env();
    let _env = runtime_env(Some("gemini-api"), None, None);

    assert_eq!(
        derive_session_provider_key("snapshot-provider").as_deref(),
        Some("gemini-api")
    );
}
