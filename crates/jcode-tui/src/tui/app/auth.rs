#[path = "auth_account_commands.rs"]
mod auth_account_commands;
#[path = "auth_account_picker.rs"]
mod auth_account_picker;
#[path = "auth_types.rs"]
mod auth_types;
pub(crate) use self::auth_account_commands::{
    account_command_from_picker, execute_account_command_local, execute_account_command_remote,
    handle_account_command_remote, handle_auth_command, resolve_account_provider_descriptor,
};
pub(super) use self::auth_types::{AccountCommand, PendingAccountInput, PendingLogin};

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};
use std::sync::Arc;

impl App {
    fn open_auth_browser(url: &str) -> bool {
        // Honors --no-browser/NO_BROWSER/JCODE_NO_BROWSER and never opens real
        // browser windows from test binaries (login flows are exercised by TUI
        // tests; without this guard a test run pops OAuth pages on the
        // developer's desktop).
        super::helpers::open_path_or_url_detached(url).is_ok()
    }

    fn record_oauth_preflight(
        provider_id: &str,
        browser_opened: bool,
        callback_target: Option<&str>,
        callback_available: Option<bool>,
    ) -> String {
        let mut notices = Vec::new();
        if !browser_opened {
            notices.push("This machine could not open a browser automatically.".to_string());
        }
        if matches!(callback_available, Some(false)) {
            if let Some(target) = callback_target {
                notices.push(format!(
                    "Local callback target {} is unavailable, so jcode is using manual-safe paste completion instead.",
                    target
                ));
            } else {
                notices.push(
                    "The local callback listener is unavailable, so jcode is using manual-safe paste completion instead."
                        .to_string(),
                );
            }
        }
        if !notices.is_empty() {
            notices.push(format!(
                "If login still fails, run jcode auth doctor {} for a guided diagnosis.",
                provider_id
            ));
        }
        notices.join("\n")
    }

    pub(super) fn show_auth_status(&mut self) {
        let status = crate::auth::AuthStatus::check();
        let validation = crate::auth::validation::load_all();
        let icon = |state: crate::auth::AuthState| match state {
            crate::auth::AuthState::Available => "ok",
            crate::auth::AuthState::Expired => "needs attention",
            crate::auth::AuthState::NotConfigured => "not configured",
        };
        let providers = crate::provider_catalog::auth_status_login_providers();
        let mut rows: Vec<[String; 5]> = vec![[
            "Provider".to_string(),
            "Status".to_string(),
            "Method".to_string(),
            "Health".to_string(),
            "Validation".to_string(),
        ]];
        for provider in providers {
            let assessment = status.assessment_for_provider(provider);
            rows.push([
                provider.display_name.to_string(),
                icon(assessment.state).to_string(),
                assessment.method_detail.to_string(),
                assessment.health_summary(),
                validation
                    .get(provider.id)
                    .map(crate::auth::validation::format_record_label)
                    .unwrap_or_else(|| "not validated".to_string()),
            ]);
        }
        let mut widths = [0usize; 5];
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let mut message = String::from("Authentication Status:\n\n");
        for row in &rows {
            let line = row
                .iter()
                .enumerate()
                .map(|(i, cell)| format!("{:width$}", cell, width = widths[i]))
                .collect::<Vec<_>>()
                .join("  ");
            message.push_str(line.trim_end());
            message.push('\n');
        }
        message.push_str(
            "\nUse /login <provider> to authenticate. /account opens the provider/account management center, /account <provider> settings shows provider-specific controls, and /auth doctor or /account <provider> doctor shows recovery steps.",
        );
        self.push_display_message(DisplayMessage::system(message));
    }

    pub(super) fn show_interactive_login(&mut self) {
        self.open_login_picker_inline();
        self.set_status_notice("Login: choose a provider");
    }

    pub(super) fn show_interactive_logout(&mut self) {
        self.open_logout_picker_inline();
        self.set_status_notice("Logout: choose a provider");
    }

