use crate::{ModelRoute, normalize_dotted_model_version};
use std::borrow::Cow;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActiveProvider {
    Claude,
    OpenAI,
    Antigravity,
    Gemini,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ProviderAvailability {
    pub openai: bool,
    pub claude: bool,
    pub antigravity: bool,
    pub gemini: bool,
}

impl ProviderAvailability {
    pub fn is_configured(self, provider: ActiveProvider) -> bool {
        match provider {
            ActiveProvider::Claude => self.claude,
            ActiveProvider::OpenAI => self.openai,
            ActiveProvider::Antigravity => self.antigravity,
            ActiveProvider::Gemini => self.gemini,
        }
    }
}

pub fn auto_default_provider(availability: ProviderAvailability) -> ActiveProvider {
    if availability.claude {
        ActiveProvider::Claude
    } else if availability.openai {
        ActiveProvider::OpenAI
    } else if availability.antigravity {
        ActiveProvider::Antigravity
    } else if availability.gemini {
        ActiveProvider::Gemini
    } else {
        ActiveProvider::Claude
    }
}

pub fn parse_provider_hint(value: &str) -> Option<ActiveProvider> {
    match value.trim().to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => Some(ActiveProvider::Claude),
        "openai" => Some(ActiveProvider::OpenAI),
        "antigravity" => Some(ActiveProvider::Antigravity),
        "gemini" => Some(ActiveProvider::Gemini),
        _ => None,
    }
}

pub fn provider_label(provider: ActiveProvider) -> &'static str {
    match provider {
        ActiveProvider::Claude => "Anthropic",
        ActiveProvider::OpenAI => "OpenAI",
        ActiveProvider::Antigravity => "Antigravity",
        ActiveProvider::Gemini => "Gemini",
    }
}

pub fn provider_key(provider: ActiveProvider) -> &'static str {
    match provider {
        ActiveProvider::Claude => "claude",
        ActiveProvider::OpenAI => "openai",
        ActiveProvider::Antigravity => "antigravity",
        ActiveProvider::Gemini => "gemini",
    }
}

pub fn provider_from_model_key(key: &str) -> Option<ActiveProvider> {
    match key {
        "claude" => Some(ActiveProvider::Claude),
        "openai" => Some(ActiveProvider::OpenAI),
        "antigravity" => Some(ActiveProvider::Antigravity),
        "gemini" => Some(ActiveProvider::Gemini),
        _ => None,
    }
}

/// Translate a persisted session/runtime provider key (the `RuntimeKey`
/// stable-id or `ModelRouteApiMethod` vocabulary, e.g. `anthropic-api-key`,
/// `claude-oauth`, `openai-api-key`) into the CLI `--provider` argument value
/// (the `ProviderChoice` vocabulary, e.g. `anthropic-api`, `claude`,
/// `openai-api`).
///
/// These two vocabularies overlap but are NOT identical: the runtime key
/// distinguishes auth method (`anthropic-api-key` vs `claude-oauth`) while the
/// CLI `--provider` enum uses `anthropic-api` / `claude`. Passing a raw runtime
/// key straight to `--provider` makes clap reject it (`invalid value
/// 'anthropic-api-key'`) and the spawned process exits immediately.
///
/// Returns `None` when there is no clean, unambiguous CLI provider to pass; in
/// that case callers should omit the flag entirely and rely on the persisted
/// session (model + provider_key + route_api_method) to reconstruct the exact
/// route on resume.
pub fn cli_provider_arg_for_session_key(key: &str) -> Option<&'static str> {
    let normalized = key.trim().to_ascii_lowercase();
    let base = normalized
        .split_once(':')
        .map(|(prefix, _rest)| prefix)
        .unwrap_or(normalized.as_str());
    // Dual-auth (Anthropic/OpenAI OAuth-vs-API) keys share one canonical alias
    // table, so the CLI arg never drifts from the route/runtime vocabularies.
    if let Some(route) = crate::auth_mode::AuthRoute::parse(base) {
        return Some(route.cli_provider_arg());
    }
    match base {
        "gemini" => Some("gemini"),
        "antigravity" => Some("antigravity"),
        "code-assist-oauth" | "google" => Some("google"),
        // remote-catalog, current, and any unknown key have no clean
        // standalone CLI provider value, so omit the flag and let the
        // persisted session route.
        _ => None,
    }
}

