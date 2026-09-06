// Copy/expand badge and selection-shortcut behaviour (split out of part_02.rs to keep each part under the test-size budget).
/// FULL end-to-end reproduction of the user's "clicking the image does
/// nothing" report. Unlike `test_click_on_inline_image_label_line_cycles_level`
/// (which records a synthetic `ChatFrame` snapshot directly), this drives the
/// *real* draw: a local App whose session carries a `read`-tool result image,
/// anchored into the transcript body, rendered through `terminal.draw()`, which
/// is what records the live copy-viewport snapshot. We then locate the rendered
/// image label line in the actual frame buffer and inject a real left click,
/// asserting the image size cycles. This exercises the body-anchored image path
/// (`render_images` -> `resolve_anchored_items` -> `anchored_image_lines`), the
/// path actually used in production, not the isolated `build_section` helper.
#[test]
fn test_real_draw_click_on_body_anchored_image_label_cycles_level() {
    use crate::message::{ContentBlock, Role};
    use crate::tui::ui::inline_image_ui::ImageExpandLevel;

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    assert!(!app.is_remote, "repro must use the local image render path");

    const TOOL_ID: &str = "read-shot-1";

    // Build a real transcript: user asks, assistant calls `read`, tool result
    // carries the screenshot image. This is exactly what produces a
    // body-anchored inline image with a `RenderedImageAnchor::ToolCall`.
    app.session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "read the screenshot".to_string(),
            cache_control: None,
        }],
    );
    app.session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: TOOL_ID.to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "shot.png"}),
            thought_signature: None,
        }],
    );
    app.session.add_message(
        Role::User,
        vec![
            ContentBlock::ToolResult {
                tool_use_id: TOOL_ID.to_string(),
                content: "read image".to_string(),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: REPRO_TINY_PNG_B64.to_string(),
            },
        ],
    );

    // Mirror the session into the display transcript the body renderer walks.
    app.display_messages = vec![
        DisplayMessage::user("read the screenshot"),
        DisplayMessage::tool(
            "read shot.png",
            crate::message::ToolCall {
                id: TOOL_ID.to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file_path": "shot.png"}),
                intent: None,
                thought_signature: None,
            },
        ),
    ];
    app.bump_display_messages_version();
    app.invalidate_side_pane_images_signature();
    app.pin_images = true;
    app.inline_images_visible = true;
    app.scroll_offset = 0;
    app.auto_scroll_paused = false;
    app.is_processing = false;
    app.status = ProcessingStatus::Idle;
    app.session.short_name = Some("test".to_string());

    // Sanity: the local render path must actually surface the anchored image.
    let images = <App as crate::tui::TuiState>::side_pane_images(&app);
    assert_eq!(
        images.len(),
        1,
        "session should render exactly one anchored tool image"
    );
    let image_id = {
        let img = &images[0];
        crate::tui::mermaid::inline_image_dims(&img.media_type, &img.data)
            .expect("tiny png should decode")
            .0
    };

    let backend = ratatui::backend::TestBackend::new(80, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");

    // REAL draw: this records the live copy-viewport snapshot used by clicks.
    let rendered = render_and_snap(&app, &mut terminal);
    assert!(
        rendered.contains("shot.png"),
        "image label line must render in the live frame, got:\n{rendered}"
    );

    // Find the label line in the actual buffer: scan rows for the row carrying
    // the image label, then click a cell inside the label text.
    let buf = terminal.backend().buffer();
    let area = *buf.area();
    let mut badge: Option<(u16, u16)> = None;
    'rows: for row in 0..area.height {
        let mut line = String::new();
        for col in 0..area.width {
            line.push_str(buf[(col, row)].symbol());
        }
        // The transcript also shows the tool-call row ("read shot.png"); the
        // image label row is the one that carries the show/hide badge keys.
        if !line.contains("shot.png") || !line.contains("[I]") {
            continue;
        }
        // Click the first cell of the label text (the hit-region is the whole
        // label line, so any cell on the row works).
        for col in 0..area.width {
            if buf[(col, row)].symbol() == "s" {
                badge = Some((col, row));
                break 'rows;
            }
        }
    }
    let (badge_col, badge_row) = badge.expect("image label cell should be visible in the frame");

    assert_eq!(
        app.image_expand_level(image_id),
        ImageExpandLevel::Fit,
        "image should start at Fit before any click"
    );

    // REAL click on the rendered label cell. A terminal delivers a *pair* of
    // events for one physical click: `Down` then `Up`. We must replay both, just
    // like the live event loop, or we silently skip the copy-selection state the
    // `Down` arms (which is exactly what the user's click goes through).
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: badge_col,
        row: badge_row,
        modifiers: KeyModifiers::empty(),
    });
    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: badge_col,
        row: badge_row,
        modifiers: KeyModifiers::empty(),
    });

    assert_eq!(
        app.image_expand_level(image_id),
        ImageExpandLevel::Large,
        "clicking the rendered image label must cycle Fit -> Large \
         (this is the exact path the user reported as broken)"
    );
    assert_eq!(app.status_notice(), Some("Image size: large".to_string()));
}

