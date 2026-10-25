use crate::search::matcher::{FieldMatch, FieldMatcher, Query};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Context {
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub workspace_label: Option<String>,
    pub tab_label: Option<String>,
    pub focused_pane_cwd: Option<String>,
    pub selected_text: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Scope {
    #[serde(alias = "workspace")]
    Workspace,
    #[serde(alias = "tab")]
    Tab,
    #[serde(alias = "pane")]
    Pane,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Tab => "Tab",
            Self::Pane => "Pane",
        }
    }
    pub fn id(self, context: &Context) -> Option<&str> {
        match self {
            Self::Workspace => context.workspace_id.as_deref(),
            Self::Tab => context.tab_id.as_deref(),
            Self::Pane => context.focused_pane_id.as_deref(),
        }
    }
    pub fn label(self, context: &Context) -> &str {
        match self {
            Self::Workspace => context.workspace_label.as_deref(),
            Self::Tab => context.tab_label.as_deref(),
            Self::Pane => context.focused_pane_id.as_deref(),
        }
        .or_else(|| self.id(context))
        .unwrap_or("unavailable")
    }
    pub fn cli(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Tab => "tab",
            Self::Pane => "pane",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Direction {
    #[serde(alias = "left")]
    Left,
    #[serde(alias = "right")]
    Right,
    #[serde(alias = "up")]
    Up,
    #[serde(alias = "down")]
    Down,
}
impl Direction {
    pub fn cli(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Command {
    ChooseWorktree {
        root: std::path::PathBuf,
        new: bool,
        invocation_branch: String,
    },
    WorktreeBranch {
        root: std::path::PathBuf,
        branch: super::worktree::Branch,
        new: bool,
    },
    Task {
        task: crate::tasks::config::Task,
        root: std::path::PathBuf,
    },
    SearchWorkspaces,
    OpenRepo {
        url: String,
    },
    ViewPullRequest {
        number: u64,
        directory: std::path::PathBuf,
    },
    Create(Scope),
    Rename(Scope),
    Close(Scope),
    Split(Direction),
    Focus(Direction),
    Zoom,
    Plugin {
        plugin_id: String,
        action_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub source: String,
    pub description: String,
    pub command: Command,
    pub behavior: super::registry::Behavior,
}

impl Item {
    pub fn new(
        id: String,
        title: String,
        source: String,
        description: String,
        command: Command,
    ) -> Result<Self> {
        super::registry::Registry::bundled()?.bind(id, title, source, description, command)
    }
    pub fn fields(&self) -> [&str; 4] {
        match self.behavior.search {
            super::registry::SearchProfile::Title => ["", "", "", &self.title],
            super::registry::SearchProfile::SourceTitle => [&self.source, "", "", &self.title],
            super::registry::SearchProfile::All => {
                [&self.source, &self.id, &self.description, &self.title]
            }
        }
    }
}

pub struct Hit {
    pub item: usize,
    pub matched: FieldMatch,
}

pub fn search(items: &[Item], text: &str, matcher: &mut FieldMatcher) -> Vec<Hit> {
    search_with_recent(items, text, matcher, &[])
}

pub fn search_with_recent(
    items: &[Item],
    text: &str,
    matcher: &mut FieldMatcher,
    recent_ids: &[String],
) -> Vec<Hit> {
    let query = Query::new(text);
    let mut hits: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(item, value)| {
            matcher
                .matches(&value.fields(), &query)
                .map(|matched| Hit { item, matched })
        })
        .collect();
    if query.is_empty() {
        let ranks: std::collections::HashMap<_, _> = recent_ids
            .iter()
            .enumerate()
            .rev()
            .map(|(rank, id)| (id.as_str(), rank))
            .collect();
        hits.sort_by_key(|hit| {
            ranks
                .get(items[hit.item].id.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
    } else {
        hits.sort_by_key(|hit| std::cmp::Reverse(hit.matched.score));
    }
    hits
}

pub fn builtins(context: &Context) -> Result<Vec<Item>> {
    super::registry::Registry::bundled()?.builtins(context)
}

#[derive(Deserialize)]
struct Plugin {
    plugin_id: String,
    name: String,
    enabled: bool,
}
#[derive(Deserialize)]
struct PluginAction {
    plugin_id: String,
    action_id: String,
    title: String,
    description: Option<String>,
    #[serde(default)]
    contexts: Vec<String>,
    platforms: Option<Vec<String>>,
}

pub fn plugin_items(
    plugins: Value,
    actions: Value,
    context: &Context,
    platform: &str,
) -> Result<Vec<Item>> {
    let plugins: Vec<Plugin> = serde_json::from_value(
        plugins
            .get("plugins")
            .cloned()
            .context("missing plugin list")?,
    )?;
    let actions: Vec<PluginAction> = serde_json::from_value(
        actions
            .get("actions")
            .cloned()
            .context("missing action list")?,
    )?;
    let mut items = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for action in actions {
        let Some(plugin) = plugins
            .iter()
            .find(|plugin| plugin.plugin_id == action.plugin_id && plugin.enabled)
        else {
            continue;
        };
        if action
            .platforms
            .as_ref()
            .is_some_and(|platforms| !platforms.iter().any(|value| value == platform))
        {
            continue;
        }
        if !action.contexts.is_empty()
            && !action.contexts.iter().any(|kind| match kind.as_str() {
                "global" => true,
                "workspace" => context.workspace_id.is_some(),
                "tab" => context.tab_id.is_some(),
                "pane" => context.focused_pane_id.is_some(),
                "selection" => context
                    .selected_text
                    .as_ref()
                    .is_some_and(|text| !text.is_empty()),
                _ => false,
            })
        {
            continue;
        }
        ensure!(
            !action.plugin_id.trim().is_empty()
                && !action.action_id.trim().is_empty()
                && !action.title.trim().is_empty(),
            "invalid plugin action identity"
        );
        let id = format!("{}.{}", action.plugin_id, action.action_id);
        ensure!(ids.insert(id.clone()), "duplicate plugin action ID");
        items.push(Item::new(
            id,
            action.title,
            plugin.name.clone(),
            action.description.unwrap_or_default(),
            Command::Plugin {
                plugin_id: action.plugin_id,
                action_id: action.action_id,
            },
        )?);
    }
    sort(&mut items);
    Ok(items)
}

pub fn sort(items: &mut [Item]) {
    items.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
}
