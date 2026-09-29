use crate::auth::{AuthState, AuthStatus};

use super::pricing::cheapness_for_route;
use super::{
    ALL_OPENAI_MODELS, AccountModelAvailabilityState, ModelRoute, MultiProvider,
    anthropic_api_key_route_availability, anthropic_oauth_route_availability,
    build_anthropic_oauth_route, build_openai_api_key_route, build_openai_oauth_route,
    dedupe_model_routes, format_account_model_availability_detail, is_listable_model_name,
    known_openai_model_ids, model_availability_for_account, provider_for_model,
};

/// Build the fast local route snapshot used by the TUI model picker while the
/// full provider catalog is hydrating.
///
/// This intentionally lives in the provider layer rather than the TUI so auth,
/// provider, and catalog policy have one source of truth. The TUI should only
/// group, sort, and render the returned routes.
pub fn simplified_model_routes_for_picker(
    current_provider_name: &str,
    current_model: &str,
    display_models: impl IntoIterator<Item = String>,
) -> Vec<ModelRoute> {
    let auth = AuthStatus::check_fast();
    let mut routes = Vec::new();

    for model in display_models {
        if provider_for_model(&model) == Some("openai") {
            // Platform-API-only GPT Pro models: never advertise an OAuth route.
            if jcode_provider_core::is_openai_api_only_pro_model(&model) {
                routes.push(ModelRoute {
                    model: model.clone(),
                    provider: "OpenAI".to_string(),
                    api_method: "openai-api-key".to_string(),
                    available: auth.openai_has_api_key,
                    detail: if auth.openai_has_api_key {
                        String::new()
                    } else {
                        "requires OPENAI_API_KEY".to_string()
                    },
                    cheapness: None,
                });
                continue;
            }
            if auth.openai_has_oauth {
                routes.push(ModelRoute {
                    model: model.clone(),
                    provider: "OpenAI".to_string(),
                    api_method: "openai-oauth".to_string(),
                    available: true,
                    detail: String::new(),
                    cheapness: None,
                });
            }
            if auth.openai_has_api_key {
                routes.push(ModelRoute {
                    model: model.clone(),
                    provider: "OpenAI".to_string(),
                    api_method: "openai-api-key".to_string(),
                    available: true,
                    detail: String::new(),
                    cheapness: None,
                });
            }
            if auth.openai == AuthState::NotConfigured {
                routes.push(ModelRoute {
                    model,
                    provider: "OpenAI".to_string(),
                    api_method: "openai-oauth".to_string(),
                    available: false,
                    detail: "no credentials".to_string(),
                    cheapness: None,
                });
            }
            continue;
        }

        let (provider, api_method, available, detail) = match provider_for_model(&model) {
            Some("claude") => {
                append_simplified_anthropic_model_routes(&mut routes, model, &auth);
                continue;
            }
            Some("openai") => unreachable!("OpenAI models are handled above"),
            Some("gemini") => (
                "Gemini".to_string(),
                "code-assist-oauth".to_string(),
                auth.gemini != AuthState::NotConfigured,
                String::new(),
            ),
            Some(other) => (other.to_string(), other.to_string(), true, String::new()),
            None => (
                current_provider_name.to_string(),
                "current".to_string(),
                true,
                String::new(),
            ),
        };

        routes.push(ModelRoute {
            model,
            provider,
            api_method,
            available,
            detail,
            cheapness: None,
        });
    }

    if routes.is_empty() && !current_model.is_empty() && current_model != "unknown" {
        routes.push(ModelRoute {
            model: current_model.to_string(),
            provider: current_provider_name.to_string(),
            api_method: "current".to_string(),
            available: true,
            detail: "simplified catalog".to_string(),
            cheapness: None,
        });
    }

    routes
}

pub fn append_simplified_anthropic_model_routes(
    routes: &mut Vec<ModelRoute>,
    model: impl Into<String>,
    auth: &AuthStatus,
) {
    let model = model.into();
    if auth.anthropic.has_oauth {
        routes.push(ModelRoute {
            model: model.clone(),
            provider: "Anthropic".to_string(),
            api_method: "claude-oauth".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        });
    }
    if auth.anthropic.has_api_key {
        routes.push(ModelRoute {
            model: model.clone(),
            provider: "Anthropic".to_string(),
            api_method: "claude-api".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        });
    }
    if !auth.anthropic.has_oauth && !auth.anthropic.has_api_key {
        routes.push(ModelRoute {
            model,
            provider: "Anthropic".to_string(),
            api_method: "claude-oauth".to_string(),
            available: false,
            detail: "no credentials".to_string(),
            cheapness: None,
        });
    }
}

