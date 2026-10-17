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
    Workspace,
    Tab,
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
    Left,
    Right,
    Up,
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
    SearchWorkspaces,
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
}

impl Item {
    pub fn fields(&self) -> [&str; 4] {
        [&self.source, &self.id, &self.description, &self.title]
    }
}

pub struct Hit {
    pub item: usize,
    pub matched: FieldMatch,
}

pub fn search(items: &[Item], text: &str, matcher: &mut FieldMatcher) -> Vec<Hit> {
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
    if !query.is_empty() {
        hits.sort_by_key(|hit| std::cmp::Reverse(hit.matched.score));
    }
    hits
}

pub fn builtins(context: &Context) -> Vec<Item> {
    let mut items = Vec::new();
    let mut add = |id: String, title: String, command| {
        items.push(Item {
            id,
            title,
            source: "Herdr".into(),
            description: String::new(),
            command,
        })
    };
    add(
        "herdr:search_workspaces".into(),
        "Search Workspaces".into(),
        Command::SearchWorkspaces,
    );
    for scope in [Scope::Workspace, Scope::Tab, Scope::Pane] {
        if scope != Scope::Pane && context.workspace_id.is_some() {
            add(
                format!("herdr:{}_create", scope.cli()),
                format!("New {}", scope.name()),
                Command::Create(scope),
            );
        }
        if scope.id(context).is_some() {
            add(
                format!("herdr:{}_rename", scope.cli()),
                format!("Rename {}", scope.name()),
                Command::Rename(scope),
            );
            add(
                format!("herdr:{}_close", scope.cli()),
                format!("Close {}", scope.name()),
                Command::Close(scope),
            );
        }
    }
    if context.focused_pane_id.is_some() {
        for direction in [Direction::Right, Direction::Down] {
            add(
                format!("herdr:split_{}", direction.cli()),
                format!("Split Pane {}", title_case(direction.cli())),
                Command::Split(direction),
            );
        }
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            add(
                format!("herdr:focus_{}", direction.cli()),
                format!("Focus Pane {}", title_case(direction.cli())),
                Command::Focus(direction),
            );
        }
        add(
            "herdr:zoom".into(),
            "Toggle Pane Zoom".into(),
            Command::Zoom,
        );
    }
    sort(&mut items);
    items
}

fn title_case(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|ch| ch.to_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
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
        items.push(Item {
            id,
            title: action.title,
            source: plugin.name.clone(),
            description: action.description.unwrap_or_default(),
            command: Command::Plugin {
                plugin_id: action.plugin_id,
                action_id: action.action_id,
            },
        });
    }
    sort(&mut items);
    Ok(items)
}

pub fn sort(items: &mut [Item]) {
    items.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
}
