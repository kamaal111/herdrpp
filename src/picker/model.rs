use std::collections::HashSet;

use anyhow::{Result, ensure};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    pub label: String,
    pub number: usize,
    pub active_tab_id: String,
    pub worktree: Option<WorktreeProvenance>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorktreeProvenance {
    pub repo_key: Option<String>,
    pub is_linked_worktree: Option<bool>,
}

impl Workspace {
    fn repository(&self) -> Option<(&str, bool)> {
        let provenance = self.worktree.as_ref()?;
        let key = provenance.repo_key.as_deref()?;
        (!key.trim().is_empty()).then_some((key, provenance.is_linked_worktree?))
    }
}

pub struct WorkspaceGroup<'a> {
    pub root: &'a Workspace,
    pub worktrees: Vec<&'a Workspace>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Working,
    Blocked,
    Done,
    Idle,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Tab {
    #[serde(default)]
    pub agent_status: AgentStatus,
    pub tab_id: String,
    pub workspace_id: String,
    pub label: String,
    pub number: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Agent {
    pub workspace_id: String,
    pub tab_id: String,
    #[serde(default)]
    pub agent_status: AgentStatus,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub agents: Vec<Agent>,
    pub focused_workspace_id: Option<String>,
    pub focused_tab_id: Option<String>,
}

impl Snapshot {
    pub fn workspace_groups(&self) -> Vec<WorkspaceGroup<'_>> {
        let mut ordered: Vec<_> = self.workspaces.iter().collect();
        ordered.sort_by_key(|workspace| workspace.number);
        let parents: Vec<_> = ordered
            .iter()
            .map(|workspace| {
                let (key, linked) = workspace.repository()?;
                if !linked {
                    return None;
                }
                let mut roots = ordered
                    .iter()
                    .copied()
                    .filter(|root| root.repository() == Some((key, false)));
                let root = roots.next()?;
                roots.next().is_none().then_some(root.workspace_id.as_str())
            })
            .collect();
        ordered
            .iter()
            .zip(&parents)
            .filter(|(_, parent)| parent.is_none())
            .map(|(root, _)| WorkspaceGroup {
                root,
                worktrees: ordered
                    .iter()
                    .zip(&parents)
                    .filter(|(_, parent)| **parent == Some(root.workspace_id.as_str()))
                    .map(|(workspace, _)| *workspace)
                    .collect(),
            })
            .collect()
    }

    pub fn validate(&mut self) -> Result<()> {
        let mut workspace_ids = HashSet::new();
        for workspace in &self.workspaces {
            ensure!(
                !workspace.workspace_id.trim().is_empty(),
                "missing workspace ID"
            );
            ensure!(
                workspace_ids.insert(&workspace.workspace_id),
                "duplicate workspace ID"
            );
        }
        let mut tab_ids = HashSet::new();
        for tab in &self.tabs {
            ensure!(!tab.tab_id.trim().is_empty(), "missing tab ID");
            ensure!(tab_ids.insert(&tab.tab_id), "duplicate tab ID");
            ensure!(
                workspace_ids.contains(&tab.workspace_id),
                "tab has unknown parent workspace"
            );
        }
        for workspace in &self.workspaces {
            ensure!(
                self.tabs
                    .iter()
                    .any(|tab| tab.tab_id == workspace.active_tab_id
                        && tab.workspace_id == workspace.workspace_id),
                "workspace has invalid active tab ID"
            );
        }
        ensure!(
            self.focused_workspace_id
                .as_ref()
                .is_none_or(|id| workspace_ids.contains(id)),
            "unknown focused workspace ID"
        );
        ensure!(
            self.focused_tab_id.as_ref().is_none_or(|id| self
                .tabs
                .iter()
                .any(|tab| &tab.tab_id == id
                    && Some(&tab.workspace_id) == self.focused_workspace_id.as_ref())),
            "invalid focused tab ID"
        );
        self.workspaces.sort_by_key(|w| w.number);
        self.tabs.sort_by_key(|t| t.number);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Workspace(String),
    Tab(String),
}
