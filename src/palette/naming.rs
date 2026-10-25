use super::catalog::{Context, Scope};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, process::Command, sync::atomic::AtomicBool, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NamePolicy {
    Visible,
    Automatic,
}

pub fn validate_name(input: &str, policy: NamePolicy, error: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !input.chars().any(char::is_control)
            && (policy == NamePolicy::Automatic || !input.trim().is_empty()),
        "{error}"
    );
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Naming {
    pub workspace_name: String,
    pub tab_name: String,
    pub pane_name: String,
    pub tab_auto_named: bool,
    pub workspace_cwd: Option<String>,
    pub workspace_suggestion: String,
    pub tab_suggestion: String,
}

impl Default for Naming {
    fn default() -> Self {
        Self {
            workspace_name: String::new(),
            tab_name: String::new(),
            pane_name: String::new(),
            tab_auto_named: false,
            workspace_cwd: None,
            workspace_suggestion: "workspace".into(),
            tab_suggestion: "1".into(),
        }
    }
}

impl Naming {
    pub fn from_snapshot(snapshot: &Value, context: &Context) -> Self {
        let find = |collection: &str, key: &str, id: Option<&str>| {
            snapshot[collection].as_array().and_then(|items| {
                items
                    .iter()
                    .find(|item| id.is_some() && item[key].as_str() == id)
            })
        };
        let workspace = find(
            "workspaces",
            "workspace_id",
            context.workspace_id.as_deref(),
        );
        let tab = find("tabs", "tab_id", context.tab_id.as_deref());
        let pane = find("panes", "pane_id", context.focused_pane_id.as_deref());
        let name = |item: Option<&Value>, fallback: Option<&str>| {
            item.and_then(|item| item["label"].as_str())
                .or(fallback)
                .unwrap_or_default()
                .to_owned()
        };
        let tab_count = snapshot["tabs"].as_array().map_or(0, |tabs| {
            tabs.iter()
                .filter(|tab| {
                    context.workspace_id.is_some()
                        && tab["workspace_id"].as_str() == context.workspace_id.as_deref()
                })
                .count()
        });
        Self {
            workspace_name: name(workspace, context.workspace_label.as_deref()),
            tab_name: name(tab, context.tab_label.as_deref()),
            pane_name: name(pane, None),
            tab_auto_named: tab.is_some_and(|tab| tab["custom_label"].as_bool() == Some(false)),
            workspace_cwd: workspace
                .and_then(|workspace| workspace["new_workspace_cwd"].as_str())
                .map(str::to_owned),
            tab_suggestion: (tab_count + 1).to_string(),
            ..Self::default()
        }
    }

    pub fn rename_name(&self, scope: Scope) -> &str {
        match scope {
            Scope::Workspace => &self.workspace_name,
            Scope::Tab => &self.tab_name,
            Scope::Pane => &self.pane_name,
        }
    }

    pub fn suggestion(&self, scope: Scope) -> &str {
        match scope {
            Scope::Workspace => &self.workspace_suggestion,
            Scope::Tab => &self.tab_suggestion,
            Scope::Pane => "",
        }
    }

    pub fn resolve_workspace_suggestion(&mut self, cancelled: &AtomicBool) {
        let Some(cwd) = self.workspace_cwd.as_deref().map(Path::new) else {
            return;
        };
        let output = crate::command::run_cancellable(
            Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(["rev-parse", "--show-toplevel"]),
            Duration::from_secs(2),
            64 * 1024,
            cancelled,
            "workspace name suggestion",
        );
        let repository_name = output
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|root| {
                Path::new(root.trim_end())
                    .file_name()?
                    .to_str()
                    .map(str::to_owned)
            });
        let home = std::env::var_os("HOME");
        self.workspace_suggestion = repository_name.unwrap_or_else(|| {
            if home.as_deref().map(Path::new) == Some(cwd) {
                "~".into()
            } else {
                cwd.file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| cwd.display().to_string())
            }
        });
    }
}
