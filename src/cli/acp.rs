use super::dispatch;
use super::provider_init::ProviderChoice;
use crate::protocol::{Request, ServerEvent};
use crate::transport::{ReadHalf, WriteHalf};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

mod event_mapper;
mod runtime;
mod session_config;

use event_mapper::EventMapper;
use session_config::{
    CONFIG_ID_EFFORT, CONFIG_ID_MODEL, acp_available_commands, available_efforts,
    insert_session_configuration, session_config_options, wait_for_effort_changed,
    wait_for_model_changed,
};

const ACP_PROTOCOL_VERSION: u64 = 1;

const JSONRPC_PARSE_ERROR: i64 = -32700;
const JSONRPC_INVALID_REQUEST: i64 = -32600;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;
const JSONRPC_INTERNAL_ERROR: i64 = -32603;
const JSONRPC_SERVER_ERROR: i64 = -32000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcpProfile {
    Standard,
    Extended,
    Full,
}

impl AcpProfile {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "extended" => Self::Extended,
            "full" => Self::Full,
            _ => Self::Standard,
        }
    }

    fn is_extended(self) -> bool {
        matches!(self, Self::Extended | Self::Full)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Extended => "extended",
            Self::Full => "full",
        }
    }
}

#[derive(Debug)]
struct JsonRpcMessage {
    id: Option<Value>,
    method: Option<String>,
    params: Value,
}

impl JsonRpcMessage {
    fn parse(line: &str) -> std::result::Result<Self, (i64, String)> {
        let value: Value =
            serde_json::from_str(line).map_err(|err| (JSONRPC_PARSE_ERROR, err.to_string()))?;
        let object = value.as_object().ok_or_else(|| {
            (
                JSONRPC_INVALID_REQUEST,
                "JSON-RPC message must be an object".to_string(),
            )
        })?;
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err((
                JSONRPC_INVALID_REQUEST,
                "JSON-RPC message must include jsonrpc=\"2.0\"".to_string(),
            ));
        }
        Ok(Self {
            id: object.get("id").cloned(),
            method: object
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_string),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        })
    }
}

struct DaemonSession {
    session_id: String,
    reader: Mutex<BufReader<ReadHalf>>,
    writer: Mutex<WriteHalf>,
    next_request_id: AtomicU64,
    active_prompt_id: Mutex<Option<u64>>,
    prompt_running: AtomicBool,
    ui_state: Mutex<SessionUiState>,
}

/// Session-scoped provider/model state used to surface ACP `configOptions`
/// (model selector, reasoning effort) and `usage_update` notifications.
#[derive(Clone, Debug, Default)]
struct SessionUiState {
    provider_name: Option<String>,
    model: Option<String>,
    available_models: Vec<String>,
    reasoning_effort: Option<String>,
}

#[derive(Debug, Default)]
struct TurnUsage {
    reported: bool,
    input_tokens: u64,
    output_tokens: u64,
    cached_read_tokens: Option<u64>,
    cached_write_tokens: Option<u64>,
}

impl TurnUsage {
    fn add(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cached_read_tokens: Option<u64>,
        cached_write_tokens: Option<u64>,
    ) {
        self.reported = true;
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        add_optional_tokens(&mut self.cached_read_tokens, cached_read_tokens);
        add_optional_tokens(&mut self.cached_write_tokens, cached_write_tokens);
    }

    fn to_acp(&self) -> Option<Value> {
        if !self.reported {
            return None;
        }

        let total_tokens = self
            .input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cached_read_tokens.unwrap_or(0))
            .saturating_add(self.cached_write_tokens.unwrap_or(0));
        // Built as a `Map` rather than a `json!` object that is then downcast
        // with `as_object_mut()`: the downcast can only ever succeed here, so
        // the `expect` it required was an unreachable panic guarding nothing.
        let mut usage = serde_json::Map::new();
        usage.insert("totalTokens".to_string(), json!(total_tokens));
        usage.insert("inputTokens".to_string(), json!(self.input_tokens));
        usage.insert("outputTokens".to_string(), json!(self.output_tokens));
        if let Some(tokens) = self.cached_read_tokens {
            usage.insert("cachedReadTokens".to_string(), json!(tokens));
        }
        if let Some(tokens) = self.cached_write_tokens {
            usage.insert("cachedWriteTokens".to_string(), json!(tokens));
        }
        Some(Value::Object(usage))
    }
}

