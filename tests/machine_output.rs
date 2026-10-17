use herdrpp::machines::output::Output;

fn feed(output: &mut Output, input: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    output.forward(input, &mut bytes).unwrap();
    bytes
}

#[test]
fn picker_can_interrupt_fragmented_kitty_upload_without_receiving_half_control_string() {
    let mut output = Output::default();
    assert_eq!(feed(&mut output, b"before\x1b_Ga=t,f=24;AAAA"), b"before");
    assert!(output.pending_len() > 0);
    assert_eq!(feed(&mut output, b"BBBB\x1b"), b"");
    assert_eq!(
        feed(&mut output, b"\\after"),
        b"\x1b_Ga=t,f=24;AAAABBBB\x1b\\after"
    );
    assert_eq!(output.pending_len(), 0);
}

#[test]
fn clipboard_osc_is_preserved_through_bell_and_string_terminator_forms() {
    let mut output = Output::default();
    assert_eq!(feed(&mut output, b"\x1b]52;c;YWJj"), b"");
    assert_eq!(
        feed(&mut output, b"\x07\x1b]52;c;ZGVm\x1b"),
        b"\x1b]52;c;YWJj\x07"
    );
    assert_eq!(feed(&mut output, b"\\"), b"\x1b]52;c;ZGVm\x1b\\");
}

#[test]
fn dcs_and_csi_are_emitted_only_when_complete() {
    let mut output = Output::default();
    assert_eq!(feed(&mut output, b"\x1bP+q544e"), b"");
    assert_eq!(feed(&mut output, b"\x1b\\\x1b[?2004"), b"\x1bP+q544e\x1b\\");
    assert_eq!(feed(&mut output, b"h"), b"\x1b[?2004h");
}

#[test]
fn multibyte_text_is_not_split_when_picker_borrows_terminal() {
    let mut output = Output::default();
    assert_eq!(feed(&mut output, &[0xf0, 0x9f]), b"");
    assert_eq!(feed(&mut output, &[0x98, 0x80]), "😀".as_bytes());
}

#[test]
fn invalid_utf8_does_not_expose_the_next_escape_prefix_to_picker() {
    let mut output = Output::default();
    assert_eq!(feed(&mut output, &[0xe2, 0x1b]), b"");
    assert_eq!(feed(&mut output, b"[?2004h"), b"\xe2\x1b[?2004h");
}

#[test]
fn escape_restarts_csi_without_emitting_the_new_incomplete_prefix() {
    let mut output = Output::default();
    assert_eq!(feed(&mut output, b"\x1b[1;\x1b"), b"");
    assert_eq!(feed(&mut output, b"[?2004"), b"");
    assert_eq!(feed(&mut output, b"h"), b"\x1b[1;\x1b[?2004h");
}

#[test]
fn repeated_escape_keeps_final_escape_pending() {
    let mut output = Output::default();
    assert_eq!(feed(&mut output, b"\x1b\x1b"), b"");
    assert_eq!(feed(&mut output, b"_Ghello\x1b"), b"");
    assert_eq!(feed(&mut output, b"\\"), b"\x1b\x1b_Ghello\x1b\\");
}

#[test]
fn large_clipboard_control_spools_without_losing_payload() {
    let mut output = Output::default();
    let payload = vec![b'A'; 200_000];
    assert_eq!(feed(&mut output, b"\x1b]52;c;"), b"");
    assert_eq!(feed(&mut output, &payload), b"");
    assert_eq!(output.pending_len(), payload.len() + 7);
    let mut expected = b"\x1b]52;c;".to_vec();
    expected.extend_from_slice(&payload);
    expected.push(7);
    assert_eq!(feed(&mut output, b"\x07"), expected);
}

#[test]
fn unbalanced_owned_keyboard_and_sync_modes_are_restored_once() {
    let mut output = Output::default();
    let _ = feed(&mut output, b"\x1b[>1u\x1b[>4;2m\x1b[?2026h");
    let mut restored = Vec::new();
    output.restore_host_modes(&mut restored).unwrap();
    assert_eq!(restored, b"\x1b[?2026l\x1b[>4;0m\x1b[<u");
    let mut again = Vec::new();
    output.restore_host_modes(&mut again).unwrap();
    assert!(again.is_empty());
}

#[test]
fn normal_child_cleanup_does_not_pop_the_callers_keyboard_stack() {
    let mut output = Output::default();
    let _ = feed(&mut output, b"\x1b[>1u\x1b[<u");
    let mut restored = Vec::new();
    output.restore_host_modes(&mut restored).unwrap();
    assert!(restored.is_empty());
}

#[test]
fn keyboard_input_waits_for_setup_and_a_complete_rendered_frame() {
    let mut output = Output::default();
    let _ = feed(&mut output, b"\x1b[?7l");
    assert!(!output.input_ready);
    let _ = feed(&mut output, b"\x1b[?2026hfirst frame");
    assert!(!output.input_ready);
    let _ = feed(&mut output, b"\x1b[?2026l");
    assert!(output.input_ready);
}
