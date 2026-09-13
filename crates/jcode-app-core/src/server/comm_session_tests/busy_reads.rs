// Swarm reads against a worker that is mid-turn. Included from
// `comm_session_tests.rs`, which owns the `use` list and the fixtures.

/// A worker holds its agent lock for the whole turn, so requiring that lock
/// made `summary` fail exactly while the worker was working - the one moment a
/// coordinator has a reason to ask. The summary is a pure function of the
/// transcript, so the persisted one answers instead of an error.
#[tokio::test]
async fn summary_reads_the_persisted_transcript_while_the_worker_is_busy() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let (requester, _req_rx) = member("session_coord_1_aaaa", Some("swarm-1"), "coordinator");
    let (worker, _worker_rx) = member("session_busy_2_bbbb", Some("swarm-1"), "agent");
    {
        let mut members = swarm_members.write().await;
        members.insert(requester.session_id.clone(), requester);
        members.insert(worker.session_id.clone(), worker);
    }

    let agent = test_agent_with_working_dir("session_busy_2_bbbb", "/tmp").await;
    let sessions = Arc::new(RwLock::new(HashMap::new()));
    sessions
        .write()
        .await
        .insert("session_busy_2_bbbb".to_string(), Arc::clone(&agent));
    // Persist a transcript containing one tool call, the way a working agent
    // journals its turn.
    let mut persisted =
        crate::session::Session::create_with_id("session_busy_2_bbbb".to_string(), None, None);
    persisted.model = Some("mock".to_string());
    persisted.append_stored_message(crate::session::StoredMessage {
        id: "msg-busy-summary".to_string(),
        role: crate::message::Role::Assistant,
        content: vec![crate::message::ContentBlock::ToolUse {
            id: "call-1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({ "command": "cargo test" }),
            thought_signature: None,
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    });
    persisted.save().expect("persist worker transcript");

    // Hold the lock for the whole call: this is what a mid-turn worker does.
    let _busy = agent.lock().await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    crate::server::comm_sync::handle_comm_summary(
        1,
        "session_coord_1_aaaa".to_string(),
        "session_busy_2_bbbb".to_string(),
        Some(10),
        &sessions,
        &swarm_members,
        &tx,
    )
    .await;

    match rx.try_recv() {
        Ok(ServerEvent::CommSummaryResponse { tool_calls, .. }) => {
            assert!(
                tool_calls.iter().any(|call| call.tool_name == "bash"),
                "the persisted transcript's tool calls must be reported: {tool_calls:?}"
            );
        }
        other => panic!("expected a summary from the persisted transcript, got {other:?}"),
    }
}
