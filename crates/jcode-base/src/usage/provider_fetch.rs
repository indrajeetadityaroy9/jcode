use super::*;

pub(super) async fn fetch_anthropic_usage_for_token(
    display_name: String,
    access_token: String,
    refresh_token: String,
    account_label: String,
    expires_at: i64,
) -> ProviderUsage {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let access_token = if expires_at < now_ms + 300_000 && !refresh_token.is_empty() {
        match crate::auth::oauth::refresh_claude_tokens_for_account(&refresh_token, &account_label)
            .await
        {
            Ok(refreshed) => refreshed.access_token,
            Err(_) => {
                if expires_at < now_ms {
                    return ProviderUsage {
                        provider_name: display_name,
                        error: Some(
                            "OAuth token expired - use `/login claude` to re-authenticate"
                                .to_string(),
                        ),
                        ..Default::default()
                    };
                }
                access_token
            }
        }
    } else {
        access_token
    };

    let cache_key = anthropic_usage_cache_key(&access_token, Some(&account_label));
    match fetch_anthropic_usage_data(access_token, cache_key).await {
        Ok(data) => provider_report_from_usage_data(display_name, &data),
        Err(e) => ProviderUsage {
            provider_name: display_name,
            error: Some(e.to_string()),
            ..Default::default()
        },
    }
}

pub(super) async fn fetch_all_openai_usage_reports() -> Vec<ProviderUsage> {
    let accounts = auth::codex::list_accounts().unwrap_or_default();
    if !accounts.is_empty() {
        let active_label = auth::codex::active_account_label();
        let mut reports = Vec::with_capacity(accounts.len());
        for account in &accounts {
            let display_name = openai_provider_display_name(
                &account.label,
                account.email.as_deref(),
                accounts.len(),
                active_label.as_deref() == Some(&account.label),
            );
            reports.push(
                fetch_openai_usage_for_account(
                    display_name,
                    auth::codex::CodexCredentials {
                        access_token: account.access_token.clone(),
                        refresh_token: account.refresh_token.clone(),
                        id_token: account.id_token.clone(),
                        account_id: account.account_id.clone(),
                        expires_at: account.expires_at,
                    },
                    Some(account.label.as_str()),
                )
                .await,
            );
        }
        return reports;
    }

    let creds = match auth::codex::load_credentials() {
        Ok(creds) => creds,
        Err(_) => return Vec::new(),
    };
    let is_chatgpt = !creds.refresh_token.is_empty() || creds.id_token.is_some();
    if !is_chatgpt || creds.access_token.is_empty() {
        return Vec::new();
    }

    vec![
        fetch_openai_usage_for_account(
            openai_provider_display_name("default", None, 1, true),
            creds,
            None,
        )
        .await,
    ]
}

pub(super) async fn fetch_openai_usage_report() -> Option<ProviderUsage> {
    let reports = fetch_all_openai_usage_reports().await;
    active_openai_usage_report(&reports)
        .cloned()
        .or_else(|| reports.into_iter().next())
}