/// The inline-image placeholder marker row must never reach the terminal as
/// text. It used to be drawn black-on-black and relied on staying invisible,
/// but terminal-side compositing (kitty translucent background + contrast
/// compositing) and selection highlighting can recolor it, leaking raw
/// "IIMG:<hash>:..." into the transcript whenever the image is not painted
/// over it (cold cache after reload, prewarm in flight, no image protocol).
/// The draw path must blank marker rows instead.
#[test]
fn test_real_draw_never_emits_inline_image_marker_text() {
    use crate::message::{ContentBlock, Role};

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    assert!(!app.is_remote, "repro must use the local image render path");

    const TOOL_ID: &str = "read-shot-marker";

    app.session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "read the screenshot".to_string(),
            cache_control: None,
        }],
    );
    app.session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: TOOL_ID.to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "shot.png"}),
            thought_signature: None,
        }],
    );
    app.session.add_message(
        Role::User,
        vec![
            ContentBlock::ToolResult {
                tool_use_id: TOOL_ID.to_string(),
                content: "read image".to_string(),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: REPRO_TINY_PNG_B64.to_string(),
            },
        ],
    );

    app.display_messages = vec![
        DisplayMessage::user("read the screenshot"),
        DisplayMessage::tool(
            "read shot.png",
            crate::message::ToolCall {
                id: TOOL_ID.to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file_path": "shot.png"}),
                intent: None,
                thought_signature: None,
            },
        ),
    ];
    app.bump_display_messages_version();
    app.invalidate_side_pane_images_signature();
    app.pin_images = true;
    app.inline_images_visible = true;
    app.scroll_offset = 0;
    app.auto_scroll_paused = false;
    app.is_processing = false;
    app.status = ProcessingStatus::Idle;
    app.session.short_name = Some("test".to_string());

    let backend = ratatui::backend::TestBackend::new(80, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");
    let rendered = render_and_snap(&app, &mut terminal);

    assert!(
        rendered.contains("shot.png"),
        "sanity: the anchored image's label line must render, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("IIMG"),
        "raw inline-image marker text must never be drawn to the terminal, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("MERMAID_IMAGE"),
        "raw mermaid marker text must never be drawn to the terminal, got:\n{rendered}"
    );
}

/// Clicking anywhere on the image body (its placeholder rows) must cycle the
/// expand level, exactly like the label badge. Clicks in the blank area to
/// the RIGHT of a narrow image must not.
#[test]
fn test_click_on_inline_image_body_cycles_level() {
    use crate::tui::ui::inline_image_ui::{
        AllFit, ImageExpandLevel, InlineImageItem, build_section,
    };
    use jcode_tui_messages::PreparedChatFrame;

    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();

    const IMAGE_ID: u64 = 0xBEEF;
    let chat_width: u16 = 80;

    let items = vec![InlineImageItem {
        id: IMAGE_ID,
        width: 320,
        height: 200,
        label: "shot.png".to_string(),
        uses_text_fallback: false,
    }];
    let section = build_section(&items, chat_width, 40, false, true, &AllFit);
    let region = *section
        .image_regions
        .iter()
        .find(|r| r.hash == IMAGE_ID)
        .expect("section should carry the image region");
    assert!(region.width > 0, "fit regions record their rendered width");
    assert!(
        region.width < chat_width,
        "test image must be narrower than the chat so the right side is blank"
    );

    let prepared =
        std::sync::Arc::new(PreparedChatFrame::from_single(std::sync::Arc::new(section)));
    let visible_end = prepared.wrapped_plain_line_count();
    let content_area = Rect::new(0, 0, chat_width, visible_end as u16 + 1);

    crate::tui::ui::clear_copy_viewport_snapshot();
    crate::tui::ui::record_copy_viewport_frame_snapshot_for_test(
        prepared,
        0,
        visible_end,
        content_area,
        &vec![0u16; visible_end],
    );

    assert_eq!(app.image_expand_level(IMAGE_ID), ImageExpandLevel::Fit);

    // Click in the middle of the image body (a placeholder row, inside the
    // rendered width). Down then Up, like a real terminal click.
    let body_row = content_area.y + region.abs_line_idx as u16 + 1;
    let body_col = content_area.x + region.width / 2;
    let click = |app: &mut App, col: u16, row: u16| {
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::empty(),
        });
    };
    click(&mut app, body_col, body_row);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Large,
        "clicking the image body should expand Fit -> Large"
    );

    // Clicking the body again advances the cycle.
    click(&mut app, body_col, body_row);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Full,
        "second body click should expand Large -> Full"
    );
    click(&mut app, body_col, body_row);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Fit,
        "third body click should wrap Full -> Fit"
    );

    // A click in the blank space to the right of the image must stay inert.
    let far_right = content_area.x + chat_width - 2;
    assert!(far_right > content_area.x + region.width);
    click(&mut app, far_right, body_row);
    assert_eq!(
        app.image_expand_level(IMAGE_ID),
        ImageExpandLevel::Fit,
        "clicking blank space beside the image must not cycle it"
    );
}
