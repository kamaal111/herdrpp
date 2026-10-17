use herdrpp::{
    machines::picker::MachinePicker,
    picker::{api::parse_snapshot, config::SearchMode},
};

fn picker() -> MachinePicker {
    MachinePicker::new(
        vec![
            ("local".into(), "Local".into()),
            ("alpha".into(), "Alpha".into()),
            ("beta".into(), "Beta".into()),
        ],
        "local",
        SearchMode::Hierarchical,
    )
}
#[test]
fn cycling_preserves_query_clears_destinations_and_wraps_both_directions() {
    let mut picker = picker();
    picker.app.set_query("logs".into());
    picker.cycle(-1);
    assert_eq!(picker.machine(), "beta");
    assert_eq!(picker.app.query, "logs");
    assert!(picker.app.selected.is_none());
    picker.cycle(1);
    assert_eq!(picker.machine(), "local");
    assert!(picker.loading);
}
#[test]
fn a_late_snapshot_cannot_replace_the_machine_currently_being_browsed() {
    let mut picker = picker();
    let alpha = picker.cycle(1);
    let beta = picker.cycle(1);
    picker.complete(
        alpha,
        parse_snapshot(include_bytes!("fixtures/snapshot.json")),
    );
    assert_eq!(picker.machine(), "beta");
    assert!(picker.loading);
    assert!(picker.app.results.is_empty());
    picker.complete(
        beta,
        parse_snapshot(include_bytes!("fixtures/snapshot.json")),
    );
    assert!(!picker.loading);
    assert!(!picker.app.results.is_empty());
}
#[test]
fn unavailable_machine_keeps_the_picker_open_without_an_activatable_destination() {
    let mut picker = picker();
    let generation = picker.cycle(1);
    picker.complete(generation, Err(anyhow::anyhow!("Alpha is offline")));
    assert_eq!(picker.machine(), "alpha");
    assert_eq!(picker.error.as_deref(), Some("Alpha is offline"));
    assert!(picker.app.selected.is_none());
    assert!(!picker.loading);
}

#[test]
fn configured_cmd_p_opens_the_same_picker_and_is_literal_inside_paste() {
    use herdrpp::machines::{
        input::{Action, Input},
        shortcuts::Bindings,
    };
    let bindings = Bindings::parse(
        "[[keys.command]]\nkey='cmd+p'\ntype='plugin_action'\ncommand='herdrpp.open'\n",
    )
    .unwrap();
    let mut input = Input::with_bindings(bindings);
    let actions: Vec<_> = b"\x1b[112;9u".iter().flat_map(|b| input.byte(*b)).collect();
    assert_eq!(actions, vec![Action::Picker]);
    let pasted: Vec<_> = b"\x1b[200~\x1b[112;9u\x1b[201~"
        .iter()
        .flat_map(|b| input.byte(*b))
        .collect();
    assert!(
        pasted
            .iter()
            .all(|action| matches!(action, Action::Bytes(_)))
    );
}
#[test]
fn configured_prefix_shortcut_opens_the_picker_without_forwarding_prefix_to_the_pane() {
    use herdrpp::machines::{
        input::{Action, Input},
        shortcuts::Bindings,
    };
    let bindings = Bindings::parse(
        "[[keys.command]]\nkey='prefix+f'\ntype='plugin_action'\ncommand='herdrpp.open'\n",
    )
    .unwrap();
    let mut input = Input::with_bindings(bindings);
    let actions: Vec<_> = b"\x02f".iter().flat_map(|b| input.byte(*b)).collect();
    assert_eq!(actions, vec![Action::Picker]);
    input.reset();
    let ordinary: Vec<_> = b"\x02x".iter().flat_map(|b| input.byte(*b)).collect();
    assert_eq!(ordinary, vec![Action::Bytes(b"\x02x".to_vec())]);
}
#[test]
fn mouse_activation_holds_following_input_until_the_workspace_selection_is_acknowledged() {
    use herdrpp::machines::handoff::{Handoff, PickerEvent};
    let mut handoff = Handoff::default();
    assert_eq!(
        handoff.feed(b"\x1b[<0;15;8Mcommand").unwrap(),
        b"\x1b[<0;15;8M"
    );
    assert!(
        handoff
            .acknowledge(&PickerEvent::Cancelled)
            .unwrap()
            .is_empty()
    );
    assert_eq!(handoff.take_queued(), b"command");
}

#[test]
fn dragging_does_not_wait_for_a_nonexistent_mouse_click_acknowledgement() {
    let mut handoff = herdrpp::machines::handoff::Handoff::default();
    assert_eq!(
        handoff.feed(b"\x1b[<32;15;8Mtyping").unwrap(),
        b"\x1b[<32;15;8Mtyping"
    );
    assert!(handoff.take_queued().is_empty());
}

#[test]
fn alternate_layout_cmd_p_uses_the_physical_base_key_and_consumes_its_release() {
    use herdrpp::machines::{
        input::{Action, Input},
        shortcuts::Bindings,
    };
    let bindings = Bindings::parse(
        "[[keys.command]]\nkey='cmd+p'\ntype='plugin_action'\ncommand='herdrpp.open'\n",
    )
    .unwrap();
    let mut input = Input::with_bindings(bindings);
    let actions: Vec<_> = b"\x1b[1096::112;73:1u\x1b[1096::112;73:3u"
        .iter()
        .flat_map(|b| input.byte(*b))
        .collect();
    assert_eq!(actions, vec![Action::Picker]);
}

#[test]
fn a_prefix_release_does_not_close_prefix_mode_and_enhanced_quit_still_works() {
    use herdrpp::machines::{
        input::{Action, Input},
        shortcuts::Bindings,
    };
    let bindings = Bindings::parse(
        "[[keys.command]]\nkey='prefix+f'\ntype='plugin_action'\ncommand='herdrpp.open'\n",
    )
    .unwrap();
    let mut input = Input::with_bindings(bindings);
    let actions: Vec<_> = b"\x1b[98;5:1u\x1b[98;5:3uf"
        .iter()
        .flat_map(|b| input.byte(*b))
        .collect();
    assert_eq!(actions, vec![Action::Picker]);
    let quit: Vec<_> = b"\x02\x1b[92;5u"
        .iter()
        .flat_map(|b| input.byte(*b))
        .collect();
    assert_eq!(quit, vec![Action::Quit]);
}

#[test]
fn modifier_aliases_follow_herdr_meta_alt_command_super_and_hyper_bindings() {
    use herdrpp::machines::{
        input::{Action, Input},
        shortcuts::Bindings,
    };
    for (chord, bytes) in [
        ("meta+p", b"\x1bp".as_slice()),
        ("command+p", b"\x1b[112;9u"),
        ("hyper+p", b"\x1b[112;17u"),
    ] {
        let bindings = Bindings::parse(&format!(
            "[[keys.command]]\nkey='{chord}'\ntype='plugin_action'\ncommand='herdrpp.open'\n"
        ))
        .unwrap();
        let mut input = Input::with_bindings(bindings);
        let actions: Vec<_> = bytes.iter().flat_map(|byte| input.byte(*byte)).collect();
        assert_eq!(actions, vec![Action::Picker]);
    }
}
