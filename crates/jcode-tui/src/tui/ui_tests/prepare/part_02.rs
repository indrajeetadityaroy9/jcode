// Live batch progress rows: soft wrap, centered padding, spinner, and placement.
#[test]
fn test_prepare_messages_live_batch_rows_do_not_soft_wrap_on_narrow_width() {
    let state = TestState {
        display_messages: vec![DisplayMessage::user("build it")],
        status: ProcessingStatus::RunningTool("batch".to_string()),
        anim_elapsed: 0.0,
        batch_progress: Some(crate::bus::BatchProgress {
            session_id: "s".to_string(),
            tool_call_id: "tc".to_string(),
            total: 1,
            completed: 0,
            last_completed: None,
            running: vec![ToolCall {
                id: "batch-1-bash".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({
                    "command": "cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width -- --nocapture"
                }),
                intent: None,
                thought_signature: None,
            }],
            subcalls: vec![crate::bus::BatchSubcallProgress {
                index: 1,
                tool_call: ToolCall {
                    id: "batch-1-bash".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({
                        "command": "cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width -- --nocapture"
                    }),
                    intent: None,
                    thought_signature: None,
                },
                state: crate::bus::BatchSubcallState::Running,
            }],
        }),
        ..Default::default()
    };

    let prepared = prepare::prepare_messages(&state, 34, 20);
    let rendered: Vec<String> = prepared
        .materialize_all_lines()
        .iter()
        .map(extract_line_text)
        .collect();

    let batch_rows: Vec<&String> = rendered
        .iter()
        .filter(|line| line.contains("batch") || line.contains("bash $ cargo"))
        .collect();
    assert!(batch_rows.len() >= 2, "rendered={rendered:?}");
    assert!(
        batch_rows.iter().all(|line| line.width() <= 33),
        "rendered={rendered:?}"
    );
    assert!(
        batch_rows.iter().any(|line| line.contains('…')),
        "rendered={rendered:?}"
    );
}

#[test]
fn test_prepare_messages_centered_live_batch_rows_keep_dedicated_padding_span() {
    let state = TestState {
        centered_mode: true,
        display_messages: vec![DisplayMessage::user("build it")],
        status: ProcessingStatus::RunningTool("batch".to_string()),
        anim_elapsed: 0.0,
        batch_progress: Some(crate::bus::BatchProgress {
            session_id: "s".to_string(),
            tool_call_id: "tc".to_string(),
            total: 1,
            completed: 0,
            last_completed: None,
            running: vec![ToolCall {
                id: "batch-1-bash".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({
                    "command": "cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width -- --nocapture --exact with-extra-flags-and-output-to-stretch-the-line"
                }),
                intent: None,
                thought_signature: None,
            }],
            subcalls: vec![crate::bus::BatchSubcallProgress {
                index: 1,
                tool_call: ToolCall {
                    id: "batch-1-bash".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({
                        "command": "cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width -- --nocapture --exact with-extra-flags-and-output-to-stretch-the-line"
                    }),
                    intent: None,
                    thought_signature: None,
                },
                state: crate::bus::BatchSubcallState::Running,
            }],
        }),
        ..Default::default()
    };

    let prepared = prepare::prepare_messages(&state, 120, 20);
    let prepared_lines = prepared.materialize_all_lines();
    let batch_rows: Vec<&Line<'static>> = prepared_lines
        .iter()
        .filter(|line| {
            let text = extract_line_text(line);
            text.contains("batch") || text.contains("bash")
        })
        .collect();
    let rendered: Vec<String> = batch_rows
        .iter()
        .map(|line| extract_line_text(line))
        .collect();

    assert!(batch_rows.len() >= 2, "rendered={rendered:?}");
    for line in batch_rows {
        let Some(first_span) = line.spans.first() else {
            panic!("missing spans: {rendered:?}");
        };
        assert!(
            !first_span.content.is_empty() && first_span.content.chars().all(|ch| ch == ' '),
            "expected a dedicated padding span for centered live batch rows: {rendered:?}"
        );
    }
}

