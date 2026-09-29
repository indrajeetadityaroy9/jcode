use anyhow::Result;
use std::io::{self, Write};
use std::sync::Arc;

use crate::auth;
use crate::provider;
use crate::provider::Provider;
use crate::provider_catalog::{
    LoginProviderDescriptor, LoginProviderTarget, is_safe_env_file_name, is_safe_env_key_name,
    resolve_login_selection,
};
use crate::tool;

use super::login::run_login_provider;
use super::output;

pub(crate) use crate::external_auth::maybe_run_external_auth_auto_import_flow;
use crate::external_auth::{
    can_prompt_for_external_auth, external_auth_blocked_message, prompt_to_trust_external_auth,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ProviderChoice {
    Claude,
    #[value(alias = "claude-api", alias = "anthropic-key", alias = "claude-key")]
    AnthropicApi,
    #[deprecated(
        note = "Claude Code CLI subprocess transport is deprecated; use ProviderChoice::Claude for native Anthropic OAuth/API transport"
    )]
    #[value(alias = "claude-subprocess", hide = true)]
    ClaudeSubprocess,
    Openai,
    #[value(
        alias = "openai-key",
        alias = "openai-apikey",
        alias = "openai-platform"
    )]
    OpenaiApi,
    Gemini,
    #[value(
        alias = "gemini-key",
        alias = "gemini-apikey",
        alias = "google-ai-studio",
        alias = "ai-studio"
    )]
    GeminiApi,
    Antigravity,
    Auto,
}

impl ProviderChoice {
    #[allow(deprecated)]
    pub fn as_arg_value(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::AnthropicApi => "anthropic-api",
            Self::ClaudeSubprocess => "claude-subprocess",
            Self::Openai => "openai",
            Self::OpenaiApi => "openai-api",
            Self::Gemini => "gemini",
            Self::GeminiApi => "gemini-api",
            Self::Antigravity => "antigravity",
            Self::Auto => "auto",
        }
    }
}

#[allow(deprecated)]
const PROVIDER_CHOICE_LOGIN_PROVIDERS: &[(ProviderChoice, LoginProviderDescriptor)] = &[
    (
        ProviderChoice::Claude,
        crate::provider_catalog::CLAUDE_LOGIN_PROVIDER,
    ),
    (
        ProviderChoice::AnthropicApi,
        crate::provider_catalog::ANTHROPIC_API_LOGIN_PROVIDER,
    ),
    (
        ProviderChoice::ClaudeSubprocess,
        crate::provider_catalog::CLAUDE_LOGIN_PROVIDER,
    ),
    (
        ProviderChoice::Openai,
        crate::provider_catalog::OPENAI_LOGIN_PROVIDER,
    ),
    (
        ProviderChoice::OpenaiApi,
        crate::provider_catalog::OPENAI_API_LOGIN_PROVIDER,
    ),
    (
        ProviderChoice::Gemini,
        crate::provider_catalog::GEMINI_LOGIN_PROVIDER,
    ),
    (
        ProviderChoice::GeminiApi,
        crate::provider_catalog::GEMINI_API_LOGIN_PROVIDER,
    ),
    (
        ProviderChoice::Antigravity,
        crate::provider_catalog::ANTIGRAVITY_LOGIN_PROVIDER,
    ),
];

pub fn login_provider_choice_mappings() -> &'static [(ProviderChoice, LoginProviderDescriptor)] {
    PROVIDER_CHOICE_LOGIN_PROVIDERS
}

#[allow(deprecated)]
pub fn login_provider_for_choice(choice: &ProviderChoice) -> Option<LoginProviderDescriptor> {
    PROVIDER_CHOICE_LOGIN_PROVIDERS
        .iter()
        .find(|(candidate, _)| candidate == choice)
        .map(|(_, provider)| *provider)
}

#[allow(deprecated)]
pub fn choice_for_login_provider(provider: LoginProviderDescriptor) -> Option<ProviderChoice> {
    PROVIDER_CHOICE_LOGIN_PROVIDERS
        .iter()
        .find(|(choice, candidate)| {
            candidate.id == provider.id && !matches!(choice, ProviderChoice::ClaudeSubprocess)
        })
        .map(|(choice, _)| *choice)
}

