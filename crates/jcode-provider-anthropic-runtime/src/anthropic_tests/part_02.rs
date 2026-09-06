// Prompt-caching behavior: message cache breakpoints, system-prompt block split,
// and the cross-turn two-marker sliding window.

#[test]
fn test_cache_breakpoint_no_messages() {
    let mut messages: Vec<ApiMessage> = vec![];
    add_message_cache_breakpoint(&mut messages);
    // Should not panic, just return early
    assert!(messages.is_empty());
}

#[test]
fn test_cache_breakpoint_too_few_messages() {
    let mut messages = vec![
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "World".to_string(),
                cache_control: None,
            }],
        },
    ];
    add_message_cache_breakpoint(&mut messages);
    // With only 2 messages, should not add cache control
    for msg in &messages {
        for block in &msg.content {
            if let ApiContentBlock::Text { cache_control, .. } = block {
                assert!(cache_control.is_none());
            }
        }
    }
}

#[test]
fn test_cache_breakpoint_adds_to_assistant_message() {
    let mut messages = vec![
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Identity".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "assistant".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Hi there!".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "How are you?".to_string(),
                cache_control: None,
            }],
        },
    ];

    add_message_cache_breakpoint(&mut messages);

    // Assistant message (index 2) should have cache_control
    if let ApiContentBlock::Text { cache_control, .. } = &messages[2].content[0] {
        assert!(cache_control.is_some());
    } else {
        panic!("Expected Text block");
    }

    // Other messages should NOT have cache_control
    for (i, msg) in messages.iter().enumerate() {
        if i == 2 {
            continue; // Skip the assistant message we just checked
        }
        for block in &msg.content {
            if let ApiContentBlock::Text { cache_control, .. } = block {
                assert!(
                    cache_control.is_none(),
                    "Message {} should not have cache_control",
                    i
                );
            }
        }
    }
}

#[test]
fn test_cache_breakpoint_finds_text_in_mixed_content() {
    // Assistant message with tool_use followed by text
    let mut messages = vec![
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Identity".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Run a command".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "assistant".to_string(),
            content: vec![
                ApiContentBlock::Text {
                    text: "Running command...".to_string(),
                    cache_control: None,
                },
                ApiContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"command": "ls"}),
                    cache_control: None,
                },
            ],
        },
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Thanks".to_string(),
                cache_control: None,
            }],
        },
    ];

    add_message_cache_breakpoint(&mut messages);

    // The last block (ToolUse) in the assistant message should have cache_control
    // (we prefer the last block for maximum cache coverage)
    let assistant_msg = &messages[2];
    let has_cached_block = assistant_msg.content.iter().any(|block| {
        matches!(
            block,
            ApiContentBlock::ToolUse {
                cache_control: Some(_),
                ..
            }
        )
    });
    assert!(
        has_cached_block,
        "Should have added cache_control to last block (ToolUse) in assistant message"
    );
}

#[test]
fn test_system_param_split_oauth() {
    let static_content = "This is static content";
    let dynamic_content = "This is dynamic content";

    let result = build_system_param_split(static_content, dynamic_content, true);

    if let Some(ApiSystem::Blocks(blocks)) = result {
        // Should have 4 blocks: identity, notice, static (cached), dynamic (not cached)
        assert_eq!(blocks.len(), 4);

        // Block 0: identity (no cache)
        assert!(blocks[0].cache_control.is_none());

        // Block 1: notice (no cache)
        assert!(blocks[1].cache_control.is_none());

        // Block 2: static (cached)
        assert!(blocks[2].cache_control.is_some());
        assert!(blocks[2].text.contains("static"));

        // Block 3: dynamic (not cached)
        assert!(blocks[3].cache_control.is_none());
        assert!(blocks[3].text.contains("dynamic"));
    } else {
        panic!("Expected Blocks variant");
    }
}

#[test]
fn test_system_param_split_non_oauth() {
    let static_content = "This is static content";
    let dynamic_content = "This is dynamic content";

    let result = build_system_param_split(static_content, dynamic_content, false);

    if let Some(ApiSystem::Blocks(blocks)) = result {
        // Should have 2 blocks: static (cached), dynamic (not cached)
        assert_eq!(blocks.len(), 2);

        // Block 0: static (cached)
        assert!(blocks[0].cache_control.is_some());

        // Block 1: dynamic (not cached)
        assert!(blocks[1].cache_control.is_none());
    } else {
        panic!("Expected Blocks variant");
    }
}

#[test]
fn test_cache_one_exchange_single_marker() {
    // Turn 2: only one assistant reply exists → one marker (WRITE only)
    let mut messages = build_conversation(1);
    add_message_cache_breakpoint(&mut messages);

    let indices = cached_message_indices(&messages);
    assert_eq!(indices.len(), 1, "One assistant message → one cache marker");
    // The assistant message is at index 2 (identity=0, user=1, assistant=2, user=3)
    assert_eq!(indices[0], 2);
}

