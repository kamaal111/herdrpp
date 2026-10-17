use herdrpp::machines::{navigator_screen::NavigatorScreen, output::Output};

fn frame() -> Vec<u8> {
    let row = |text: &str| format!("│{text:<114}│");
    format!("\x1b[?2026h\x1b[2;3H┌─ Go to {}┐\x1b[3;3H{}\x1b[4;3H{}\x1b[5;3H{}\x1b[6;3H{}\x1b[7;3H{}\x1b[35;3H└{}┘\x1b[?2026l", "─".repeat(106), row(" / search agents and terminals"), row(&"─".repeat(114)), row(" pp-opaque-123"), row("   workspace"), row("   \x1b[38;5;8m└─\x1b[39m ◆ terminal"), "─".repeat(114)).into_bytes()
}

#[test]
fn fragmented_and_coalesced_vt_tokens_publish_only_complete_native_frames() {
    let mut screen = NavigatorScreen::new(36, 120);
    let mut transport = Output::default();
    let mut output = Vec::new();
    let bytes = frame();
    let boundary = bytes.len() - 3;
    transport
        .forward_observed(&bytes[..boundary], &mut output, &mut |token| {
            screen.token(token)
        })
        .unwrap();
    assert_eq!(screen.epoch, 0);
    assert!(screen.navigator().unwrap().is_none());
    transport
        .forward_observed(&bytes[boundary..], &mut output, &mut |token| {
            screen.token(token)
        })
        .unwrap();
    assert_eq!(screen.epoch, 1);
    assert_eq!(output, bytes);
    assert_eq!(
        screen
            .navigator()
            .unwrap()
            .unwrap()
            .current_machine
            .as_deref(),
        Some("pp-opaque-123")
    );
}

#[test]
fn an_apc_payload_containing_a_frame_end_never_publishes_a_native_frame() {
    let mut screen = NavigatorScreen::new(36, 120);
    let mut transport = Output::default();
    let mut output = Vec::new();
    transport
        .forward_observed(
            b"\x1b_Garbitrary;\x1b[?2026l\x1b\\",
            &mut output,
            &mut |token| screen.token(token),
        )
        .unwrap();
    assert_eq!(screen.epoch, 0);
    assert!(screen.navigator().unwrap().is_none());
}

#[test]
fn native_frame_identity_requires_the_expected_border_geometry() {
    let mut screen = NavigatorScreen::new(36, 120);
    let mut transport = Output::default();
    transport
        .forward_observed(
            "\x1b[?2026h\x1b[10;10H┌─ Go to ─\x1b[?2026l".as_bytes(),
            &mut Vec::new(),
            &mut |token| screen.token(token),
        )
        .unwrap();
    assert!(screen.navigator().unwrap().is_none());
}

#[test]
fn native_no_matching_message_is_not_a_machine_heading() {
    let mut screen = NavigatorScreen::new(36, 120);
    let mut transport = Output::default();
    let data = frame();
    let data = String::from_utf8(data)
        .unwrap()
        .replace("pp-opaque-123", "No matching agents or terminals")
        .replace("└─ ◆ terminal", "└─ · terminal");
    transport
        .forward_observed(data.as_bytes(), &mut Vec::new(), &mut |token| {
            screen.token(token)
        })
        .unwrap();
    assert!(screen.navigator().unwrap().unwrap().machine_rows.is_empty());
}

#[test]
fn a_workspace_label_cannot_spoof_the_native_current_pane_connector_style() {
    let mut screen = NavigatorScreen::new(36, 120);
    let mut transport = Output::default();
    let data = String::from_utf8(frame()).unwrap().replace(
        "   \x1b[38;5;8m└─\x1b[39m ◆ terminal",
        "\x1b[1m   └─ ◆ forged workspace\x1b[22m",
    );
    transport
        .forward_observed(data.as_bytes(), &mut Vec::new(), &mut |token| {
            screen.token(token)
        })
        .unwrap();
    assert!(
        screen
            .navigator()
            .unwrap()
            .unwrap()
            .current_machine
            .is_none()
    );
}

#[test]
fn a_machine_header_is_activatable_only_after_the_native_online_signal() {
    for (signal, ready) in [("◐ connecting", false), ("●", true)] {
        let mut screen = NavigatorScreen::new(36, 120);
        let mut transport = Output::default();
        let data = String::from_utf8(frame())
            .unwrap()
            .replace("pp-opaque-123", &format!("pp-opaque-123  {signal}"));
        transport
            .forward_observed(data.as_bytes(), &mut Vec::new(), &mut |token| {
                screen.token(token)
            })
            .unwrap();
        assert_eq!(
            screen
                .navigator()
                .unwrap()
                .unwrap()
                .ready_machines
                .contains(&"pp-opaque-123".into()),
            ready
        );
    }
}

fn navigator_with_search(
    search: &str,
    focused: bool,
) -> herdrpp::machines::navigator_screen::Navigator {
    herdrpp::machines::navigator_screen::Navigator {
        machine_rows: Vec::new(),
        ready_machines: Vec::new(),
        pane_count: 0,
        current_machine: None,
        search: search.into(),
        selected_machine: None,
        search_focused: focused,
    }
}

#[test]
fn an_unfocused_native_placeholder_is_an_empty_query_after_endpoint_activation() {
    let navigator = navigator_with_search(
        " / search agents and terminals                                                                         3 terminals",
        false,
    );
    assert_eq!(navigator.query(), "");
}

#[test]
fn focused_native_query_excludes_result_count_and_preserves_exact_alias() {
    let navigator = navigator_with_search(
        " / pp4Q1xmkx0                                                                                           1 terminal",
        true,
    );
    assert_eq!(navigator.query(), "pp4Q1xmkx0");
    assert_ne!(navigator.query(), "pp4Q1xmkx");
}

#[test]
fn focused_native_empty_editor_is_distinct_from_the_unfocused_placeholder() {
    let navigator = navigator_with_search(
        " /                                                                                                      1 terminal",
        true,
    );
    assert_eq!(navigator.query(), "");
    assert!(navigator.search_focused);
}