pub fn prompt_login_provider_selection(
    providers: &[LoginProviderDescriptor],
    heading: &str,
) -> Result<LoginProviderDescriptor> {
    prompt_login_provider_selection_optional(providers, heading)?.ok_or_else(|| {
        anyhow::anyhow!("Login skipped. Run `jcode login` when you're ready to authenticate.")
    })
}

pub fn prompt_login_provider_selection_optional(
    providers: &[LoginProviderDescriptor],
    heading: &str,
) -> Result<Option<LoginProviderDescriptor>> {
    let status = auth::AuthStatus::check_fast();
    eprint!(
        "{}",
        render_login_provider_selection_menu(heading, providers, &status)
    );
    eprint!(
        "\nEnter 1-{}, provider name, or Enter=skip: ",
        providers.len()
    );
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    parse_login_provider_selection_input(&input, providers)
}

pub fn parse_login_provider_selection_input(
    input: &str,
    providers: &[LoginProviderDescriptor],
) -> Result<Option<LoginProviderDescriptor>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let normalized = trimmed.to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "s" | "skip" | "q" | "quit" | "cancel" | "none"
    ) {
        return Ok(None);
    }

    resolve_login_selection(trimmed, providers)
        .map(Some)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid choice '{}'. Enter 1-{}, a provider name, or 'skip'.",
                trimmed,
                providers.len()
            )
        })
}

pub fn render_login_provider_selection_menu(
    heading: &str,
    providers: &[LoginProviderDescriptor],
    status: &auth::AuthStatus,
) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "{heading}");
    let _ = writeln!(out);

    let detected = providers
        .iter()
        .copied()
        .filter_map(|provider| {
            let assessment = status.assessment_for_provider(provider);
            (assessment.state != auth::AuthState::NotConfigured).then(|| {
                format!(
                    "  - {}: {}",
                    provider.display_name,
                    login_provider_detection_detail(provider, &assessment)
                )
            })
        })
        .collect::<Vec<_>>();

    if detected.is_empty() {
        let _ = writeln!(out, "Autodetected auth: none found yet.");
    } else {
        let _ = writeln!(out, "Autodetected auth:");
        for line in detected {
            let _ = writeln!(out, "{line}");
        }
    }

    let _ = writeln!(out);
    for (index, provider) in providers.iter().copied().enumerate() {
        let assessment = status.assessment_for_provider(provider);
        let _ = writeln!(
            out,
            "  {}. {:<22} [{:<15}] - {}",
            index + 1,
            provider.display_name,
            login_provider_state_badge(provider, assessment.state),
            provider.menu_detail
        );
    }

    let recommended = providers
        .iter()
        .filter(|provider| provider.recommended)
        .map(|provider| provider.display_name)
        .collect::<Vec<_>>();
    if !recommended.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "  Recommended if you have a subscription: {}.",
            recommended.join(", ")
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "  Skip: press Enter, or type `skip`.");
    out
}

fn login_provider_state_badge(
    provider: LoginProviderDescriptor,
    state: auth::AuthState,
) -> &'static str {
    match state {
        auth::AuthState::Available => {
            if matches!(provider.target, LoginProviderTarget::AutoImport) {
                "detected"
            } else {
                "configured"
            }
        }
        auth::AuthState::Expired => "needs attention",
        auth::AuthState::NotConfigured => "not configured",
    }
}

fn login_provider_detection_detail(
    provider: LoginProviderDescriptor,
    assessment: &auth::ProviderAuthAssessment,
) -> String {
    match assessment.state {
        auth::AuthState::Available => {
            let prefix = if matches!(provider.target, LoginProviderTarget::AutoImport) {
                "detected"
            } else {
                "configured"
            };
            format!("{}: {}", prefix, assessment.method_detail)
        }
        auth::AuthState::Expired => format!("needs attention: {}", assessment.method_detail),
        auth::AuthState::NotConfigured => "not configured".to_string(),
    }
}

struct AutoProviderAvailability {
    has_claude: bool,
    has_openai: bool,
    has_antigravity: bool,
    has_gemini: bool,
}