#[test]
fn test_cache_two_exchanges_two_markers() {
    // Turn 3: two assistant replies → two markers (READ prev + WRITE new)
    let mut messages = build_conversation(2);
    // identity=0, user=1, assistant=2, user=3, assistant=4, user=5
    add_message_cache_breakpoint(&mut messages);

    let indices = cached_message_indices(&messages);
    assert_eq!(
        indices.len(),
        2,
        "Two assistant messages → two cache markers"
    );
    assert!(
        indices.contains(&2),
        "Second-to-last assistant (READ marker) at index 2"
    );
    assert!(
        indices.contains(&4),
        "Last assistant (WRITE marker) at index 4"
    );
}

#[test]
fn test_cache_many_exchanges_still_two_markers() {
    // 10 exchanges → still only 2 markers (within the 4-breakpoint API limit)
    let mut messages = build_conversation(10);
    add_message_cache_breakpoint(&mut messages);

    let count = count_message_cache_breakpoints(&messages);
    assert_eq!(
        count, 2,
        "Should always place exactly 2 markers regardless of conversation length"
    );
}

#[test]
fn test_cache_cross_turn_read_marker_preserved() {
    // THE KEY REGRESSION TEST: simulates turn N → turn N+1 and verifies that the
    // assistant message from turn N still has cache_control in the turn N+1 request.
    // Without this, the turn N cache snapshot is written but never read.

    // Turn 2: one assistant reply
    let mut turn2 = build_conversation(1);
    // identity=0, user=1, assistant=2, user=3
    add_message_cache_breakpoint(&mut turn2);
    let turn2_cached = cached_message_indices(&turn2);
    assert_eq!(
        turn2_cached,
        vec![2],
        "Turn 2: cache marker at assistant index 2"
    );

    // The content of the assistant message from turn 2 (what gets written to cache)
    let cached_text = match &turn2[2].content[0] {
        ApiContentBlock::Text { text, .. } => text.clone(),
        _ => panic!("Expected text block"),
    };

    // Turn 3: same conversation + one more exchange (assistant[2] is now second-to-last)
    let mut turn3 = build_conversation(2);
    // identity=0, user=1, assistant=2(same as before), user=3, assistant=4(new), user=5
    add_message_cache_breakpoint(&mut turn3);
    let turn3_cached = cached_message_indices(&turn3);

    // CRITICAL: assistant at index 2 MUST still have cache_control in turn 3,
    // so Anthropic can serve a cache READ hit for the turn-2 snapshot.
    assert!(
        turn3_cached.contains(&2),
        "Turn 3 MUST keep cache_control on the turn-2 assistant message (index 2) \
             so Anthropic can serve a cache_read hit. Without this, turn-2's cache is \
             written but never read, wasting cache_creation tokens every turn."
    );
    assert!(
        turn3_cached.contains(&4),
        "Turn 3 must add cache_control on the new assistant message (index 4) to \
             write a fresh cache snapshot for turn 4 to read"
    );

    // Verify it's actually the same content (same assistant message, not a different one)
    match &turn3[2].content[0] {
        ApiContentBlock::Text {
            text,
            cache_control,
        } => {
            assert_eq!(text, &cached_text);
            assert!(cache_control.is_some(), "Must have cache_control set");
        }
        _ => panic!("Expected text block"),
    }
}

#[test]
fn test_cache_non_oauth_path_gets_breakpoints() {
    // Non-OAuth path should now also get conversation cache breakpoints
    // (previously it returned early without calling add_message_cache_breakpoint)
    let messages = vec![
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "assistant".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Hi there!".to_string(),
                cache_control: None,
            }],
        },
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Follow-up".to_string(),
                cache_control: None,
            }],
        },
    ];

    let result = format_messages_with_identity(messages, false);
    let indices = cached_message_indices(&result);
    assert_eq!(
        indices,
        vec![1],
        "Non-OAuth path should add cache breakpoint to assistant message"
    );
}

#[test]
fn test_cache_total_breakpoints_within_api_limit() {
    // Anthropic allows at most 4 cache_control parameters per request total
    // (system blocks + tool definitions + message blocks).
    // System: 1 (static block) + Tools: 1 (last tool) + Messages: up to 2 = 4 max.
    // This test verifies messages never exceed 2 breakpoints.
    for exchanges in 1..=20 {
        let mut messages = build_conversation(exchanges);
        add_message_cache_breakpoint(&mut messages);
        let count = count_message_cache_breakpoints(&messages);
        assert!(
            count <= 2,
            "Conversation with {} exchanges produced {} message breakpoints, exceeding \
                 the 2-message budget (system+tools use the other 2 of Anthropic's 4-limit)",
            exchanges,
            count
        );
    }
}
