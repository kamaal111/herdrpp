//! Export otherwise unreachable UI states for the visual snapshot harness.
use anyhow::{Context, Result, bail};
use herdrpp::picker::{app::App, config::SearchMode, model::Snapshot, theme::Theme, ui};
use ratatui::{Terminal, backend::TestBackend, style::Modifier};
use serde_json::json;

fn main() -> Result<()> {
    let case = std::env::args().nth(1).context("expected snapshot case")?;
    let (width, height) = match case.as_str() {
        "empty-snapshot" | "loading" => (109, 20),
        "tiny-terminal" => (6, 1),
        "tasks-search" | "tasks-diagnostics" | "palette-search" | "palette-empty"
        | "palette-rename" | "palette-create" | "palette-confirm" | "palette-loading" => (109, 20),
        "palette-narrow" => (32, 12),
        _ => bail!("unknown controlled snapshot: {case}"),
    };
    let mut app = App::new(
        Snapshot {
            workspaces: vec![],
            tabs: vec![],
            focused_workspace_id: None,
            focused_tab_id: None,
        },
        SearchMode::Hierarchical,
    );
    app.theme = Theme::parse("[theme]\nname = 'dracula'\n")?;
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    let context = serde_json::from_value(
        json!({"workspace_id":"w2","tab_id":"w2:t7","focused_pane_id":"w2:p8","tab_label":"Codex"}),
    )?;
    let mut palette = herdrpp::palette::app::App::new(
        context,
        vec![],
        herdrpp::palette::naming::Naming {
            tab_name: "Codex".into(),
            tab_suggestion: "3".into(),
            ..Default::default()
        },
        Theme::parse("[theme]\nname = 'dracula'\n")?,
        None,
    );
    palette.items = herdrpp::palette::catalog::builtins(&palette.context);
    palette.set_query(
        match case.as_str() {
            "palette-search" | "palette-narrow" => "spl rgt",
            "palette-empty" => "nonexistent",
            "palette-rename" => "Rename Tab",
            "palette-create" => "New Tab",
            "palette-confirm" => "Close Tab",
            _ => "",
        }
        .into(),
    );
    if matches!(
        case.as_str(),
        "palette-rename" | "palette-create" | "palette-confirm"
    ) {
        palette.handle(terminal_input::event::Event::Key(
            terminal_input::event::KeyEvent::new(
                terminal_input::event::KeyCode::Enter,
                terminal_input::event::KeyModifiers::NONE,
            ),
        ));
    }
    if case.starts_with("tasks-") {
        let task = herdrpp::tasks::config::Task {
            label: "just: dev-server".into(),
            command: "just".into(),
            args: vec!["dev-server".into()],
            new_tab: true,
        };
        palette.tasks = true;
        palette.items = vec![herdrpp::palette::catalog::Item {
            id: "task:0".into(),
            title: task.label.clone(),
            source: "Task".into(),
            description: "just dev-server".into(),
            command: herdrpp::palette::catalog::Command::Task {
                task,
                root: "/project".into(),
            },
        }];
        palette.set_query("dev".into());
        if case == "tasks-diagnostics" {
            palette.items.clear();
            palette.set_query(String::new());
            palette.warning = Some("Fix /project/.herdr/tasks.json\n\nTask 1 (\"test\"), new_tab: expected true or false, received string. Remove the quotes.\n\nTask 2 (\"build\"), command: expected a nonblank string; add a quoted string value.\n\nSave the file and reopen tasks to reload.".into());
        }
    }
    terminal.draw(|frame| {
        if case == "palette-loading" {
            herdrpp::palette::ui::render_loading(frame, palette.theme.palette());
        } else if case.starts_with("palette-") || case.starts_with("tasks-") {
            herdrpp::palette::ui::render(frame, &mut palette);
        } else if case == "loading" {
            ui::render_loading(frame, app.theme.palette());
        } else {
            ui::render(frame, &mut app);
        }
    })?;
    let cells: Vec<_> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| {
            json!({
                "text": cell.symbol(),
                "fg": format!("{:?}", cell.fg),
                "bg": format!("{:?}", cell.bg),
                "bold": cell.modifier.contains(Modifier::BOLD),
                "underline": cell.modifier.contains(Modifier::UNDERLINED),
                "reverse": cell.modifier.contains(Modifier::REVERSED),
            })
        })
        .collect();
    println!(
        "{}",
        json!({"width": width, "height": height, "cells": cells})
    );
    Ok(())
}