impl AutoProviderAvailability {
    fn has_any_provider(&self) -> bool {
        self.has_claude || self.has_openai || self.has_antigravity || self.has_gemini
    }
}

async fn detect_auto_provider_flags() -> AutoProviderAvailability {
    // An exec-based daemon reload inherits this one-shot, non-secret snapshot
    // from its predecessor. Consuming it avoids repeating credential discovery
    // on the reload critical path while ensuring later processes cannot reuse it.
    let auth_status = std::env::var("JCODE_RELOAD_AUTH_STATUS")
        .ok()
        .and_then(|snapshot| {
            crate::env::remove_var("JCODE_RELOAD_AUTH_STATUS");
            serde_json::from_str::<auth::AuthStatus>(&snapshot).ok()
        })
        .unwrap_or_else(auth::AuthStatus::check_fast);
    AutoProviderAvailability {
        has_claude: auth_status.anthropic.has_oauth || auth_status.anthropic.has_api_key,
        has_openai: auth_status.openai_has_oauth || auth_status.openai_has_api_key,
        has_antigravity: auth::antigravity::load_tokens().is_ok(),
        has_gemini: auth_status.gemini == auth::AuthState::Available,
    }
}

fn ensure_external_api_key_auth_allowed_for_explicit_choice(env_key: &str) -> Result<()> {
    if direct_api_key_configured_for_env(env_key) {
        return Ok(());
    }
    let Some(source) = auth::external::preferred_unconsented_api_key_source_for_env(env_key) else {
        return Ok(());
    };
    let path = source.path()?;
    let provider_name = env_key;
    let login_hint = "jcode login";
    if !can_prompt_for_external_auth() {
        anyhow::bail!(external_auth_blocked_message(
            provider_name,
            source.display_name(),
            &path,
            login_hint,
        ));
    }
    if prompt_to_trust_external_auth(provider_name, source.display_name(), &path)? {
        auth::external::trust_external_auth_source(source)?;
        return Ok(());
    }
    anyhow::bail!(
        "Skipped trusting external {} credentials. Run `{}` to authenticate jcode directly.",
        provider_name,
        login_hint
    )
}

fn direct_api_key_configured_for_env(env_key: &str) -> bool {
    let env_key = env_key.trim();
    !env_key.is_empty()
        && std::env::var(env_key)
            .ok()
            .map(|key| !key.trim().is_empty())
            .unwrap_or(false)
}

fn maybe_prompt_for_generic_oauth_source(
    provider_name: &str,
    source: Option<auth::external::ExternalAuthSource>,
    login_hint: &str,
    auto: bool,
    validation: impl Fn() -> bool,
) -> Result<bool> {
    let Some(source) = source else {
        return Ok(false);
    };
    let path = source.path()?;
    if !can_prompt_for_external_auth() {
        if auto {
            crate::logging::warn(&external_auth_blocked_message(
                provider_name,
                source.display_name(),
                &path,
                login_hint,
            ));
            return Ok(false);
        }
        anyhow::bail!(external_auth_blocked_message(
            provider_name,
            source.display_name(),
            &path,
            login_hint,
        ));
    }
    if prompt_to_trust_external_auth(provider_name, source.display_name(), &path)? {
        auth::external::trust_external_auth_source(source)?;
        return Ok(if auto { validation() } else { true });
    }
    Ok(false)
}

fn ensure_openai_auth_allowed_for_explicit_choice() -> Result<()> {
    if auth::codex::load_credentials().is_ok() {
        return Ok(());
    }

    if maybe_prompt_for_generic_oauth_source(
        "OpenAI/Codex",
        auth::external::preferred_unconsented_openai_oauth_source(),
        "jcode login --provider openai",
        false,
        || auth::codex::load_credentials().is_ok(),
    )? {
        return Ok(());
    }

    if !auth::codex::has_unconsented_legacy_credentials() {
        return Ok(());
    }

    let path = auth::codex::legacy_auth_file_path()?;

    if !can_prompt_for_external_auth() {
        anyhow::bail!(external_auth_blocked_message(
            "OpenAI/Codex",
            "Codex",
            &path,
            "jcode login --provider openai"
        ));
    }

    if prompt_to_trust_external_auth("OpenAI/Codex", "Codex", &path)? {
        auth::codex::trust_legacy_auth_for_future_use()?;
        return Ok(());
    }

    anyhow::bail!(
        "Skipped trusting existing ~/.codex/auth.json credentials. Run `jcode login --provider openai` to authenticate jcode directly."
    )
}