/// Build the full multi-provider route catalog.
///
/// Orchestration only: each provider family contributes routes through its
/// own `append_*_routes` builder below, so provider-specific policy stays in
/// one place per provider instead of one 400-line function.
pub(super) fn multiprovider_model_routes(provider: &MultiProvider) -> Vec<ModelRoute> {
    let routes_started = std::time::Instant::now();
    provider.spawn_anthropic_catalog_refresh_if_needed();
    provider.spawn_openai_catalog_refresh_if_needed();

    let mut routes = Vec::new();

    let has_oauth = crate::auth::claude::load_credentials().is_ok();
    let has_api_key = crate::provider::anthropic::has_anthropic_api_key();
    let openai_auth = crate::auth::AuthStatus::check_fast();

    append_anthropic_routes(provider, &mut routes, has_oauth, has_api_key);
    append_openai_routes(provider, &mut routes, &openai_auth);
    let added_named_profile_routes = append_named_provider_profile_routes(&mut routes);
    append_gemini_routes(provider, &mut routes);
    append_antigravity_routes(provider, &mut routes);

    let total_ms = routes_started.elapsed().as_millis();
    if total_ms >= 250 || std::env::var("JCODE_LOG_MODEL_PICKER_TIMING").is_ok() {
        crate::logging::info(&format!(
            "[TIMING] model_routes: routes={}, total={}ms",
            routes.len(),
            total_ms,
        ));
    }

    let routes_before_filter = routes.len();

    // Drop obviously non-chat models (embeddings, speech, rerankers, etc.) that
    // some providers (e.g. Gemini) dump wholesale into their catalogs. Without
    // this the picker is flooded with unusable entries.
    routes.retain(|route| is_listable_model_name(&route.model));

    let routes = dedupe_model_routes(routes);

    // Structured, always-on summary of catalog route building. This is the
    // single most useful line for the recurring "model picker empty / only
    // OpenAI+Anthropic appear / configured provider's models missing" reports
    // (issues #292, #268, #312, #304): it records which credentials were
    // detected and how many routes each provider contributed, so a shared log
    // explains exactly why a model was or was not offered. No secrets here.
    log_model_routes_summary(
        "build",
        &routes,
        routes_before_filter,
        has_oauth,
        has_api_key,
        openai_auth.openai_has_oauth,
        openai_auth.openai_has_api_key,
        added_named_profile_routes,
        total_ms,
    );

    routes
}

/// Anthropic models via OAuth and/or API key.
fn append_anthropic_routes(
    provider: &MultiProvider,
    routes: &mut Vec<ModelRoute>,
    has_oauth: bool,
    has_api_key: bool,
) {
    let anthropic_models = if let Some(anthropic) = provider.anthropic_provider() {
        anthropic.available_models_for_switching()
    } else if let Some(claude) = provider.claude_provider() {
        claude.available_models_for_switching()
    } else {
        super::known_anthropic_model_ids()
    };

    for model in anthropic_models {
        let (available, detail) = if has_oauth && !has_api_key {
            anthropic_oauth_route_availability(&model)
        } else {
            (true, String::new())
        };

        if has_oauth {
            routes.push(build_anthropic_oauth_route(
                &model,
                available,
                detail.clone(),
            ));
        }
        if has_api_key {
            let (ak_available, ak_detail) = anthropic_api_key_route_availability(&model);
            routes.push(ModelRoute {
                model: model.to_string(),
                provider: "Anthropic".to_string(),
                api_method: "claude-api".to_string(),
                available: ak_available,
                detail: ak_detail,
                cheapness: cheapness_for_route(&model, "Anthropic", "claude-api"),
            });
        }
        if !has_oauth && !has_api_key {
            routes.push(ModelRoute {
                model: model.to_string(),
                provider: "Anthropic".to_string(),
                api_method: "claude-oauth".to_string(),
                available: false,
                detail: "no credentials".to_string(),
                cheapness: cheapness_for_route(&model, "Anthropic", "claude-oauth"),
            });
        }
    }
}

