//! Declarative palette definitions; all domain bindings live at this boundary.
use super::{
    app::Selection,
    catalog::{Command, Context, Direction, Item, Scope},
    history::UsageKey,
    naming::Naming,
};
use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProfile {
    #[default]
    All,
    Title,
    SourceTitle,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    #[default]
    Source,
    Id,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum History {
    #[default]
    Command,
    Task,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Initial {
    #[default]
    Empty,
    CurrentLabel,
    SuggestedName,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Validator {
    #[default]
    VisibleName,
    AutomaticName,
    NewBranch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Commands,
    Tasks,
    Branches,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Text {
        key: String,
        #[serde(default)]
        initial: Initial,
        #[serde(default)]
        validator: Validator,
        #[serde(default)]
        select_all: bool,
        #[serde(default = "title_message")]
        message: String,
        #[serde(default = "save_help")]
        help: String,
        #[serde(default = "name_error")]
        error: String,
    },
    Confirm {
        message: String,
        #[serde(default = "confirm_help")]
        help: String,
    },
    Choice {
        provider: Provider,
        screen: String,
    },
}
fn title_message() -> String {
    "{title}".into()
}
fn save_help() -> String {
    "Enter Save  Esc Back".into()
}
fn name_error() -> String {
    "Enter a nonblank name".into()
}
fn confirm_help() -> String {
    "Enter confirms closing; Esc goes back".into()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Behavior {
    #[serde(default)]
    pub target: Option<Scope>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub search: SearchProfile,
    #[serde(default)]
    pub detail: Detail,
    #[serde(default)]
    pub history: History,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    pub title: String,
    pub search_label: String,
    pub empty: String,
    pub loading: String,
    pub help: String,
    pub entrypoint: String,
    pub provider: Provider,
    #[serde(default)]
    pub diagnostics: bool,
    #[serde(default = "yes")]
    pub history: bool,
    #[serde(default)]
    pub abort_flow: bool,
}
fn yes() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    Workspace,
    Tab,
    Pane,
    Repository,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Handler {
    SearchWorkspaces,
    Create { scope: Scope },
    Rename { scope: Scope },
    Close { scope: Scope },
    Split { direction: Direction },
    Focus { direction: Direction },
    Zoom,
    OpenRepo,
    ViewPullRequest,
    ChooseWorktree { new: bool },
    WorktreeBranch { new: bool },
    Task,
    Plugin,
}

impl Handler {
    fn subject(&self) -> Option<Scope> {
        match *self {
            Self::Create { scope } => Some(scope),
            _ => self.target(),
        }
    }
    fn target(&self) -> Option<Scope> {
        match *self {
            Self::Create { .. }
            | Self::ChooseWorktree { .. }
            | Self::WorktreeBranch { .. }
            | Self::Task
            | Self::ViewPullRequest => Some(Scope::Workspace),
            Self::Rename { scope } | Self::Close { scope } => Some(scope),
            Self::Split { .. } | Self::Focus { .. } | Self::Zoom => Some(Scope::Pane),
            Self::SearchWorkspaces | Self::OpenRepo | Self::Plugin => None,
        }
    }
    fn command(&self) -> Option<Command> {
        Some(match *self {
            Self::SearchWorkspaces => Command::SearchWorkspaces,
            Self::Create { scope } => Command::Create(scope),
            Self::Rename { scope } => Command::Rename(scope),
            Self::Close { scope } => Command::Close(scope),
            Self::Split { direction } => Command::Split(direction),
            Self::Focus { direction } => Command::Focus(direction),
            Self::Zoom => Command::Zoom,
            _ => return None,
        })
    }
    fn accepts(&self, command: &Command) -> bool {
        match (self, command) {
            (Self::ChooseWorktree { new: expected }, Command::ChooseWorktree { new, .. })
            | (Self::WorktreeBranch { new: expected }, Command::WorktreeBranch { new, .. }) => {
                expected == new
            }
            (Self::OpenRepo, Command::OpenRepo { .. })
            | (Self::ViewPullRequest, Command::ViewPullRequest { .. })
            | (Self::Task, Command::Task { .. })
            | (Self::Plugin, Command::Plugin { .. }) => true,
            _ => self.command().as_ref() == Some(command),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub template: bool,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    pub handler: Handler,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub search: SearchProfile,
    #[serde(default)]
    pub detail: Detail,
    #[serde(default)]
    pub history: History,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    #[serde(default)]
    pub commands: Vec<Definition>,
    #[serde(default)]
    pub screens: HashMap<String, Screen>,
}

static BUNDLED: LazyLock<std::result::Result<Registry, String>> = LazyLock::new(|| {
    Registry::parse(include_str!("commands.toml")).map_err(|error| format!("{error:#}"))
});

impl Registry {
    pub fn bundled() -> Result<&'static Self> {
        BUNDLED
            .as_ref()
            .map_err(|error| anyhow::anyhow!("invalid bundled command registry: {error}"))
    }
    pub fn parse(text: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Document {
            #[serde(default)]
            commands: Vec<toml::Value>,
            #[serde(default)]
            screens: HashMap<String, Screen>,
        }
        let document: Document = toml::from_str(text).context("invalid command registry")?;
        let commands = document
            .commands
            .into_iter()
            .map(|value| {
                let id = value
                    .get("id")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("missing ID")
                    .to_owned();
                value.try_into().with_context(|| format!("command {id}"))
            })
            .collect::<Result<Vec<Definition>>>()?;
        let registry = Self {
            commands,
            screens: document.screens,
        };
        let mut ids = HashSet::new();
        for definition in &registry.commands {
            ensure!(
                !definition.id.trim().is_empty() && ids.insert(&definition.id),
                "duplicate or blank command ID: {}",
                definition.id
            );
            definition
                .validate(&registry)
                .with_context(|| format!("command {}", definition.id))?;
        }
        for (index, definition) in registry
            .commands
            .iter()
            .enumerate()
            .filter(|(_, definition)| definition.template)
        {
            ensure!(
                !registry.commands[..index]
                    .iter()
                    .any(|other| other.template && other.handler == definition.handler),
                "ambiguous templates for command {}",
                definition.id
            );
        }
        Ok(registry)
    }
    pub fn screen(&self, key: &str) -> Result<Screen> {
        self.screens
            .get(key)
            .cloned()
            .with_context(|| format!("unknown palette screen: {key}"))
    }
    pub fn builtins(&self, context: &Context) -> Result<Vec<Item>> {
        self.builtins_in(context, false)
    }

    pub fn builtins_in(&self, context: &Context, repository: bool) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        for definition in &self.commands {
            if !definition.template
                && definition.available(context, repository)
                && let Some(command) = definition.handler.command()
            {
                items.push(definition.bind(
                    definition.id.clone(),
                    String::new(),
                    "Herdr".into(),
                    String::new(),
                    command,
                )?);
            }
        }
        super::catalog::sort(&mut items);
        Ok(items)
    }
    pub fn bind(
        &self,
        id: String,
        title: String,
        source: String,
        description: String,
        command: Command,
    ) -> Result<Item> {
        let definition = self
            .commands
            .iter()
            .find(|definition| definition.id == id)
            .or_else(|| {
                self.commands
                    .iter()
                    .find(|definition| definition.template && definition.handler.accepts(&command))
            })
            .context("command has no registered definition")?;
        definition.bind(id, title, source, description, command)
    }

    /// Expand operation data into configured declarations without UI special cases.
    pub fn expand(
        &self,
        items: Vec<Item>,
        context: &Context,
        repository: bool,
    ) -> Result<Vec<Item>> {
        let mut expanded = Vec::new();
        let mut ids = HashSet::new();
        for item in items {
            let definitions: Vec<_> = self
                .commands
                .iter()
                .filter(|definition| {
                    !definition.template && definition.handler.accepts(&item.command)
                })
                .collect();
            if definitions.is_empty() {
                if ids.insert(item.id.clone()) {
                    expanded.push(item);
                }
            } else {
                for definition in definitions {
                    if definition.available(context, repository)
                        && ids.insert(definition.id.clone())
                    {
                        expanded.push(definition.bind(
                            definition.id.clone(),
                            item.title.clone(),
                            item.source.clone(),
                            item.description.clone(),
                            item.command.clone(),
                        )?);
                    }
                }
            }
        }
        Ok(expanded)
    }

    pub fn worktree_actions(
        &self,
        root: &std::path::Path,
        invocation_branch: &str,
        context: &Context,
    ) -> Result<Vec<Item>> {
        self.commands
            .iter()
            .filter_map(|definition| match definition.handler {
                Handler::ChooseWorktree { new }
                    if !definition.template && definition.available(context, true) =>
                {
                    Some(definition.bind(
                        definition.id.clone(),
                        String::new(),
                        "Herdr".into(),
                        String::new(),
                        Command::ChooseWorktree {
                            root: root.into(),
                            new,
                            invocation_branch: invocation_branch.into(),
                        },
                    ))
                }
                _ => None,
            })
            .collect()
    }
}

impl Definition {
    fn available(&self, context: &Context, repository: bool) -> bool {
        self.handler
            .target()
            .is_none_or(|scope| scope.id(context).is_some())
            && self.requires.iter().all(|requirement| match requirement {
                Requirement::Workspace => context.workspace_id.is_some(),
                Requirement::Tab => context.tab_id.is_some(),
                Requirement::Pane => context.focused_pane_id.is_some(),
                Requirement::Repository => repository,
            })
    }
    fn validate(&self, registry: &Registry) -> Result<()> {
        ensure!(
            self.template
                || self
                    .title
                    .as_ref()
                    .is_some_and(|title| !title.trim().is_empty()),
            "title is required"
        );
        let texts: Vec<_> = self
            .steps
            .iter()
            .filter(|step| matches!(step, Step::Text { .. }))
            .collect();
        ensure!(
            texts.len() <= 1,
            "handler bindings accept one named text input"
        );
        ensure!(
            self.steps
                .iter()
                .filter(|step| matches!(step, Step::Confirm { .. }))
                .count()
                <= 1,
            "duplicate confirmation step"
        );
        for step in &self.steps {
            match step {
                Step::Text {
                    key,
                    initial,
                    validator,
                    message,
                    help,
                    error,
                    ..
                } => {
                    ensure!(key == "name", "unsupported input key: {key}");
                    ensure!(
                        matches!(
                            (initial, &self.handler),
                            (Initial::Empty, _)
                                | (Initial::CurrentLabel, Handler::Rename { .. })
                                | (Initial::SuggestedName, Handler::Create { .. })
                        ),
                        "initial value requires a scoped name handler"
                    );
                    ensure!(
                        !matches!(validator, Validator::NewBranch)
                            || matches!(self.handler, Handler::WorktreeBranch { new: true }),
                        "new_branch validator requires a new worktree branch binding"
                    );
                    validate_text(message)?;
                    validate_text(help)?;
                    validate_text(error)?;
                }
                Step::Confirm { message, help } => {
                    validate_text(message)?;
                    validate_text(help)?;
                }
                Step::Choice { provider, screen } => {
                    ensure!(
                        self.steps.len() == 1,
                        "catalog choices continue through registered row templates"
                    );
                    ensure!(
                        registry.screen(screen)?.provider == *provider,
                        "choice provider does not match screen"
                    );
                    ensure!(
                        matches!(
                            (&self.handler, provider),
                            (Handler::ChooseWorktree { .. }, Provider::Branches)
                        ),
                        "incompatible choice provider and handler"
                    );
                }
            }
        }
        match self.handler {
            Handler::Create { scope } => {
                ensure!(scope != Scope::Pane, "pane creation requires split");
                ensure!(
                    matches!(
                        texts.first(),
                        Some(Step::Text {
                            validator: Validator::AutomaticName,
                            ..
                        })
                    ),
                    "creation requires an automatic_name text prompt"
                );
            }
            Handler::Rename { .. } => ensure!(
                matches!(
                    texts.first(),
                    Some(Step::Text {
                        validator: Validator::VisibleName,
                        ..
                    })
                ),
                "rename requires a visible_name text prompt"
            ),
            Handler::Close { .. } => ensure!(
                self.steps
                    .iter()
                    .any(|step| matches!(step, Step::Confirm { .. })),
                "closing requires confirmation"
            ),
            Handler::WorktreeBranch { new: true } => ensure!(
                matches!(
                    texts.first(),
                    Some(Step::Text {
                        validator: Validator::NewBranch,
                        ..
                    })
                ),
                "new worktree requires a new_branch text prompt"
            ),
            Handler::ChooseWorktree { .. } => ensure!(
                matches!(self.steps.first(), Some(Step::Choice { .. })),
                "worktree selection requires a choice step"
            ),
            Handler::Split { direction } => ensure!(
                matches!(direction, Direction::Right | Direction::Down),
                "unsupported split direction"
            ),
            _ => ensure!(texts.is_empty(), "handler does not accept text input"),
        }
        ensure!(
            (self.history == History::Task) == matches!(self.handler, Handler::Task),
            "task history requires a task handler"
        );
        Ok(())
    }
    fn bind(
        &self,
        id: String,
        title: String,
        source: String,
        description: String,
        command: Command,
    ) -> Result<Item> {
        ensure!(
            self.handler.accepts(&command),
            "command {id}: handler binding mismatch"
        );
        Ok(Item {
            id,
            title: self.title.clone().unwrap_or(title),
            source: self.source.clone().unwrap_or(source),
            description: self.description.clone().unwrap_or(description),
            command,
            behavior: Behavior {
                target: self.handler.subject(),
                steps: self.steps.clone(),
                search: self.search,
                detail: self.detail,
                history: self.history,
            },
        })
    }
}

fn validate_text(text: &str) -> Result<()> {
    let remainder = text.replace("{title}", "").replace("{target}", "");
    ensure!(
        !remainder.contains(['{', '}']),
        "unknown text placeholder in {text:?}"
    );
    Ok(())
}

pub fn format_text(text: &str, item: &Item, context: &Context) -> String {
    let target = item
        .behavior
        .target
        .map(|scope| scope.label(context))
        .unwrap_or("");
    let mut formatted = String::new();
    for part in text.split_inclusive('}') {
        if let Some(prefix) = part.strip_suffix("{title}") {
            formatted.push_str(prefix);
            formatted.push_str(&item.title);
        } else if let Some(prefix) = part.strip_suffix("{target}") {
            formatted.push_str(prefix);
            formatted.push_str(target);
        } else {
            formatted.push_str(part);
        }
    }
    formatted
}

pub fn initial_value(initial: Initial, target: Option<Scope>, naming: &Naming) -> String {
    match (initial, target) {
        (Initial::CurrentLabel, Some(scope)) => naming.rename_name(scope).into(),
        (Initial::SuggestedName, Some(scope)) => naming.suggestion(scope).into(),
        _ => String::new(),
    }
}

pub fn validate_input(
    validator: Validator,
    command: &Command,
    input: &str,
    error: &str,
) -> Result<()> {
    match validator {
        Validator::AutomaticName => {
            super::naming::validate_name(input, super::naming::NamePolicy::Automatic, error)
        }
        Validator::VisibleName => {
            super::naming::validate_name(input, super::naming::NamePolicy::Visible, error)
        }
        Validator::NewBranch => match command {
            Command::WorktreeBranch {
                root, new: true, ..
            } => super::worktree::validate_name(root, input),
            _ => bail!("invalid new branch validator binding"),
        },
    }
}

pub fn validate_submission(item: &Item, selection: &Selection) -> Result<()> {
    let text = item.behavior.steps.iter().find_map(|step| match step {
        Step::Text {
            validator, error, ..
        } => Some((*validator, error)),
        _ => None,
    });
    match (text, selection.input.as_deref()) {
        (Some((validator, error)), Some(input)) => {
            validate_input(validator, &item.command, input, error)?
        }
        (None, None) => {}
        _ => bail!("unexpected palette command input"),
    }
    ensure!(
        selection.confirmed
            == item
                .behavior
                .steps
                .iter()
                .any(|step| matches!(step, Step::Confirm { .. })),
        "confirmation does not match command requirements"
    );
    Ok(())
}

pub fn usage_key(item: &Item) -> UsageKey {
    match (&item.behavior.history, &item.command) {
        (History::Task, Command::Task { root, task }) => UsageKey::Task {
            root: root.clone(),
            label: task.label.clone(),
        },
        _ => UsageKey::Command(item.id.clone()),
    }
}
