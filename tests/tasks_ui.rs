use herdrpp::{
    palette::{
        app::{Action, App},
        catalog::{self, Command, Item},
        ui,
    },
    picker::theme::Theme,
    tasks::{config::Task, runner::idle},
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use terminal_input::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

fn task_app() -> App {
    let task = Task {
        label: "dev server".into(),
        command: "hidden-command".into(),
        args: vec![],
        new_tab: true,
    };
    let item = Item {
        id: "task:0".into(),
        title: task.label.clone(),
        source: "Task".into(),
        description: task.command.clone(),
        command: Command::Task {
            task,
            root: "/tmp".into(),
        },
    };
    let mut app = App::new(
        catalog::Context::default(),
        vec![item],
        herdrpp::palette::naming::Naming::default(),
        Theme::default(),
        None,
    );
    app.tasks = true;
    app
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn draw(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn tasks_search_only_labels_and_keyboard_selection_runs_without_prompts() {
    let mut app = task_app();
    app.set_query("hidden-command".into());
    assert!(app.results.is_empty());
    app.set_query("dv srv".into());
    assert_eq!(app.results.len(), 1);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Execute(herdrpp::palette::app::Selection {
            id: "task:0".into(),
            input: None,
            confirmed: false
        })
    );
}

#[test]
fn task_ui_has_task_copy_and_mouse_selection_reuses_palette_behavior() {
    let mut app = task_app();
    let text = draw(&mut app, 80, 20);
    assert!(text.contains("Search tasks"));
    assert!(text.contains("Task Palette"));
    let point = app.hits[0].0;
    let result = app.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: point.x,
        row: point.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        result,
        Action::Execute(herdrpp::palette::app::Selection {
            id: "task:0".into(),
            input: None,
            confirmed: false
        })
    );
}

#[test]
fn diagnostic_view_scrolls_wrapped_errors_and_disables_execution() {
    let mut app = task_app();
    app.warning = Some(format!(
        "Fix /project/.herdr/tasks.json\n{}\nFINAL FIX",
        "Task 1: replace new_tab with true or false.\n".repeat(30)
    ));
    let first = draw(&mut app, 32, 12);
    assert!(first.contains("Fix /project/.herdr/tasks.json"));
    assert!(!first.contains("FINAL FIX"));
    assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
    assert_eq!(app.handle(key(KeyCode::End)), Action::Continue);
    assert!(draw(&mut app, 32, 12).contains("FINAL FIX"));
    assert_eq!(app.handle(key(KeyCode::Esc)), Action::Cancel);
}

#[test]
fn unicode_diagnostics_preserve_every_character_when_wrapped() {
    let message = "Task 世界: fix this field\n\nsecond line";
    let lines = ui::diagnostic_lines(message, 8);
    assert_eq!(lines.join("").replace('\n', ""), message.replace('\n', ""));
    assert!(
        lines
            .iter()
            .all(|line| unicode_width::UnicodeWidthStr::width(line.as_str()) <= 8)
    );
}

#[test]
fn only_a_known_foreground_shell_is_idle() {
    assert!(idle(
        &json!({"shell_pid":10,"foreground_process_group_id":10,"foreground_processes":[{"pid":10,"name":"zsh"}]})
    ));
    assert!(!idle(
        &json!({"shell_pid":10,"foreground_process_group_id":20,"foreground_processes":[{"pid":20,"name":"cargo"}]})
    ));
    assert!(!idle(
        &json!({"shell_pid":10,"foreground_process_group_id":10,"foreground_processes":[{"pid":10,"name":"herdrpp"}]})
    ));
    assert!(!idle(
        &json!({"shell_pid":10,"foreground_process_group_id":10,"foreground_processes":[]})
    ));
    assert!(!idle(&json!({})));
}

#[test]
fn task_search_uses_cursor_editing_and_single_character_backspace() {
    let mut app = task_app();
    app.set_query("dev servxr".into());
    app.handle(key(KeyCode::Left));
    app.handle(key(KeyCode::Backspace));
    assert_eq!(app.query(), "dev servr");
    app.handle(key(KeyCode::Char('e')));
    assert_eq!(app.query(), "dev server");
    assert_eq!(app.results.len(), 1);
    app.handle(key(KeyCode::Right));
    app.handle(Event::Paste("\n!".into()));
    assert_eq!(app.query(), "dev server!");
}

#[test]
fn task_search_clear_shortcuts_clear_every_cursor_position() {
    for (code, modifiers) in [
        (KeyCode::Backspace, KeyModifiers::SUPER),
        (KeyCode::Char('u'), KeyModifiers::CONTROL),
    ] {
        for lefts in [0, 2, 4] {
            let mut app = task_app();
            app.set_query("zzzz".into());
            for _ in 0..lefts {
                app.handle(key(KeyCode::Left));
            }
            app.handle(Event::Key(KeyEvent::new(code, modifiers)));
            assert_eq!(app.query(), "");
            assert_eq!(app.input().cursor(), 0);
            assert_eq!(app.results.len(), 1);
            assert_eq!(app.selected, Some(0));
        }
    }
}

#[test]
fn recent_tasks_precede_unused_file_order_and_clear_restores_selection() {
    let mut app = task_app();
    let mut build = app.items[0].clone();
    build.id = "task:1".into();
    build.title = "build".into();
    let mut test = app.items[0].clone();
    test.id = "task:2".into();
    test.title = "test".into();
    app.items = vec![build, app.items[0].clone(), test];
    app.set_recent_ids(vec!["task:2".into(), "task:0".into()]);
    assert_eq!(
        app.results
            .iter()
            .map(|hit| app.items[hit.item].title.as_str())
            .collect::<Vec<_>>(),
        vec!["test", "dev server", "build"]
    );
    app.set_query("build".into());
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::SUPER,
    )));
    assert_eq!(app.selected, Some(0));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Execute(herdrpp::palette::app::Selection {
            id: "task:2".into(),
            input: None,
            confirmed: false
        })
    );
}

#[test]
fn task_search_command_arrows_jump_without_resetting_results() {
    let mut app = task_app();
    app.set_query("dev server".into());
    let selected = app.selected;
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::SUPER,
    )));
    assert_eq!(app.input().cursor(), 0);
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::SUPER,
    )));
    assert_eq!(app.input().cursor(), 10);
    assert_eq!(app.query(), "dev server");
    assert_eq!(app.results.len(), 1);
    assert_eq!(app.selected, selected);
}
