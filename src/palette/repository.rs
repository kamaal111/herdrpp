use super::catalog::{Command, Context, Item};
use crate::pr::herdr::Snapshot;
use anyhow::Result;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process,
    sync::atomic::AtomicBool,
    time::Duration,
};

pub fn directory_from_snapshot(snapshot: &Value, context: &Context) -> Option<PathBuf> {
    let workspace_id = context.workspace_id.as_deref()?;
    let snapshot: Snapshot = serde_json::from_value(snapshot.clone()).ok()?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == workspace_id)?;
    let directory = snapshot.directory(workspace)?;
    directory.is_absolute().then(|| directory.to_path_buf())
}

pub fn item(directory: &Path, cancelled: &AtomicBool) -> Result<Option<Item>> {
    crate::repository::repository_url(directory, cancelled)?
        .map(|url| {
            Item::new(
                "herdr:open_repo".into(),
                String::new(),
                "Herdr".into(),
                String::new(),
                Command::OpenRepo { url },
            )
        })
        .transpose()
}

pub fn open_url(url: &str) -> Result<()> {
    let launcher = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    crate::command::run(
        process::Command::new(launcher).arg(url),
        Duration::from_secs(5),
    )?;
    Ok(())
}