    pub(super) fn start_logout_provider(
        &mut self,
        provider: crate::provider_catalog::LoginProviderDescriptor,
    ) {
        use crate::provider_catalog::LoginProviderTarget;

        let result: anyhow::Result<String> = (|| match provider.target {
            LoginProviderTarget::Claude => {
                let removed = crate::auth::claude::clear_accounts()?;
                Ok(format!("Logged out of {} Anthropic account(s).", removed))
            }
            LoginProviderTarget::ClaudeApiKey => {
                Self::clear_api_key_login("ANTHROPIC_API_KEY", "anthropic.env")?;
                Ok("Logged out of Anthropic API key.".to_string())
            }
            LoginProviderTarget::OpenAi => {
                let removed = crate::auth::codex::clear_accounts()?;
                Ok(format!("Logged out of {} OpenAI account(s).", removed))
            }
            LoginProviderTarget::OpenAiApiKey => {
                Self::clear_api_key_login("OPENAI_API_KEY", "openai.env")?;
                Ok("Logged out of OpenAI API key.".to_string())
            }
            LoginProviderTarget::Gemini => {
                crate::auth::gemini::clear_tokens()?;
                Ok("Logged out of Gemini.".to_string())
            }
            LoginProviderTarget::GeminiApiKey => {
                Self::clear_api_key_login(
                    crate::auth::gemini::GEMINI_API_KEY_ENV_VARS[0],
                    crate::auth::gemini::GEMINI_API_KEY_ENV_FILE,
                )?;
                Ok("Logged out of Gemini API key.".to_string())
            }
            _ => Ok(format!(
                "Logout for {} is not automated yet. Remove its saved API key or external CLI session from /account {} settings.",
                provider.display_name, provider.id
            )),
        })();

        match result {
            Ok(message) => {
                crate::auth::AuthStatus::invalidate_cache();
                self.push_display_message(DisplayMessage::system(message));
                self.set_status_notice(format!("Logout: {}", provider.display_name));
            }
            Err(err) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Failed to log out of {}: {}",
                    provider.display_name, err
                )));
                self.set_status_notice("Logout failed");
            }
        }
    }

    pub(super) fn start_logout_all(&mut self) {
        let mut summary: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        match crate::auth::claude::clear_accounts() {
            Ok(removed) if removed > 0 => summary.push(format!("{} Anthropic account(s)", removed)),
            Ok(_) => {}
            Err(err) => errors.push(format!("Anthropic: {}", err)),
        }
        match crate::auth::codex::clear_accounts() {
            Ok(removed) if removed > 0 => summary.push(format!("{} OpenAI account(s)", removed)),
            Ok(_) => {}
            Err(err) => errors.push(format!("OpenAI: {}", err)),
        }

        Self::clear_api_key_logout_summary(
            &mut summary,
            &mut errors,
            "Anthropic API key",
            "ANTHROPIC_API_KEY",
            "anthropic.env",
        );
        Self::clear_api_key_logout_summary(
            &mut summary,
            &mut errors,
            "OpenAI API key",
            "OPENAI_API_KEY",
            "openai.env",
        );
        Self::clear_api_key_logout_summary(
            &mut summary,
            &mut errors,
            "Gemini API key",
            crate::auth::gemini::GEMINI_API_KEY_ENV_VARS[0],
            crate::auth::gemini::GEMINI_API_KEY_ENV_FILE,
        );
        match crate::auth::gemini::clear_tokens() {
            Ok(()) => summary.push("Gemini".to_string()),
            Err(err) => errors.push(format!("Gemini: {}", err)),
        }

        crate::auth::AuthStatus::invalidate_cache();

        let message = if summary.is_empty() {
            "No automated logins to clear.".to_string()
        } else {
            format!("Logged out of: {}.", summary.join(", "))
        };
        self.push_display_message(DisplayMessage::system(message));

        if errors.is_empty() {
            self.set_status_notice("Logout: all providers");
        } else {
            self.push_display_message(DisplayMessage::error(format!(
                "Some logouts failed: {}",
                errors.join("; ")
            )));
            self.set_status_notice("Logout: completed with errors");
        }
    }

    fn clear_api_key_login(env_key: &str, env_file: &str) -> anyhow::Result<()> {
        crate::provider_catalog::save_env_value_to_env_file(env_key, env_file, None)
    }

    fn clear_api_key_logout_summary(
        summary: &mut Vec<String>,
        errors: &mut Vec<String>,
        label: &str,
        env_key: &str,
        env_file: &str,
    ) {
        let configured =
            crate::provider_catalog::load_env_value_from_env_or_config(env_key, env_file).is_some();
        match Self::clear_api_key_login(env_key, env_file) {
            Ok(()) if configured => summary.push(label.to_string()),
            Ok(()) => {}
            Err(err) => errors.push(format!("{}: {}", label, err)),
        }
    }

    pub(super) fn start_login_provider(
        &mut self,
        provider: crate::provider_catalog::LoginProviderDescriptor,
    ) {
        crate::logging::event_info(
            "login_started",
            vec![
                ("provider_id", provider.id.to_string()),
                ("auth_kind", provider.auth_kind.label().to_string()),
            ],
        );
        match provider.target {
            crate::provider_catalog::LoginProviderTarget::AutoImport => {
                match crate::external_auth::pending_external_auth_review_candidates() {
                    Ok(candidates) if candidates.is_empty() => {
                        self.push_display_message(DisplayMessage::system(
                            "No importable external logins were found.".to_string(),
                        ));
                        self.set_status_notice("Login: no external imports found");
                    }
                    Ok(candidates) => {
                        self.push_display_message(DisplayMessage::system(
                            crate::external_auth::format_external_auth_review_candidates_markdown(
                                &candidates,
                            ),
                        ));
                        self.set_status_notice("Login: choose sources to import");
                        self.pending_login = Some(PendingLogin::AutoImportSelection { candidates });
                    }
                    Err(err) => {
                        self.push_display_message(DisplayMessage::error(format!(
                            "Failed to inspect external login sources: {}",
                            err
                        )));
                        self.set_status_notice("Login: auto import failed");
                    }
                }
            }
            crate::provider_catalog::LoginProviderTarget::Claude => self.start_claude_login(),
            crate::provider_catalog::LoginProviderTarget::ClaudeApiKey => {
                self.start_anthropic_api_key_login()
            }
            crate::provider_catalog::LoginProviderTarget::OpenAi => self.start_openai_login(),
            crate::provider_catalog::LoginProviderTarget::OpenAiApiKey => {
                self.start_openai_api_key_login()
            }
            crate::provider_catalog::LoginProviderTarget::Gemini => self.start_gemini_login(),
            crate::provider_catalog::LoginProviderTarget::GeminiApiKey => {
                self.start_gemini_api_key_login()
            }
            crate::provider_catalog::LoginProviderTarget::Antigravity => {
                self.start_antigravity_login()
            }
        }
    }

    fn begin_pending_login(&mut self, pending: PendingLogin) {
        self.pending_login = Some(pending);
    }

    fn start_claude_login(&mut self) {
        let label = crate::auth::claude::login_target_label(None)
            .unwrap_or_else(|_| crate::auth::claude::primary_account_label());
        self.start_claude_login_for_account(&label);
    }

    pub(super) fn start_claude_login_for_account(&mut self, label: &str) {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        use sha2::{Digest, Sha256};

        let verifier: String = {
            use rand::Rng;
            const CHARSET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            let mut rng = rand::rng();
            (0..64)
                .map(|_| {
                    let idx = rng.random_range(0..CHARSET.len());
                    CHARSET[idx] as char
                })
                .collect()
        };

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(hash);

        // Try a loopback callback first so the user never has to copy/paste the
        // authorization code (mirrors the OpenAI/Gemini flows). Claude uses the
        // PKCE verifier as the OAuth `state`, so we wait for that on the
        // listener. If binding fails we fall back to manual paste with the
        // hosted redirect page.
        let callback_listener = crate::auth::oauth::bind_callback_listener(0).ok();
        let callback_port = callback_listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|addr| addr.port());
        let callback_available = callback_listener.is_some() && callback_port.is_some();

        let (auth_url, redirect_uri) = match callback_port {
            Some(port) if callback_available => {
                let redirect_uri = format!("http://localhost:{}/callback", port);
                let auth_url =
                    crate::auth::oauth::claude_auth_url(&redirect_uri, &challenge, &verifier);
                (auth_url, redirect_uri)
            }
            _ => {
                let redirect_uri = crate::auth::oauth::claude::REDIRECT_URI.to_string();
                let auth_url =
                    crate::auth::oauth::claude_auth_url(&redirect_uri, &challenge, &verifier);
                (auth_url, redirect_uri)
            }
        };
        let qr_section = crate::login_qr::markdown_section_for_tui(
            &auth_url,
            "Scan this on another device if this machine has no browser:",
        )
        .map(|section| format!("\n\n{section}"))
        .unwrap_or_default();

        let browser_opened = Self::open_auth_browser(&auth_url);
        let preflight = Self::record_oauth_preflight(
            "claude",
            browser_opened,
            callback_port.map(|p| format!("localhost:{}", p)).as_deref(),
            Some(callback_available),
        );

        // Spawn the loopback waiter. On success it publishes LoginCompleted just
        // like the manual paste path, so the account UI reacts identically.
        if let (Some(listener), true) = (callback_listener, callback_available) {
            let verifier_clone = verifier.clone();
            let label_clone = label.to_string();
            let redirect_clone = redirect_uri.clone();
            tokio::spawn(async move {
                match Self::claude_login_callback(
                    verifier_clone,
                    label_clone,
                    redirect_clone,
                    listener,
                )
                .await
                {
                    Ok(msg) => {
                        crate::logging::info(&format!("Claude login: {}", msg));
                        Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                            provider: "claude".to_string(),
                            success: true,
                            message: msg,
                        }));
                    }
                    Err(e) => {
                        crate::logging::info(&format!(
                            "Claude automatic callback did not complete: {}",
                            e
                        ));
                    }
                }
            });
        }

        let callback_line = if callback_available {
            "Waiting for the browser callback... (this completes automatically)\n".to_string()
        } else {
            "After logging in, copy the callback URL or authorization code and paste it here.\n"
                .to_string()
        };

        self.push_display_message(DisplayMessage::system(format!(
            "Claude OAuth Login (account: {})\n\n\
             Opening browser for authentication...\n\n\
             If the browser didn't open, visit:\n{}\n\n\
             {}{}{}\
             Or paste the full callback URL or authorization code here to finish from another device. Type /cancel to abort.{}",
            label,
            auth_url,
            if preflight.is_empty() {
                String::new()
            } else {
                format!("{}\n", preflight)
            },
            callback_line,
            if preflight.is_empty() {
                String::new()
            } else {
                "Manual-safe fallback is already active here.\n".to_string()
            },
            qr_section
        )));
        if callback_available {
            self.set_status_notice(format!("Login [{}]: waiting...", label));
        } else {
            self.set_status_notice(format!("Login [{}]: paste code...", label));
        }
        self.begin_pending_login(PendingLogin::ClaudeAccount {
            verifier,
            label: label.to_string(),
            redirect_uri: if callback_available {
                Some(redirect_uri)
            } else {
                None
            },
        });
    }

    async fn claude_login_callback(
        verifier: String,
        label: String,
        redirect_uri: String,
        listener: tokio::net::TcpListener,
    ) -> Result<String, String> {
        // Claude uses the PKCE verifier as the OAuth `state` value.
        let code = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            crate::auth::oauth::wait_for_callback_async_on_listener(listener, &verifier),
        )
        .await
        .map_err(|_| "Login timed out after 5 minutes. Please try again.".to_string())?
        .map_err(|e| format!("Callback failed: {}", e))?;

        Self::claude_token_exchange(verifier, code, &label, Some(redirect_uri)).await
    }

    pub(super) fn switch_account(&mut self, label: &str) {
        match crate::auth::claude::set_active_account(label) {
            Ok(()) => {
                {
                    let provider = self.provider.clone();
                    let label_owned = label.to_string();
                    tokio::spawn(async move {
                        provider.invalidate_credentials().await;
                        crate::logging::info(&format!(
                            "Switched to Anthropic account '{}'",
                            label_owned
                        ));
                    });
                }
                self.push_display_message(DisplayMessage::system(format!(
                    "Switched to Anthropic account {}.",
                    label
                )));
                // Keep account-sensitive UI state in sync immediately.
                crate::auth::AuthStatus::invalidate_cache();
                self.context_limit = self.provider.context_window() as u64;
                self.context_warning_shown = false;
            }
            Err(e) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Failed to switch account: {}",
                    e
                )));
            }
        }
    }

    pub(super) fn switch_account_by_label(&mut self, label: &str) {
        let has_anthropic = crate::auth::claude::list_accounts()
            .unwrap_or_default()
            .iter()
            .any(|account| account.label == label);
        let has_openai = crate::auth::codex::list_accounts()
            .unwrap_or_default()
            .iter()
            .any(|account| account.label == label);

        match (has_anthropic, has_openai) {
            (true, false) => self.switch_account(label),
            (false, true) => self.switch_openai_account(label),
            (true, true) => self.push_display_message(DisplayMessage::error(format!(
                "Account label {} exists for both Anthropic and OpenAI. Use /account switch {} or /account openai switch {} explicitly.",
                label, label, label
            ))),
            (false, false) => self.push_display_message(DisplayMessage::error(format!(
                "No Anthropic or OpenAI account with label {} found.",
                label
            ))),
        }
    }

    pub(super) fn remove_account(&mut self, label: &str) {
        match crate::auth::claude::remove_account(label) {
            Ok(()) => {
                self.push_display_message(DisplayMessage::system(format!(
                    "Removed Anthropic account {}.",
                    label
                )));
            }
            Err(e) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Failed to remove account: {}",
                    e
                )));
            }
        }
    }

    pub(super) fn switch_openai_account(&mut self, label: &str) {
        match crate::auth::codex::set_active_account(label) {
            Ok(()) => {
                {
                    let provider = self.provider.clone();
                    let label_owned = label.to_string();
                    tokio::spawn(async move {
                        provider.invalidate_credentials().await;
                        crate::logging::info(&format!(
                            "Switched to OpenAI account '{}'",
                            label_owned
                        ));
                    });
                }
                self.push_display_message(DisplayMessage::system(format!(
                    "Switched to OpenAI account {}.",
                    label
                )));
                crate::auth::AuthStatus::invalidate_cache();
                self.context_limit = self.provider.context_window() as u64;
                self.context_warning_shown = false;
            }
            Err(e) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Failed to switch OpenAI account: {}",
                    e
                )));
            }
        }
    }

    pub(super) fn remove_openai_account(&mut self, label: &str) {
        match crate::auth::codex::remove_account(label) {
            Ok(()) => {
                self.push_display_message(DisplayMessage::system(format!(
                    "Removed OpenAI account {}.",
                    label
                )));
            }
            Err(e) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Failed to remove OpenAI account: {}",
                    e
                )));
            }
        }
    }

    fn start_openai_login(&mut self) {
        let label = crate::auth::codex::login_target_label(None)
            .unwrap_or_else(|_| crate::auth::codex::primary_account_label());
        self.start_openai_login_for_account(&label);
    }

    pub(super) fn start_openai_login_for_account(&mut self, label: &str) {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        use sha2::{Digest, Sha256};

        let verifier: String = {
            use rand::Rng;
            const CHARSET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            let mut rng = rand::rng();
            (0..64)
                .map(|_| {
                    let idx = rng.random_range(0..CHARSET.len());
                    CHARSET[idx] as char
                })
                .collect()
        };

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(hash);

        let state: String = {
            let bytes: [u8; 16] = rand::random();
            hex::encode(bytes)
        };

        let port = crate::auth::oauth::openai::DEFAULT_PORT;
        let redirect_uri = crate::auth::oauth::openai::redirect_uri(port);
        let auth_url = crate::auth::oauth::openai_auth_url_with_prompt(
            &redirect_uri,
            &challenge,
            &state,
            Some("login"),
        );
        let qr_section = crate::login_qr::markdown_section_for_tui(
            &auth_url,
            "Scan this on another device if this machine has no browser, then paste the full callback URL here:",
        )
        .map(|section| format!("\n\n{section}"))
        .unwrap_or_default();

        let callback_listener = crate::auth::oauth::bind_callback_listener(port).ok();
        let callback_available = callback_listener.is_some();
        let browser_opened = Self::open_auth_browser(&auth_url);
        let label_owned = label.to_string();

        if let Some(listener) = callback_listener {
            let verifier_clone = verifier.clone();
            let state_clone = state.clone();
            let label_clone = label_owned.clone();
            tokio::spawn(async move {
                match Self::openai_login_callback(
                    verifier_clone,
                    state_clone,
                    Some(label_clone),
                    listener,
                )
                .await
                {
                    Ok(msg) => {
                        crate::logging::info(&format!("OpenAI login: {}", msg));
                        Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                            provider: "openai".to_string(),
                            success: true,
                            message: msg,
                        }));
                    }
                    Err(e) => {
                        crate::logging::info(&format!(
                            "OpenAI automatic callback did not complete: {}",
                            e
                        ));
                    }
                }
            });
        }

        let callback_line = if callback_available {
            format!(
                "Waiting for callback on localhost:{}... (this will complete automatically)\n",
                port
            )
        } else {
            format!(
                "Local callback port localhost:{} is unavailable, so finish in any browser and paste the full callback URL here.\n",
                port
            )
        };
        let preflight = Self::record_oauth_preflight(
            "openai",
            browser_opened,
            Some(&format!("localhost:{}", port)),
            Some(callback_available),
        );

        self.push_display_message(DisplayMessage::system(format!(
            "OpenAI OAuth Login (account: {})\n\n\
             Opening browser for authentication...\n\n\
             If the browser didn't open, visit:\n{}\n\n\
             Note: Wait a few seconds for the page to fully load before clicking Continue. \
             OpenAI's verification system may briefly disable the button.\n\n\
             {}{}{}\
             Or paste the full callback URL or query string here to finish from another device. Type /cancel to abort.{}",
            label,
            auth_url,
            if preflight.is_empty() {
                String::new()
            } else {
                format!("{}\n", preflight)
            },
            callback_line,
            if preflight.is_empty() {
                String::new()
            } else {
                "Manual-safe fallback is already active here.\n".to_string()
            },
            qr_section
        )));
        self.set_status_notice(format!("Login [{}]: waiting...", label));
        self.begin_pending_login(PendingLogin::OpenAiAccount {
            verifier,
            label: label.to_string(),
            expected_state: state,
            redirect_uri,
        });
    }

    async fn openai_login_callback(
        verifier: String,
        expected_state: String,
        label: Option<String>,
        listener: tokio::net::TcpListener,
    ) -> Result<String, String> {
        let port = crate::auth::oauth::openai::DEFAULT_PORT;
        let redirect_uri = crate::auth::oauth::openai::redirect_uri(port);
        let code = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            crate::auth::oauth::wait_for_callback_async_on_listener(listener, &expected_state),
        )
        .await
        .map_err(|_| "Login timed out after 5 minutes. Please try again.".to_string())?
        .map_err(|e| format!("Callback failed: {}", e))?;

        Self::openai_token_exchange(verifier, code, label, None, &redirect_uri).await
    }

    async fn openai_token_exchange(
        verifier: String,
        input: String,
        label: Option<String>,
        expected_state: Option<String>,
        redirect_uri: &str,
    ) -> Result<String, String> {
        let oauth_tokens = if let Some(expected_state) = expected_state {
            crate::auth::oauth::exchange_openai_callback_input(
                &verifier,
                input.trim(),
                &expected_state,
                redirect_uri,
            )
            .await
            .map_err(|e| e.to_string())?
        } else {
            crate::auth::oauth::exchange_openai_code(&input, &verifier, redirect_uri)
                .await
                .map_err(|e| e.to_string())?
        };

        let label = label.unwrap_or_else(crate::auth::codex::primary_account_label);
        crate::auth::oauth::save_openai_tokens_for_account(&oauth_tokens, &label)
            .map_err(|e| format!("Failed to save tokens: {}", e))?;

        Ok(format!(
            "Successfully logged in to OpenAI! (account: {})",
            label
        ))
    }

    fn start_gemini_login(&mut self) {
        let (verifier, challenge) = crate::auth::oauth::generate_pkce_public();
        let state = crate::auth::oauth::generate_state_public();

        let callback_listener = crate::auth::oauth::bind_callback_listener(0).ok();
        let maybe_redirect_uri = callback_listener
            .as_ref()
            .and_then(|listener| listener.local_addr().ok())
            .map(|addr| format!("http://127.0.0.1:{}/oauth2callback", addr.port()));

        let auth_setup: anyhow::Result<(String, Option<String>, String)> =
            if let Some(redirect_uri) = maybe_redirect_uri {
                crate::auth::gemini::build_web_auth_url(&redirect_uri, &challenge, &state)
                    .map(|auth_url| (auth_url, Some(state.clone()), redirect_uri))
            } else {
                crate::auth::gemini::build_manual_auth_url(
                    "https://codeassist.google.com/authcode",
                    &challenge,
                    &state,
                )
                .map(|auth_url| {
                    (
                        auth_url,
                        None,
                        "https://codeassist.google.com/authcode".to_string(),
                    )
                })
            };

        let (auth_url, pending_state, redirect_uri) = match auth_setup {
            Ok(values) => values,
            Err(e) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Gemini login is unavailable: {}",
                    e
                )));
                self.set_status_notice("Login: failed");
                return;
            }
        };

        let qr_section = crate::login_qr::markdown_section_for_tui(
            &auth_url,
            "Scan this on another device if this machine has no browser, then paste the callback URL or authorization code here:",
        )
        .map(|section| format!("\n\n{section}"))
        .unwrap_or_default();

        let browser_opened = Self::open_auth_browser(&auth_url);
        let callback_available = callback_listener.is_some() && pending_state.is_some();

        if let (Some(listener), Some(expected_state)) = (callback_listener, pending_state.clone()) {
            let redirect_clone = redirect_uri.clone();
            let verifier_clone = verifier.clone();
            tokio::spawn(async move {
                let code = tokio::time::timeout(
                    std::time::Duration::from_secs(300),
                    crate::auth::oauth::wait_for_callback_async_on_listener(
                        listener,
                        &expected_state,
                    ),
                )
                .await
                .map_err(|_| "Login timed out after 5 minutes. Please try again.".to_string())
                .and_then(|result| result.map_err(|e| format!("Callback failed: {}", e)));

                match code {
                    Ok(code) => {
                        match crate::auth::gemini::exchange_callback_code(
                            &code,
                            &verifier_clone,
                            &redirect_clone,
                        )
                        .await
                        {
                            Ok(tokens) => {
                                let msg = if let Some(email) = tokens.email {
                                    format!(
                                        "Successfully logged in to Gemini! (account: {})",
                                        email
                                    )
                                } else {
                                    "Successfully logged in to Gemini!".to_string()
                                };
                                Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                    provider: "gemini".to_string(),
                                    success: true,
                                    message: msg,
                                }));
                            }
                            Err(e) => {
                                let message = format!("Gemini login failed: {}", e);
                                crate::logging::info(&format!(
                                    "Gemini automatic callback did not complete: {}",
                                    e
                                ));
                                Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                    provider: "gemini".to_string(),
                                    success: false,
                                    message,
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        crate::logging::info(&format!(
                            "Gemini automatic callback did not complete: {}",
                            e
                        ));
                        Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                            provider: "gemini".to_string(),
                            success: false,
                            message: format!("Gemini login failed: {}", e),
                        }));
                    }
                }
            });
        }

        let callback_line = if callback_available {
            format!(
                "Waiting for callback on {}... (this will complete automatically)\n",
                redirect_uri
            )
        } else {
            "Finish login in any browser, then paste the callback URL or authorization code here.\n"
                .to_string()
        };
        let preflight = Self::record_oauth_preflight(
            "gemini",
            browser_opened,
            Some(&redirect_uri),
            Some(callback_available),
        );

        self.push_display_message(DisplayMessage::system(format!(
            "Gemini OAuth Login\n\n\
             Opening browser for authentication...\n\n\
             If the browser didn't open, visit:\n{}\n\n\
             {}{}{}\
             Or paste the full callback URL, query string, or authorization code here to finish. Type /cancel to abort.{}",
            auth_url,
            if preflight.is_empty() {
                String::new()
            } else {
                format!("{}\n", preflight)
            },
            callback_line,
            if preflight.is_empty() {
                String::new()
            } else {
                "Manual-safe fallback is already active here.\n".to_string()
            },
            qr_section
        )));
        self.set_status_notice("Login: waiting...");
        self.begin_pending_login(PendingLogin::Gemini {
            verifier,
            expected_state: pending_state,
            redirect_uri,
        });
    }

    fn start_openai_api_key_login(&mut self) {
        self.start_api_key_login(
            "OpenAI API",
            "https://platform.openai.com/api-keys",
            "openai.env",
            "OPENAI_API_KEY",
            Some("https://api.openai.com/v1"),
        );
    }

    fn start_anthropic_api_key_login(&mut self) {
        self.start_api_key_login(
            "Anthropic API",
            "https://console.anthropic.com/settings/keys",
            "anthropic.env",
            "ANTHROPIC_API_KEY",
            Some("https://api.anthropic.com"),
        );
    }

    fn start_gemini_api_key_login(&mut self) {
        self.start_api_key_login(
            "Gemini API",
            "https://aistudio.google.com/apikey",
            crate::auth::gemini::GEMINI_API_KEY_ENV_FILE,
            crate::auth::gemini::GEMINI_API_KEY_ENV_VARS[0],
            Some("https://generativelanguage.googleapis.com"),
        );
    }

    fn start_api_key_login(
        &mut self,
        provider: &str,
        docs_url: &str,
        env_file: &str,
        key_name: &str,
        endpoint: Option<&str>,
    ) {
        let endpoint_hint = endpoint
            .map(|endpoint| format!("Endpoint: {}\n", endpoint))
            .unwrap_or_default();
        self.push_display_message(DisplayMessage::system(format!(
            "{} API Key\n\n\
             Setup docs: {}\n\
             Stored variable: {}\n\
             {}\n\
             Paste your API key below (it will be saved securely), or type /cancel to abort.",
            provider, docs_url, key_name, endpoint_hint,
        )));
        self.set_status_notice("Login: paste key...");
        self.begin_pending_login(PendingLogin::ApiKeyProfile {
            provider_id: provider.to_ascii_lowercase().replace(' ', "-"),
            provider: provider.to_string(),
            auth_method: "api_key".to_string(),
            docs_url: docs_url.to_string(),
            env_file: env_file.to_string(),
            key_name: key_name.to_string(),
            endpoint: endpoint.map(|value| value.to_string()),
        });
    }

    fn start_antigravity_login(&mut self) {
        let (verifier, challenge) = crate::auth::oauth::generate_pkce_public();
        let expected_state = crate::auth::oauth::generate_state_public();
        let port = crate::auth::antigravity::DEFAULT_PORT;
        let redirect_uri = crate::auth::antigravity::redirect_uri(port);

        let auth_url = match crate::auth::antigravity::build_auth_url(
            &redirect_uri,
            &challenge,
            &expected_state,
        ) {
            Ok(url) => url,
            Err(e) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Antigravity login is unavailable: {}",
                    e
                )));
                self.set_status_notice("Login: failed");
                return;
            }
        };

        let qr_section = crate::login_qr::markdown_section_for_tui(
            &auth_url,
            "Scan this on another device if this machine has no browser, then paste the full callback URL or query string here:",
        )
        .map(|section| format!("\n\n{section}"))
        .unwrap_or_default();

        let callback_listener = crate::auth::oauth::bind_callback_listener(port).ok();
        let callback_available = callback_listener.is_some();
        let browser_opened = Self::open_auth_browser(&auth_url);

        if let Some(listener) = callback_listener {
            let verifier_clone = verifier.clone();
            let expected_state_clone = expected_state.clone();
            let redirect_clone = redirect_uri.clone();
            tokio::spawn(async move {
                let code = tokio::time::timeout(
                    std::time::Duration::from_secs(300),
                    crate::auth::oauth::wait_for_callback_async_on_listener(
                        listener,
                        &expected_state_clone,
                    ),
                )
                .await
                .map_err(|_| "Login timed out after 5 minutes. Please try again.".to_string())
                .and_then(|result| result.map_err(|e| format!("Callback failed: {}", e)));

                match code {
                    Ok(code) => {
                        match Self::antigravity_token_exchange(
                            verifier_clone,
                            code,
                            Some(expected_state_clone),
                            redirect_clone,
                        )
                        .await
                        {
                            Ok(msg) => {
                                Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                    provider: "antigravity".to_string(),
                                    success: true,
                                    message: msg,
                                }));
                            }
                            Err(e) => {
                                Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                    provider: "antigravity".to_string(),
                                    success: false,
                                    message: format!("Antigravity login failed: {}", e),
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        crate::logging::info(&format!(
                            "Antigravity automatic callback did not complete: {}",
                            e
                        ));
                    }
                }
            });
        }

        let callback_line = if callback_available {
            format!(
                "Waiting for callback on {}... (this will complete automatically)\n",
                redirect_uri
            )
        } else {
            format!(
                "Local callback port {} is unavailable, so finish in any browser and paste the full callback URL or query string here.\n",
                redirect_uri
            )
        };
        let preflight = Self::record_oauth_preflight(
            "antigravity",
            browser_opened,
            Some(&redirect_uri),
            Some(callback_available),
        );
        let manual_hint = "If the browser ends on a loopback/callback error page, copy the full URL from the address bar and paste it here immediately.\n";

        self.push_display_message(DisplayMessage::system(format!(
            "Antigravity OAuth Login\n\n\
             Opening browser for authentication...\n\n\
             If the browser didn't open, visit:\n{}\n\n\
             {}{}{}{}\
             Or paste the full callback URL or query string here to finish. Type /cancel to abort.{}",
            auth_url,
            if preflight.is_empty() {
                String::new()
            } else {
                format!("{}\n", preflight)
            },
            callback_line,
            manual_hint,
            if preflight.is_empty() {
                String::new()
            } else {
                "Manual-safe fallback is already active here.\n".to_string()
            },
            qr_section
        )));
        self.set_status_notice("Login: antigravity waiting...");
        self.begin_pending_login(PendingLogin::Antigravity {
            verifier,
            expected_state,
            redirect_uri,
        });
    }

    async fn antigravity_token_exchange(
        verifier: String,
        input: String,
        expected_state: Option<String>,
        redirect_uri: String,
    ) -> Result<String, String> {
        let trimmed = input.trim();
        let tokens =
            if antigravity_input_requires_state_validation(trimmed, expected_state.as_deref()) {
                crate::auth::antigravity::exchange_callback_input(
                    &verifier,
                    trimmed,
                    expected_state.as_deref(),
                    &redirect_uri,
                )
                .await
            } else {
                crate::auth::antigravity::exchange_callback_code(trimmed, &verifier, &redirect_uri)
                    .await
            }
            .map_err(|e| e.to_string())?;

        let mut msg = if let Some(email) = tokens.email.as_deref() {
            format!(
                "Successfully logged in to Antigravity! (account: {})",
                email
            )
        } else {
            "Successfully logged in to Antigravity!".to_string()
        };
        if let Some(project_id) = tokens.project_id.as_deref() {
            msg.push_str(&format!(" (project: {})", project_id));
        }
        Ok(msg)
    }

    pub(super) fn handle_login_input(&mut self, pending: PendingLogin, input: String) {
        let trimmed = input.trim();
        if trimmed == "/cancel" {
            self.push_display_message(DisplayMessage::system("Login cancelled.".to_string()));
            return;
        }

        if trimmed.is_empty() {
            let help = match &pending {
                PendingLogin::AutoImportSelection { .. } => {
                    "Auto import is waiting for your selection. Reply with a to approve all, 1,3 to approve specific sources, or /cancel to abort.".to_string()
                }
                _ => "Login still in progress. Complete it in your browser, or paste the callback URL / authorization code here. Type /cancel to abort.".to_string(),
            };
            self.push_display_message(DisplayMessage::system(help));
            self.pending_login = Some(pending);
            return;
        }

        match &pending {
            PendingLogin::OpenAiAccount { .. } if !looks_like_oauth_callback_input(trimmed) => {
                self.push_display_message(DisplayMessage::system(
                    "Still waiting for the browser callback. Paste the full callback URL or query string if you want to finish manually, or keep waiting for the automatic redirect.".to_string(),
                ));
                self.pending_login = Some(pending);
                return;
            }
            PendingLogin::Antigravity { .. } if !looks_like_oauth_callback_input(trimmed) => {
                self.push_display_message(DisplayMessage::system(
                    "Still waiting for the browser callback. Paste the full callback URL or query string if you want to finish manually, or keep waiting for the automatic redirect.".to_string(),
                ));
                self.pending_login = Some(pending);
                return;
            }
            _ => {}
        }

        match pending {
            PendingLogin::ClaudeAccount {
                verifier,
                label,
                redirect_uri,
            } => {
                self.set_status_notice(format!("Login [{}]: exchanging...", label));
                let input_owned = input.clone();
                let label_clone = label.clone();
                tokio::spawn(async move {
                    match Self::claude_token_exchange(
                        verifier,
                        input_owned,
                        &label_clone,
                        redirect_uri,
                    )
                    .await
                    {
                        Ok(msg) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "claude".to_string(),
                                success: true,
                                message: msg,
                            }));
                        }
                        Err(e) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "claude".to_string(),
                                success: false,
                                message: format!("Claude login [{}] failed: {}", label_clone, e),
                            }));
                        }
                    }
                });
                self.push_display_message(DisplayMessage::system(format!(
                    "Exchanging authorization code for account {}...",
                    label
                )));
            }
            PendingLogin::OpenAiAccount {
                verifier,
                label,
                expected_state,
                redirect_uri,
            } => {
                self.set_status_notice(format!("Login [{}]: exchanging...", label));
                let input_owned = input.clone();
                let label_clone = label.clone();
                tokio::spawn(async move {
                    match Self::openai_token_exchange(
                        verifier,
                        input_owned,
                        Some(label_clone.clone()),
                        Some(expected_state),
                        &redirect_uri,
                    )
                    .await
                    {
                        Ok(msg) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "openai".to_string(),
                                success: true,
                                message: msg,
                            }));
                        }
                        Err(e) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "openai".to_string(),
                                success: false,
                                message: format!("OpenAI login [{}] failed: {}", label_clone, e),
                            }));
                        }
                    }
                });
                self.push_display_message(DisplayMessage::system(format!(
                    "Exchanging OpenAI callback for account {}...",
                    label
                )));
            }
            PendingLogin::Gemini {
                verifier,
                expected_state,
                redirect_uri,
            } => {
                self.set_status_notice("Login: exchanging...");
                let input_owned = input.clone();
                tokio::spawn(async move {
                    match crate::auth::gemini::exchange_callback_input(
                        &verifier,
                        input_owned.trim(),
                        expected_state.as_deref(),
                        &redirect_uri,
                    )
                    .await
                    {
                        Ok(tokens) => {
                            let msg = if let Some(email) = tokens.email {
                                format!("Successfully logged in to Gemini! (account: {})", email)
                            } else {
                                "Successfully logged in to Gemini!".to_string()
                            };
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "gemini".to_string(),
                                success: true,
                                message: msg,
                            }));
                        }
                        Err(e) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "gemini".to_string(),
                                success: false,
                                message: format!("Gemini login failed: {}", e),
                            }));
                        }
                    }
                });
                self.push_display_message(DisplayMessage::system(
                    "Exchanging Gemini callback for tokens...".to_string(),
                ));
            }
            PendingLogin::Antigravity {
                verifier,
                expected_state,
                redirect_uri,
            } => {
                self.set_status_notice("Login: exchanging...");
                let input_owned = input.clone();
                tokio::spawn(async move {
                    match Self::antigravity_token_exchange(
                        verifier,
                        input_owned,
                        Some(expected_state),
                        redirect_uri,
                    )
                    .await
                    {
                        Ok(msg) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "antigravity".to_string(),
                                success: true,
                                message: msg,
                            }));
                        }
                        Err(e) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "antigravity".to_string(),
                                success: false,
                                message: format!("Antigravity login failed: {}", e),
                            }));
                        }
                    }
                });
                self.push_display_message(DisplayMessage::system(
                    "Exchanging Antigravity callback for tokens...".to_string(),
                ));
            }
            PendingLogin::ApiKeyProfile {
                provider_id,
                provider,
                auth_method,
                docs_url,
                env_file,
                key_name,
                endpoint,
            } => {
                let key = input.trim().to_string();
                if key.is_empty() {
                    self.push_display_message(DisplayMessage::error(
                        "API key cannot be empty.".to_string(),
                    ));
                    self.pending_login = Some(PendingLogin::ApiKeyProfile {
                        provider_id,
                        provider,
                        auth_method,
                        docs_url,
                        env_file,
                        key_name,
                        endpoint,
                    });
                    return;
                }
                // Real API keys are never short digit strings. Users sometimes
                // type a menu number like `1` here, trying to select from a
                // numbered list shown earlier; silently saving that as the key
                // bricks the provider until they log in again (issue #496).
                if key.len() < 8 && key.chars().all(|c| c.is_ascii_digit()) {
                    self.push_display_message(DisplayMessage::error(format!(
                        "'{}' looks like a menu selection, not an API key. This prompt is waiting for the {} API key itself. Paste the key (see {}), or type /cancel to abort.",
                        key, provider, docs_url
                    )));
                    self.pending_login = Some(PendingLogin::ApiKeyProfile {
                        provider_id,
                        provider,
                        auth_method,
                        docs_url,
                        env_file,
                        key_name,
                        endpoint,
                    });
                    return;
                }

                // Record the key-save attempt before touching disk. This is the
                // single most important breadcrumb for issue #312 ("paste API
                // key silently returns to menu"): it proves the input was
                // received and which env var/file jcode tried to write, without
                // logging the key itself.
                crate::logging::event_info(
                    "login_api_key_save_attempt",
                    vec![
                        ("provider_id", provider_id.clone()),
                        ("provider", provider.clone()),
                        ("auth_method", auth_method.clone()),
                        ("env_var", key_name.clone()),
                        ("env_file", env_file.clone()),
                        ("input_len", key.len().to_string()),
                    ],
                );

                match Self::save_named_api_key(&env_file, &key_name, &key) {
                    Ok(()) => {
                        crate::auth::AuthStatus::invalidate_cache();
                        crate::logging::event_info(
                            "login_api_key_saved",
                            vec![
                                ("provider_id", provider_id.clone()),
                                ("provider", provider.clone()),
                                ("env_var", key_name.clone()),
                                ("env_file", env_file.clone()),
                            ],
                        );
                        Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                            provider: provider.clone(),
                            success: true,
                            message: format!(
                                "{} API key saved.\n\n\
                                 Stored at ~/.config/jcode/{}.\n\
                                 API key saved. Run /refresh-model-list to refresh model discovery, then use /model to pick an accessible model.",
                                provider, env_file
                            ),
                        }));
                    }
                    Err(e) => {
                        let reason = crate::auth::login_diagnostics::classify_auth_failure_message(
                            &e.to_string(),
                        );
                        crate::logging::event_error(
                            "login_api_key_save_failed",
                            vec![
                                ("provider_id", provider_id.clone()),
                                ("provider", provider.clone()),
                                ("env_var", key_name.clone()),
                                ("env_file", env_file.clone()),
                                ("reason", reason.label().to_string()),
                                ("error", e.to_string()),
                            ],
                        );
                        self.push_display_message(DisplayMessage::error(format!(
                            "Failed to save {} key: {}",
                            provider, e
                        )));
                        self.pending_login = Some(PendingLogin::ApiKeyProfile {
                            provider_id,
                            provider,
                            auth_method,
                            docs_url,
                            env_file,
                            key_name,
                            endpoint,
                        });
                    }
                }
            }
            PendingLogin::AutoImportSelection { candidates } => {
                let selected = match crate::external_auth::parse_external_auth_review_selection(
                    &input,
                    candidates.len(),
                ) {
                    Ok(selected) => selected,
                    Err(err) => {
                        self.push_display_message(DisplayMessage::error(err.to_string()));
                        self.pending_login = Some(PendingLogin::AutoImportSelection { candidates });
                        return;
                    }
                };

                self.set_status_notice("Login: importing approved sources...");
                tokio::spawn(async move {
                    match crate::external_auth::run_external_auth_auto_import_candidates(
                        &candidates,
                        &selected,
                    )
                    .await
                    {
                        Ok(outcome) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "auto-import".to_string(),
                                success: outcome.imported > 0,
                                message: outcome.render_markdown(),
                            }));
                        }
                        Err(err) => {
                            Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
                                provider: "auto-import".to_string(),
                                success: false,
                                message: format!("Auto import failed: {}", err),
                            }));
                        }
                    }
                });
            }
        }
    }

    fn trigger_provider_auth_changed(
        &mut self,
        provider_hint: Option<&str>,
        select_local_model: bool,
    ) {
        crate::logging::auth_event(
            "auth_changed_triggered",
            self.provider.name(),
            &[("surface", "tui")],
        );
        crate::bus::Bus::global().publish(crate::bus::BusEvent::UiActivity(
            crate::bus::UiActivity::auth(
                Some(self.session.id.clone()),
                "",
                Some("Auth: refreshing model routes..."),
            ),
        ));
        // Remote mode forwards the auth change to the server immediately after
        // this handler returns. Refreshing the client-side provider as well used
        // to duplicate every catalog network request and could race a second
        // model switch against the first one.
        if self.is_remote {
            return;
        }
        let provider = Arc::clone(&self.provider);
        let provider_hint = provider_hint.map(str::to_string);
        let session_id = self.session.id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let activation = crate::auth::lifecycle::activate_auth_change(
                    &crate::auth::lifecycle::AuthActivationRequest::new(provider_hint, None),
                );
                provider.on_auth_changed();
                if select_local_model && activation.provider_id.is_some() {
                    // Select from hot/local routes once. Live catalogs publish
                    // ModelsUpdated and are handled by the UI without holding the
                    // login flow in a polling loop.
                    let routes = provider.model_routes();
                    let current_model = provider.model();
                    let selection = crate::auth::lifecycle::provider_model_to_select_after_auth(
                        &activation,
                        Some(&current_model),
                        &routes,
                    )
                    .map(|model| {
                        let model_request =
                            activation.model_switch_request(provider.name(), &model);
                        let provider_key = crate::provider::MultiProvider::session_provider_key_for_model_request(
                            &model_request,
                            provider.name(),
                        );
                        (model, model_request, provider_key)
                    });
                    if let Some((model, model_request, provider_key)) = selection
                        && provider.set_model(&model_request).is_ok()
                    {
                        crate::bus::Bus::global().publish_models_updated();
                        crate::bus::Bus::global().publish(
                            crate::bus::BusEvent::ProviderModelActivated {
                                session_id: session_id.clone(),
                                model: model.clone(),
                                provider_key,
                                message: format!(
                                    "Login ready. Switched to the strongest available default model: {model}."
                                ),
                                open_picker: false,
                            },
                        );
                    }
                }
                // Hot provider initialization is complete even if live catalog
                // prefetches are still running. Wake the picker now so it can use
                // the newly available routes instead of the pre-login snapshot.
                crate::bus::Bus::global().publish(crate::bus::BusEvent::AuthCatalogRefreshReady);
            });
        } else {
            let activation = crate::auth::lifecycle::activate_auth_change(
                &crate::auth::lifecycle::AuthActivationRequest::new(provider_hint, None),
            );
            provider.on_auth_changed();
            if select_local_model && activation.provider_id.is_some() {
                let routes = provider.model_routes();
                let current_model = provider.model();
                if let Some(model) = crate::auth::lifecycle::provider_model_to_select_after_auth(
                    &activation,
                    Some(&current_model),
                    &routes,
                ) {
                    let model_request = activation.model_switch_request(provider.name(), &model);
                    if provider.set_model(&model_request).is_ok() {
                        self.finalize_model_switch(&model_request);
                    }
                }
            }
            self.finish_auth_catalog_refresh();
        }
    }

    pub(super) fn handle_login_completed(&mut self, login: LoginCompleted) {
        crate::auth::AuthStatus::invalidate_cache();
        crate::logging::event_info(
            "login_completed",
            vec![
                ("provider", login.provider.clone()),
                ("success", login.success.to_string()),
            ],
        );
        if login.success {
            self.recent_authenticated_provider = Some((login.provider.clone(), Instant::now()));
            // A fresh login is exactly what the credential-failure breaker is
            // waiting for: give automatic retries a fresh budget.
            self.reset_credential_failure_breaker();
            self.auth_catalog_refresh_pending = true;
            self.invalidate_model_picker_cache();
            self.push_display_message(DisplayMessage::system(login.message));
            self.set_status_notice(format!("Login: {} ready", login.provider));
            self.trigger_provider_auth_changed(Some(&login.provider), true);
        } else {
            let message = crate::auth::login_diagnostics::augment_auth_error_message(
                &login.provider,
                &login.message,
            );
            self.push_display_message(DisplayMessage::error(message));
            self.set_status_notice(format!("Login: {} failed", login.provider));
        }
        if self.pending_login.is_some() {
            self.pending_login = None;
        }
    }

    async fn claude_token_exchange(
        verifier: String,
        input: String,
        label: &str,
        redirect_uri: Option<String>,
    ) -> Result<String, String> {
        let fallback_redirect_uri =
            redirect_uri.unwrap_or_else(|| crate::auth::oauth::claude::REDIRECT_URI.to_string());
        let redirect_uri =
            crate::auth::oauth::claude_redirect_uri_for_input(input.trim(), &fallback_redirect_uri);
        let oauth_tokens =
            crate::auth::oauth::exchange_claude_code(&verifier, input.trim(), &redirect_uri)
                .await
                .map_err(|e| e.to_string())?;

        crate::auth::oauth::save_claude_tokens_for_account(&oauth_tokens, label)
            .map_err(|e| format!("Failed to save tokens: {}", e))?;

        let profile_suffix = match crate::auth::oauth::update_claude_account_profile(
            label,
            &oauth_tokens.access_token,
        )
        .await
        {
            Ok(Some(email)) => format!(" (email: {})", mask_email(&email)),
            Ok(None) => String::new(),
            Err(e) => {
                crate::logging::warn(&format!(
                    "Claude login [{}] profile fetch failed: {}",
                    label, e
                ));
                String::new()
            }
        };

        Ok(format!(
            "Successfully logged in to Claude! (account: {}){}",
            label, profile_suffix
        ))
    }

    fn save_named_api_key(env_file: &str, key_name: &str, key: &str) -> anyhow::Result<()> {
        if !crate::provider_catalog::is_safe_env_key_name(key_name) {
            anyhow::bail!("Invalid API key variable name: {}", key_name);
        }
        if !crate::provider_catalog::is_safe_env_file_name(env_file) {
            anyhow::bail!("Invalid env file name: {}", env_file);
        }

        let config_dir = crate::storage::app_config_dir()?;
        let file_path = config_dir.join(env_file);
        crate::storage::upsert_env_file_value(&file_path, key_name, Some(key))?;
        crate::env::set_var(key_name, key);
        Ok(())
    }
}

fn looks_like_oauth_callback_input(input: &str) -> bool {
    let input = input.trim();
    input.starts_with("http://")
        || input.starts_with("https://")
        || input.starts_with('?')
        || input.contains("code=")
        || input.contains("state=")
}

fn antigravity_input_requires_state_validation(input: &str, expected_state: Option<&str>) -> bool {
    expected_state.is_some() && looks_like_oauth_callback_input(input)
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