#[test]
fn test_prepare_messages_shows_live_batch_progress_in_chat_history() {
    let state = TestState {
        display_messages: vec![DisplayMessage {
            role: "user".to_string(),
            content: "build it".to_string(),
            tool_calls: vec![],
            duration_secs: None,
            title: None,
            tool_data: None,
        }],
        status: ProcessingStatus::RunningTool("batch".to_string()),
        anim_elapsed: 0.0,
        batch_progress: Some(crate::bus::BatchProgress {
            session_id: "s".to_string(),
            tool_call_id: "tc".to_string(),
            total: 2,
            completed: 1,
            last_completed: Some("read".to_string()),
            running: vec![ToolCall {
                id: "batch-2-bash".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "cargo build --release --workspace"}),
                intent: None,
                thought_signature: None,
            }],
            subcalls: vec![
                crate::bus::BatchSubcallProgress {
                    index: 1,
                    tool_call: ToolCall {
                        id: "batch-1-read".to_string(),
                        name: "read".to_string(),
                        input: serde_json::json!({"file_path": "Cargo.toml"}),
                        intent: None,
                        thought_signature: None,
                    },
                    state: crate::bus::BatchSubcallState::Succeeded,
                },
                crate::bus::BatchSubcallProgress {
                    index: 2,
                    tool_call: ToolCall {
                        id: "batch-2-bash".to_string(),
                        name: "bash".to_string(),
                        input: serde_json::json!({"command": "cargo build --release --workspace"}),
                        intent: None,
                        thought_signature: None,
                    },
                    state: crate::bus::BatchSubcallState::Running,
                },
            ],
        }),
        ..Default::default()
    };

    let prepared = prepare::prepare_messages(&state, 100, 30);
    let rendered: Vec<String> = prepared
        .materialize_all_lines()
        .iter()
        .map(extract_line_text)
        .collect();

    assert!(
        rendered
            .iter()
            .any(|line| line.contains("⠋ batch · 1/2 done")),
        "missing live batch header in {:?}",
        rendered
    );
    assert!(
        rendered.iter().any(|line| line.contains("… 1 completed")),
        "missing completed subcall summary in {:?}",
        rendered
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("⠋ bash $ cargo build --release --workspace")),
        "missing running batch subcall in {:?}",
        rendered
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("#1") && !line.contains("#2")),
        "live batch rows should align with completed rows in {:?}",
        rendered
    );
}

#[test]
fn test_prepare_messages_places_live_batch_after_committed_assistant_text() {
    let _guard = crate::storage::lock_test_env();
    clear_test_render_state_for_tests();
    let state = TestState {
        display_messages: vec![
            DisplayMessage::user("build it"),
            DisplayMessage::assistant("Let me inspect the relevant files first."),
        ],
        status: ProcessingStatus::RunningTool("batch".to_string()),
        anim_elapsed: 0.0,
        batch_progress: Some(crate::bus::BatchProgress {
            session_id: "s".to_string(),
            tool_call_id: "tc".to_string(),
            total: 1,
            completed: 0,
            last_completed: None,
            running: vec![ToolCall {
                id: "batch-1-read".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file_path": "src/main.rs"}),
                intent: None,
                thought_signature: None,
            }],
            subcalls: vec![crate::bus::BatchSubcallProgress {
                index: 1,
                tool_call: ToolCall {
                    id: "batch-1-read".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({"file_path": "src/main.rs"}),
                    intent: None,
                    thought_signature: None,
                },
                state: crate::bus::BatchSubcallState::Running,
            }],
        }),
        ..Default::default()
    };

    let prepared = prepare::prepare_messages(&state, 100, 30);
    let rendered: Vec<String> = prepared
        .materialize_all_lines()
        .iter()
        .map(extract_line_text)
        .collect();

    let assistant_idx = rendered
        .iter()
        .position(|line| line.contains("Let me inspect the relevant files first."))
        .expect("missing assistant text");
    let batch_idx = rendered
        .iter()
        .position(|line| line.contains("batch · 0/1 done"))
        .expect("missing live batch progress");

    assert!(
        assistant_idx < batch_idx,
        "assistant text should render before live batch block in {:?}",
        rendered
    );
}

