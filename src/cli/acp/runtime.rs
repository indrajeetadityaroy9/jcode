//! The ACP request loop: JSON-RPC method dispatch and the per-turn streaming
//! bridge between an editor client and the jcode daemon.
//!
//! Split out of `acp.rs`, which now owns only the wire types, the protocol
//! constants and the parameter/response shaping the loop calls into.
//!
//! `use super::*` is deliberate here rather than a 20-line import list: this
//! module is one `impl` block lifted verbatim out of its parent and it reaches
//! for that parent's private types, free functions and struct fields
//! throughout.
use super::*;

impl AcpRuntime {
    pub(super) fn new(
        profile: AcpProfile,
        provider_choice: ProviderChoice,
        model: Option<String>,
        provider_profile: Option<String>,
    ) -> Self {
        Self {
            stdout: Arc::new(Mutex::new(tokio::io::stdout())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            profile,
            provider_choice,
            model,
            provider_profile,
        }
    }

    pub(super) async fn run(self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(());
            }
            if line.trim().is_empty() {
                continue;
            }

            let message = match JsonRpcMessage::parse(&line) {
                Ok(message) => message,
                Err((code, message)) => {
                    self.write_error_value(
                        Value::Null,
                        code,
                        format!("Invalid JSON-RPC request: {message}"),
                    )
                    .await?;
                    continue;
                }
            };

            self.handle_message(message).await?;
        }
    }

    async fn handle_message(&self, message: JsonRpcMessage) -> Result<()> {
        let Some(method) = message.method.as_deref() else {
            if let Some(id) = message.id {
                self.write_error_value(
                    id,
                    JSONRPC_INVALID_REQUEST,
                    "JSON-RPC request missing method".to_string(),
                )
                .await?;
            }
            return Ok(());
        };

        match method {
            "initialize" => {
                if let Some(id) = message.id {
                    self.write_result(id, initialize_result(&message.params, self.profile))
                        .await?;
                }
            }
            "session/new" => self.handle_session_new(message).await?,
            "session/load" => self.handle_session_load(message, true).await?,
            "session/resume" => self.handle_session_load(message, false).await?,
            "session/prompt" => self.handle_session_prompt(message).await?,
            "session/cancel" => self.handle_session_cancel(message).await?,
            "session/close" => self.handle_session_close(message).await?,
            "session/set_config_option" => self.handle_set_config_option(message).await?,
            "session/set_model" => {
                self.handle_compat_config_option(
                    message,
                    CONFIG_ID_MODEL,
                    &["modelId", "model"],
                    "session/set_model",
                )
                .await?
            }
            "session/set_reasoning_effort" => {
                self.handle_compat_config_option(
                    message,
                    CONFIG_ID_EFFORT,
                    &["effort", "reasoningEffort"],
                    "session/set_reasoning_effort",
                )
                .await?
            }
            _ if method.starts_with('_') => {
                if let Some(id) = message.id {
                    self.write_error_value(
                        id,
                        JSONRPC_METHOD_NOT_FOUND,
                        format!("Unsupported Jcode ACP extension method: {method}"),
                    )
                    .await?;
                }
            }
            _ => {
                if let Some(id) = message.id {
                    self.write_error_value(
                        id,
                        JSONRPC_METHOD_NOT_FOUND,
                        format!("Unsupported ACP method: {method}"),
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    async fn handle_session_new(&self, message: JsonRpcMessage) -> Result<()> {
        let Some(id) = message.id else {
            return Ok(());
        };
        let cwd = match cwd_from_params(&message.params) {
            Ok(cwd) => cwd,
            Err(err) => {
                self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                    .await?;
                return Ok(());
            }
        };
        if let Err(err) = validate_acp_mcp_servers(&message.params) {
            self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                .await?;
            return Ok(());
        }

        match self.create_new_session(cwd).await {
            Ok(session) => {
                let session_id = session.session_id.clone();
                let state = session.ui_state.lock().await.clone();
                self.sessions
                    .lock()
                    .await
                    .insert(session_id.clone(), Arc::new(session));
                let mut result = json!({ "sessionId": session_id });
                insert_session_configuration(&mut result, &state);
                self.write_result(id, result).await?;
                self.write_available_commands(&session_id).await?;
            }
            Err(err) => {
                self.write_error_value(
                    id,
                    JSONRPC_INTERNAL_ERROR,
                    format!("Failed to create Jcode session: {err:#}"),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn handle_session_load(
        &self,
        message: JsonRpcMessage,
        replay_history: bool,
    ) -> Result<()> {
        let Some(id) = message.id else {
            return Ok(());
        };
        let session_id = match required_session_id(&message.params) {
            Ok(session_id) => session_id,
            Err(err) => {
                self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                    .await?;
                return Ok(());
            }
        };
        let cwd = match cwd_from_params(&message.params) {
            Ok(cwd) => cwd,
            Err(err) => {
                self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                    .await?;
                return Ok(());
            }
        };
        if let Err(err) = validate_acp_mcp_servers(&message.params) {
            self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                .await?;
            return Ok(());
        }

        match self
            .attach_existing_session(session_id.clone(), cwd, replay_history)
            .await
        {
            Ok(session) => {
                let state = session.ui_state.lock().await.clone();
                self.sessions
                    .lock()
                    .await
                    .insert(session.session_id.clone(), Arc::new(session));
                let mut result = json!({});
                insert_session_configuration(&mut result, &state);
                self.write_result(id, result).await?;
                self.write_available_commands(&session_id).await?;
            }
            Err(err) => {
                self.write_error_value(
                    id,
                    JSONRPC_INTERNAL_ERROR,
                    format!("Failed to attach Jcode session '{session_id}': {err:#}"),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn handle_session_prompt(&self, message: JsonRpcMessage) -> Result<()> {
        let Some(id) = message.id else {
            return Ok(());
        };
        let session_id = match required_session_id(&message.params) {
            Ok(session_id) => session_id,
            Err(err) => {
                self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                    .await?;
                return Ok(());
            }
        };
        let (text, images) = match prompt_from_params(&message.params) {
            Ok(prompt) => prompt,
            Err(err) => {
                self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                    .await?;
                return Ok(());
            }
        };
        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(&session_id).cloned()
        };
        let Some(session) = session else {
            self.write_error_value(
                id,
                JSONRPC_INVALID_PARAMS,
                format!("Unknown ACP session id: {session_id}"),
            )
            .await?;
            return Ok(());
        };

        if session
            .prompt_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.write_error_value(
                id,
                JSONRPC_SERVER_ERROR,
                format!("Session {session_id} is already processing a prompt"),
            )
            .await?;
            return Ok(());
        }

        let runtime = self.clone();
        tokio::spawn(async move {
            let result = runtime.run_prompt(id.clone(), session, text, images).await;
            if let Err(err) = result {
                let _ = runtime
                    .write_error_value(
                        id,
                        JSONRPC_INTERNAL_ERROR,
                        format!("Prompt failed: {err:#}"),
                    )
                    .await;
            }
        });
        Ok(())
    }

    async fn handle_session_cancel(&self, message: JsonRpcMessage) -> Result<()> {
        let session_id = match required_session_id(&message.params) {
            Ok(session_id) => session_id,
            Err(err) => {
                if let Some(id) = message.id {
                    self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                        .await?;
                }
                return Ok(());
            }
        };
        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(&session_id).cloned()
        };
        if let Some(session) = session {
            let cancel_id = session.next_id();
            let _ = session.send(&Request::Cancel { id: cancel_id }).await;
        }
        if let Some(id) = message.id {
            self.write_result(id, json!({})).await?;
        }
        Ok(())
    }

    async fn handle_session_close(&self, message: JsonRpcMessage) -> Result<()> {
        let Some(id) = message.id else {
            return Ok(());
        };
        let session_id = match required_session_id(&message.params) {
            Ok(session_id) => session_id,
            Err(err) => {
                self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                    .await?;
                return Ok(());
            }
        };
        if let Some(session) = self.sessions.lock().await.remove(&session_id) {
            let cancel_id = session.next_id();
            let _ = session.send(&Request::Cancel { id: cancel_id }).await;
        }
        self.write_result(id, json!({})).await?;
        Ok(())
    }

    async fn handle_set_config_option(&self, message: JsonRpcMessage) -> Result<()> {
        let Some(id) = message.id else {
            return Ok(());
        };
        let session_id = match required_session_id(&message.params) {
            Ok(session_id) => session_id,
            Err(err) => {
                self.write_error_value(id, JSONRPC_INVALID_PARAMS, err)
                    .await?;
                return Ok(());
            }
        };
        let config_id = message
            .params
            .get("configId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let value = message
            .params
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (Some(config_id), Some(value)) = (config_id, value) else {
            self.write_error_value(
                id,
                JSONRPC_INVALID_PARAMS,
                "session/set_config_option requires string configId and value".to_string(),
            )
            .await?;
            return Ok(());
        };

        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(&session_id).cloned()
        };
        let Some(session) = session else {
            self.write_error_value(
                id,
                JSONRPC_INVALID_PARAMS,
                format!("Unknown ACP session id: {session_id}"),
            )
            .await?;
            return Ok(());
        };
        if session.prompt_running.load(Ordering::SeqCst) {
            self.write_error_value(
                id,
                JSONRPC_SERVER_ERROR,
                format!("Session {session_id} is processing a prompt; retry when it finishes"),
            )
            .await?;
            return Ok(());
        }

        let request_id = session.next_id();
        let apply_result = match config_id.as_str() {
            CONFIG_ID_MODEL => {
                session
                    .send(&Request::SetModel {
                        id: request_id,
                        model: value.clone(),
                    })
                    .await?;
                wait_for_model_changed(&session, request_id).await
            }
            CONFIG_ID_EFFORT => {
                session
                    .send(&Request::SetReasoningEffort {
                        id: request_id,
                        effort: value.clone(),
                        target_session_id: None,
                    })
                    .await?;
                wait_for_effort_changed(&session, request_id).await
            }
            other => Err(anyhow::anyhow!("Unknown config option id: {other}")),
        };

        match apply_result {
            Ok(()) => {
                let config_options = session_config_options(&*session.ui_state.lock().await);
                // The spec requires the full option set in the response itself.
                self.write_result(id, json!({ "configOptions": config_options }))
                    .await?;
                self.write_notification(
                    "session/update",
                    json!({
                        "sessionId": session.session_id,
                        "update": {
                            "sessionUpdate": "config_option_update",
                            "configOptions": config_options,
                        }
                    }),
                )
                .await?;
            }
            Err(err) => {
                self.write_error_value(
                    id,
                    JSONRPC_SERVER_ERROR,
                    format!("Failed to set {config_id}: {err:#}"),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Compatibility entry points used by ACP hosts that implemented the
    /// pre-configOptions model and reasoning controls. Normalize them through
    /// the standard config option path so both interfaces stay in sync.
    async fn handle_compat_config_option(
        &self,
        mut message: JsonRpcMessage,
        config_id: &str,
        value_fields: &[&str],
        method: &str,
    ) -> Result<()> {
        let value = match compatibility_option_value(&message.params, value_fields, method) {
            Ok(value) => value,
            Err(error) => {
                if let Some(id) = message.id {
                    self.write_error_value(id, JSONRPC_INVALID_PARAMS, error)
                        .await?;
                }
                return Ok(());
            }
        };
        let Some(params) = message.params.as_object_mut() else {
            if let Some(id) = message.id {
                self.write_error_value(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    format!("{method} params must be an object"),
                )
                .await?;
            }
            return Ok(());
        };
        params.insert("configId".to_string(), Value::String(config_id.to_string()));
        params.insert("value".to_string(), Value::String(value));
        self.handle_set_config_option(message).await
    }

    async fn write_available_commands(&self, session_id: &str) -> Result<()> {
        self.write_notification(
            "session/update",
            json!({
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": acp_available_commands(),
                }
            }),
        )
        .await
    }

    async fn ensure_daemon(&self) -> Result<()> {
        if dispatch::server_is_running().await {
            return Ok(());
        }
        dispatch::spawn_server(
            &self.provider_choice,
            self.model.as_deref(),
            self.provider_profile.as_deref(),
        )
        .await
    }

    async fn connect_daemon(&self) -> Result<(ReadHalf, WriteHalf)> {
        self.ensure_daemon().await?;
        let stream = crate::server::connect_socket(&crate::server::socket_path()).await?;
        Ok(stream.into_split())
    }

    async fn create_new_session(&self, cwd: PathBuf) -> Result<DaemonSession> {
        let (reader, writer) = self.connect_daemon().await?;
        let session = DaemonSession::new(String::new(), reader, writer, 2);
        let subscribe_id = 1;
        session
            .send(&Request::Subscribe {
                id: subscribe_id,
                working_dir: Some(cwd.display().to_string()),
                target_session_id: None,
                client_instance_id: Some("acp".to_string()),
                client_has_local_history: false,
                allow_session_takeover: false,
                terminal_env: crate::terminal_launch::snapshot_client_terminal_env(),
            })
            .await?;
        wait_for_done(&session, subscribe_id).await?;
        let history = request_history(&session).await?;
        let (session_id, ui_state) = match history {
            ServerEvent::History {
                session_id,
                provider_name,
                provider_model,
                available_models,
                reasoning_effort,
                ..
            } => (
                session_id,
                SessionUiState::from_history_fields(
                    provider_name,
                    provider_model,
                    available_models,
                    reasoning_effort,
                ),
            ),
            other => anyhow::bail!("expected history after session creation, got {other:?}"),
        };
        Ok(DaemonSession::new(
            session_id,
            session.reader.into_inner().into_inner(),
            session.writer.into_inner(),
            session.next_request_id.load(Ordering::Relaxed),
        )
        .with_ui_state(ui_state))
    }

    async fn attach_existing_session(
        &self,
        target_session_id: String,
        cwd: PathBuf,
        replay_history: bool,
    ) -> Result<DaemonSession> {
        let (reader, writer) = self.connect_daemon().await?;
        let session = DaemonSession::new(String::new(), reader, writer, 2);
        let resume_id = 1;
        session
            .send(&Request::Subscribe {
                id: resume_id,
                working_dir: Some(cwd.display().to_string()),
                target_session_id: Some(target_session_id.clone()),
                client_instance_id: Some("acp".to_string()),
                client_has_local_history: false,
                allow_session_takeover: false,
                terminal_env: crate::terminal_launch::snapshot_client_terminal_env(),
            })
            .await?;

        let mut attached_id = target_session_id;
        let mut ui_state = SessionUiState::default();
        loop {
            let event = session.read_event().await?;
            match event {
                ServerEvent::Ack { .. } => {}
                ServerEvent::History {
                    session_id,
                    messages,
                    provider_name,
                    provider_model,
                    available_models,
                    reasoning_effort,
                    ..
                } => {
                    attached_id = session_id.clone();
                    ui_state = SessionUiState::from_history_fields(
                        provider_name,
                        provider_model,
                        available_models,
                        reasoning_effort,
                    );
                    if replay_history {
                        self.replay_history(&session_id, messages).await?;
                    }
                }
                ServerEvent::Done { id } if id == resume_id => break,
                ServerEvent::Error { id, message, .. } if id == resume_id => {
                    anyhow::bail!(message);
                }
                other => {
                    if self.profile.is_extended() {
                        self.write_jcode_extension_event(&attached_id, &other)
                            .await?;
                    }
                }
            }
        }

        Ok(DaemonSession::new(
            attached_id,
            session.reader.into_inner().into_inner(),
            session.writer.into_inner(),
            session.next_request_id.load(Ordering::Relaxed),
        )
        .with_ui_state(ui_state))
    }

    async fn replay_history(
        &self,
        session_id: &str,
        messages: Vec<crate::protocol::HistoryMessage>,
    ) -> Result<()> {
        for message in messages {
            let update_name = match message.role.as_str() {
                "user" => "user_message_chunk",
                "assistant" => "agent_message_chunk",
                _ => "agent_message_chunk",
            };
            self.write_notification(
                "session/update",
                json!({
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": update_name,
                        "content": {
                            "type": "text",
                            "text": message.content,
                        }
                    }
                }),
            )
            .await?;
        }
        Ok(())
    }

    async fn run_prompt(
        &self,
        rpc_id: Value,
        session: Arc<DaemonSession>,
        text: String,
        images: Vec<(String, String)>,
    ) -> Result<()> {
        if let Some(command) = parse_acp_slash_command(&text) {
            let response = match command {
                Ok(command) => self.run_session_command(&session, command).await,
                Err(err) => Err(err),
            };
            cleanup_prompt_state(&session).await;
            let response = response?;
            self.write_notification(
                "session/update",
                json!({
                    "sessionId": session.session_id,
                    "update": agent_message_chunk(response),
                }),
            )
            .await?;
            self.write_result(rpc_id, prompt_response("end_turn", &TurnUsage::default()))
                .await?;
            return Ok(());
        }

        let prompt_id = session.next_id();
        {
            let mut active = session.active_prompt_id.lock().await;
            *active = Some(prompt_id);
        }

        let send_result = session
            .send(&Request::Message {
                id: prompt_id,
                content: text,
                images,
                system_reminder: None,
                active_skill: None,
                no_reply: false,
            })
            .await;
        if let Err(err) = send_result {
            cleanup_prompt_state(&session).await;
            return Err(err);
        }

        let mut mapper = EventMapper::new(self.profile);
        let mut stop_reason = "end_turn".to_string();
        let mut turn_usage = TurnUsage::default();
        loop {
            let event = match session.read_event().await {
                Ok(event) => event,
                Err(err) => {
                    cleanup_prompt_state(&session).await;
                    return Err(err);
                }
            };
            if self.profile.is_extended() {
                self.write_jcode_extension_event(&session.session_id, &event)
                    .await?;
            }
            match event {
                ServerEvent::Ack { .. } => {}
                ServerEvent::Done { id } if id == prompt_id => break,
                ServerEvent::Interrupted => {
                    stop_reason = "cancelled".to_string();
                }
                ServerEvent::Error { id, message, .. } if id == prompt_id => {
                    cleanup_prompt_state(&session).await;
                    self.write_error_value(rpc_id, JSONRPC_SERVER_ERROR, message)
                        .await?;
                    return Ok(());
                }
                ServerEvent::TokenUsage {
                    input,
                    output,
                    cache_read_input,
                    cache_creation_input,
                } => {
                    turn_usage.add(input, output, cache_read_input, cache_creation_input);
                    let (provider_name, context_limit) = {
                        let state = session.ui_state.lock().await;
                        (
                            state.provider_name.clone().unwrap_or_default(),
                            state.context_limit(),
                        )
                    };
                    let used = crate::compaction::effective_context_tokens_from_usage(
                        &provider_name,
                        input,
                        cache_read_input,
                        cache_creation_input,
                    );
                    self.write_notification(
                        "session/update",
                        json!({
                            "sessionId": session.session_id,
                            "update": {
                                "sessionUpdate": "usage_update",
                                "used": used,
                                "size": context_limit,
                            }
                        }),
                    )
                    .await?;
                }
                ServerEvent::ModelChanged {
                    model,
                    provider_name,
                    error,
                    ..
                } => {
                    // Mid-prompt model changes happen on provider failover;
                    // keep the selector in sync.
                    if error.is_none() {
                        let config_options = {
                            let mut state = session.ui_state.lock().await;
                            state.model = Some(model);
                            if provider_name.is_some() {
                                state.provider_name = provider_name;
                            }
                            session_config_options(&state)
                        };
                        if !config_options.is_empty() {
                            self.write_notification(
                                "session/update",
                                json!({
                                    "sessionId": session.session_id,
                                    "update": {
                                        "sessionUpdate": "config_option_update",
                                        "configOptions": config_options,
                                    }
                                }),
                            )
                            .await?;
                        }
                    }
                }
                other => {
                    for update in mapper.map_event(other) {
                        self.write_notification(
                            "session/update",
                            json!({
                                "sessionId": session.session_id,
                                "update": update,
                            }),
                        )
                        .await?;
                    }
                }
            }
        }

        cleanup_prompt_state(&session).await;
        self.write_result(rpc_id, prompt_response(&stop_reason, &turn_usage))
            .await?;
        Ok(())
    }

    async fn run_session_command(
        &self,
        session: &DaemonSession,
        command: AcpSlashCommand,
    ) -> Result<String> {
        match command {
            AcpSlashCommand::Model(None) => {
                let state = session.ui_state.lock().await;
                Ok(match state.model.as_deref() {
                    Some(model) => format!("Current model: `{model}`"),
                    None => "The daemon did not report a current model.".to_string(),
                })
            }
            AcpSlashCommand::Model(Some(model)) => {
                let id = session.next_id();
                session
                    .send(&Request::SetModel {
                        id,
                        model: model.clone(),
                    })
                    .await?;
                wait_for_model_changed(session, id).await?;
                self.write_config_option_update(session).await?;
                let selected = session.ui_state.lock().await.model.clone().unwrap_or(model);
                Ok(format!("Switched model to `{selected}`."))
            }
            AcpSlashCommand::Models => {
                let event = request_model_catalog(session).await?;
                let ServerEvent::History {
                    provider_name,
                    provider_model,
                    available_models,
                    ..
                } = event
                else {
                    unreachable!("request_model_catalog only returns history")
                };
                let (current, models) = {
                    let mut state = session.ui_state.lock().await;
                    if provider_name.is_some() {
                        state.provider_name = provider_name;
                    }
                    if provider_model.is_some() {
                        state.model = provider_model;
                    }
                    state.available_models = available_models;
                    (state.model.clone(), state.available_models.clone())
                };
                self.write_config_option_update(session).await?;
                Ok(format_model_catalog(current.as_deref(), &models))
            }
            AcpSlashCommand::Effort(None) => {
                let state = session.ui_state.lock().await;
                let current = state
                    .reasoning_effort
                    .as_deref()
                    .unwrap_or("provider default");
                let available = available_efforts(&state);
                if available.is_empty() {
                    Ok(format!("Current reasoning effort: `{current}`."))
                } else {
                    Ok(format!(
                        "Current reasoning effort: `{current}`. Available: {}.",
                        available
                            .iter()
                            .map(|effort| format!("`{effort}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            AcpSlashCommand::Effort(Some(effort)) => {
                let id = session.next_id();
                session
                    .send(&Request::SetReasoningEffort {
                        id,
                        effort: effort.clone(),
                        target_session_id: None,
                    })
                    .await?;
                wait_for_effort_changed(session, id).await?;
                self.write_config_option_update(session).await?;
                let selected = session
                    .ui_state
                    .lock()
                    .await
                    .reasoning_effort
                    .clone()
                    .unwrap_or(effort);
                Ok(format!("Set reasoning effort to `{selected}`."))
            }
        }
    }

    async fn write_config_option_update(&self, session: &DaemonSession) -> Result<()> {
        let config_options = session_config_options(&*session.ui_state.lock().await);
        self.write_notification(
            "session/update",
            json!({
                "sessionId": session.session_id,
                "update": {
                    "sessionUpdate": "config_option_update",
                    "configOptions": config_options,
                }
            }),
        )
        .await
    }

    async fn write_result(&self, id: Value, result: Value) -> Result<()> {
        self.write_value(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .await
    }

    async fn write_error_value(&self, id: Value, code: i64, message: String) -> Result<()> {
        self.write_value(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
            }
        }))
        .await
    }

    async fn write_notification(&self, method: &str, params: Value) -> Result<()> {
        self.write_value(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn write_jcode_extension_event(
        &self,
        session_id: &str,
        event: &ServerEvent,
    ) -> Result<()> {
        self.write_notification(
            "_jcode/server_event",
            json!({
                "sessionId": session_id,
                "event": serde_json::to_value(event).unwrap_or(Value::Null),
            }),
        )
        .await
    }

    async fn write_value(&self, value: Value) -> Result<()> {
        let mut stdout = self.stdout.lock().await;
        let mut line = serde_json::to_string(&value)?;
        line.push('\n');
        stdout.write_all(line.as_bytes()).await?;
        stdout.flush().await?;
        Ok(())
    }
}
