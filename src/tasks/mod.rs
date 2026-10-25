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

#[derive(Debug)]
struct Discovered {
    path: PathBuf,
    format: config::Format,
    file: std::io::Result<fs::File>,
    ignored: Vec<PathBuf>,
}

impl Discovered {
    fn notice(&self) -> Option<String> {
        if self.ignored.is_empty() {
            return None;
        }
        let names = self
            .ignored
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!("Using {}. Ignored: {names}.", self.path.display()))
    }
}

fn discover(root: &Path) -> Result<Option<Discovered>> {
    for directory in root.ancestors().take(11) {
        for (index, (name, format)) in config::FILES.iter().enumerate() {
            let path = directory.join(".herdr").join(name);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    let file = fs::File::open(&path);
                    let ignored = config::FILES[index + 1..]
                        .iter()
                        .map(|(name, _)| directory.join(".herdr").join(name))
                        .filter(|path| match fs::symlink_metadata(path) {
                            Ok(_) => true,
                            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
                        })
                        .collect();
                    return Ok(Some(Discovered {
                        path,
                        format: *format,
                        file,
                        ignored,
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("Cannot read {}; check the file permissions", path.display())
                    });
                }
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
    let mut notice = None;
    let loaded = (|| -> Result<(Vec<Item>, Option<String>)> {
        let root = root(api, context)?;
        let discovered = discover(&root)?;
        let missing = discovered.is_none();
        let (path, format, file) = match discovered {
            Some(found) => {
                notice = found.notice();
                (found.path, found.format, Some(found.file))
            }
            None => (root.join(".herdr/tasks.toml"), config::Format::Toml, None),
        };
        let empty = || {
            let example = match format {
                config::Format::Toml => "[[tasks]]\nlabel = \"test\"\ncommand = \"just test\"",
                config::Format::Yaml => "tasks:\n  - label: test\n    command: just test",
                config::Format::Json5 => {
                    "{\"tasks\": [{\"label\": \"test\", \"command\": \"just test\"}]}"
                }
            };
            let names = config::FILES
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "No tasks configured\n{}\n{}Supported files (in priority order): {names}.\nCreate a task configuration, for example:\n{example}",
                path.display(),
                if missing {
                    "Searched the checkout and up to 10 parent folders.\n"
                } else {
                    ""
                }
            )
        };
        let Some(file) = file else {
            return Ok((vec![], Some(empty())));
        };
        let mut file = file.with_context(|| {
            format!("Cannot read {}; check the file permissions", path.display())
        })?;
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
        let tasks = match config::parse_with_format(text, format) {
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
        notice,
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
        assert_eq!(discover(&checkout).unwrap().unwrap().path, local_path);
        fs::remove_file(local_path).unwrap();
        assert_eq!(discover(&checkout).unwrap().unwrap().path, parent_path);
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
        assert_eq!(discover(&checkout).unwrap().unwrap().path, path);
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
                .contains(&checkout.join(".herdr/tasks.toml").display().to_string())
        );
        assert!(error.to_string().contains("check the file permissions"));
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn discovery_orders_all_formats_and_reports_ignored_siblings() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join(".herdr");
        fs::create_dir(&directory).unwrap();
        for (name, _) in config::FILES {
            fs::write(directory.join(name), "not parsed during discovery").unwrap();
        }
        let expected = [
            ("tasks.toml", config::Format::Toml),
            ("tasks.yaml", config::Format::Yaml),
            ("tasks.yml", config::Format::Yaml),
            ("tasks.json", config::Format::Json5),
            ("tasks.json5", config::Format::Json5),
        ];
        for (index, (name, format)) in expected.iter().enumerate() {
            let discovered = discover(temp.path()).unwrap().unwrap();
            assert_eq!(discovered.path, directory.join(name));
            assert_eq!(discovered.format, *format);
            assert_eq!(
                discovered.ignored,
                expected[index + 1..]
                    .iter()
                    .map(|(name, _)| directory.join(name))
                    .collect::<Vec<_>>()
            );
            if discovered.ignored.is_empty() {
                assert!(discovered.notice().is_none());
            } else {
                let notice = discovered.notice().unwrap();
                assert!(notice.contains(&directory.display().to_string()));
                assert!(notice.contains("Ignored:"));
                for (name, _) in &expected[index + 1..] {
                    assert!(notice.contains(name));
                }
            }
            fs::remove_file(discovered.path).unwrap();
        }
    }

    #[test]
    fn nearer_json5_wins_over_parent_toml_and_parent_notices_use_resolved_folder() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("api");
        fs::create_dir_all(checkout.join(".herdr")).unwrap();
        fs::create_dir(temp.path().join(".herdr")).unwrap();
        let parent_toml = temp.path().join(".herdr/tasks.toml");
        fs::write(&parent_toml, "tasks=[]").unwrap();
        fs::write(temp.path().join(".herdr/tasks.yaml"), "tasks: []").unwrap();
        let local_json5 = checkout.join(".herdr/tasks.json5");
        fs::write(&local_json5, "[]").unwrap();
        let found = discover(&checkout).unwrap().unwrap();
        assert_eq!(found.path, local_json5);
        assert!(found.notice().is_none());
        fs::remove_file(local_json5).unwrap();
        let found = discover(&checkout).unwrap().unwrap();
        assert_eq!(found.path, parent_toml);
        assert_eq!(found.ignored, [temp.path().join(".herdr/tasks.yaml")]);
        assert!(
            found
                .notice()
                .unwrap()
                .contains(&parent_toml.display().to_string())
        );
    }
}