fn add_optional_tokens(total: &mut Option<u64>, tokens: Option<u64>) {
    if let Some(tokens) = tokens {
        *total = Some(total.unwrap_or(0).saturating_add(tokens));
    }
}

fn prompt_response(stop_reason: &str, usage: &TurnUsage) -> Value {
    let mut response = serde_json::Map::new();
    response.insert("stopReason".to_string(), json!(stop_reason));
    if let Some(usage) = usage.to_acp() {
        response.insert("usage".to_string(), usage);
    }
    Value::Object(response)
}

impl SessionUiState {
    fn from_history_fields(
        provider_name: Option<String>,
        provider_model: Option<String>,
        available_models: Vec<String>,
        reasoning_effort: Option<String>,
    ) -> Self {
        Self {
            provider_name,
            model: provider_model,
            available_models,
            reasoning_effort,
        }
    }

    fn context_limit(&self) -> u64 {
        self.model
            .as_deref()
            .and_then(|model| {
                crate::provider::context_limit_for_model_with_provider(
                    model,
                    self.provider_name.as_deref(),
                )
            })
            .unwrap_or(crate::provider::DEFAULT_CONTEXT_LIMIT) as u64
    }
}

impl DaemonSession {
    fn new(session_id: String, reader: ReadHalf, writer: WriteHalf, next_request_id: u64) -> Self {
        Self {
            session_id,
            reader: Mutex::new(BufReader::new(reader)),
            writer: Mutex::new(writer),
            next_request_id: AtomicU64::new(next_request_id),
            active_prompt_id: Mutex::new(None),
            prompt_running: AtomicBool::new(false),
            ui_state: Mutex::new(SessionUiState::default()),
        }
    }

    fn with_ui_state(self, state: SessionUiState) -> Self {
        Self {
            ui_state: Mutex::new(state),
            ..self
        }
    }

    fn next_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send(&self, request: &Request) -> Result<()> {
        let mut json = serde_json::to_string(request)?;
        json.push('\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(json.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    async fn read_event(&self) -> Result<ServerEvent> {
        let mut line = String::new();
        let mut reader = self.reader.lock().await;
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("Jcode daemon disconnected");
        }
        let event = serde_json::from_str(&line)
            .with_context(|| format!("failed to decode Jcode daemon event: {}", line.trim_end()))?;
        Ok(event)
    }
}

#[derive(Clone)]
struct AcpRuntime {
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    sessions: Arc<Mutex<HashMap<String, Arc<DaemonSession>>>>,
    profile: AcpProfile,
    provider_choice: ProviderChoice,
    model: Option<String>,
    provider_profile: Option<String>,
}

async fn cleanup_prompt_state(session: &DaemonSession) {
    {
        let mut active = session.active_prompt_id.lock().await;
        *active = None;
    }
    session.prompt_running.store(false, Ordering::SeqCst);
}

async fn wait_for_done(session: &DaemonSession, request_id: u64) -> Result<()> {
    loop {
        match session.read_event().await? {
            ServerEvent::Ack { .. } => {}
            ServerEvent::Done { id } if id == request_id => return Ok(()),
            ServerEvent::Error { id, message, .. } if id == request_id => anyhow::bail!(message),
            _ => {}
        }
    }
}

async fn request_history(session: &DaemonSession) -> Result<ServerEvent> {
    let id = session.next_id();
    session.send(&Request::GetHistory { id }).await?;
    loop {
        match session.read_event().await? {
            ServerEvent::Ack { .. } => {}
            event @ ServerEvent::History { id: event_id, .. } if event_id == id => {
                return Ok(event);
            }
            ServerEvent::Error {
                id: event_id,
                message,
                ..
            } if event_id == id => anyhow::bail!(message),
            _ => {}
        }
    }
}

