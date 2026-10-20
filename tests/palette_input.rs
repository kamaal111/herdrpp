use herdrpp::{
    palette::{
        app::{Action, App, Selection},
        catalog::{self, Context},
        naming::Naming,
    },
    picker::theme::Theme,
};
use terminal_input::event::{Event, KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn app() -> App {
    let context: Context = serde_json::from_value(
        serde_json::json!({"workspace_id":"w2","tab_id":"w2:t7","focused_pane_id":"w2:p8"}),
    )
    .unwrap();
    App::new(
        context.clone(),
        catalog::builtins(&context),
        Naming {
            tab_name: "Codex".into(),
            tab_suggestion: "3".into(),
            ..Naming::default()
        },
        Theme::default(),
        None,
    )
}

#[test]
fn search_arrows_insert_paste_and_backspace_at_the_cursor() {
    let mut app = app();
    app.set_query("abcd".into());
    app.handle(key(KeyCode::Left));
    app.handle(key(KeyCode::Left));
    app.handle(key(KeyCode::Right));
    app.handle(key(KeyCode::Backspace));
    assert_eq!(app.query(), "abd");
    app.handle(Event::Paste("X\nY".into()));
    assert_eq!(app.query(), "abXYd");
    app.handle(key(KeyCode::Char('!')));
    assert_eq!(app.query(), "abXY!d");
}

#[test]
fn rename_backspace_deletes_one_character_before_the_cursor_and_submits() {
    let mut app = app();
    app.set_query("Rename Tab".into());
    app.handle(key(KeyCode::Enter));
    app.handle(key(KeyCode::Left));
    app.handle(key(KeyCode::Backspace));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Execute(Selection {
            id: "herdr:tab_rename".into(),
            input: Some("Codx".into()),
            confirmed: false,
        })
    );
}

#[test]
fn creation_left_collapses_selection_and_inserts_without_replacing_suggestion() {
    let mut app = app();
    app.set_query("New Tab".into());
    app.handle(key(KeyCode::Enter));
    app.handle(key(KeyCode::Left));
    app.handle(Event::Paste("Review".into()));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Execute(Selection {
            id: "herdr:tab_create".into(),
            input: Some("Review3".into()),
            confirmed: false,
        })
    );
}

#[test]
fn search_clear_shortcuts_clear_all_cursor_positions_and_restore_results() {
    for (code, modifiers) in [
        (KeyCode::Backspace, KeyModifiers::SUPER),
        (KeyCode::Char('u'), KeyModifiers::CONTROL),
    ] {
        for lefts in [0, 2, 4] {
            let mut app = app();
            app.set_query("zzzz".into());
            for _ in 0..lefts {
                app.handle(key(KeyCode::Left));
            }
            app.handle(Event::Key(KeyEvent::new(code, modifiers)));
            assert_eq!(app.query(), "");
            assert_eq!(app.input().cursor(), 0);
            assert!(!app.results.is_empty());
            assert_eq!(app.selected, Some(0));
        }
    }
}

#[test]
fn name_clear_shortcuts_clear_all_cursor_positions_for_create_and_rename() {
    for query in [
        "New Tab",
        "New Workspace",
        "Rename Tab",
        "Rename Workspace",
        "Rename Pane",
    ] {
        for (code, modifiers) in [
            (KeyCode::Backspace, KeyModifiers::SUPER),
            (KeyCode::Char('u'), KeyModifiers::CONTROL),
        ] {
            for lefts in [0, 2, 4] {
                let mut app = app();
                app.set_query(query.into());
                app.handle(key(KeyCode::Enter));
                app.handle(Event::Key(KeyEvent::new(code, modifiers)));
                app.handle(Event::Paste("abcd".into()));
                for _ in 0..lefts {
                    app.handle(key(KeyCode::Left));
                }
                app.handle(Event::Key(KeyEvent::new(code, modifiers)));
                app.handle(Event::Paste("Review".into()));
                let expected_id = app.items[app.results[0].item].id.clone();
                assert_eq!(
                    app.handle(key(KeyCode::Enter)),
                    Action::Execute(Selection {
                        id: expected_id,
                        input: Some("Review".into()),
                        confirmed: false,
                    })
                );
            }
        }
    }
}

#[test]
fn cancelling_name_prompt_restores_search_cursor_and_selection() {
    let mut app = app();
    app.set_query("Rename Tab".into());
    app.handle(key(KeyCode::Left));
    let selected = app.selected;
    app.handle(key(KeyCode::Enter));
    app.handle(key(KeyCode::Left));
    app.handle(key(KeyCode::Esc));
    assert_eq!(app.query(), "Rename Tab");
    assert_eq!(app.input().cursor(), 9);
    assert_eq!(app.selected, selected);
}

#[test]
fn search_arrows_do_not_reset_manual_selection_scroll_or_hits() {
    let mut app = app();
    app.set_query("Tab".into());
    app.navigate(1);
    let selected = app.selected;
    app.offset = 2;
    app.hits.push((ratatui::layout::Rect::new(0, 2, 10, 1), 3));
    app.handle(key(KeyCode::Left));
    app.handle(key(KeyCode::Right));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::SUPER,
    )));
    assert_eq!(app.input().cursor(), 0);
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::SUPER,
    )));
    assert_eq!(app.input().cursor(), 3);
    assert_eq!(app.query(), "Tab");
    assert_eq!(app.selected, selected);
    assert_eq!(app.offset, 2);
    assert_eq!(app.hits.len(), 1);
}

#[test]
fn command_arrows_insert_at_both_edges_of_rename_and_creation_prompts() {
    for (query, id, expected) in [
        ("Rename Tab", "herdr:tab_rename", "ACodexZ"),
        ("New Tab", "herdr:tab_create", "A3Z"),
    ] {
        let mut app = app();
        app.set_query(query.into());
        app.handle(key(KeyCode::Enter));
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::SUPER,
        )));
        app.handle(key(KeyCode::Char('A')));
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::SUPER,
        )));
        app.handle(key(KeyCode::Char('Z')));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Execute(Selection {
                id: id.into(),
                input: Some(expected.into()),
                confirmed: false,
            })
        );
    }
}
