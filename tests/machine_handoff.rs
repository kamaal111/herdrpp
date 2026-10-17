use herdrpp::machines::handoff::{Handoff, PickerEvent, Selection};
fn selection() -> Selection {
    Selection {
        machine: "alpha".into(),
        profile: None,
        target: herdrpp::picker::model::Target::Tab("w1:t1".into()),
        query: String::new(),
    }
}

#[test]
fn selection_acknowledgement_preserves_bytes_after_enter_for_the_next_attachment() {
    let mut handoff = Handoff::default();
    assert_eq!(handoff.feed(b"Alpha\rprintf proof\r").unwrap(), b"Alpha\r");
    assert!(
        handoff
            .acknowledge(&PickerEvent::Selected(selection()))
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"printf proof\r");
}

#[test]
fn no_hit_enter_releases_queued_query_edits_until_the_next_enter() {
    let mut handoff = Handoff::default();
    assert_eq!(
        handoff.feed(b"missing\r\x7fAlpha\rcommand\r").unwrap(),
        b"missing\r"
    );
    assert_eq!(
        handoff.acknowledge(&PickerEvent::Continue).unwrap(),
        b"\x7fAlpha\r"
    );
    assert!(
        handoff
            .acknowledge(&PickerEvent::Selected(selection()))
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"command\r");
}

#[test]
fn cancellation_acknowledgement_preserves_typeahead_and_does_not_consume_it_in_the_picker() {
    let mut handoff = Handoff::default();
    assert_eq!(handoff.timeout(b"\x1b").unwrap(), b"\x1b");
    assert!(handoff.feed(b"old client command\r").unwrap().is_empty());
    assert!(
        handoff
            .acknowledge(&PickerEvent::Cancelled)
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"old client command\r");
}

#[test]
fn fragmented_enhanced_enter_gates_at_the_final_byte_and_ignores_release() {
    let mut handoff = Handoff::default();
    assert_eq!(
        handoff.feed(b"\x1b[13;1:3uquery").unwrap(),
        b"\x1b[13;1:3uquery"
    );
    assert_eq!(handoff.feed(b"\x1b[13;1:").unwrap(), b"\x1b[13;1:");
    assert_eq!(handoff.feed(b"1ucommand").unwrap(), b"1u");
    assert!(
        handoff
            .acknowledge(&PickerEvent::Selected(selection()))
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"command");
}

#[test]
fn pasted_newlines_and_control_c_never_close_the_selector() {
    let mut handoff = Handoff::default();
    let pasted = b"\x1b[200~line\r\n\x03\x1b[13u\x1b[201~";
    assert_eq!(handoff.feed(pasted).unwrap(), pasted);
    assert_eq!(handoff.feed(b"query").unwrap(), b"query");
    assert!(handoff.take_queued().is_empty());
}

#[test]
fn unexpected_acknowledgement_cannot_release_queued_user_input() {
    let mut handoff = Handoff::default();
    assert_eq!(handoff.feed(b"\x03command").unwrap(), b"\x03");
    assert!(handoff.acknowledge(&PickerEvent::Continue).is_err());
    assert_eq!(handoff.take_queued(), b"command");
}

#[test]
fn control_j_and_shifted_control_c_are_forwarded_without_waiting_for_nonexistent_acknowledgements()
{
    let mut handoff = Handoff::default();
    let ordinary = b"\n\x1b[99:67;6uquery\x1bOM\x1b[27;5;13~";
    assert_eq!(handoff.feed(ordinary).unwrap(), ordinary);
    assert!(handoff.take_queued().is_empty());
}

#[test]
fn keypad_enter_uses_the_same_acknowledged_handoff_as_return() {
    let mut handoff = Handoff::default();
    assert_eq!(handoff.feed(b"\x1b[57414ucommand").unwrap(), b"\x1b[57414u");
    assert!(
        handoff
            .acknowledge(&PickerEvent::Selected(selection()))
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"command");
}

#[test]
fn alt_return_closes_the_picker_and_keeps_following_command_bytes() {
    let mut handoff = Handoff::default();
    assert_eq!(handoff.feed(b"Alpha\x1b\rcommand").unwrap(), b"Alpha\x1b\r");
    assert!(
        handoff
            .acknowledge(&PickerEvent::Selected(selection()))
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"command");
}

#[test]
fn bare_escape_timeout_queued_behind_no_hit_enter_still_cancels_before_typeahead() {
    let mut handoff = Handoff::default();
    assert_eq!(handoff.feed(b"missing\r").unwrap(), b"missing\r");
    assert!(handoff.timeout(b"\x1b").unwrap().is_empty());
    assert!(handoff.feed(b"old client command\r").unwrap().is_empty());
    assert_eq!(
        handoff.acknowledge(&PickerEvent::Continue).unwrap(),
        b"\x1b"
    );
    assert!(
        handoff
            .acknowledge(&PickerEvent::Cancelled)
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"old client command\r");
}

#[test]
fn declared_literal_timeout_survives_selection_handoff_without_reinterpretation() {
    use herdrpp::machines::handoff::{InputChunk, PickerEvent};
    let mut handoff = Handoff::default();
    handoff.feed(b"\r").unwrap();
    handoff.timeout(b"\x1b]title").unwrap();
    handoff.feed(b"\x1c").unwrap();
    handoff
        .acknowledge(&PickerEvent::Selected(selection()))
        .unwrap();
    let choice = handoff.finish(Some(selection()));
    assert_eq!(
        choice.queued,
        vec![
            InputChunk::Literal(b"\x1b]title".to_vec()),
            InputChunk::Keyboard(vec![0x1c])
        ]
    );
}

#[test]
fn declared_partial_string_timeout_resets_handoff_before_next_enter() {
    use herdrpp::machines::handoff::PickerEvent;
    let mut handoff = Handoff::default();
    handoff.timeout(b"\x1b]title").unwrap();
    handoff.feed(b"\r").unwrap();
    handoff
        .acknowledge(&PickerEvent::Selected(selection()))
        .unwrap();
    assert!(handoff.finish(Some(selection())).queued.is_empty());
}