pub fn explicit_model_provider_prefix(model: &str) -> Option<(ActiveProvider, &'static str, &str)> {
    if let Some(rest) = model.strip_prefix("claude-api:") {
        Some((ActiveProvider::Claude, "claude-api:", rest))
    } else if let Some(rest) = model.strip_prefix("claude-oauth:") {
        Some((ActiveProvider::Claude, "claude-oauth:", rest))
    } else if let Some(rest) = model.strip_prefix("claude:") {
        Some((ActiveProvider::Claude, "claude:", rest))
    } else if let Some(rest) = model.strip_prefix("anthropic:") {
        Some((ActiveProvider::Claude, "anthropic:", rest))
    } else if let Some(rest) = model.strip_prefix("openai-api:") {
        Some((ActiveProvider::OpenAI, "openai-api:", rest))
    } else if let Some(rest) = model.strip_prefix("openai-oauth:") {
        Some((ActiveProvider::OpenAI, "openai-oauth:", rest))
    } else if let Some(rest) = model.strip_prefix("openai:") {
        Some((ActiveProvider::OpenAI, "openai:", rest))
    } else if let Some(rest) = model.strip_prefix("antigravity:") {
        Some((ActiveProvider::Antigravity, "antigravity:", rest))
    } else if let Some(rest) = model.strip_prefix("gemini:") {
        Some((ActiveProvider::Gemini, "gemini:", rest))
    } else {
        None
    }
}

pub fn model_name_for_provider(provider: ActiveProvider, model: &str) -> Cow<'_, str> {
    if matches!(provider, ActiveProvider::Claude)
        && let Some(canonical) = normalize_dotted_model_version(model)
    {
        return Cow::Borrowed(canonical);
    }
    Cow::Borrowed(model)
}

/// Strip a provider runtime's own routing prefix from a model id.
///
/// Session restore and the model picker speak in routing specs such as
/// `antigravity:gemini-3-flash`. `MultiProvider` peels the prefix off before
/// delegating, but `--provider <name>` hands the concrete runtime back to the
/// agent directly, so the prefixed spec reaches `Provider::set_model`
/// untouched. Storing it verbatim makes every later turn ask the backend for a
/// model whose id contains our prefix, which the backend has never heard of
/// (Antigravity answers HTTP 404 "Requested entity was not found." on turn 2
/// of any resumed session).
///
/// A runtime only ever puts a bare model id on the wire, so each runtime calls
/// this with its own prefix when accepting a model id. Only the runtime's own
/// prefix is stripped: a foreign prefix is a real routing error and must stay
/// visible rather than being silently reinterpreted as a local model.
pub fn strip_own_model_prefix<'a>(model: &'a str, own_prefix: &str) -> &'a str {
    let model = model.trim();
    match model.strip_prefix(own_prefix) {
        Some(rest) if !own_prefix.is_empty() => rest.trim(),
        _ => model,
    }
}

pub fn dedupe_model_routes(routes: Vec<ModelRoute>) -> Vec<ModelRoute> {
    use std::collections::HashSet;

    let mut seen: HashSet<(String, String, String)> = HashSet::with_capacity(routes.len());
    let mut deduped: Vec<ModelRoute> = Vec::with_capacity(routes.len());
    for route in routes {
        if seen.insert((
            route.provider.clone(),
            route.model.clone(),
            route.api_method.clone(),
        )) {
            deduped.push(route);
        }
    }
    deduped
}