/// OpenAI models via OAuth and/or API key, with per-account availability.
fn append_openai_routes(
    provider: &MultiProvider,
    routes: &mut Vec<ModelRoute>,
    openai_auth: &crate::auth::AuthStatus,
) {
    let openai_models = if let Some(openai) = provider.openai_provider() {
        openai.available_models_for_switching()
    } else {
        known_openai_model_ids()
    };

    for model in openai_models {
        let availability = model_availability_for_account(&model);
        let (available, detail) = if provider.openai_provider().is_none() {
            (false, "no credentials".to_string())
        } else {
            match availability.state {
                AccountModelAvailabilityState::Available => (true, String::new()),
                AccountModelAvailabilityState::Unavailable => (
                    false,
                    format_account_model_availability_detail(&availability)
                        .unwrap_or_else(|| "not available".to_string()),
                ),
                AccountModelAvailabilityState::Unknown => {
                    let detail = format_account_model_availability_detail(&availability)
                        .unwrap_or_else(|| "availability unknown".to_string());
                    (true, detail)
                }
            }
        };
        // GPT Pro models are platform-API-only: never offer an OAuth route
        // for them (the Codex backend rejects them for ChatGPT accounts).
        if jcode_provider_core::is_openai_api_only_pro_model(&model) {
            if openai_auth.openai_has_api_key {
                routes.push(build_openai_api_key_route(
                    &model,
                    provider.openai_provider().is_some(),
                    String::new(),
                ));
            } else {
                routes.push(build_openai_api_key_route(
                    &model,
                    false,
                    "requires OPENAI_API_KEY",
                ));
            }
            continue;
        }
        if openai_auth.openai_has_oauth {
            routes.push(build_openai_oauth_route(&model, available, detail.clone()));
        }
        if openai_auth.openai_has_api_key {
            routes.push(build_openai_api_key_route(
                &model,
                provider.openai_provider().is_some(),
                String::new(),
            ));
        }
        if !openai_auth.openai_has_oauth && !openai_auth.openai_has_api_key {
            routes.push(build_openai_oauth_route(&model, false, detail));
        }
    }
}

/// User-defined named provider profiles (`[providers.<name>]` in
/// config.toml). Their statically declared `[[providers.<name>.models]]`
/// entries (and `default_model`) must surface in the picker with a route back
/// to that profile, even when the profile is not the active provider
/// (issue #444). Returns whether any routes were added.
fn append_named_provider_profile_routes(routes: &mut Vec<ModelRoute>) -> bool {
    let mut added_any = false;
    for (profile_name, profile_config) in &crate::config::config().providers {
        let named_routes = named_provider_profile_routes(profile_name, profile_config);
        added_any |= !named_routes.is_empty();
        routes.extend(named_routes);
    }
    added_any
}

