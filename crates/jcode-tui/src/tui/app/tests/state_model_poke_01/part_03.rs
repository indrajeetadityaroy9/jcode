// Model/poke state-machine cases continued (split out of part_01.rs to keep each part under the test-size budget).
#[test]
fn test_pinned_tall_diagram_does_not_crush_transcript() {
    // Regression: a very tall diagram (portrait aspect) must not make the
    // pinned side pane balloon past the configured ratio and crush the
    // transcript. The pane is capped at `diagram_pane_ratio`; the diagram
    // scales down to fit instead of eating the chat column. The transcript
    // still renders the diagram inline, so a wide chat area keeps it visible.
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.diagram_mode = crate::config::DiagramDisplayMode::Pinned;
    app.diagram_pane_enabled = true;
    app.diagram_pane_position = crate::config::DiagramPanePosition::Side;
    app.diagram_pane_ratio = 40;

    crate::tui::mermaid::clear_active_diagrams();
    // Tall portrait diagram like the flowchart that triggered the bug.
    crate::tui::mermaid::register_active_diagram(0x444, 1320, 1800, Some("tall".to_string()));

    crate::tui::visual_debug::enable();
    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create terminal");
    terminal
        .draw(|f| crate::tui::ui::draw(f, &app))
        .expect("draw failed");

    let frame = crate::tui::visual_debug::latest_frame().expect("frame capture");
    let diagram = frame.layout.diagram_area.expect("diagram area");
    let messages = frame.layout.messages_area.expect("messages area");

    // Pane must not exceed the configured ratio (40% of 120 = 48).
    assert!(
        diagram.width <= 48,
        "pinned pane exceeded configured ratio: width={} (ratio cap=48)",
        diagram.width
    );
    // The transcript keeps the majority of the width so the inline diagram
    // and text stay readable.
    assert!(
        messages.width >= 72,
        "transcript crushed by pinned pane: messages width={}",
        messages.width
    );
    assert_eq!(
        diagram.width + messages.width,
        120,
        "chat + diagram widths should tile the full terminal"
    );

    crate::tui::visual_debug::disable();
    crate::tui::mermaid::clear_active_diagrams();
}

#[test]
fn test_workspace_info_widget_appears_in_visual_debug_frame_when_enabled() {
    let _render_lock = scroll_render_test_lock();

    let mut app = create_test_app();
    app.workspace_client.reset_for_tests();
    app.centered = true;
    app.display_messages = vec![
        DisplayMessage::system("Workspace widget render test".to_string()),
        DisplayMessage::assistant("Short content keeps room for info widgets.".to_string()),
    ];
    app.bump_display_messages_version();

    let current_session = app.session.id.clone();
    app.workspace_client.enable(
        Some(current_session.as_str()),
        &[current_session.clone(), "workspace_peer".to_string()],
    );

    crate::tui::visual_debug::enable();
    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).expect("failed to create terminal");
    terminal
        .draw(|f| crate::tui::ui::draw(f, &app))
        .expect("draw failed");

    let frame = crate::tui::visual_debug::latest_frame().expect("frame capture");
    let widget = frame
        .layout
        .widget_placements
        .iter()
        .find(|placement| placement.kind == "workspace")
        .expect("workspace widget placement");

    assert_eq!(widget.side, "right");
    assert!(
        widget.rect.width > 0,
        "workspace widget width should be non-zero"
    );
    assert!(
        widget.rect.height > 0,
        "workspace widget height should be non-zero"
    );
    assert!(
        frame
            .info_widgets
            .as_ref()
            .expect("info widget capture")
            .placements
            .iter()
            .any(|placement| placement.kind == "workspace"),
        "workspace widget should be present in info widget capture"
    );

    crate::tui::visual_debug::disable();
    app.workspace_client.reset_for_tests();
}

#[test]
fn test_mouse_scroll_over_diff_pane_scrolls_side_panel_without_changing_focus() {
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::File;
    app.diff_pane_scroll = 5;
    app.diff_pane_focus = false;
    app.diff_pane_auto_scroll = true;

    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 40, 20),
        None,
        Some(Rect::new(40, 0, 20, 20)),
        None,
    );

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 45,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert_eq!(app.diff_pane_scroll, 8);
    assert!(!app.diff_pane_focus);
    assert!(!app.diff_pane_auto_scroll);
}

#[test]
fn test_mouse_scroll_animation_preserves_side_pane_scroll_sensitivity() {
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::File;
    app.diff_pane_scroll = 5;
    app.diff_pane_auto_scroll = true;

    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 40, 20),
        None,
        Some(Rect::new(40, 0, 20, 20)),
        None,
    );

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 45,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert_eq!(
        app.diff_pane_scroll, 8,
        "one wheel notch should drain the full side-pane scroll amount"
    );

    let _ = crate::tui::app::local::handle_tick(&mut app);
    assert_eq!(app.diff_pane_scroll, 8);

    crate::tui::app::local::handle_tick(&mut app);
    assert_eq!(
        app.diff_pane_scroll, 8,
        "ticks should not add extra scroll after the wheel notch drained"
    );
}

#[test]
fn test_mouse_scroll_over_tool_side_panel_scrolls_shared_right_pane_without_changing_focus() {
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    app.diff_pane_scroll = 5;
    app.diff_pane_focus = false;
    app.diff_pane_auto_scroll = true;
    app.side_panel = crate::side_panel::SidePanelSnapshot {
        focused_page_id: Some("plan".to_string()),
        pages: vec![crate::side_panel::SidePanelPage {
            id: "plan".to_string(),
            title: "Plan".to_string(),
            file_path: "".to_string(),
            format: crate::side_panel::SidePanelPageFormat::Markdown,
            source: crate::side_panel::SidePanelPageSource::Managed,
            content: "hello".to_string(),
            updated_at_ms: 1,
        }],
    };

    crate::tui::ui::record_layout_snapshot(
        Rect::new(0, 0, 40, 20),
        None,
        Some(Rect::new(40, 0, 20, 20)),
        None,
    );

    let scroll_only = app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 45,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert!(
        !scroll_only,
        "side-panel wheel scroll should request an immediate redraw"
    );
    assert_eq!(app.diff_pane_scroll, 8);
    assert!(!app.diff_pane_focus);
    assert!(!app.diff_pane_auto_scroll);
}

#[test]
fn test_side_pane_scroll_by_clamps_to_rendered_extent() {
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();

    // Simulate a rendered frame: 30 content lines in a 20-line viewport.
    crate::tui::ui::set_pinned_pane_total_lines(30);
    crate::tui::ui::set_last_diff_pane_max_scroll(10);
    crate::tui::ui::set_last_diff_pane_effective_scroll(10);

    // Follow-bottom sentinel resolves to the on-screen position before moving.
    app.diff_pane_scroll = usize::MAX;
    assert!(app.side_pane_scroll_by(-3));
    assert_eq!(app.diff_pane_scroll, 7);
    assert!(!app.diff_pane_auto_scroll);

    // Downward motion clamps at the rendered max instead of accumulating
    // phantom offset past the bottom.
    app.diff_pane_scroll = 9;
    assert!(app.side_pane_scroll_by(3));
    assert_eq!(app.diff_pane_scroll, 10);
    assert!(!app.side_pane_scroll_by(3), "already at the bottom");
    assert_eq!(app.diff_pane_scroll, 10);

    // A stale stored offset beyond the rendered extent snaps back so the very
    // next upward scroll moves the visible view immediately.
    app.diff_pane_scroll = 25;
    assert!(app.side_pane_scroll_by(-3));
    assert_eq!(app.diff_pane_scroll, 7);
}
