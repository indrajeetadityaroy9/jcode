pub(crate) fn anthropic_oauth_route_availability(model: &str) -> (bool, String) {
    if model.ends_with("[1m]") && !crate::usage::has_extra_usage() {
        (false, "requires extra usage".to_string())
    } else if model.contains("opus") && !crate::auth::claude::is_max_subscription() {
        (false, "requires Max subscription".to_string())
    } else {
        (true, String::new())
    }
}

pub(crate) fn anthropic_api_key_route_availability(model: &str) -> (bool, String) {
    if model.ends_with("[1m]") && !crate::usage::has_extra_usage() {
        (false, "requires extra usage".to_string())
    } else if !anthropic_api_key_configured() {
        // Without a key this route cannot authenticate, so reporting it as
        // available sends both the model picker and `pick_next_fallback_route`
        // at an endpoint that always fails. The OAuth sibling above already
        // gates on credential state the same way.
        (false, "no Anthropic API key".to_string())
    } else {
        (true, String::new())
    }
}

/// Credential-resolution order used by the Anthropic provider itself: the key
/// may come from the process env *or* the persisted `anthropic.env`, so an
/// env-only check would mark file-keyed setups unavailable.
fn anthropic_api_key_configured() -> bool {
    crate::provider_catalog::load_api_key_from_env_or_config("ANTHROPIC_API_KEY", "anthropic.env")
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_route_is_unavailable_without_a_key() {
        let _lock = crate::storage::lock_test_env();
        let home = tempfile::tempdir().expect("temp home");
        let prev_home = std::env::var_os("JCODE_HOME");
        let prev_key = std::env::var_os("ANTHROPIC_API_KEY");
        crate::env::set_var("JCODE_HOME", home.path());
        crate::env::remove_var("ANTHROPIC_API_KEY");

        let (available, reason) = anthropic_api_key_route_availability("claude-fable-5");
        assert!(!available);
        assert_eq!(reason, "no Anthropic API key");

        crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test");
        let (available, reason) = anthropic_api_key_route_availability("claude-fable-5");
        assert!(available, "a configured key must keep the route usable");
        assert!(reason.is_empty());

        match prev_key {
            Some(value) => crate::env::set_var("ANTHROPIC_API_KEY", value),
            None => crate::env::remove_var("ANTHROPIC_API_KEY"),
        }
        match prev_home {
            Some(value) => crate::env::set_var("JCODE_HOME", value),
            None => crate::env::remove_var("JCODE_HOME"),
        }
    }
}