fn maybe_enable_legacy_codex_auth_for_auto(has_other_provider: bool) -> Result<bool> {
    if auth::codex::load_credentials().is_ok() {
        return Ok(true);
    }

    if let Some(source) = auth::external::preferred_unconsented_openai_oauth_source() {
        if has_other_provider {
            return Ok(false);
        }
        return maybe_prompt_for_generic_oauth_source(
            "OpenAI/Codex",
            Some(source),
            "jcode login --provider openai",
            true,
            || auth::codex::load_credentials().is_ok(),
        );
    }

    if !auth::codex::has_unconsented_legacy_credentials() {
        return Ok(false);
    }

    if has_other_provider {
        return Ok(false);
    }

    let path = auth::codex::legacy_auth_file_path()?;

    if !can_prompt_for_external_auth() {
        crate::logging::warn(&external_auth_blocked_message(
            "OpenAI/Codex",
            "Codex",
            &path,
            "jcode login --provider openai",
        ));
        return Ok(false);
    }

    if prompt_to_trust_external_auth("OpenAI/Codex", "Codex", &path)? {
        auth::codex::trust_legacy_auth_for_future_use()?;
        return Ok(auth::codex::load_credentials().is_ok());
    }

    Ok(false)
}

fn ensure_claude_auth_allowed_for_explicit_choice() -> Result<()> {
    if auth::claude::load_credentials().is_ok() {
        return Ok(());
    }

    if maybe_prompt_for_generic_oauth_source(
        "Claude",
        auth::external::preferred_unconsented_anthropic_oauth_source(),
        "jcode login --provider claude",
        false,
        || auth::claude::load_credentials().is_ok(),
    )? {
        return Ok(());
    }

    let Some(source) = auth::claude::has_unconsented_external_auth() else {
        return Ok(());
    };
    let path = source.path()?;
    if !can_prompt_for_external_auth() {
        anyhow::bail!(external_auth_blocked_message(
            "Claude",
            source.display_name(),
            &path,
            "jcode login --provider claude"
        ));
    }
    if prompt_to_trust_external_auth("Claude", source.display_name(), &path)? {
        auth::claude::trust_external_auth_source(source)?;
        return Ok(());
    }
    anyhow::bail!(
        "Skipped trusting external Claude credentials. Run `jcode login --provider claude` to authenticate jcode directly."
    )
}

fn maybe_enable_claude_auth_for_auto(has_other_provider: bool) -> Result<bool> {
    if auth::claude::load_credentials().is_ok() {
        return Ok(true);
    }

    if let Some(source) = auth::external::preferred_unconsented_anthropic_oauth_source() {
        if has_other_provider {
            return Ok(false);
        }
        return maybe_prompt_for_generic_oauth_source(
            "Claude",
            Some(source),
            "jcode login --provider claude",
            true,
            || auth::claude::load_credentials().is_ok(),
        );
    }

    let Some(source) = auth::claude::has_unconsented_external_auth() else {
        return Ok(false);
    };
    if has_other_provider {
        return Ok(false);
    }
    let path = source.path()?;
    if !can_prompt_for_external_auth() {
        crate::logging::warn(&external_auth_blocked_message(
            "Claude",
            source.display_name(),
            &path,
            "jcode login --provider claude",
        ));
        return Ok(false);
    }
    if prompt_to_trust_external_auth("Claude", source.display_name(), &path)? {
        auth::claude::trust_external_auth_source(source)?;
        return Ok(auth::claude::load_credentials().is_ok());
    }
    Ok(false)
}

