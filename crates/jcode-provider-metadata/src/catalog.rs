use super::{
    LoginProviderAuthKind, LoginProviderAuthStateKey, LoginProviderDescriptor,
    LoginProviderSurfaceOrder, LoginProviderTarget,
};

pub const CLAUDE_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "claude",
    display_name: "Anthropic/Claude",
    auth_kind: LoginProviderAuthKind::OAuth,
    auth_state_key: LoginProviderAuthStateKey::Anthropic,
    auth_status_method: "OAuth",
    aliases: &["anthropic"],
    menu_detail: "requires Claude Pro or Max subscription",
    recommended: true,
    target: LoginProviderTarget::Claude,
    order: LoginProviderSurfaceOrder::new(Some(1), Some(1), Some(1), Some(1), Some(1)),
};

pub const ANTHROPIC_API_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "anthropic-api",
    display_name: "Anthropic API",
    auth_kind: LoginProviderAuthKind::ApiKey,
    auth_state_key: LoginProviderAuthStateKey::Anthropic,
    auth_status_method: "API key",
    aliases: &["claude-api", "anthropic-key", "claude-key"],
    menu_detail: "direct Anthropic Messages API",
    recommended: false,
    target: LoginProviderTarget::ClaudeApiKey,
    order: LoginProviderSurfaceOrder::new(Some(2), Some(2), Some(2), Some(2), Some(2)),
};

pub const AUTO_IMPORT_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "auto-import",
    display_name: "Auto Import",
    auth_kind: LoginProviderAuthKind::Local,
    auth_state_key: LoginProviderAuthStateKey::ExternalImport,
    auth_status_method: "Reuse detected logins",
    aliases: &["import", "reuse", "autoimport"],
    menu_detail: "review and reuse logins from other tools",
    recommended: false,
    target: LoginProviderTarget::AutoImport,
    order: LoginProviderSurfaceOrder::new(Some(1), Some(1), None, None, None),
};

pub const OPENAI_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "openai",
    display_name: "OpenAI",
    auth_kind: LoginProviderAuthKind::OAuth,
    auth_state_key: LoginProviderAuthStateKey::OpenAi,
    auth_status_method: "OAuth",
    aliases: &[],
    menu_detail: "requires ChatGPT Plus or Pro subscription",
    recommended: true,
    target: LoginProviderTarget::OpenAi,
    order: LoginProviderSurfaceOrder::new(Some(2), Some(2), Some(2), Some(2), Some(2)),
};

pub const OPENAI_API_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "openai-api",
    display_name: "OpenAI API",
    auth_kind: LoginProviderAuthKind::ApiKey,
    auth_state_key: LoginProviderAuthStateKey::OpenAi,
    auth_status_method: "API key",
    aliases: &[
        "openai-key",
        "openai-apikey",
        "openai-platform",
        "platform-openai",
    ],
    menu_detail: "native OpenAI API key, pay-per-token",
    recommended: false,
    target: LoginProviderTarget::OpenAiApiKey,
    order: LoginProviderSurfaceOrder::new(Some(99), Some(99), Some(99), Some(99), Some(99)),
};

pub const GEMINI_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "gemini",
    display_name: "Google Gemini",
    auth_kind: LoginProviderAuthKind::OAuth,
    auth_state_key: LoginProviderAuthStateKey::Gemini,
    auth_status_method: "OAuth",
    aliases: &[],
    menu_detail: "Google Gemini Code Assist OAuth login",
    recommended: false,
    target: LoginProviderTarget::Gemini,
    order: LoginProviderSurfaceOrder::new(Some(13), Some(11), Some(4), Some(11), Some(13)),
};

pub const GEMINI_API_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "gemini-api",
    display_name: "Gemini API",
    auth_kind: LoginProviderAuthKind::ApiKey,
    auth_state_key: LoginProviderAuthStateKey::Gemini,
    auth_status_method: "API key",
    aliases: &[
        "gemini-key",
        "gemini-apikey",
        "google-ai-studio",
        "ai-studio",
    ],
    menu_detail: "Google AI Studio Developer API key",
    recommended: false,
    target: LoginProviderTarget::GeminiApiKey,
    order: LoginProviderSurfaceOrder::new(Some(38), Some(38), Some(38), Some(38), Some(38)),
};

pub const ANTIGRAVITY_LOGIN_PROVIDER: LoginProviderDescriptor = LoginProviderDescriptor {
    id: "antigravity",
    display_name: "Antigravity",
    auth_kind: LoginProviderAuthKind::OAuth,
    auth_state_key: LoginProviderAuthStateKey::Antigravity,
    auth_status_method: "OAuth",
    aliases: &[],
    menu_detail: "Google Antigravity OAuth login",
    recommended: false,
    target: LoginProviderTarget::Antigravity,
    order: LoginProviderSurfaceOrder::new(Some(12), Some(12), None, Some(12), Some(12)),
};

pub(crate) const LOGIN_PROVIDERS: [LoginProviderDescriptor; 8] = [
    AUTO_IMPORT_LOGIN_PROVIDER,
    CLAUDE_LOGIN_PROVIDER,
    ANTHROPIC_API_LOGIN_PROVIDER,
    OPENAI_LOGIN_PROVIDER,
    OPENAI_API_LOGIN_PROVIDER,
    GEMINI_LOGIN_PROVIDER,
    GEMINI_API_LOGIN_PROVIDER,
    ANTIGRAVITY_LOGIN_PROVIDER,
];
