// Client attachment, working-directory validation, reload gating and event
// fan-out. Split out of client_lifecycle_tests.rs to keep each file under the
// test-size budget; the first half of that file covers busy-agent and cancel
// control-request handling.
#[test]
fn initial_subscribe_requires_an_absolute_client_working_dir() {
    for invalid in [None, Some(""), Some("relative/project")] {
        let error = initial_subscribe_working_dir(&subscribe_request(invalid))
            .expect_err("invalid client cwd must be rejected before session creation");
        assert!(error.contains("working_dir") || error.contains("working directory"));
    }

    let absolute = std::env::temp_dir().join("jcode-client-project");
    assert_eq!(
        initial_subscribe_working_dir(&subscribe_request(absolute.to_str()))
            .expect("absolute client cwd"),
        absolute.to_string_lossy()
    );

    let error = initial_subscribe_working_dir(&Request::GetState { id: 2 })
        .expect_err("stateful requests must not create an unbound session");
    assert!(error.contains("must Subscribe"));
}

#[tokio::test]
async fn new_client_agent_stamps_client_cwd_into_initial_context() {
    let provider: Arc<dyn Provider> = Arc::new(CompleteImmediatelyProvider);
    let registry = Registry::new(Arc::clone(&provider)).await;
    let client_cwd = std::env::temp_dir().join("jcode-authoritative-client-project");
    let client_cwd = client_cwd.to_string_lossy();
    let agent = Agent::new_with_initial_working_dir(provider, registry, Some(&client_cwd));

    assert_eq!(agent.working_dir(), Some(client_cwd.as_ref()));
    let context = agent.messages()[0].content_preview();
    assert!(
        context.contains(&format!("Working directory: {client_cwd}")),
        "initial context must be created from the client cwd: {context}"
    );
}

#[test]
fn server_reload_starting_is_true_only_for_recent_starting_marker() {
    let _guard = crate::storage::lock_test_env();
    let _runtime = IsolatedRuntimeDir::new();

    assert!(!server_reload_starting());

    crate::server::write_reload_state(
        "reload-lifecycle-test",
        "test-hash",
        crate::server::ReloadPhase::Starting,
        Some("session_test_reload".to_string()),
    );
    assert!(server_reload_starting());

    crate::server::write_reload_state(
        "reload-lifecycle-test",
        "test-hash",
        crate::server::ReloadPhase::SocketReady,
        Some("session_test_reload".to_string()),
    );
    assert!(!server_reload_starting());
}

#[test]
fn reload_starting_rejects_new_turn_without_spawning_processing_task() {
    let _guard = crate::storage::lock_test_env();
    let _runtime = IsolatedRuntimeDir::new();
    crate::server::write_reload_state(
        "reload-lifecycle-starting",
        "test-hash",
        crate::server::ReloadPhase::Starting,
        Some("session_guard".to_string()),
    );

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let forked = Arc::new(AtomicBool::new(false));
        let provider: Arc<dyn Provider> = Arc::new(PanicOnForkProvider {
            forked: Arc::clone(&forked),
        });
        let registry = Registry::new(Arc::clone(&provider)).await;
        let mut session =
            crate::session::Session::create_with_id("session_guard".to_string(), None, None);
        session.model = Some("panic-on-fork".to_string());
        let agent = Arc::new(Mutex::new(Agent::new_with_session(
            provider, registry, session, None,
        )));

        let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();
        let (processing_done_tx, mut processing_done_rx) = mpsc::unbounded_channel();
        let mut client_is_processing = false;
        let mut processing_message_id = None;
        let mut processing_session_id = None;
        let mut processing_task = None;
        let swarm_members = Arc::new(RwLock::new(HashMap::new()));
        let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
        let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
        let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (swarm_event_tx, _) = broadcast::channel(8);

        start_processing_message(
            ProcessingMessage {
                id: 42,
                content: "do not start during reload".to_string(),
                images: Vec::new(),
                system_reminder: None,
                active_skill: None,
            },
            "session_guard",
            &mut ProcessingState {
                client_is_processing: &mut client_is_processing,
                message_id: &mut processing_message_id,
                session_id: &mut processing_session_id,
                task: &mut processing_task,
            },
            &agent,
            &client_event_tx,
            &processing_done_tx,
            Vec::new(),
            &SwarmStatusRefs {
                members: &swarm_members,
                swarms_by_id: &swarms_by_id,
                event_history: &event_history,
                event_counter: &event_counter,
                event_tx: &swarm_event_tx,
            },
        )
        .await;

        let event = client_event_rx
            .recv()
            .await
            .expect("reload event should be sent to client");
        assert!(matches!(event, ServerEvent::Reloading { new_socket: None }));
        assert!(
            client_event_rx.try_recv().is_err(),
            "reload guard should only emit the reload notification"
        );
        assert!(!client_is_processing);
        assert_eq!(processing_message_id, None);
        assert_eq!(processing_session_id, None);
        assert!(processing_task.is_none());
        assert!(processing_done_rx.try_recv().is_err());
        assert!(
            !forked.load(Ordering::SeqCst),
            "rejecting during reload should not fork or invoke provider work"
        );
    });
}