async fn request_model_catalog(session: &DaemonSession) -> Result<ServerEvent> {
    let id = session.next_id();
    session.send(&Request::GetModelCatalog { id }).await?;
    loop {
        match session.read_event().await? {
            ServerEvent::Ack { .. } => {}
            event @ ServerEvent::History { id: event_id, .. } if event_id == id => {
                return Ok(event);
            }
            ServerEvent::Error {
                id: event_id,
                message,
                ..
            } if event_id == id => anyhow::bail!(message),
            _ => {}
        }
    }
}

fn parse_json_object(input: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(input).ok()?;
    value.as_object()?;
    Some(value)
}

fn compatibility_option_value(
    params: &Value,
    value_fields: &[&str],
    method: &str,
) -> std::result::Result<String, String> {
    if !params.is_object() {
        return Err(format!("{method} params must be an object"));
    }
    value_fields
        .iter()
        .find_map(|field| {
            params
                .get(*field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            format!(
                "{method} requires a non-empty string {}",
                value_fields.join(" or ")
            )
        })
}

fn initialize_result(_params: &Value, profile: AcpProfile) -> Value {
    // The `_params` binding is deliberate: we only speak exactly
    // ACP_PROTOCOL_VERSION, so the response pins to our version regardless of
    // the `protocolVersion` the client requested.
    let protocol_version = ACP_PROTOCOL_VERSION;
    let mut agent_capabilities = json!({
        "loadSession": true,
        "promptCapabilities": {
            "image": true,
            "audio": false,
            "embeddedContext": true,
        },
        "mcpCapabilities": {
            "http": false,
            "sse": false,
        },
        "sessionCapabilities": {
            "close": {},
            "resume": {},
        }
    });

    if profile.is_extended()
        && let Some(object) = agent_capabilities.as_object_mut()
    {
        object.insert(
            "_meta".to_string(),
            json!({
                "jcode": {
                    "profile": profile.as_str(),
                    "extensions": ["raw_server_event"]
                }
            }),
        );
    }

    json!({
        "protocolVersion": protocol_version,
        "agentCapabilities": agent_capabilities,
        "agentInfo": {
            "name": "jcode",
            "title": "Jcode",
            "version": jcode_build_meta::pkg_version(),
        },
        "authMethods": [],
    })
}

fn cwd_from_params(params: &Value) -> std::result::Result<PathBuf, String> {
    let cwd = match params.get("cwd").and_then(Value::as_str) {
        Some(cwd) if !cwd.trim().is_empty() => PathBuf::from(cwd),
        _ => std::env::current_dir().map_err(|err| err.to_string())?,
    };
    if !cwd.is_absolute() {
        return Err(format!("ACP cwd must be absolute: {}", cwd.display()));
    }
    Ok(cwd)
}

fn required_session_id(params: &Value) -> std::result::Result<String, String> {
    params
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Missing required sessionId".to_string())
}

fn validate_acp_mcp_servers(params: &Value) -> std::result::Result<(), String> {
    match params.get("mcpServers") {
        None | Some(Value::Null) => Ok(()),
        // Session-scoped MCP is not supported yet, but rejecting this required
        // ACP field prevents hosts with MCP servers from creating a session.
        Some(Value::Array(_)) => Ok(()),
        Some(_) => Err("ACP mcpServers must be an array".to_string()),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AcpSlashCommand {
    Model(Option<String>),
    Models,
    Effort(Option<String>),
}

fn parse_acp_slash_command(text: &str) -> Option<Result<AcpSlashCommand>> {
    // A leading space is the ACP client convention for escaping slash command
    // interpretation and sending the text to the model literally.
    let trimmed = text.trim_end();
    let body = trimmed.strip_prefix('/')?;
    let mut parts = body.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default();
    let argument = parts
        .next()
        .map(str::trim)
        .filter(|argument| !argument.is_empty())
        .map(str::to_string);
    match name {
        "model" => Some(Ok(AcpSlashCommand::Model(argument))),
        "models" if argument.is_none() => Some(Ok(AcpSlashCommand::Models)),
        "models" => Some(Err(anyhow::anyhow!("/models does not accept an argument"))),
        "effort" => Some(Ok(AcpSlashCommand::Effort(argument))),
        _ => None,
    }
}

fn format_model_catalog(current: Option<&str>, models: &[String]) -> String {
    if models.is_empty() {
        return match current {
            Some(current) => format!("Current model: `{current}`. No model catalog was reported."),
            None => "The active provider did not report a model catalog.".to_string(),
        };
    }
    let mut output = String::from("Available models:\n");
    for model in models {
        let selected = if Some(model.as_str()) == current {
            " (current)"
        } else {
            ""
        };
        output.push_str(&format!("- `{model}`{selected}\n"));
    }
    output.pop();
    output
}

fn prompt_from_params(
    params: &Value,
) -> std::result::Result<(String, Vec<(String, String)>), String> {
    let prompt = params
        .get("prompt")
        .and_then(Value::as_array)
        .ok_or_else(|| "Missing required prompt array".to_string())?;
    let mut text_parts = Vec::new();
    let mut images = Vec::new();

    for block in prompt {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    text_parts.push(text.to_string());
                }
            }
            Some("image") => {
                let mime_type = block
                    .get("mimeType")
                    .or_else(|| block.get("mime_type"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Image content block missing mimeType".to_string())?;
                let data = block
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Image content block missing data".to_string())?;
                images.push((mime_type.to_string(), data.to_string()));
            }
            Some("resource") => {
                if let Some(resource) = block.get("resource") {
                    text_parts.push(format_resource_block(resource));
                }
            }
            Some("resource_link") => {
                let uri = block.get("uri").and_then(Value::as_str).unwrap_or("");
                let name = block.get("name").and_then(Value::as_str).unwrap_or(uri);
                text_parts.push(format!("[Resource link: {name} <{uri}>]"));
            }
            Some(other) => {
                return Err(format!(
                    "Unsupported ACP prompt content block type: {other}"
                ));
            }
            None => return Err("Prompt content block missing type".to_string()),
        }
    }

    Ok((text_parts.join("\n\n"), images))
}

fn format_resource_block(resource: &Value) -> String {
    let uri = resource
        .get("uri")
        .and_then(Value::as_str)
        .unwrap_or("resource");
    if let Some(text) = resource.get("text").and_then(Value::as_str) {
        format!("[Embedded resource: {uri}]\n{text}")
    } else if let Some(blob) = resource.get("blob").and_then(Value::as_str) {
        let mime = resource
            .get("mimeType")
            .or_else(|| resource.get("mime_type"))
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream");
        format!(
            "[Embedded binary resource: {uri} ({mime}, {} base64 bytes)]",
            blob.len()
        )
    } else {
        format!("[Embedded resource: {uri}]")
    }
}

fn agent_message_chunk(text: String) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": {
            "type": "text",
            "text": text,
        }
    })
}

