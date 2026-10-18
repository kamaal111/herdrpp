pub mod config;
pub mod runner;

use crate::{
    palette::{
        catalog::{Command, Context, Item},
        cli::Catalog,
    },
    picker::api::Herdr,
};
use anyhow::{Context as _, Result};
use std::{fs, io::Read, path::PathBuf, process, time::Duration};

pub fn root(api: &Herdr, context: &Context) -> Result<PathBuf> {
    let snapshot = api.raw_snapshot()?;
    crate::palette::api::validate_context(&snapshot, context)?;
    let workspace = snapshot["workspaces"]
        .as_array()
        .context("missing workspaces")?
        .iter()
        .find(|w| w["workspace_id"].as_str() == context.workspace_id.as_deref())
        .context("opening workspace is unavailable")?;
    if let Some(path) = workspace["worktree"]["checkout_path"].as_str() {
        return fs::canonicalize(path).context("resolve workspace checkout root");
    }
    let pane = snapshot["panes"]
        .as_array()
        .context("missing panes")?
        .iter()
        .find(|p| p["pane_id"].as_str() == context.focused_pane_id.as_deref())
        .context("opening pane is unavailable")?;
    let cwd = pane["foreground_cwd"]
        .as_str()
        .or(pane["cwd"].as_str())
        .or(context.focused_pane_cwd.as_deref())
        .context("opening pane has no working directory")?;
    let bytes = crate::command::run(
        process::Command::new("git").args(["-C", cwd, "rev-parse", "--show-toplevel"]),
        Duration::from_secs(5),
    )
    .context("cannot resolve checkout root; open tasks from a Git checkout")?;
    let path = std::str::from_utf8(&bytes)?.trim_end_matches('\n');
    fs::canonicalize(path).context("resolve checkout root")
}

pub fn load(api: &Herdr, context: &Context) -> Catalog {
    let loaded = (|| -> Result<(Vec<Item>, Option<String>)> {
        let root = root(api, context)?;
        let path = root.join(".herdr/tasks.json");
        let empty = || {
            format!(
                "No tasks configured\n{}\nCreate a JSON array, for example:\n[{{\"label\":\"test\",\"command\":\"just\",\"args\":[\"test\"]}}]",
                path.display()
            )
        };
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((vec![], Some(empty())));
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("Cannot read {}; check the file permissions", path.display())
                });
            }
        };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        anyhow::ensure!(
            bytes.len() <= 1024 * 1024,
            "{}: task file exceeds 1 MiB; reduce its size",
            path.display()
        );
        let text = std::str::from_utf8(&bytes)
            .with_context(|| format!("{}: save the file as UTF-8", path.display()))?;
        let tasks = match config::parse(text) {
            Ok(tasks) => tasks,
            Err(errors) => {
                return Ok((
                    vec![],
                    Some(format!(
                        "Fix {}\n\n{}\n\nSave the file and reopen tasks to reload.",
                        path.display(),
                        errors.join("\n\n")
                    )),
                ));
            }
        };
        if tasks.is_empty() {
            return Ok((vec![], Some(empty())));
        }
        let items = tasks
            .into_iter()
            .enumerate()
            .map(|(index, task)| Item {
                id: format!("task:{index}"),
                title: task.label.clone(),
                source: "Task".into(),
                description: task.command.clone(),
                command: Command::Task {
                    task,
                    root: root.clone(),
                },
            })
            .collect();
        Ok((items, None))
    })();
    let (items, warning) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => (vec![], Some(format!("Cannot load tasks\n\n{error:#}"))),
    };
    Catalog {
        context: context.clone(),
        naming: crate::palette::naming::Naming::default(),
        items,
        warning,
        tasks: true,
    }
}