#[tokio::test]
async fn client_initiated_turn_fans_out_stream_and_terminal_events_to_live_attachments() {
    let _guard = crate::storage::lock_test_env();
    let _runtime = IsolatedRuntimeDir::new();
    let session_id = "session_live_attachment_fanout";

    let provider: Arc<dyn Provider> = Arc::new(FanoutStreamProvider);
    let registry = Registry::new(Arc::clone(&provider)).await;
    let mut session = crate::session::Session::create_with_id(session_id.to_string(), None, None);
    session.model = Some("fanout-stream".to_string());
    let agent = Arc::new(Mutex::new(Agent::new_with_session(
        provider, registry, session, None,
    )));

    let (origin_tx, mut origin_rx) = mpsc::unbounded_channel::<ServerEvent>();
    let (attached_tx, mut attached_rx) = mpsc::unbounded_channel::<ServerEvent>();
    let swarm_members = Arc::new(RwLock::new(HashMap::from([(
        session_id.to_string(),
        SwarmMember {
            session_id: session_id.to_string(),
            event_tx: origin_tx.clone(),
            event_txs: HashMap::from([("origin".to_string(), origin_tx.clone())]),
            working_dir: None,
            swarm_id: None,
            swarm_enabled: false,
            status: "ready".to_string(),
            detail: None,
            task_label: None,
            friendly_name: None,
            report_back_to_session_id: None,
            latest_completion_report: None,
            role: "agent".to_string(),
            joined_at: Instant::now(),
            last_status_change: Instant::now(),
            is_headless: false,
            output_tail: None,
            todo_progress: None,
            todo_items: Vec::new(),
            runtime: crate::protocol::SwarmMemberRuntime::default(),
        },
    )])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
    let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (swarm_event_tx, _) = broadcast::channel(8);
    let (processing_done_tx, mut processing_done_rx) = mpsc::unbounded_channel();
    let mut client_is_processing = false;
    let mut processing_message_id = None;
    let mut processing_session_id = None;
    let mut processing_task = None;

    start_processing_message(
        ProcessingMessage {
            id: 479,
            content: "stream to every attachment".to_string(),
            images: Vec::new(),
            system_reminder: None,
            active_skill: None,
        },
        session_id,
        &mut ProcessingState {
            client_is_processing: &mut client_is_processing,
            message_id: &mut processing_message_id,
            session_id: &mut processing_session_id,
            task: &mut processing_task,
        },
        &agent,
        &origin_tx,
        &processing_done_tx,
        Vec::new(),
        &SwarmStatusRefs {
            members: &swarm_members,
            swarms_by_id: &swarms_by_id,
            event_history: &event_history,
            event_counter: &event_counter,
            event_tx: &swarm_event_tx,
        },
    )
    .await;

    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), origin_rx.recv())
            .await
            .expect("origin should receive the initial stream event promptly")
            .expect("origin event channel should remain open");
        if matches!(event, ServerEvent::TextDelta { ref text } if text == "before attach") {
            break;
        }
    }

    crate::server::register_session_event_sender(
        &swarm_members,
        session_id,
        "attached",
        attached_tx,
    )
    .await;

    for rx in [&mut origin_rx, &mut attached_rx] {
        let mut saw_post_attach_delta = false;
        let mut saw_message_end = false;
        let mut saw_done = false;
        while !saw_post_attach_delta || !saw_message_end || !saw_done {
            let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("attachment should receive streamed event promptly")
                .expect("attachment event channel should remain open");
            saw_post_attach_delta |= matches!(
                event,
                ServerEvent::TextDelta { ref text } if text == "after attach"
            );
            if matches!(event, ServerEvent::MessageEnd { .. }) {
                assert!(!saw_done, "MessageEnd must precede the terminal Done event");
                saw_message_end = true;
            }
            saw_done |= matches!(event, ServerEvent::Done { id: 479 });
        }
    }

    let (done_id, result, _) =
        tokio::time::timeout(Duration::from_secs(2), processing_done_rx.recv())
            .await
            .expect("processing should complete promptly")
            .expect("processing completion channel should remain open");
    assert_eq!(done_id, 479);
    result.expect("turn should complete successfully");

    if let Some(handle) = processing_task.take() {
        handle.await.expect("processing task join");
    }
}

