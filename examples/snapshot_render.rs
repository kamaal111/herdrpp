//! Export otherwise unreachable UI states for the visual snapshot harness.
use anyhow::{Context, Result, bail};
use herdrpp::picker::{app::App, config::SearchMode, model::Snapshot, theme::Theme, ui};
use ratatui::{Terminal, backend::TestBackend, style::Modifier};
use serde_json::json;

fn main() -> Result<()> {
    let case = std::env::args().nth(1).context("expected snapshot case")?;
    let (width, height) = match case.as_str() {
        "empty-snapshot" | "loading" | "agent-status-dots" | "agent-status-symbols" => (109, 20),
        "tiny-terminal" => (6, 1),
        "tasks-search"
        | "tasks-diagnostics"
        | "palette-search"
        | "palette-empty"
        | "palette-rename"
        | "palette-create"
        | "palette-confirm"
        | "palette-loading"
        | "palette-branches"
        | "palette-worktree-name"
        | "palette-branch-warning" => (109, 20),
        "palette-narrow" => (32, 12),
        _ => bail!("unknown controlled snapshot: {case}"),
    };
    let mut app = App::new(
        Snapshot {
            agents: vec![],
            workspaces: vec![],
            tabs: vec![],
            focused_workspace_id: None,
            focused_tab_id: None,
        },
        SearchMode::Hierarchical,
    );
    app.theme = Theme::parse("[theme]\nname = 'dracula'\n")?;
    if case.starts_with("agent-status-") {
        let agents: Vec<_> = [
            ("w1", "w1:t1", "working"),
            ("w1", "w1:t1", "blocked"),
            ("w1", "w1:t1", "done"),
            ("w1", "w1:t1", "working"),
            ("w2", "w2:t1", "done"),
            ("w2", "w2:t1", "idle"),
        ]
        .into_iter()
        .chain(std::iter::repeat_n(("w2", "w2:t2", "idle"), 40))
        .map(|(workspace, tab, status)| {
            json!({"workspace_id":workspace,"tab_id":tab,"agent_status":status})
        })
        .collect();
        let mut snapshot: Snapshot = serde_json::from_value(json!({
            "agents": agents,
            "workspaces": [
                {"workspace_id":"w1","number":1,"label":"herdrpp","active_tab_id":"w1:t1"},
                {"workspace_id":"w2","number":2,"label":"feature/status","active_tab_id":"w2:t1","worktree":{"repo_key":"herdrpp","is_linked_worktree":true}},
                {"workspace_id":"w3","number":3,"label":"docs","active_tab_id":"w3:t1"}
            ],
            "tabs": [
                {"workspace_id":"w1","tab_id":"w1:t1","number":1,"label":"Working","agent_status":"working"},
                {"workspace_id":"w1","tab_id":"w1:t2","number":2,"label":"Blocked","agent_status":"blocked"},
                {"workspace_id":"w2","tab_id":"w2:t1","number":1,"label":"Done","agent_status":"done"},
                {"workspace_id":"w2","tab_id":"w2:t2","number":2,"label":"Read","agent_status":"idle"},
                {"workspace_id":"w3","tab_id":"w3:t1","number":1,"label":"Terminal","agent_status":"unknown"}
            ],
            "focused_workspace_id":"w1","focused_tab_id":"w1:t1"
        }))?;
        snapshot.workspaces[0].worktree =
            serde_json::from_value(json!({"repo_key":"herdrpp","is_linked_worktree":false}))?;
        snapshot.validate()?;
        app = App::new(snapshot, SearchMode::Hierarchical);
        let indicators = if case == "agent-status-symbols" {
            "symbols"
        } else {
            "dots"
        };
        app.theme = Theme::parse(&format!(
            "[theme]\nname = 'dracula'\n[ui]\nstatus_indicators = '{indicators}'"
        ))?;
    }
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
        herdrpp::palette::registry::Registry::bundled()?.screen("commands")?,
    );
    palette.items = herdrpp::palette::catalog::builtins(&palette.context)?;
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
        palette.screen = herdrpp::palette::registry::Registry::bundled()?.screen("tasks")?;
        palette.items = vec![herdrpp::palette::catalog::Item::new(
            "task:0".into(),
            task.label.clone(),
            "Task".into(),
            "just dev-server".into(),
            herdrpp::palette::catalog::Command::Task {
                task,
                root: "/project".into(),
            },
        )?];
        palette.set_query("dev".into());
        if case == "tasks-diagnostics" {
            palette.items.clear();
            palette.set_query(String::new());
            palette.warning = Some("Fix /project/.herdr/tasks.json\n\nTask 1 (\"test\"), new_tab: expected true or false, received string. Remove the quotes.\n\nTask 2 (\"build\"), command: expected a nonblank string; add a quoted string value.\n\nSave the file and reopen tasks to reload.".into());
        }
    }
    if matches!(
        case.as_str(),
        "palette-branches" | "palette-worktree-name" | "palette-branch-warning"
    ) {
        use herdrpp::palette::worktree::{Branch, branch_items};
        let branches = [
            Branch {
                reference: "refs/heads/main".into(),
                name: "main".into(),
                remote: None,
            },
            Branch {
                reference: "refs/heads/feature/palette".into(),
                name: "feature/palette".into(),
                remote: None,
            },
            Branch {
                reference: "refs/remotes/origin/feature/report".into(),
                name: "feature/report".into(),
                remote: Some("origin".into()),
            },
        ];
        palette.items = branch_items(
            std::path::Path::new("/project"),
            &branches,
            true,
            Some("feature/palette"),
        )?;
        palette.screen =
            herdrpp::palette::registry::Registry::bundled()?.screen("base_branches")?;
        palette.set_query(String::new());
        if case == "palette-worktree-name" {
            palette.handle(terminal_input::event::Event::Key(
                terminal_input::event::KeyEvent::new(
                    terminal_input::event::KeyCode::Enter,
                    terminal_input::event::KeyModifiers::NONE,
                ),
            ));
            palette.handle(terminal_input::event::Event::Paste(
                "feature/new-worktree".into(),
            ));
        }
        if case == "palette-branch-warning" {
            palette.warning =
                Some("Fetch origin failed: remote unavailable; using known branches".into());
        }
    }
    terminal.draw(|frame| {
        if case == "palette-loading" {
            herdrpp::palette::ui::render_loading(frame, palette.theme.palette(), &palette.screen);
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
