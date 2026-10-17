use herdrpp::machines::input::{Action, Input};

fn input(bytes: &[u8]) -> Vec<Action> {
    let mut parser = Input::default();
    bytes.iter().flat_map(|byte| parser.byte(*byte)).collect()
}

#[test]
fn legacy_and_enhanced_picker_and_quit_are_recognized() {
    assert_eq!(
        input(b"\x1d\x1b[93;5u\x1c\x1b[92;5:1u"),
        vec![Action::Picker, Action::Picker, Action::Quit, Action::Quit]
    );
}

#[test]
fn enhanced_key_releases_are_consumed_without_triggering_actions() {
    assert_eq!(input(b"\x1b[93;5:3u\x1b[92;5:3u"), vec![]);
}

#[test]
fn shortcuts_inside_bracketed_paste_remain_literal_input() {
    let actions = input(b"\x1b[200~hello\x1d\x1c\x1b[93;5u\x1b[201~\x1d");
    let forwarded: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Bytes(bytes) => Some(bytes.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect();
    assert_eq!(forwarded, b"\x1b[200~hello\x1d\x1c\x1b[93;5u\x1b[201~");
    assert_eq!(actions.last(), Some(&Action::Picker));
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, Action::Picker))
            .count(),
        1
    );
    assert!(!actions.contains(&Action::Quit));
}

fn forwarded(actions: &[Action]) -> Vec<u8> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Bytes(bytes) => Some(bytes.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

#[test]
fn kitty_alternate_layout_base_keys_preserve_the_reserved_physical_shortcuts() {
    assert_eq!(
        input(b"\x1b[1098::93;5u\x1b[93:125:93;5u\x1b[1098::92;5u"),
        vec![Action::Picker, Action::Picker, Action::Quit]
    );
}

#[test]
fn kitty_caps_and_num_lock_state_do_not_change_control_shortcuts() {
    assert_eq!(
        input(b"\x1b[93;69u\x1b[92;133u\x1b[1098::93;197:2;93u"),
        vec![Action::Picker, Action::Quit, Action::Picker]
    );
}

#[test]
fn kitty_repeat_release_and_associated_text_fields_have_explicit_shortcut_semantics() {
    assert_eq!(
        input(b"\x1b[93;5:2;93:32u\x1b[92;5:2u\x1b[1098::93;5:3;93u\x1b[92:124:92;5:3u"),
        vec![Action::Picker, Action::Quit]
    );
}

#[test]
fn modify_other_keys_reports_trigger_only_the_reserved_control_chords() {
    assert_eq!(
        input(b"\x1b[27;5;93~\x1b[27;5;92~"),
        vec![Action::Picker, Action::Quit]
    );
}

#[test]
fn unmodified_shifted_alt_and_unknown_keys_remain_byte_exact_input() {
    let bytes = b"\x1b[93u\x1b[93;6u\x1b[93;7u\x1b[94;5u\x1b[1098:93;5u\x1b[27;6;93~";
    let actions = input(bytes);
    assert_eq!(forwarded(&actions), bytes);
    assert!(
        actions
            .iter()
            .all(|action| matches!(action, Action::Bytes(_)))
    );
}

#[test]
fn malformed_keyboard_fields_are_forwarded_without_triggering_a_shortcut() {
    let bytes =
        b"\x1b[93;0u\x1b[93;5:4u\x1b[93;5;u\x1b[93:125:93:92;5u\x1b[93;5;1114112u\x1b[27;5;93;92~";
    let actions = input(bytes);
    assert_eq!(forwarded(&actions), bytes);
    assert!(
        actions
            .iter()
            .all(|action| matches!(action, Action::Bytes(_)))
    );
}

#[test]
fn typed_keyboard_reports_inside_bracketed_paste_and_control_strings_are_opaque() {
    let bytes = b"\x1b[200~\x1b[1098::93;5u\x1b[27;5;92~\x1d\x1b[201~\x1b]title\x1d\x1c\x07\x1b_other\x1d\x1b\\";
    let actions = input(bytes);
    assert_eq!(forwarded(&actions), bytes);
    assert!(
        actions
            .iter()
            .all(|action| matches!(action, Action::Bytes(_)))
    );
}

fn expired_prefix_then_quit(bytes: &[u8]) -> Vec<Action> {
    let mut parser = Input::default();
    let mut actions = bytes
        .iter()
        .flat_map(|byte| parser.byte(*byte))
        .collect::<Vec<_>>();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(flushed) = parser.flush() {
            actions.push(flushed);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "incomplete prefix was never flushed"
        );
        std::thread::yield_now();
    }
    actions.extend(parser.byte(0x1c));
    actions
}

#[test]
fn a_bare_alt_close_bracket_expires_without_trapping_later_launcher_shortcuts() {
    assert_eq!(
        expired_prefix_then_quit(b"\x1b]"),
        vec![Action::Bytes(b"\x1b]".to_vec()), Action::Quit]
    );
}

#[test]
fn a_bare_apc_prefix_expires_without_trapping_later_launcher_shortcuts() {
    assert_eq!(
        expired_prefix_then_quit(b"\x1b_"),
        vec![Action::Bytes(b"\x1b_".to_vec()), Action::Quit]
    );
}
