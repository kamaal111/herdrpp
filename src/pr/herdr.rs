use crate::command::run;
use crate::pr::github::{Lifecycle, PrSummary};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const SOURCE: &str = crate::PLUGIN_ID;
pub const TOKEN: &str = "pr_number";

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    pub active_tab_id: String,
    pub worktree: Option<Worktree>,
}

#[derive(Debug, Deserialize)]
pub struct Worktree {
    pub checkout_path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub cwd: Option<PathBuf>,
    pub foreground_cwd: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct Layout {
    pub workspace_id: String,
    pub tab_id: String,
    pub focused_pane_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub panes: Vec<Pane>,
    pub layouts: Vec<Layout>,
}

impl Snapshot {
    pub fn directory<'a>(&'a self, workspace: &'a Workspace) -> Option<&'a Path> {
        if let Some(worktree) = &workspace.worktree {
            return Some(worktree.checkout_path.as_path());
        }
        let layout = self.layouts.iter().find(|layout| {
            layout.workspace_id == workspace.workspace_id
                && layout.tab_id == workspace.active_tab_id
        })?;
        let pane = self.panes.iter().find(|pane| {
            pane.pane_id == layout.focused_pane_id
                && pane.workspace_id == workspace.workspace_id
                && pane.tab_id == workspace.active_tab_id
        })?;
        pane.foreground_cwd.as_deref().or(pane.cwd.as_deref())
    }

    fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for workspace in &self.workspaces {
            ensure!(
                !workspace.workspace_id.trim().is_empty(),
                "empty workspace ID"
            );
            ensure!(
                ids.insert(&workspace.workspace_id),
                "duplicate workspace ID"
            );
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Envelope {
    result: SnapshotResult,
}
#[derive(Deserialize)]
struct SnapshotResult {
    #[serde(rename = "type")]
    kind: String,
    snapshot: Snapshot,
}

#[derive(Deserialize)]
struct PluginEnvelope {
    result: PluginResult,
}
#[derive(Deserialize)]
struct PluginResult {
    plugins: Vec<Plugin>,
}
#[derive(Deserialize)]
pub struct Plugin {
    plugin_id: String,
    pub enabled: bool,
    pub plugin_root: Option<PathBuf>,
}

pub fn cli(args: &[&str]) -> Result<Vec<u8>> {
    run(
        Command::new(std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()))
            .args(args),
        Duration::from_secs(5),
    )
}

pub fn snapshot() -> Result<Snapshot> {
    let envelope: Envelope =
        serde_json::from_slice(&cli(&["api", "snapshot"])?).context("parse Herdr snapshot")?;
    ensure!(
        envelope.result.kind == "session_snapshot",
        "unexpected Herdr snapshot response"
    );
    envelope.result.snapshot.validate()?;
    Ok(envelope.result.snapshot)
}

pub fn installation() -> Result<Option<Plugin>> {
    let envelope: PluginEnvelope = serde_json::from_slice(&cli(&["plugin", "list", "--json"])?)
        .context("parse Herdr plugin list")?;
    Ok(envelope
        .result
        .plugins
        .into_iter()
        .find(|plugin| plugin.plugin_id == SOURCE))
}

pub fn enabled() -> Result<bool> {
    Ok(installation()?.is_some_and(|plugin| plugin.enabled))
}

const LIFECYCLE_TOKENS: [(Lifecycle, &str); 4] = [
    (Lifecycle::Open, "pr_open"),
    (Lifecycle::Draft, "pr_draft"),
    (Lifecycle::Merged, "pr_merged"),
    (Lifecycle::Closed, "pr_closed"),
];

pub fn report(workspace_id: &str, pr: Option<PrSummary>) -> Result<()> {
    let mut args = vec![
        "workspace".to_owned(),
        "report-metadata".to_owned(),
        workspace_id.to_owned(),
        "--source".to_owned(),
        SOURCE.to_owned(),
    ];
    if let Some(pr) = pr {
        args.extend(["--token".into(), format!("{TOKEN}=#{}", pr.number)]);
    }
    for (lifecycle, token) in LIFECYCLE_TOKENS {
        if let Some(pr) = pr.filter(|pr| pr.lifecycle == lifecycle) {
            let suffix = pr
                .ci
                .filter(|_| lifecycle != Lifecycle::Merged)
                .map(|ci| format!(" {}", ci.icon()))
                .unwrap_or_default();
            args.extend(["--token".into(), format!("{token}=#{}{suffix}", pr.number)]);
        } else {
            args.extend(["--clear-token".into(), token.into()]);
        }
    }
    if pr.is_some() {
        args.extend(["--ttl-ms".into(), "180000".into()]);
    } else {
        args.extend(["--clear-token".into(), TOKEN.into()]);
    }
    cli(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    Ok(())
}
