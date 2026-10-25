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
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process,
    time::Duration,
};

fn discover(root: &Path) -> Result<Option<(PathBuf, fs::File)>> {
    for directory in root.ancestors().take(11) {
        let path = directory.join(".herdr/tasks.json");
        match fs::File::open(&path) {
            Ok(file) => return Ok(Some((path, file))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Cannot read {}; check the file permissions", path.display())
                });
            }
        }
    }
    Ok(None)
}

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

pub fn load(
    api: &Herdr,
    context: &Context,
    screen: crate::palette::registry::Screen,
) -> Result<Catalog> {
    let loaded = (|| -> Result<(Vec<Item>, Option<String>)> {
        let root = root(api, context)?;
        let discovered = discover(&root)?;
        let missing = discovered.is_none();
        let (path, file) = match discovered {
            Some((path, file)) => (path, Some(file)),
            None => (root.join(".herdr/tasks.json"), None),
        };
        let empty = || {
            format!(
                "No tasks configured\n{}\n{}Create a JSON array, for example:\n[{{\"label\":\"test\",\"command\":\"just\",\"args\":[\"test\"]}}]",
                path.display(),
                if missing {
                    "Searched the checkout and up to 10 parent folders.\n"
                } else {
                    ""
                }
            )
        };
        let Some(mut file) = file else {
            return Ok((vec![], Some(empty())));
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
            .map(|(index, task)| {
                Item::new(
                    format!("task:{index}"),
                    task.label.clone(),
                    String::new(),
                    task.command.clone(),
                    Command::Task {
                        task,
                        root: root.clone(),
                    },
                )
            })
            .collect::<Result<_>>()?;
        Ok((items, None))
    })();
    let (items, warning) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => (vec![], Some(format!("Cannot load tasks\n\n{error:#}"))),
    };
    Ok(Catalog {
        screen,
        payload: crate::palette::providers::Payload::None,
        recent_ids: vec![],
        context: context.clone(),
        naming: crate::palette::naming::Naming::default(),
        items,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tasks(directory: &Path) -> PathBuf {
        fs::create_dir_all(directory.join(".herdr")).unwrap();
        let path = directory.join(".herdr/tasks.json");
        fs::write(&path, "[]").unwrap();
        path
    }

    #[test]
    fn discovery_prefers_checkout_then_nearest_parent() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("projects");
        let checkout = parent.join("api");
        write_tasks(temp.path());
        let parent_path = write_tasks(&parent);
        let local_path = write_tasks(&checkout);
        assert_eq!(discover(&checkout).unwrap().unwrap().0, local_path);
        fs::remove_file(local_path).unwrap();
        assert_eq!(discover(&checkout).unwrap().unwrap().0, parent_path);
    }

    #[test]
    fn discovery_includes_tenth_parent_and_excludes_eleventh() {
        let temp = tempfile::tempdir().unwrap();
        let tenth = temp.path().join("base");
        let checkout = tenth.join("1/2/3/4/5/6/7/8/9/10");
        fs::create_dir_all(&checkout).unwrap();
        write_tasks(temp.path());
        assert!(discover(&checkout).unwrap().is_none());
        let path = write_tasks(&tenth);
        assert_eq!(discover(&checkout).unwrap().unwrap().0, path);
    }

    #[test]
    fn discovery_propagates_non_missing_open_errors() {
        let temp = tempfile::tempdir().unwrap();
        write_tasks(temp.path());
        let checkout = temp.path().join("api");
        fs::create_dir(&checkout).unwrap();
        fs::write(checkout.join(".herdr"), "not a directory").unwrap();
        let error = discover(&checkout).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&checkout.join(".herdr/tasks.json").display().to_string())
        );
        assert!(error.to_string().contains("check the file permissions"));
    }
}
