use herdrpp::search::input::{InputChange, TextInput};
use terminal_input::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn backspace_deletes_one_scalar_at_end_middle_and_start() {
    for (lefts, expected, cursor) in [(0, "abc", 3), (2, "acd", 1), (4, "abcd", 0)] {
        let mut input = TextInput::new("abcd".into(), false);
        for _ in 0..lefts {
            input.handle(&key(KeyCode::Left));
        }
        input.handle(&key(KeyCode::Backspace));
        assert_eq!(input.text(), expected);
        assert_eq!(input.cursor(), cursor);
    }
}

#[test]
fn clear_shortcuts_clear_entire_text_at_every_position_and_reset_selection() {
    for (code, modifiers) in [
        (KeyCode::Backspace, KeyModifiers::SUPER),
        (KeyCode::Char('u'), KeyModifiers::CONTROL),
    ] {
        for lefts in [0, 2, 4] {
            let mut input = TextInput::new("abcd".into(), false);
            for _ in 0..lefts {
                input.handle(&key(KeyCode::Left));
            }
            assert_eq!(
                input.handle(&Event::Key(KeyEvent::new(code, modifiers))),
                InputChange::Edited
            );
            assert_eq!(input.text(), "");
            assert_eq!(input.cursor(), 0);
            assert!(!input.selected());
            input.handle(&key(KeyCode::Char('x')));
            assert_eq!(input.text(), "x");
        }
        let mut input = TextInput::new("suggestion".into(), true);
        input.handle(&Event::Key(KeyEvent::new(code, modifiers)));
        assert_eq!(input, TextInput::default());
    }
}

#[test]
fn unicode_arrows_and_backspace_use_valid_scalar_boundaries() {
    let mut input = TextInput::new("a界b".into(), false);
    input.handle(&key(KeyCode::Left));
    assert_eq!(input.cursor(), 4);
    input.handle(&key(KeyCode::Left));
    assert_eq!(input.cursor(), 1);
    input.handle(&key(KeyCode::Right));
    input.handle(&key(KeyCode::Backspace));
    assert_eq!(input.text(), "ab");
    assert_eq!(input.cursor(), 1);
    input.handle(&Event::Paste("🙂\nX".into()));
    assert_eq!(input.text(), "a🙂Xb");
    assert_eq!(input.cursor(), 6);
}

#[test]
fn empty_input_and_boundary_arrows_are_safe() {
    let mut input = TextInput::default();
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Backspace,
        KeyCode::Left,
    ] {
        input.handle(&key(code));
        assert_eq!(input, TextInput::default());
    }
    let mut input = TextInput::new("x".into(), false);
    input.handle(&key(KeyCode::Right));
    assert_eq!(input.cursor(), 1);
    input.handle(&key(KeyCode::Left));
    input.handle(&key(KeyCode::Left));
    assert_eq!(input.cursor(), 0);
}

#[test]
fn selected_suggestions_collapse_to_each_edge_without_deleting_text() {
    for (code, expected, cursor) in [
        (KeyCode::Left, "Review3", 6),
        (KeyCode::Right, "3Review", 7),
    ] {
        let mut input = TextInput::new("3".into(), true);
        input.handle(&key(code));
        assert!(!input.selected());
        input.handle(&Event::Paste("Review".into()));
        assert_eq!(input.text(), expected);
        assert_eq!(input.cursor(), cursor);
    }
}

#[test]
fn selected_suggestions_keep_replace_and_backspace_behavior() {
    for event in [key(KeyCode::Char('R')), Event::Paste("R".into())] {
        let mut input = TextInput::new("suggestion".into(), true);
        input.handle(&event);
        assert_eq!(input.text(), "R");
        assert_eq!(input.cursor(), 1);
        assert!(!input.selected());
    }
    let mut input = TextInput::new("suggestion".into(), true);
    input.handle(&key(KeyCode::Backspace));
    assert_eq!(input, TextInput::default());
}

#[test]
fn empty_sanitized_paste_preserves_selected_suggestion() {
    let mut input = TextInput::new("3".into(), true);
    input.handle(&Event::Paste("\n\t\u{1b}".into()));
    assert_eq!(input, TextInput::new("3".into(), true));
}

#[test]
fn releases_and_modified_characters_do_not_edit_or_move_cursor() {
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Backspace,
        KeyCode::Char('u'),
    ] {
        let mut input = TextInput::new("abc".into(), true);
        let mut release = KeyEvent::new(code, KeyModifiers::SUPER);
        release.kind = KeyEventKind::Release;
        assert_eq!(input.handle(&Event::Key(release)), InputChange::Unhandled);
        assert_eq!(input, TextInput::new("abc".into(), true));
    }
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ] {
        let mut input = TextInput::new("abc".into(), false);
        input.handle(&Event::Key(KeyEvent::new(KeyCode::Char('x'), modifiers)));
        assert_eq!(input.text(), "abc");
    }
}

#[test]
fn repeats_move_once_and_plain_u_types() {
    let mut input = TextInput::new("abc".into(), false);
    let mut repeat = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
    repeat.kind = KeyEventKind::Repeat;
    input.handle(&Event::Key(repeat));
    input.handle(&key(KeyCode::Char('u')));
    assert_eq!(input.text(), "abuc");
    assert_eq!(input.cursor(), 3);
}
