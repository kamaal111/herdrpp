use herdrpp::{
    palette::{
        app::{Action, App, Mode},
        catalog::{self, Context},
        ui,
    },
    picker::theme::Theme,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use terminal_input::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

fn app() -> App {
    let context: Context = serde_json::from_value(
        json!({"workspace_id":"w2","tab_id":"w2:t7","focused_pane_id":"w2:p8","tab_label":"Codex"}),
    )
    .unwrap();
    App::new(
        context.clone(),
        catalog::builtins(&context),
        Theme::default(),
        None,
    )
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn draw(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn typing_paste_and_backspace_filter_and_empty_enter_does_nothing() {
    let mut app = app();
    app.handle(Event::Paste("SPL\n RGT".into()));
    assert_eq!(app.query, "SPL RGT");
    assert_eq!(app.items[app.results[0].item].title, "Split Pane Right");
    app.handle(key(KeyCode::Backspace));
    assert_eq!(app.query, "SPL RG");
    app.set_query("nonexistent".into());
    assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
    assert!(draw(&mut app, 80, 20).contains("No matching commands"));
}

#[test]
fn clear_shortcuts_restore_commands_selection_scroll_and_click_targets() {
    for (code, modifiers) in [
        (KeyCode::Backspace, KeyModifiers::SUPER),
        (KeyCode::Char('u'), KeyModifiers::CONTROL),
    ] {
        let mut app = app();
        app.set_query("Split Pane Right".into());
        draw(&mut app, 80, 20);
        assert!(!app.hits.is_empty());
        app.offset = 3;
        assert_eq!(
            app.handle(Event::Key(KeyEvent::new(code, modifiers))),
            Action::Continue
        );
        assert_eq!(app.query, "");
        assert_eq!(app.results.len(), 16);
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.offset, 0);
        assert!(app.hits.is_empty());
        assert_eq!(app.mode, Mode::Search);
        app.set_query("nonexistent".into());
        assert_eq!(
            app.handle(Event::Key(KeyEvent::new(code, modifiers))),
            Action::Continue
        );
        assert_eq!(app.results.len(), 16);
        assert_eq!(app.selected, Some(0));
    }
}

#[test]
fn clear_shortcuts_clear_rename_input_and_validation_error_without_leaving_prompt() {
    for (code, modifiers) in [
        (KeyCode::Backspace, KeyModifiers::SUPER),
        (KeyCode::Char('u'), KeyModifiers::CONTROL),
    ] {
        let mut app = app();
        app.set_query("Rename Tab".into());
        let item = app.results[0].item;
        app.handle(key(KeyCode::Enter));
        app.handle(Event::Paste("--世界 Review".into()));
        assert_eq!(
            app.handle(Event::Key(KeyEvent::new(code, modifiers))),
            Action::Continue
        );
        assert_eq!(
            app.mode,
            Mode::Rename {
                item,
                input: String::new(),
                error: false
            }
        );
        assert_eq!(app.query, "Rename Tab");
        assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
        assert!(draw(&mut app, 80, 20).contains("Enter a nonblank name"));
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
        assert_eq!(
            app.mode,
            Mode::Rename {
                item,
                input: String::new(),
                error: false
            }
        );
        app.handle(Event::Paste("New name".into()));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Execute(herdrpp::palette::app::Selection {
                id: "herdr:tab_rename".into(),
                input: Some("New name".into()),
                confirmed: false,
            })
        );
    }
}

#[test]
fn clear_shortcuts_leave_close_confirmation_unchanged() {
    let mut app = app();
    app.set_query("Close Tab".into());
    let item = app.results[0].item;
    app.handle(key(KeyCode::Enter));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::SUPER,
    )));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(app.mode, Mode::Confirm { item });
    assert_eq!(app.query, "Close Tab");
}

#[test]
fn clear_key_releases_do_not_edit_and_plain_u_still_types() {
    let mut app = app();
    app.set_query("split".into());
    for (code, modifiers) in [
        (KeyCode::Backspace, KeyModifiers::SUPER),
        (KeyCode::Char('u'), KeyModifiers::CONTROL),
    ] {
        let mut release = KeyEvent::new(code, modifiers);
        release.kind = terminal_input::event::KeyEventKind::Release;
        assert_eq!(app.handle(Event::Key(release)), Action::Continue);
        assert_eq!(app.query, "split");
    }
    app.handle(key(KeyCode::Char('u')));
    assert_eq!(app.query, "splitu");
}

#[test]
fn rename_rejects_blank_input_preserves_literal_unicode_and_escape_returns_to_results() {
    let mut app = app();
    app.set_query("Rename Tab".into());
    app.handle(key(KeyCode::Enter));
    assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
    assert!(draw(&mut app, 80, 20).contains("Enter a nonblank name"));
    app.handle(Event::Paste("--世界 Review".into()));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Execute(herdrpp::palette::app::Selection {
            id: "herdr:tab_rename".into(),
            input: Some("--世界 Review".into()),
            confirmed: false,
        })
    );
    assert_eq!(app.handle(key(KeyCode::Esc)), Action::Continue);
    assert_eq!(app.mode, Mode::Search);
    assert_eq!(app.query, "Rename Tab");
    assert_eq!(app.handle(key(KeyCode::Esc)), Action::Cancel);
}

#[test]
fn close_requires_confirmation_and_displays_original_target() {
    let mut app = app();
    app.set_query("Close Tab".into());
    assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
    assert!(draw(&mut app, 80, 20).contains("Close Tab: Codex?"));
    assert_eq!(app.handle(key(KeyCode::Esc)), Action::Continue);
    app.handle(key(KeyCode::Enter));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Execute(herdrpp::palette::app::Selection {
            id: "herdr:tab_close".into(),
            input: None,
            confirmed: true,
        })
    );
}

#[test]
fn shared_view_has_search_input_count_actions_and_unicode_cursor_stays_in_bounds() {
    let mut app = app();
    let rendered = draw(&mut app, 80, 20);
    assert!(rendered.contains("Search commands"));
    assert!(rendered.contains("16 results"));
    assert!(rendered.contains("↵ OK"));
    assert!(rendered.contains("Esc Cancel"));
    app.set_query("世界".repeat(100));
    assert!(draw(&mut app, 16, 8).contains("0 results"));
}

#[test]
fn resize_clears_hits_and_mouse_selection_executes_the_clicked_result() {
    let mut app = app();
    app.set_query("Split Right".into());
    draw(&mut app, 80, 20);
    let area = app.hits[0].0;
    assert_eq!(
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE
        })),
        Action::Execute(herdrpp::palette::app::Selection {
            id: "herdr:split_right".into(),
            input: None,
            confirmed: false
        })
    );
    app.handle(Event::Resize(5, 2));
    assert!(app.hits.is_empty());
    assert!(draw(&mut app, 5, 2).contains("Esc c"));
}

#[test]
fn navigation_clamps_scrolls_and_ctrl_c_cancels_a_prompt() {
    let mut app = app();
    app.navigate(100);
    draw(&mut app, 80, 10);
    assert_eq!(app.selected, Some(15));
    assert!(app.offset > 0);
    app.navigate(-100);
    assert_eq!(app.selected, Some(0));
    app.handle(key(KeyCode::Enter));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        ))),
        Action::Cancel
    );
}