#[test]
fn accepted_reload_recovery_continuation_marks_intent_delivered() -> anyhow::Result<()> {
    let _lock = crate::storage::lock_test_env();
    let _env = IsolatedReloadRecoveryEnv::new();
    let session_id = "session_accepted_reload_recovery";
    let continuation = "stored continuation accepted by server";

    super::super::reload_recovery::persist_intent(
        "reload-accepted-continuation",
        session_id,
        super::super::reload_recovery::ReloadRecoveryRole::InterruptedPeer,
        crate::server::reload_context::ReloadRecoveryDirective {
            reconnect_notice: Some("stored notice".to_string()),
            continuation_message: continuation.to_string(),
        },
        "synthetic accepted continuation test",
    )?;
    assert!(super::super::reload_recovery::has_pending_for_session(
        session_id
    ));

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let provider: Arc<dyn Provider> = Arc::new(CompleteImmediatelyProvider);
        let registry = Registry::new(Arc::clone(&provider)).await;
        let mut session =
            crate::session::Session::create_with_id(session_id.to_string(), None, None);
        session.model = Some("complete-immediately".to_string());
        let agent = Arc::new(Mutex::new(Agent::new_with_session(
            provider, registry, session, None,
        )));

        let (client_event_tx, _client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();
        let (processing_done_tx, mut processing_done_rx) = mpsc::unbounded_channel();
        let mut client_is_processing = false;
        let mut processing_message_id = None;
        let mut processing_session_id = None;
        let mut processing_task = None;
        let swarm_members = Arc::new(RwLock::new(HashMap::new()));
        let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
        let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
        let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (swarm_event_tx, _) = broadcast::channel(8);

        start_processing_message(
            ProcessingMessage {
                id: 77,
                content: "continue after reload".to_string(),
                images: Vec::new(),
                system_reminder: Some(continuation.to_string()),
                active_skill: None,
            },
            session_id,
            &mut ProcessingState {
                client_is_processing: &mut client_is_processing,
                message_id: &mut processing_message_id,
                session_id: &mut processing_session_id,
                task: &mut processing_task,
            },
            &agent,
            &client_event_tx,
            &processing_done_tx,
            Vec::new(),
            &SwarmStatusRefs {
                members: &swarm_members,
                swarms_by_id: &swarms_by_id,
                event_history: &event_history,
                event_counter: &event_counter,
                event_tx: &swarm_event_tx,
            },
        )
        .await;

        assert!(client_is_processing);
        assert_eq!(processing_message_id, Some(77));
        assert_eq!(processing_session_id.as_deref(), Some(session_id));
        assert!(processing_task.is_some());
        assert!(
            !super::super::reload_recovery::has_pending_for_session(session_id),
            "server acceptance of the exact hidden continuation should consume the durable intent"
        );

        let (done_id, result, _report) =
            tokio::time::timeout(std::time::Duration::from_secs(5), processing_done_rx.recv())
                .await
                .expect("processing task should finish")
                .expect("processing task should report completion");
        assert_eq!(done_id, 77);
        result?;
        if let Some(handle) = processing_task.take() {
            handle.await.expect("processing task join");
        }
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn reload_starting_rejects_new_turns_for_multiple_sessions() {
    let _guard = crate::storage::lock_test_env();
    let _runtime = IsolatedRuntimeDir::new();
    crate::server::write_reload_state(
        "reload-lifecycle-multi-starting",
        "test-hash",
        crate::server::ReloadPhase::Starting,
        Some("session_alpha".to_string()),
    );

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let forked = Arc::new(AtomicBool::new(false));
        let provider: Arc<dyn Provider> = Arc::new(PanicOnForkProvider {
            forked: Arc::clone(&forked),
        });
        let registry = Registry::new(Arc::clone(&provider)).await;
        let swarm_members = Arc::new(RwLock::new(HashMap::new()));
        let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
        let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
        let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (swarm_event_tx, _) = broadcast::channel(8);

        for (message_id, session_id) in [
            (101, "session_alpha"),
            (102, "session_beta"),
            (103, "session_gamma"),
        ] {
            let mut session =
                crate::session::Session::create_with_id(session_id.to_string(), None, None);
            session.model = Some("panic-on-fork".to_string());
            let agent = Arc::new(Mutex::new(Agent::new_with_session(
                Arc::clone(&provider),
                registry.clone(),
                session,
                None,
            )));

            let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel::<ServerEvent>();
            let (processing_done_tx, mut processing_done_rx) = mpsc::unbounded_channel();
            let mut client_is_processing = false;
            let mut processing_message_id = None;
            let mut processing_session_id = None;
            let mut processing_task = None;

            start_processing_message(
                ProcessingMessage {
                    id: message_id,
                    content: format!("do not start {session_id} during reload"),
                    images: Vec::new(),
                    system_reminder: None,
                    active_skill: None,
                },
                session_id,
                &mut ProcessingState {
                    client_is_processing: &mut client_is_processing,
                    message_id: &mut processing_message_id,
                    session_id: &mut processing_session_id,
                    task: &mut processing_task,
                },
                &agent,
                &client_event_tx,
                &processing_done_tx,
                Vec::new(),
                &SwarmStatusRefs {
                    members: &swarm_members,
                    swarms_by_id: &swarms_by_id,
                    event_history: &event_history,
                    event_counter: &event_counter,
                    event_tx: &swarm_event_tx,
                },
            )
            .await;

            let event = tokio::time::timeout(
                std::time::Duration::from_millis(250),
                client_event_rx.recv(),
            )
            .await
            .expect("reload guard should emit promptly for every session")
            .expect("reload event should be sent to client");
            assert!(
                matches!(event, ServerEvent::Reloading { new_socket: None }),
                "expected Reloading event for {session_id}, got {event:?}"
            );
            assert!(
                client_event_rx.try_recv().is_err(),
                "reload guard should only emit one reload notification for {session_id}"
            );
            assert!(
                !client_is_processing,
                "{session_id} should not enter processing during reload"
            );
            assert_eq!(processing_message_id, None);
            assert_eq!(processing_session_id, None);
            assert!(
                processing_task.is_none(),
                "{session_id} should not spawn a processing task during reload"
            );
            assert!(processing_done_rx.try_recv().is_err());
        }

        assert!(
            !forked.load(Ordering::SeqCst),
            "rejecting multiple sessions during reload should not fork or invoke provider work"
        );
    });
}

