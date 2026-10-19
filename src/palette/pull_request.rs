use super::catalog::{Command, Context, Item};
use crate::pr::herdr::Snapshot;
use anyhow::Result;
use serde_json::Value;
use std::{process, time::Duration};

pub fn item_from_snapshot(snapshot: &Value, context: &Context) -> Option<Item> {
    let workspace_id = context.workspace_id.as_deref()?;
    let workspace = snapshot["workspaces"]
        .as_array()?
        .iter()
        .find(|workspace| workspace["workspace_id"].as_str() == Some(workspace_id))?;
    let digits = workspace["tokens"][crate::pr::herdr::TOKEN]
        .as_str()?
        .strip_prefix('#')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number: u64 = digits.parse().ok().filter(|number| *number > 0)?;
    let snapshot: Snapshot = serde_json::from_value(snapshot.clone()).ok()?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == workspace_id)?;
    let directory = snapshot.directory(workspace)?.to_path_buf();
    if !directory.is_absolute() {
        return None;
    }
    Some(Item {
        id: "herdr:view_pull_request".into(),
        title: "View Pull Request".into(),
        source: "Herdr".into(),
        description: String::new(),
        command: Command::ViewPullRequest { number, directory },
    })
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