/// Picker routes for one user-defined named provider profile from config.
///
/// The profile's static models are offered, falling back to its
/// `default_model` when none are declared.
fn named_provider_profile_routes(
    profile_name: &str,
    profile_config: &crate::config::NamedProviderConfig,
) -> Vec<ModelRoute> {
    let mut models: Vec<String> = profile_config
        .models
        .iter()
        .map(|model| model.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if models.is_empty()
        && let Some(default_model) = profile_config
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
    {
        models.push(default_model.to_string());
    }

    let api_method = named_profile_api_method(profile_name);
    let detail = if profile_config.base_url.trim().is_empty() {
        "configured provider profile".to_string()
    } else {
        profile_config.base_url.trim().to_string()
    };

    let mut routes: Vec<ModelRoute> = Vec::new();
    for model in models {
        if !is_listable_model_name(&model) || routes.iter().any(|route| route.model == model) {
            continue;
        }
        routes.push(ModelRoute {
            model,
            provider: profile_name.to_string(),
            api_method: api_method.clone(),
            available: true,
            detail: detail.clone(),
            cheapness: None,
        });
    }
    routes
}

fn append_gemini_routes(provider: &MultiProvider, routes: &mut Vec<ModelRoute>) {
    if let Some(gemini) = provider.gemini_provider() {
        for model in gemini.available_models_display() {
            routes.push(ModelRoute {
                model,
                provider: "Gemini".to_string(),
                api_method: "code-assist-oauth".to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            });
        }
    }
}

fn append_antigravity_routes(provider: &MultiProvider, routes: &mut Vec<ModelRoute>) {
    if let Some(antigravity) = provider.antigravity_provider() {
        routes.extend(antigravity.model_routes());
    }
}

/// Count routes per provider label (lowercased, spaces removed) so the catalog
/// summary log shows where the picker entries came from.
fn provider_route_counts(routes: &[ModelRoute]) -> std::collections::BTreeMap<String, usize> {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for route in routes {
        let key = route.provider.trim().to_ascii_lowercase().replace(' ', "_");
        let key = if key.is_empty() {
            "unknown".to_string()
        } else {
            key
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

/// Emit a structured, non-secret summary of model-route building. Callers pass
/// the credential-detection flags they already computed so the log explains why
/// each provider's routes were or were not produced.
#[allow(clippy::too_many_arguments)]
fn log_model_routes_summary(
    phase: &str,
    routes: &[ModelRoute],
    routes_before_filter: usize,
    anthropic_oauth: bool,
    anthropic_api_key: bool,
    openai_oauth: bool,
    openai_api_key: bool,
    named_provider_profiles: bool,
    total_ms: u128,
) {
    let available = routes.iter().filter(|route| route.available).count();
    let per_provider = provider_route_counts(routes)
        .into_iter()
        .map(|(provider, count)| format!("{provider}:{count}"))
        .collect::<Vec<_>>()
        .join(",");

    crate::logging::event_info(
        "model_routes_summary",
        vec![
            ("phase", phase.to_string()),
            ("routes_total", routes.len().to_string()),
            ("routes_available", available.to_string()),
            ("routes_before_filter", routes_before_filter.to_string()),
            (
                "routes_dropped",
                routes_before_filter
                    .saturating_sub(routes.len())
                    .to_string(),
            ),
            ("anthropic_oauth", anthropic_oauth.to_string()),
            ("anthropic_api", anthropic_api_key.to_string()),
            ("openai_oauth", openai_oauth.to_string()),
            ("openai_api", openai_api_key.to_string()),
            (
                "named_provider_profiles",
                named_provider_profiles.to_string(),
            ),
            ("by_provider", per_provider),
            ("build_ms", total_ms.to_string()),
        ],
    );
}

pub fn remote_model_routes_fallback(remote_available_entries: &[String]) -> Vec<ModelRoute> {
    let auth = AuthStatus::check_fast();
    let mut routes = Vec::new();
    for model in remote_available_entries {
        if !is_listable_model_name(model) {
            continue;
        }

        let mut added_any = false;

        if provider_for_model(model) == Some("claude") {
            if auth.anthropic.has_oauth {
                let (available, detail) = anthropic_oauth_route_availability(model);
                routes.push(build_anthropic_oauth_route(model, available, detail));
                added_any = true;
            }
            // An Anthropic API key is an equally valid direct route. Without
            // this, a model that only reaches the picker via the names-only
            // fallback path (e.g. a newly released model whose detailed route
            // frame was oversized) shows an OAuth route but silently loses its
            // API-key route even though the key works.
            if auth.anthropic.has_api_key {
                let (available, detail) = anthropic_api_key_route_availability(model);
                routes.push(ModelRoute {
                    model: model.clone(),
                    provider: "Anthropic".to_string(),
                    api_method: "claude-api".to_string(),
                    available,
                    detail,
                    cheapness: cheapness_for_route(model, "Anthropic", "claude-api"),
                });
                added_any = true;
            }
        }

        if jcode_provider_core::model_id::matches_known_model(model, ALL_OPENAI_MODELS) {
            let availability = model_availability_for_account(model);
            let (available, detail) = if auth.openai == AuthState::NotConfigured {
                (false, "no credentials".to_string())
            } else {
                match availability.state {
                    AccountModelAvailabilityState::Available => (true, String::new()),
                    AccountModelAvailabilityState::Unavailable => (
                        false,
                        format_account_model_availability_detail(&availability)
                            .unwrap_or_else(|| "not available".to_string()),
                    ),
                    AccountModelAvailabilityState::Unknown => (
                        true,
                        format_account_model_availability_detail(&availability)
                            .unwrap_or_else(|| "availability unknown".to_string()),
                    ),
                }
            };
            routes.push(build_openai_oauth_route(model, available, detail));
            added_any = true;
        }

        if let Some(route) = named_provider_profile_route_for_model(model) {
            routes.push(route);
            added_any = true;
        }

        if super::gemini::is_gemini_model_id(model) {
            routes.push(ModelRoute {
                model: model.clone(),
                provider: "Gemini".to_string(),
                api_method: "code-assist-oauth".to_string(),
                available: auth.gemini == AuthState::Available,
                detail: String::new(),
                cheapness: None,
            });
            added_any = true;
        }

        if !added_any {
            routes.push(ModelRoute {
                model: model.clone(),
                provider: "unknown".to_string(),
                api_method: "unknown".to_string(),
                available: false,
                detail: "no matching configured provider route".to_string(),
                cheapness: None,
            });
        }
    }
    routes
}

pub fn remote_model_routes_lightweight_fallback(
    remote_provider_name: Option<&str>,
    remote_available_entries: &[String],
    current_model: &str,
) -> Vec<ModelRoute> {
    let provider = remote_provider_name
        .map(str::to_string)
        .unwrap_or_else(|| "remote".to_string());
    let mut routes = Vec::new();
    for model in remote_available_entries {
        if !is_listable_model_name(model) {
            continue;
        }
        routes.push(ModelRoute {
            model: model.clone(),
            provider: provider.clone(),
            api_method: "remote-catalog".to_string(),
            available: true,
            detail: "refreshing route details…".to_string(),
            cheapness: None,
        });
    }

    if routes.is_empty() && !current_model.is_empty() && current_model != "unknown" {
        routes.push(ModelRoute {
            model: current_model.to_string(),
            provider,
            api_method: "current".to_string(),
            available: true,
            detail: "refreshing model catalog…".to_string(),
            cheapness: None,
        });
    }

    routes
}

/// Route for `model` when it belongs to a user-defined `[providers.<name>]`
/// profile from config.toml.
///
/// Without this, a bare model id from a custom profile matches no known
/// provider and falls through to the unknown-route placeholder instead of its
/// own profile (issue #694).
fn named_provider_profile_route_for_model(model: &str) -> Option<ModelRoute> {
    named_provider_profile_route_for_model_in(model, &crate::config::config().providers)
}

fn named_provider_profile_route_for_model_in(
    model: &str,
    providers: &std::collections::BTreeMap<String, crate::config::NamedProviderConfig>,
) -> Option<ModelRoute> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    for (profile_name, profile_config) in providers {
        if !named_provider_profile_routes(profile_name, profile_config)
            .iter()
            .any(|route| route.model == model)
        {
            continue;
        }
        let detail = if profile_config.base_url.trim().is_empty() {
            "configured provider profile".to_string()
        } else {
            profile_config.base_url.trim().to_string()
        };
        return Some(ModelRoute {
            model: model.to_string(),
            provider: profile_name.clone(),
            api_method: named_profile_api_method(profile_name),
            available: true,
            detail,
            cheapness: None,
        });
    }
    None
}

fn named_profile_api_method(profile_name: &str) -> String {
    format!(
        "{}{profile_name}",
        jcode_provider_core::NAMED_PROFILE_API_METHOD_PREFIX
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthState, ProviderAuth};

    struct EnvGuard {
        vars: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _temp: tempfile::TempDir,
        _lock: crate::storage::TestEnvGuard,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = crate::storage::lock_test_env();
            let temp = tempfile::tempdir().expect("tempdir");
            let vars = vec![("JCODE_HOME", std::env::var_os("JCODE_HOME"))];
            crate::env::set_var("JCODE_HOME", temp.path());
            Self {
                vars,
                _temp: temp,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.vars.drain(..) {
                if let Some(value) = value {
                    crate::env::set_var(key, value);
                } else {
                    crate::env::remove_var(key);
                }
            }
        }
    }

    /// Issue #694: a bare model id from a user-defined `[providers.<name>]`
    /// profile must resolve to that profile rather than falling through to the
    /// unknown-route placeholder.
    #[test]
    fn named_provider_profile_model_routes_to_its_own_profile() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert(
            "omlx".to_string(),
            crate::config::NamedProviderConfig {
                base_url: "http://127.0.0.1:18000/v1".to_string(),
                default_model: Some("KAT-Coder-V2.5-Dev-OptiQ-4bit".to_string()),
                ..Default::default()
            },
        );

        let route =
            named_provider_profile_route_for_model_in("KAT-Coder-V2.5-Dev-OptiQ-4bit", &providers)
                .expect("custom profile model must resolve to its profile");
        assert_eq!(route.provider, "omlx");
        assert_eq!(route.api_method, "profile:omlx");
        assert_eq!(route.detail, "http://127.0.0.1:18000/v1");
        assert_eq!(
            route.api_method_kind(),
            jcode_provider_core::ModelRouteApiMethod::NamedProfile("omlx".to_string())
        );
    }

    #[test]
    fn named_anthropic_profile_preserves_profile_identity_in_picker_switch() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert(
            "corp-claude".to_string(),
            crate::config::NamedProviderConfig {
                provider_type: crate::config::NamedProviderType::AnthropicCompatible,
                base_url: "https://gateway.example/anthropic/v1".to_string(),
                default_model: Some("claude-custom".to_string()),
                ..Default::default()
            },
        );

        let route = named_provider_profile_route_for_model_in("claude-custom", &providers)
            .expect("Anthropic-compatible model must resolve to its profile");
        assert_eq!(route.provider, "corp-claude");
        assert_eq!(route.api_method, "profile:corp-claude");
        assert_eq!(
            MultiProvider::model_switch_request_for_session_route(
                &route.model,
                Some(&route.provider),
                Some(&route.api_method),
            ),
            "corp-claude:claude-custom"
        );
    }

    #[test]
    fn unknown_model_does_not_match_named_provider_profiles() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert(
            "omlx".to_string(),
            crate::config::NamedProviderConfig {
                base_url: "http://127.0.0.1:18000/v1".to_string(),
                default_model: Some("KAT-Coder-V2.5-Dev-OptiQ-4bit".to_string()),
                ..Default::default()
            },
        );

        assert!(
            named_provider_profile_route_for_model_in("some-other-model", &providers).is_none()
        );
        assert!(named_provider_profile_route_for_model_in("", &providers).is_none());
    }

    #[test]
    fn simplified_anthropic_routes_preserve_oauth_vs_api_key_state_space() {
        for (has_oauth, has_api_key, expected_methods) in [
            (true, false, vec!["claude-oauth"]),
            (false, true, vec!["claude-api"]),
            (true, true, vec!["claude-oauth", "claude-api"]),
            (false, false, vec!["claude-oauth"]),
        ] {
            let auth = AuthStatus {
                anthropic: ProviderAuth {
                    state: if has_oauth || has_api_key {
                        AuthState::Available
                    } else {
                        AuthState::NotConfigured
                    },
                    has_oauth,
                    oauth_state: if has_oauth {
                        AuthState::Available
                    } else {
                        AuthState::NotConfigured
                    },
                    has_api_key,
                },
                ..AuthStatus::default()
            };
            let mut routes = Vec::new();

            append_simplified_anthropic_model_routes(
                &mut routes,
                "claude-opus-4-6".to_string(),
                &auth,
            );

            let methods = routes
                .iter()
                .map(|route| route.api_method.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                methods, expected_methods,
                "oauth={has_oauth} api={has_api_key}"
            );
            assert!(routes.iter().all(|route| route.provider == "Anthropic"));
            assert_eq!(
                routes.iter().all(|route| route.available),
                has_oauth || has_api_key
            );
        }
    }

    /// Issue #694 through the real path a user hits: a custom
    /// `[providers.<name>]` profile in config.toml. The picker must route the
    /// model to that profile.
    #[test]
    fn custom_config_profile_model_is_routed_to_its_profile() {
        let _guard = EnvGuard::new();
        let jcode_home = std::env::var_os("JCODE_HOME").expect("JCODE_HOME set");
        std::fs::write(
            std::path::PathBuf::from(jcode_home).join("config.toml"),
            "[providers.omlx]\ntype = \"anthropic-compatible\"\nbase_url = \"http://127.0.0.1:18000/v1\"\ndefault_model = \"KAT-Coder-V2.5-Dev-OptiQ-4bit\"\n",
        )
        .expect("write config.toml");
        crate::config::invalidate_config_cache();

        let model = "KAT-Coder-V2.5-Dev-OptiQ-4bit";
        let route = named_provider_profile_route_for_model(model)
            .expect("custom config profile model must be routed to its profile");
        assert_eq!(route.provider, "omlx");
        assert_eq!(route.api_method, "profile:omlx");

        // The full fallback builder (what the picker renders) agrees.
        let routes = remote_model_routes_fallback(&[model.to_string()]);
        assert!(
            routes
                .iter()
                .any(|route| route.api_method == "profile:omlx"),
            "picker routes must include the profile route: {routes:?}"
        );
    }
}