fn tool_title(name: &str) -> String {
    match name {
        "bash" => "Running shell command".to_string(),
        "read" => "Reading file".to_string(),
        "write" => "Writing file".to_string(),
        "edit" | "multiedit" | "patch" | "apply_patch" => "Editing files".to_string(),
        "agentgrep" | "grep" | "glob" | "ls" => "Searching workspace".to_string(),
        "webfetch" | "websearch" => "Fetching web content".to_string(),
        other => other.replace('_', " "),
    }
}

pub(crate) fn tool_kind(name: &str) -> &'static str {
    match name {
        "read" => "read",
        "write" | "edit" | "multiedit" | "patch" | "apply_patch" => "edit",
        "bash" | "bg" => "execute",
        "agentgrep" | "grep" | "glob" | "ls" | "session_search" | "conversation_search" => "search",
        "webfetch" | "websearch" | "codesearch" => "fetch",
        _ => "other",
    }
}

pub(crate) async fn run_acp_command(
    provider_choice: ProviderChoice,
    model: Option<String>,
    provider_profile: Option<String>,
    explicit_tool_profile: bool,
) -> Result<()> {
    crate::env::set_var("JCODE_NON_INTERACTIVE", "1");
    let acp_config = crate::config::config().acp.clone();
    if !explicit_tool_profile {
        crate::env::set_var("JCODE_TOOL_PROFILE", acp_config.tool_profile.trim());
        crate::config::invalidate_config_cache();
    }
    let profile = AcpProfile::parse(&acp_config.profile);
    AcpRuntime::new(profile, provider_choice, model, provider_profile)
        .run()
        .await
}

#[cfg(test)]
#[path = "acp_tests.rs"]
mod tests;