pub fn fallback_sequence(active: ActiveProvider) -> Vec<ActiveProvider> {
    match active {
        ActiveProvider::Claude => vec![
            ActiveProvider::Claude,
            ActiveProvider::OpenAI,
            ActiveProvider::Gemini,
        ],
        ActiveProvider::OpenAI => vec![
            ActiveProvider::OpenAI,
            ActiveProvider::Claude,
            ActiveProvider::Gemini,
        ],
        ActiveProvider::Antigravity => vec![
            ActiveProvider::Antigravity,
            ActiveProvider::Claude,
            ActiveProvider::OpenAI,
            ActiveProvider::Gemini,
        ],
        ActiveProvider::Gemini => vec![
            ActiveProvider::Gemini,
            ActiveProvider::Claude,
            ActiveProvider::OpenAI,
            ActiveProvider::Antigravity,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_hints() {
        assert_eq!(
            parse_provider_hint("Anthropic"),
            Some(ActiveProvider::Claude)
        );
        assert_eq!(parse_provider_hint("openai"), Some(ActiveProvider::OpenAI));
        assert_eq!(parse_provider_hint("unknown"), None);
    }

    #[test]
    fn cli_provider_arg_translates_runtime_keys() {
        // Anthropic API key (the regression: this is NOT a valid --provider
        // value verbatim; it must map to `anthropic-api`).
        assert_eq!(
            cli_provider_arg_for_session_key("anthropic-api-key"),
            Some("anthropic-api")
        );
        assert_eq!(
            cli_provider_arg_for_session_key("claude-api"),
            Some("anthropic-api")
        );
        // Anthropic OAuth -> claude.
        assert_eq!(
            cli_provider_arg_for_session_key("claude-oauth"),
            Some("claude")
        );
        assert_eq!(cli_provider_arg_for_session_key("claude"), Some("claude"));
        // OpenAI variants.
        assert_eq!(
            cli_provider_arg_for_session_key("openai-oauth"),
            Some("openai")
        );
        assert_eq!(
            cli_provider_arg_for_session_key("openai-api-key"),
            Some("openai-api")
        );
        // Passthrough providers.
        assert_eq!(cli_provider_arg_for_session_key("gemini"), Some("gemini"));
        // Case-insensitive and whitespace tolerant.
        assert_eq!(
            cli_provider_arg_for_session_key("  Anthropic-API-Key "),
            Some("anthropic-api")
        );
        assert_eq!(cli_provider_arg_for_session_key("remote-catalog"), None);
        assert_eq!(cli_provider_arg_for_session_key("current"), None);
        assert_eq!(cli_provider_arg_for_session_key("totally-unknown"), None);
    }

    #[test]
    fn parses_model_provider_prefixes() {
        assert_eq!(
            provider_from_model_key("gemini"),
            Some(ActiveProvider::Gemini)
        );
        assert_eq!(provider_from_model_key("missing"), None);

        for (raw, expected_provider, expected_prefix, expected_model) in [
            (
                "claude-api:sonnet",
                ActiveProvider::Claude,
                "claude-api:",
                "sonnet",
            ),
            (
                "claude-oauth:sonnet",
                ActiveProvider::Claude,
                "claude-oauth:",
                "sonnet",
            ),
            ("claude:sonnet", ActiveProvider::Claude, "claude:", "sonnet"),
            (
                "anthropic:sonnet",
                ActiveProvider::Claude,
                "anthropic:",
                "sonnet",
            ),
            ("openai:gpt-5", ActiveProvider::OpenAI, "openai:", "gpt-5"),
            (
                "openai-oauth:gpt-5",
                ActiveProvider::OpenAI,
                "openai-oauth:",
                "gpt-5",
            ),
            (
                "openai-api:gpt-5",
                ActiveProvider::OpenAI,
                "openai-api:",
                "gpt-5",
            ),
            (
                "antigravity:default",
                ActiveProvider::Antigravity,
                "antigravity:",
                "default",
            ),
            (
                "gemini:gemini-2.5-pro",
                ActiveProvider::Gemini,
                "gemini:",
                "gemini-2.5-pro",
            ),
        ] {
            let (provider, prefix, model) = explicit_model_provider_prefix(raw).unwrap();
            assert_eq!(provider, expected_provider, "{raw}");
            assert_eq!(prefix, expected_prefix, "{raw}");
            assert_eq!(model, expected_model, "{raw}");
        }
        assert_eq!(explicit_model_provider_prefix("unknown:sonnet"), None);
    }

    #[test]
    fn dedupes_model_routes_by_route_identity() {
        let routes = vec![
            ModelRoute {
                model: "m".to_string(),
                provider: "p".to_string(),
                api_method: "a".to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            },
            ModelRoute {
                model: "m".to_string(),
                provider: "p".to_string(),
                api_method: "a".to_string(),
                available: false,
                detail: "duplicate".to_string(),
                cheapness: None,
            },
            ModelRoute {
                model: "m".to_string(),
                provider: "p".to_string(),
                api_method: "b".to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            },
        ];

        let deduped = dedupe_model_routes(routes);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].detail, "");
    }

    #[test]
    fn auto_default_prefers_claude_when_both_frontier_providers_are_available() {
        let provider = auto_default_provider(ProviderAvailability {
            openai: true,
            claude: true,
            ..ProviderAvailability::default()
        });
        assert_eq!(provider, ActiveProvider::Claude);
    }

    #[test]
    fn fallback_sequence_keeps_active_first() {
        let sequence = fallback_sequence(ActiveProvider::Gemini);
        assert_eq!(sequence.first(), Some(&ActiveProvider::Gemini));
        assert!(sequence.contains(&ActiveProvider::Claude));
    }

    /// Regression: `--provider antigravity` (and the other direct runtimes)
    /// hand `Provider::set_model` a routing spec like
    /// `antigravity:gemini-3-flash` on session restore. Storing that verbatim
    /// made every resumed turn request a model whose id carried our prefix, and
    /// the backend answered HTTP 404 "Requested entity was not found." So turn
    /// 1 of a session worked and turn 2 always failed, for every model.
    #[test]
    fn strip_own_model_prefix_covers_the_routing_spec_state_space() {
        // (case, input, own prefix) -> stored model id
        let cases: [(&str, &str, &str, &str); 8] = [
            (
                "own prefix is stripped",
                "antigravity:gemini-3-flash",
                "antigravity:",
                "gemini-3-flash",
            ),
            (
                "bare id is unchanged",
                "gemini-3-flash",
                "antigravity:",
                "gemini-3-flash",
            ),
            (
                "surrounding whitespace is trimmed",
                "  antigravity:gemini-3-flash  ",
                "antigravity:",
                "gemini-3-flash",
            ),
            (
                "whitespace after the prefix is trimmed",
                "antigravity: gemini-3-flash",
                "antigravity:",
                "gemini-3-flash",
            ),
            (
                "only one level of our own prefix is peeled",
                "antigravity:antigravity:gemini-3-flash",
                "antigravity:",
                "antigravity:gemini-3-flash",
            ),
            (
                "a foreign prefix is a routing error and stays visible",
                "gemini:gemini-3-flash",
                "antigravity:",
                "gemini:gemini-3-flash",
            ),
            (
                "model ids containing a colon are otherwise untouched",
                "some:model",
                "antigravity:",
                "some:model",
            ),
            (
                "gemini runtime",
                "gemini:gemini-2.5-pro",
                "gemini:",
                "gemini-2.5-pro",
            ),
        ];

        for (case, input, own_prefix, expected) in cases {
            assert_eq!(
                strip_own_model_prefix(input, own_prefix),
                expected,
                "{case}"
            );
        }
    }

    /// A prefix-only spec must not silently become a valid empty model id; the
    /// runtimes rely on the emptiness check to reject it.
    #[test]
    fn strip_own_model_prefix_leaves_prefix_only_input_empty() {
        assert_eq!(strip_own_model_prefix("antigravity:", "antigravity:"), "");
        assert_eq!(
            strip_own_model_prefix("antigravity:   ", "antigravity:"),
            ""
        );
        assert_eq!(strip_own_model_prefix("   ", "antigravity:"), "");
    }
}