fn ensure_gemini_auth_allowed_for_explicit_choice() -> Result<()> {
    // An official Gemini Developer API key (GEMINI_API_KEY) authenticates
    // directly against generativelanguage.googleapis.com and needs no OAuth
    // consent flow, so allow it without further prompting.
    if auth::gemini::has_api_key() {
        return Ok(());
    }
    if auth::gemini::load_tokens().is_ok() {
        return Ok(());
    }

    if maybe_prompt_for_generic_oauth_source(
        "Gemini",
        auth::external::preferred_unconsented_gemini_oauth_source(),
        "jcode login --provider gemini",
        false,
        || auth::gemini::load_tokens().is_ok(),
    )? {
        return Ok(());
    }

    if !auth::gemini::has_unconsented_cli_auth() {
        return Ok(());
    }
    let path = auth::gemini::gemini_cli_oauth_path()?;
    if !can_prompt_for_external_auth() {
        anyhow::bail!(external_auth_blocked_message(
            "Gemini",
            "Gemini CLI",
            &path,
            "jcode login --provider gemini"
        ));
    }
    if prompt_to_trust_external_auth("Gemini", "Gemini CLI", &path)? {
        auth::gemini::trust_cli_auth_for_future_use()?;
        return Ok(());
    }
    anyhow::bail!(
        "Skipped trusting Gemini CLI credentials. Run `jcode login --provider gemini` to authenticate jcode directly."
    )
}

fn maybe_enable_gemini_auth_for_auto(has_other_provider: bool) -> Result<bool> {
    // A configured Gemini Developer API key is sufficient on its own.
    if auth::gemini::has_api_key() {
        return Ok(true);
    }
    if auth::gemini::load_tokens().is_ok() {
        return Ok(true);
    }

    if let Some(source) = auth::external::preferred_unconsented_gemini_oauth_source() {
        if has_other_provider {
            return Ok(false);
        }
        return maybe_prompt_for_generic_oauth_source(
            "Gemini",
            Some(source),
            "jcode login --provider gemini",
            true,
            || auth::gemini::load_tokens().is_ok(),
        );
    }

    if !auth::gemini::has_unconsented_cli_auth() {
        return Ok(false);
    }
    if has_other_provider {
        return Ok(false);
    }
    let path = auth::gemini::gemini_cli_oauth_path()?;
    if !can_prompt_for_external_auth() {
        crate::logging::warn(&external_auth_blocked_message(
            "Gemini",
            "Gemini CLI",
            &path,
            "jcode login --provider gemini",
        ));
        return Ok(false);
    }
    if prompt_to_trust_external_auth("Gemini", "Gemini CLI", &path)? {
        auth::gemini::trust_cli_auth_for_future_use()?;
        return Ok(auth::gemini::load_tokens().is_ok());
    }
    Ok(false)
}

fn ensure_antigravity_auth_allowed_for_explicit_choice() -> Result<()> {
    if auth::antigravity::load_tokens().is_ok() {
        return Ok(());
    }

    if maybe_prompt_for_generic_oauth_source(
        "Antigravity",
        auth::external::preferred_unconsented_antigravity_oauth_source(),
        "jcode login --provider antigravity",
        false,
        || auth::antigravity::load_tokens().is_ok(),
    )? {
        return Ok(());
    }

    Ok(())
}

pub fn select_initial_model_provider(provider_key: &str) {
    crate::provider::activation::select_initial_runtime_provider_key(provider_key);
}

pub fn clear_initial_model_provider() {
    crate::provider::activation::clear_initial_runtime_provider();
}

/// A CLI provider choice for a dual-auth backend is also a credential choice.
/// Pin it through the provider's credential-mode API
/// so `--provider anthropic-api` cannot remain in Auto mode and prefer a stored
/// Claude OAuth credential over `ANTHROPIC_API_KEY` (and likewise for OpenAI).
fn explicit_credential_mode(choice: &ProviderChoice) -> Option<provider::CredentialMode> {
    match choice {
        ProviderChoice::AnthropicApi | ProviderChoice::OpenaiApi => {
            Some(provider::CredentialMode::ApiKey)
        }
        _ => None,
    }
}