#[test]
fn test_prepare_messages_live_batch_spinner_advances_between_frames() {
    let batch_progress = crate::bus::BatchProgress {
        session_id: "s".to_string(),
        tool_call_id: "tc".to_string(),
        total: 1,
        completed: 0,
        last_completed: None,
        running: vec![ToolCall {
            id: "batch-1-bash".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "sleep 1"}),
            intent: None,
            thought_signature: None,
        }],
        subcalls: vec![crate::bus::BatchSubcallProgress {
            index: 1,
            tool_call: ToolCall {
                id: "batch-1-bash".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "sleep 1"}),
                intent: None,
                thought_signature: None,
            },
            state: crate::bus::BatchSubcallState::Running,
        }],
    };

    let first = TestState {
        status: ProcessingStatus::RunningTool("batch".to_string()),
        anim_elapsed: 0.0,
        batch_progress: Some(batch_progress.clone()),
        ..Default::default()
    };
    let second = TestState {
        status: ProcessingStatus::RunningTool("batch".to_string()),
        anim_elapsed: 0.1,
        batch_progress: Some(batch_progress),
        ..Default::default()
    };

    let first_rendered: Vec<String> = prepare::prepare_messages(&first, 100, 20)
        .materialize_all_lines()
        .iter()
        .map(extract_line_text)
        .collect();
    let second_rendered: Vec<String> = prepare::prepare_messages(&second, 100, 20)
        .materialize_all_lines()
        .iter()
        .map(extract_line_text)
        .collect();

    assert!(
        first_rendered
            .iter()
            .any(|line| line.contains("⠋ batch · 0/1 done")),
        "expected first spinner frame in {:?}",
        first_rendered
    );
    assert!(
        second_rendered
            .iter()
            .any(|line| line.contains("⠙ batch · 0/1 done")),
        "expected second spinner frame in {:?}",
        second_rendered
    );
    assert_ne!(
        first_rendered, second_rendered,
        "batch progress should rerender as spinner advances"
    );
}

#[test]
fn test_prepare_messages_live_batch_centered_mode_uses_left_aligned_padding() {
    let state = TestState {
        centered_mode: true,
        status: ProcessingStatus::RunningTool("batch".to_string()),
        anim_elapsed: 0.0,
        batch_progress: Some(crate::bus::BatchProgress {
            session_id: "s".to_string(),
            tool_call_id: "tc".to_string(),
            total: 1,
            completed: 0,
            last_completed: None,
            running: vec![ToolCall {
                id: "batch-1-read".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file_path": "Cargo.toml"}),
                intent: None,
                thought_signature: None,
            }],
            subcalls: vec![crate::bus::BatchSubcallProgress {
                index: 1,
                tool_call: ToolCall {
                    id: "batch-1-read".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({"file_path": "Cargo.toml"}),
                    intent: None,
                    thought_signature: None,
                },
                state: crate::bus::BatchSubcallState::Running,
            }],
        }),
        ..Default::default()
    };

    let prepared = prepare::prepare_messages(&state, 100, 20);
    let prepared_lines = prepared.materialize_all_lines();
    let batch_lines: Vec<&Line<'static>> = prepared_lines
        .iter()
        .filter(|line| {
            let text = extract_line_text(line);
            text.contains("batch") || text.contains("Cargo.toml")
        })
        .collect();

    assert!(!batch_lines.is_empty(), "expected centered batch lines");
    for line in batch_lines {
        assert_eq!(
            line.alignment,
            Some(Alignment::Left),
            "centered live batch lines should be left-aligned with padding"
        );
        assert!(
            line.spans
                .first()
                .is_some_and(|span| span.content.starts_with(' ')),
            "centered live batch lines should start with padding"
        );
    }
}