pub(super) async fn fetch_openai_usage_for_account(
    display_name: String,
    mut creds: auth::codex::CodexCredentials,
    account_label: Option<&str>,
) -> ProviderUsage {
    let is_chatgpt = !creds.refresh_token.is_empty() || creds.id_token.is_some();
    if creds.access_token.is_empty() || !is_chatgpt {
        return ProviderUsage {
            provider_name: display_name,
            error: Some("No OpenAI/Codex OAuth credentials found".to_string()),
            ..Default::default()
        };
    }

    let initial_cache_key = openai_usage_cache_key(&creds.access_token, account_label);
    if let Some(cached) = cached_openai_usage(&initial_cache_key) {
        return provider_report_from_openai_usage_data(display_name, &cached);
    }

    if let Some(expires_at) = creds.expires_at {
        let now = chrono::Utc::now().timestamp_millis();
        if expires_at < now + 300_000 && !creds.refresh_token.is_empty() {
            let refreshed = match account_label {
                Some(label) => {
                    crate::auth::oauth::refresh_openai_tokens_for_account(
                        &creds.refresh_token,
                        label,
                    )
                    .await
                }
                None => crate::auth::oauth::refresh_openai_tokens(&creds.refresh_token).await,
            };
            match refreshed {
                Ok(refreshed) => {
                    creds.access_token = refreshed.access_token;
                    creds.refresh_token = refreshed.refresh_token;
                    creds.id_token = refreshed.id_token.or(creds.id_token);
                    creds.account_id = creds.account_id.clone().or_else(|| {
                        creds
                            .id_token
                            .as_deref()
                            .and_then(auth::codex::extract_account_id)
                    });
                    creds.expires_at = Some(refreshed.expires_at);
                }
                Err(e) => {
                    let report = ProviderUsage {
                        provider_name: display_name,
                        error: Some(format!(
                            "Token refresh failed: {} - use `/login openai` to re-authenticate",
                            e
                        )),
                        ..Default::default()
                    };
                    store_openai_usage(
                        initial_cache_key,
                        openai_usage_data_from_provider_report(&report),
                    );
                    return report;
                }
            }
        }
    }

    let cache_key = openai_usage_cache_key(&creds.access_token, account_label);
    if cache_key != initial_cache_key
        && let Some(cached) = cached_openai_usage(&cache_key)
    {
        return provider_report_from_openai_usage_data(display_name, &cached);
    }

    let client = crate::provider::shared_http_client();
    let mut builder = client
        .get(OPENAI_USAGE_URL)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {}", creds.access_token));

    if let Some(ref account_id) = creds.account_id {
        builder = builder.header("chatgpt-account-id", account_id);
    }

    let response = match builder.send().await {
        Ok(response) => response,
        Err(e) => {
            let report = ProviderUsage {
                provider_name: display_name,
                error: Some(format!("Failed to fetch: {}", e)),
                ..Default::default()
            };
            store_openai_usage(cache_key, openai_usage_data_from_provider_report(&report));
            return report;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let report = ProviderUsage {
            provider_name: display_name,
            error: Some(format!("API error ({}): {}", status, body)),
            ..Default::default()
        };
        store_openai_usage(cache_key, openai_usage_data_from_provider_report(&report));
        return report;
    }

    let body_text = match response.text().await {
        Ok(text) => text,
        Err(e) => {
            let report = ProviderUsage {
                provider_name: display_name,
                error: Some(format!("Failed to read response: {}", e)),
                ..Default::default()
            };
            store_openai_usage(cache_key, openai_usage_data_from_provider_report(&report));
            return report;
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(value) => value,
        Err(e) => {
            let report = ProviderUsage {
                provider_name: display_name,
                error: Some(format!("Failed to parse response: {}", e)),
                ..Default::default()
            };
            store_openai_usage(cache_key, openai_usage_data_from_provider_report(&report));
            return report;
        }
    };

    let parsed = parse_openai_usage_payload(&json);

    let report = ProviderUsage {
        provider_name: display_name,
        limits: parsed.limits,
        extra_info: parsed.extra_info,
        hard_limit_reached: parsed.hard_limit_reached,
        error: None,
        last_used_unix_secs: None,
    };
    store_openai_usage(cache_key, openai_usage_data_from_provider_report(&report));
    report
}

/// Antigravity per-model quota report. The backend's `fetchAvailableModels`
/// response carries `quotaInfo.remainingFraction` + `resetTime` per model,
/// which is the only usage signal Antigravity exposes.
pub(super) async fn fetch_antigravity_usage_report() -> Option<ProviderUsage> {
    if !auth::antigravity::has_cached_auth() {
        return None;
    }

    let client = crate::provider::shared_http_client();
    let snapshot = match crate::provider::antigravity::fetch_catalog_snapshot(&client).await {
        Ok(snapshot) if !snapshot.models.is_empty() => {
            crate::provider::antigravity::persist_catalog(&snapshot);
            snapshot
        }
        Ok(_) => {
            return Some(ProviderUsage {
                provider_name: "Antigravity".to_string(),
                error: Some("Antigravity model catalog returned no models".to_string()),
                ..Default::default()
            });
        }
        Err(e) => {
            return Some(ProviderUsage {
                provider_name: "Antigravity".to_string(),
                error: Some(format!("Failed to fetch model quotas: {}", e)),
                ..Default::default()
            });
        }
    };

    let mut limits = Vec::new();
    let mut extra_info = Vec::new();

    if let Ok(tokens) = auth::antigravity::load_tokens()
        && let Some(email) = tokens.email.as_deref()
    {
        extra_info.push(("Account".to_string(), mask_email(email)));
    }

    let mut seen_names = std::collections::HashSet::new();
    for model in &snapshot.models {
        let Some(remaining_milli) = model.remaining_fraction_milli else {
            continue;
        };
        // Skip internal/non-chat ids (tab completion, command models) that
        // jcode never exposes for switching.
        if model.id.starts_with("chat_") || model.id.starts_with("tab_") {
            continue;
        }
        let name = model
            .display_name
            .clone()
            .unwrap_or_else(|| model.id.clone());
        // The backend lists alias ids with identical display names; report
        // each visible model once.
        if !seen_names.insert(name.clone()) {
            continue;
        }
        let used_percent = ((1000u16.saturating_sub(remaining_milli)) as f32) / 10.0;
        limits.push(UsageLimit {
            name,
            usage_percent: used_percent,
            resets_at: model.reset_time.clone(),
        });
    }

    if limits.is_empty() && extra_info.is_empty() {
        return None;
    }

    Some(ProviderUsage {
        provider_name: "Antigravity".to_string(),
        limits,
        extra_info,
        hard_limit_reached: false,
        error: None,
        last_used_unix_secs: None,
    })
}

/// Gemini API key validity report. Google does not expose per-key spend or
/// quota through a public API, so this is a free `models.list` probe plus the
/// local activity ledger.
pub(super) async fn fetch_gemini_usage_report() -> Option<ProviderUsage> {
    let api_key = auth::gemini::api_key()?;

    let client = crate::provider::shared_http_client();
    let response = client
        .get("https://generativelanguage.googleapis.com/v1beta/models?pageSize=1")
        .header("x-goog-api-key", api_key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let status = match response {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                "valid".to_string()
            } else if status.as_u16() == 400 || status.as_u16() == 401 || status.as_u16() == 403 {
                format!("invalid or unauthorized ({})", status.as_u16())
            } else if status.as_u16() == 429 {
                "rate limited (429)".to_string()
            } else {
                format!("check failed ({})", status.as_u16())
            }
        }
        Err(e) => format!("check failed ({})", e),
    };

    Some(ProviderUsage {
        provider_name: "Google Gemini (API key)".to_string(),
        limits: Vec::new(),
        extra_info: vec![("Key status".to_string(), status)],
        hard_limit_reached: false,
        error: None,
        last_used_unix_secs: None,
    })
}
