use herdrpp::{
    picker::theme::Theme,
    search::{input::TextInput, view},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::{Position, Rect},
    style::Modifier,
};
use terminal_input::event::{Event, KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn draw(input: &TextInput, width: u16) -> (Terminal<TestBackend>, Position) {
    let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
    terminal
        .draw(|frame| {
            view::render_input(
                frame,
                Rect::new(0, 0, width, 1),
                Rect::new(0, 1, width, 1),
                Theme::default().palette(),
                input,
                "Search",
                0,
            )
        })
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    (terminal, cursor)
}

fn row(terminal: &Terminal<TestBackend>) -> String {
    (0..terminal.size().unwrap().width)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
        .collect()
}

#[test]
fn long_input_scrolls_to_cursor_in_both_directions_and_after_resize() {
    let mut input = TextInput::new("abcdefghijklmnop".into(), false);
    let (terminal, cursor) = draw(&input, 20);
    assert!(row(&terminal).starts_with("> klmnop "));
    assert_eq!(cursor, Position::new(8, 0));
    for _ in 0..16 {
        input.handle(&key(KeyCode::Left));
    }
    let (terminal, cursor) = draw(&input, 20);
    assert!(row(&terminal).starts_with("> abcdefg"));
    assert_eq!(cursor, Position::new(2, 0));
    for _ in 0..16 {
        input.handle(&key(KeyCode::Right));
    }
    let (terminal, cursor) = draw(&input, 24);
    assert!(row(&terminal).starts_with("> ghijklmnop "));
    assert_eq!(cursor, Position::new(12, 0));
    assert!(row(&terminal).ends_with(" 0 results "));
}

#[test]
fn wide_characters_position_cursor_by_terminal_cells() {
    let mut input = TextInput::new("a界b".into(), false);
    let (_, cursor) = draw(&input, 30);
    assert_eq!(cursor, Position::new(6, 0));
    input.handle(&key(KeyCode::Left));
    let (_, cursor) = draw(&input, 30);
    assert_eq!(cursor, Position::new(5, 0));
    input.handle(&key(KeyCode::Left));
    let (_, cursor) = draw(&input, 30);
    assert_eq!(cursor, Position::new(3, 0));
}

#[test]
fn narrow_input_keeps_cursor_before_count_and_hides_it_without_edit_space() {
    let input = TextInput::new("世界界".into(), false);
    let (terminal, cursor) = draw(&input, 16);
    assert_eq!(cursor, Position::new(4, 0));
    assert!(row(&terminal).ends_with(" 0 results "));
    for width in 0..=13 {
        let (_, cursor) = draw(&input, width);
        assert_eq!(cursor, Position::new(0, 0));
    }
}

#[test]
fn empty_field_shows_placeholder_with_cursor_at_start() {
    let (terminal, cursor) = draw(&TextInput::default(), 30);
    assert!(row(&terminal).starts_with("> Search"));
    assert_eq!(cursor, Position::new(2, 0));
}

#[test]
fn name_input_renders_selected_suggestion_and_cursor_from_shared_model() {
    let mut input = TextInput::new("Review".into(), true);
    let mut terminal = Terminal::new(TestBackend::new(30, 3)).unwrap();
    terminal
        .draw(|frame| {
            view::render_name_input(
                frame,
                Rect::new(0, 0, 30, 1),
                Rect::new(0, 1, 30, 1),
                Theme::default().palette(),
                &input,
            )
        })
        .unwrap();
    assert!(
        terminal.backend().buffer()[(2, 0)]
            .modifier
            .contains(Modifier::REVERSED)
    );
    input.handle(&key(KeyCode::Left));
    terminal
        .draw(|frame| {
            view::render_name_input(
                frame,
                Rect::new(0, 0, 30, 1),
                Rect::new(0, 1, 30, 1),
                Theme::default().palette(),
                &input,
            )
        })
        .unwrap();
    assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(2, 0));
    assert!(
        !terminal.backend().buffer()[(2, 0)]
            .modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn command_arrows_scroll_long_input_to_each_edge() {
    let mut input = TextInput::new("abcdefghijklmnop".into(), false);
    input.handle(&Event::Key(KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::SUPER,
    )));
    let (terminal, cursor) = draw(&input, 20);
    assert!(row(&terminal).starts_with("> abcdefg"));
    assert_eq!(cursor, Position::new(2, 0));
    input.handle(&Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::SUPER,
    )));
    let (terminal, cursor) = draw(&input, 20);
    assert!(row(&terminal).starts_with("> klmnop "));
    assert_eq!(cursor, Position::new(8, 0));
}
