pub use jcode_provider_env::{
    load_api_key_from_env_or_config, load_env_value_from_config_file,
    load_env_value_from_env_or_config, register_api_key_fallback_resolver,
    save_env_value_to_env_file,
};
pub use jcode_provider_metadata::*;

fn inline_key_env_name(profile_name: &str) -> String {
    let suffix = profile_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("JCODE_PROVIDER_{}_API_KEY", suffix)
}

pub fn clear_anthropic_profile_env() {
    for key in [
        "JCODE_ANTHROPIC_API_BASE",
        "JCODE_ANTHROPIC_API_KEY_NAME",
        "JCODE_ANTHROPIC_ENV_FILE",
        "JCODE_ANTHROPIC_AUTH",
        "JCODE_ANTHROPIC_AUTH_HEADER",
        "JCODE_ANTHROPIC_HEADERS",
        "JCODE_ANTHROPIC_MODEL",
    ] {
        crate::env::remove_var(key);
    }
}

pub fn apply_named_provider_profile_env(profile_name: &str) -> anyhow::Result<String> {
    let config = crate::config::Config::load_strict()?;
    apply_named_provider_profile_env_from_config(profile_name, &config)
}

pub fn apply_named_provider_profile_env_from_config(
    profile_name: &str,
    config: &crate::config::Config,
) -> anyhow::Result<String> {
    let Some(profile) = config.providers.get(profile_name) else {
        anyhow::bail!(
            "Unknown provider profile '{}'. Add [providers.{}] to config.toml.",
            profile_name,
            profile_name
        );
    };

    let api_base = normalize_api_base(&profile.base_url).ok_or_else(|| {
        anyhow::anyhow!(
            "Provider profile '{}' has invalid base_url '{}'. Use https://... or http://localhost.",
            profile_name,
            profile.base_url
        )
    })?;

    crate::env::set_var("JCODE_NAMED_PROVIDER_PROFILE", profile_name);
    crate::env::set_var("JCODE_ANTHROPIC_API_BASE", &api_base);
    crate::env::set_var("JCODE_RUNTIME_PROVIDER", "anthropic-api");
    if let Some(model) = profile
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        crate::env::set_var("JCODE_ANTHROPIC_MODEL", model);
    }

    let key_env = profile
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            profile
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|key| {
                    let env_name = inline_key_env_name(profile_name);
                    crate::env::set_var(&env_name, key);
                    crate::logging::warn(&format!(
                        "Provider profile '{}' stores an inline API key in config.toml. Prefer api_key_env to avoid accidental leaks.",
                        profile_name
                    ));
                    env_name
                })
        });
    if let Some(key_env) = key_env {
        if !is_safe_env_key_name(&key_env) {
            anyhow::bail!(
                "Provider profile '{}' has invalid api_key_env '{}'.",
                profile_name,
                key_env
            );
        }
        crate::env::set_var("JCODE_ANTHROPIC_API_KEY_NAME", key_env);
    } else {
        crate::env::remove_var("JCODE_ANTHROPIC_API_KEY_NAME");
    }
    if let Some(env_file) = profile
        .env_file
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !is_safe_env_file_name(env_file) {
            anyhow::bail!(
                "Provider profile '{}' has invalid env_file '{}'.",
                profile_name,
                env_file
            );
        }
        crate::env::set_var("JCODE_ANTHROPIC_ENV_FILE", env_file);
    } else {
        crate::env::remove_var("JCODE_ANTHROPIC_ENV_FILE");
    }

    match profile.auth {
        crate::config::NamedProviderAuth::Bearer => {
            crate::env::set_var("JCODE_ANTHROPIC_AUTH", "bearer");
            crate::env::remove_var("JCODE_ANTHROPIC_AUTH_HEADER");
        }
        crate::config::NamedProviderAuth::Header => {
            crate::env::set_var("JCODE_ANTHROPIC_AUTH", "header");
            crate::env::set_var(
                "JCODE_ANTHROPIC_AUTH_HEADER",
                profile.auth_header.as_deref().unwrap_or("x-api-key"),
            );
        }
        crate::config::NamedProviderAuth::None => {
            crate::env::set_var("JCODE_ANTHROPIC_AUTH", "none");
            crate::env::remove_var("JCODE_ANTHROPIC_AUTH_HEADER");
        }
    }
    if profile.headers.is_empty() {
        crate::env::remove_var("JCODE_ANTHROPIC_HEADERS");
    } else {
        crate::env::set_var(
            "JCODE_ANTHROPIC_HEADERS",
            serde_json::to_string(&profile.headers).map_err(|err| {
                anyhow::anyhow!("failed to serialize Anthropic-compatible headers: {err}")
            })?,
        );
    }
    Ok(profile_name.to_string())
}

#[cfg(test)]
#[path = "provider_catalog_tests.rs"]
mod provider_catalog_tests;