pub async fn login_and_bootstrap_provider(
    provider: LoginProviderDescriptor,
    account_label: Option<&str>,
) -> Result<Arc<dyn provider::Provider>> {
    run_login_provider(
        provider,
        account_label,
        crate::cli::login::LoginOptions::default(),
    )
    .await?;
    eprintln!();

    let runtime: Arc<dyn provider::Provider> = match provider.target {
        LoginProviderTarget::AutoImport
        | LoginProviderTarget::Claude
        | LoginProviderTarget::ClaudeApiKey => Arc::new(provider::MultiProvider::new()),
        LoginProviderTarget::OpenAi => Arc::new(provider::MultiProvider::with_preference(true)),
        LoginProviderTarget::OpenAiApiKey => {
            select_initial_model_provider("openai");
            Arc::new(provider::MultiProvider::with_preference(true))
        }
        LoginProviderTarget::Gemini | LoginProviderTarget::GeminiApiKey => {
            clear_initial_model_provider();
            crate::env::set_var("JCODE_ACTIVE_PROVIDER", "gemini");
            Arc::new(jcode_provider_gemini_runtime::GeminiProvider::new())
        }
        LoginProviderTarget::Antigravity => {
            clear_initial_model_provider();
            crate::env::set_var("JCODE_ACTIVE_PROVIDER", "antigravity");
            Arc::new(jcode_provider_antigravity_runtime::AntigravityProvider::new())
        }
    };

    Ok(runtime)
}

pub fn save_named_api_key(env_file: &str, key_name: &str, key: &str) -> Result<()> {
    if !is_safe_env_key_name(key_name) {
        anyhow::bail!("Invalid API key variable name: {}", key_name);
    }
    if !is_safe_env_file_name(env_file) {
        anyhow::bail!("Invalid env file name: {}", env_file);
    }

    let config_dir = crate::storage::app_config_dir()?;
    let file_path = config_dir.join(env_file);
    crate::storage::upsert_env_file_value(&file_path, key_name, Some(key))?;

    crate::env::set_var(key_name, key);
    Ok(())
}

pub async fn init_provider(
    choice: &ProviderChoice,
    model: Option<&str>,
) -> Result<Arc<dyn provider::Provider>> {
    init_provider_with_options(choice, model, true, true).await
}

pub async fn init_provider_quiet(
    choice: &ProviderChoice,
    model: Option<&str>,
) -> Result<Arc<dyn provider::Provider>> {
    init_provider_with_options(choice, model, false, true).await
}

pub async fn init_provider_for_validation(
    choice: &ProviderChoice,
    model: Option<&str>,
) -> Result<Arc<dyn provider::Provider>> {
    init_provider_with_options(choice, model, false, false).await
}