#[tokio::test]
async fn lightweight_comm_request_skips_full_session_initialization() {
    let (server_stream, client_stream) = crate::transport::Stream::pair().expect("socket pair");
    let forked = Arc::new(AtomicBool::new(false));
    let provider_template: Arc<dyn Provider> = Arc::new(PanicOnForkProvider {
        forked: Arc::clone(&forked),
    });

    let sessions: SessionAgents = Arc::new(RwLock::new(HashMap::new()));
    let global_session_id = Arc::new(RwLock::new(String::new()));
    let client_count = Arc::new(RwLock::new(0usize));
    let client_connections = Arc::new(RwLock::new(HashMap::new()));
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::new()));
    let shared_context = Arc::new(RwLock::new(HashMap::new()));
    let swarm_plans = Arc::new(RwLock::new(HashMap::new()));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::new()));
    let file_touch = FileTouchService::new();
    let channel_subscriptions = Arc::new(RwLock::new(HashMap::new()));
    let channel_subscriptions_by_session = Arc::new(RwLock::new(HashMap::new()));
    let client_debug_state = Arc::new(RwLock::new(ClientDebugState::default()));
    let (_debug_response_tx, _) = broadcast::channel(8);
    let event_history = Arc::new(RwLock::new(std::collections::VecDeque::new()));
    let event_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (swarm_event_tx, _) = broadcast::channel(8);
    let shutdown_signals = Arc::new(RwLock::new(HashMap::new()));
    let soft_interrupt_queues: SessionInterruptQueues = Arc::new(RwLock::new(HashMap::new()));
    let mcp_pool = Arc::new(crate::mcp::SharedMcpPool::from_default_config());

    let server_task = tokio::spawn(handle_client(
        server_stream,
        Arc::clone(&sessions),
        provider_template,
        global_session_id,
        client_count,
        Arc::clone(&client_connections),
        swarm_members,
        swarms_by_id,
        shared_context,
        swarm_plans,
        swarm_coordinators,
        file_touch,
        channel_subscriptions,
        channel_subscriptions_by_session,
        client_debug_state,
        _debug_response_tx,
        event_history,
        event_counter,
        swarm_event_tx,
        "jcode-test".to_string(),
        "🧪".to_string(),
        mcp_pool,
        shutdown_signals,
        soft_interrupt_queues,
        AwaitMembersRuntime::default(),
        SwarmMutationRuntime::default(),
    ));

    let (client_reader, mut client_writer) = client_stream.into_split();
    let mut client_reader = BufReader::new(client_reader);
    let request = Request::CommList {
        id: 7,
        session_id: "not-in-swarm".to_string(),
    };
    let payload = serde_json::to_string(&request).expect("serialize request") + "\n";
    client_writer
        .write_all(payload.as_bytes())
        .await
        .expect("write request");

    let mut line = String::new();
    client_reader
        .read_line(&mut line)
        .await
        .expect("read ack bytes");
    let ack = decode_request_or_event(&line);
    assert!(matches!(ack, ServerEvent::Ack { id: 7 }));

    line.clear();
    client_reader
        .read_line(&mut line)
        .await
        .expect("read terminal response");
    let response = decode_request_or_event(&line);
    match response {
        ServerEvent::Error { id, message, .. } => {
            assert_eq!(id, 7);
            assert!(message.contains("Not in a swarm"));
        }
        other => panic!("expected error response, got {other:?}"),
    }

    drop(client_writer);
    server_task
        .await
        .expect("server task join")
        .expect("server task result");

    assert!(
        !forked.load(Ordering::SeqCst),
        "lightweight control request should not fork a provider"
    );
    assert!(
        client_connections.read().await.is_empty(),
        "lightweight control request should not register a live client session"
    );
    assert!(
        sessions.read().await.is_empty(),
        "lightweight control request should not allocate a live agent session"
    );
}

fn decode_request_or_event(line: &str) -> ServerEvent {
    serde_json::from_str(line.trim()).expect("decode server event")
}
