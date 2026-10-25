//! Rust data providers and typed continuations for configured catalog steps.
use super::{
    catalog::{self, Command, Item},
    cli::Catalog,
    registry::{Provider, Registry, Step},
};
use crate::picker::api::{Herdr, decode};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum Payload {
    #[default]
    None,
    Branches {
        root: PathBuf,
        new: bool,
        invocation_branch: String,
    },
}

pub fn initial(
    snapshot: &serde_json::Value,
    context: &super::catalog::Context,
    screen: super::registry::Screen,
) -> Result<Catalog> {
    let mut items = vec![];
    if screen.provider == Provider::Commands
        && let Some(item) = super::pull_request::item_from_snapshot(snapshot, context)
    {
        items.push(item);
    }
    Ok(Catalog {
        screen,
        payload: Payload::None,
        recent_ids: vec![],
        context: context.clone(),
        naming: super::naming::Naming::from_snapshot(snapshot, context),
        items,
        warning: None,
    })
}

pub fn load(
    api: &Herdr,
    mut catalog: Catalog,
    cancelled: &AtomicBool,
    repo_directory: Option<&Path>,
) -> Result<Catalog> {
    match catalog.screen.provider {
        Provider::Tasks => return crate::tasks::load(api, &catalog.context, catalog.screen),
        Provider::Branches => {
            let Payload::Branches {
                root,
                new,
                invocation_branch,
            } = &catalog.payload
            else {
                bail!("branch provider requires repository data");
            };
            match super::worktree::load(root, *new, Some(invocation_branch), cancelled) {
                Ok((items, warning)) => {
                    catalog.items = items;
                    catalog.warning = warning;
                }
                Err(error) => {
                    catalog.items.clear();
                    catalog.warning = Some(format!("Cannot load branches: {error:#}"));
                }
            }
            return Ok(catalog);
        }
        Provider::Commands => {}
    }
    let mut repository = false;
    if let Some(directory) = repo_directory {
        if let Ok(Some(item)) = super::repository::item(directory, cancelled) {
            catalog.items.push(item);
        }
        if let Ok(root) = super::worktree::root(directory, cancelled) {
            repository = true;
            catalog.items.extend(super::worktree::actions(
                &root,
                &catalog.context,
                cancelled,
            )?);
        }
    }
    catalog
        .items
        .extend(Registry::bundled()?.builtins_in(&catalog.context, repository)?);
    catalog.naming.resolve_workspace_suggestion(cancelled);
    let plugins = (|| {
        let plugins = decode(&api.call_cancellable(&["plugin", "list", "--json"], cancelled)?)?;
        let actions = decode(&api.call_cancellable(&["plugin", "action", "list"], cancelled)?)?;
        catalog::plugin_items(plugins, actions, &catalog.context, std::env::consts::OS)
    })();
    catalog.warning = match plugins {
        Ok(actions) => {
            catalog.items.extend(actions);
            None
        }
        Err(error) => Some(format!("Plugin commands unavailable: {error:#}")),
    };
    catalog.items = Registry::bundled()?.expand(catalog.items, &catalog.context, repository)?;
    catalog::sort(&mut catalog.items);
    let mut ids = HashSet::new();
    catalog.items.retain(|item| ids.insert(item.id.clone()));
    Ok(catalog)
}

pub fn transition(item: &Item, catalog: &Catalog) -> Result<Option<Catalog>> {
    let Some(Step::Choice { provider, screen }) = item.behavior.steps.first() else {
        return Ok(None);
    };
    let screen = Registry::bundled()?.screen(screen)?;
    ensure!(*provider == screen.provider, "catalog provider mismatch");
    let payload = match (provider, &item.command) {
        (
            Provider::Branches,
            Command::ChooseWorktree {
                root,
                new,
                invocation_branch,
            },
        ) => Payload::Branches {
            root: root.clone(),
            new: *new,
            invocation_branch: invocation_branch.clone(),
        },
        _ => bail!("catalog provider has an incompatible command binding"),
    };
    Ok(Some(Catalog {
        screen,
        payload,
        recent_ids: vec![],
        context: catalog.context.clone(),
        naming: catalog.naming.clone(),
        items: vec![],
        warning: None,
    }))
}