#[allow(deprecated)]
async fn init_provider_with_options(
    choice: &ProviderChoice,
    model: Option<&str>,
    show_init_messages: bool,
    allow_login_bootstrap: bool,
) -> Result<Arc<dyn provider::Provider>> {
    // Provider construction resolves concrete runtimes through the base
    // crate's external-runtime registry (composition-root pattern). The
    // binary's normal path registers them in `startup::run()`, but this
    // function is also entered directly by validation/login/test flows that
    // never run startup. Registration is idempotent, so do it here too;
    // otherwise Auto-init silently loses registry-backed runtimes and their
    // model-picker routes.
    super::startup::register_external_provider_runtimes();

    if let Ok(profile_name) = std::env::var("JCODE_PROVIDER_PROFILE_NAME")
        && !profile_name.trim().is_empty()
    {
        crate::provider_catalog::apply_named_provider_profile_env(profile_name.trim())?;
        crate::env::set_var("JCODE_PROVIDER_PROFILE_ACTIVE", "1");
    }

    let init_notice = |message: &str| {
        if show_init_messages {
            output::stderr_info(message);
        }
    };

    let provider: Arc<dyn provider::Provider> = match choice {
        ProviderChoice::Claude => {
            ensure_claude_auth_allowed_for_explicit_choice()?;
            init_notice("Using Claude as the initial provider (use /model to switch)");
            select_initial_model_provider("claude");
            Arc::new(provider::MultiProvider::with_preference_fast(false))
        }
        ProviderChoice::AnthropicApi => {
            ensure_external_api_key_auth_allowed_for_explicit_choice("ANTHROPIC_API_KEY")?;
            init_notice("Using Anthropic API key as the initial provider (use /model to switch)");
            select_initial_model_provider("claude");
            Arc::new(provider::MultiProvider::with_preference_fast(false))
        }
        ProviderChoice::ClaudeSubprocess => {
            ensure_claude_auth_allowed_for_explicit_choice()?;
            crate::logging::warn(
                "Using --provider claude-subprocess is deprecated and will be removed. Prefer `--provider claude`.",
            );
            crate::env::set_var("JCODE_USE_CLAUDE_CLI", "1");
            init_notice(
                "Using deprecated Claude subprocess transport as the initial provider (legacy compatibility mode)",
            );
            select_initial_model_provider("claude");
            Arc::new(provider::MultiProvider::with_preference_fast(false))
        }
        ProviderChoice::Openai => {
            ensure_openai_auth_allowed_for_explicit_choice()?;
            init_notice("Using OpenAI as the initial provider (use /model to switch)");
            select_initial_model_provider("openai");
            Arc::new(provider::MultiProvider::with_preference_fast(true))
        }
        ProviderChoice::OpenaiApi => {
            ensure_external_api_key_auth_allowed_for_explicit_choice("OPENAI_API_KEY")?;
            init_notice("Using OpenAI API key as the initial provider (use /model to switch)");
            select_initial_model_provider("openai");
            Arc::new(provider::MultiProvider::with_preference_fast(true))
        }
        ProviderChoice::Gemini | ProviderChoice::GeminiApi => {
            ensure_gemini_auth_allowed_for_explicit_choice()?;
            if auth::gemini::has_api_key() {
                init_notice(
                    "Using Gemini provider (official Gemini Developer API key, generativelanguage.googleapis.com)",
                );
            } else {
                init_notice("Using Gemini provider (native Google Code Assist OAuth)");
            }
            clear_initial_model_provider();
            crate::env::set_var("JCODE_ACTIVE_PROVIDER", "gemini");
            Arc::new(jcode_provider_gemini_runtime::GeminiProvider::new())
        }
        ProviderChoice::Antigravity => {
            ensure_antigravity_auth_allowed_for_explicit_choice()?;
            init_notice("Using Antigravity provider (experimental)");
            clear_initial_model_provider();
            crate::env::set_var("JCODE_ACTIVE_PROVIDER", "antigravity");
            Arc::new(jcode_provider_antigravity_runtime::AntigravityProvider::new())
        }
        ProviderChoice::Auto => {
            clear_initial_model_provider();
            let auto_detect_start = std::time::Instant::now();
            let mut availability = detect_auto_provider_flags().await;

            let reviewed_external_auth = if !availability.has_any_provider() {
                maybe_run_external_auth_auto_import_flow().await?.is_some()
            } else {
                false
            };

            if reviewed_external_auth {
                availability = detect_auto_provider_flags().await;
            }

            let auto_detect_ms = auto_detect_start.elapsed().as_millis();

            if !availability.has_any_provider() {
                let supplemental_start = std::time::Instant::now();
                let mut has_claude = availability.has_claude;
                let mut has_openai = availability.has_openai;
                let has_antigravity = availability.has_antigravity;
                let mut has_gemini = availability.has_gemini;
                let mut has_other_provider = has_claude || has_antigravity || has_gemini;

                if !has_openai {
                    has_openai = maybe_enable_legacy_codex_auth_for_auto(has_other_provider)?;
                }
                has_other_provider = has_openai || has_claude || has_antigravity || has_gemini;

                if !has_claude {
                    has_claude =
                        maybe_enable_claude_auth_for_auto(has_other_provider && !has_claude)?;
                }
                has_other_provider = has_openai || has_claude || has_antigravity || has_gemini;

                if !has_gemini {
                    has_gemini =
                        maybe_enable_gemini_auth_for_auto(has_other_provider && !has_gemini)?;
                }

                availability = AutoProviderAvailability {
                    has_claude,
                    has_openai,
                    has_antigravity,
                    has_gemini,
                };
                crate::logging::info(&format!(
                    "[TIMING] auto_provider_bootstrap: detect={}ms, external_import={}, supplemental={}ms, final_has_any={}",
                    auto_detect_ms,
                    reviewed_external_auth,
                    supplemental_start.elapsed().as_millis(),
                    availability.has_any_provider()
                ));
            } else {
                crate::logging::info(&format!(
                    "[TIMING] auto_provider_bootstrap: detect={}ms, external_import={}, supplemental=skipped, final_has_any=true",
                    auto_detect_ms, reviewed_external_auth
                ));
            }

            if availability.has_any_provider() {
                let multi = provider::MultiProvider::new();
                init_notice(&format!(
                    "Using {} (use /model to switch models)",
                    multi.name()
                ));
                crate::env::set_var("JCODE_ACTIVE_PROVIDER", multi.name().to_lowercase());
                Arc::new(multi)
            } else {
                let non_interactive = std::env::var("JCODE_NON_INTERACTIVE").is_ok();
                // Deferred-auth bootstrap: the interactive TUI server is spawned
                // headless (JCODE_NON_INTERACTIVE) but the user logs in *inside*
                // the TUI on a fresh install. Rather than bail, boot an empty
                // MultiProvider with no configured credentials yet. The TUI's
                // `/login` flow then activates a provider via the normal
                // auth-changed path (MultiProvider::on_auth_changed hot-inits the
                // newly logged-in provider). Only the actual TUI server opts in
                // via JCODE_DEFERRED_AUTH_BOOTSTRAP, so `jcode run` and other
                // genuinely headless callers still fail loudly.
                if std::env::var_os("JCODE_DEFERRED_AUTH_BOOTSTRAP").is_some() {
                    crate::logging::info(
                        "No credentials configured; booting deferred-auth MultiProvider for in-TUI login",
                    );
                    let multi = provider::MultiProvider::new();
                    crate::env::set_var("JCODE_ACTIVE_PROVIDER", multi.name().to_lowercase());
                    Arc::new(multi)
                } else if non_interactive {
                    anyhow::bail!(
                        "No credentials configured. Run 'jcode login' or set ANTHROPIC_API_KEY to authenticate."
                    );
                } else if !allow_login_bootstrap {
                    anyhow::bail!(
                        "No credentials configured for provider auto-detection; automatic login/bootstrap is disabled during validation."
                    );
                } else {
                    let provider_desc = prompt_login_provider_selection(
                        &crate::provider_catalog::auto_init_login_providers(),
                        "No credentials found. Let's log in!\n\nChoose a provider:",
                    )?;
                    Box::pin(login_and_bootstrap_provider(provider_desc, None)).await?
                }
            }
        }
    };

    if let Some(mode) = explicit_credential_mode(choice) {
        provider.set_credential_mode(mode).map_err(|err| {
            anyhow::anyhow!(
                "Failed to select the credential route for --provider {}: {err}",
                choice.as_arg_value()
            )
        })?;
    }

    if let Some(model_name) = model {
        if let Err(e) = provider.set_model(model_name) {
            init_notice(&format!(
                "Warning: failed to set model '{}': {}",
                model_name, e
            ));
        } else {
            init_notice(&format!("Using model: {}", model_name));
        }
    }

    Ok(provider)
}

pub async fn init_provider_and_registry(
    choice: &ProviderChoice,
    model: Option<&str>,
) -> Result<(Arc<dyn provider::Provider>, tool::Registry)> {
    let provider = init_provider(choice, model).await?;
    let registry = tool::Registry::new(provider.clone()).await;
    Ok((provider, registry))
}

pub async fn init_provider_and_registry_for_validation(
    choice: &ProviderChoice,
    model: Option<&str>,
) -> Result<(Arc<dyn provider::Provider>, tool::Registry)> {
    let provider = init_provider_for_validation(choice, model).await?;
    let registry = tool::Registry::new(provider.clone()).await;
    Ok((provider, registry))
}

#[cfg(test)]
#[path = "provider_init_tests.rs"]
mod tests;
