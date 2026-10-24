//! Branch discovery and native Herdr worktree requests share one captured source.
use super::{
    api,
    catalog::{Command, Context, Item},
};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub reference: String,
    pub name: String,
    pub remote: Option<String>,
}

impl Branch {
    pub fn label(&self) -> String {
        self.remote.as_ref().map_or_else(
            || self.name.clone(),
            |remote| format!("{remote}/{}", self.name),
        )
    }
}

fn git(root: &Path, args: &[&str], timeout: Duration, cancelled: &AtomicBool) -> Result<String> {
    ensure!(
        !cancelled.load(Ordering::Relaxed),
        "branch discovery cancelled"
    );
    let output = crate::command::run_cancellable(
        process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes"),
        timeout,
        4 * 1024 * 1024,
        cancelled,
        "Git worktree discovery",
    )?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)
        .context("Git output is not UTF-8")?
        .trim()
        .into())
}

fn inspect(root: &Path, args: &[&str], cancelled: &AtomicBool) -> Result<String> {
    git(root, args, Duration::from_secs(5), cancelled)
}

pub fn root(directory: &Path, cancelled: &AtomicBool) -> Result<PathBuf> {
    let root = PathBuf::from(inspect(
        directory,
        &["rev-parse", "--show-toplevel"],
        cancelled,
    )?);
    std::fs::canonicalize(root).context("resolve worktree repository")
}

pub fn actions(root: &Path) -> Vec<Item> {
    let invocation_branch =
        inspect(root, &["branch", "--show-current"], &AtomicBool::new(false)).unwrap_or_default();
    [
        (true, "herdr:worktree_new", "New Worktree"),
        (
            false,
            "herdr:worktree_existing",
            "Add Existing Branch as Worktree",
        ),
    ]
    .into_iter()
    .map(|(new, id, title)| Item {
        id: id.into(),
        title: title.into(),
        source: "Herdr".into(),
        description: "Create or open a worktree in this workspace’s repository group".into(),
        command: Command::ChooseWorktree {
            root: root.into(),
            new,
            invocation_branch: invocation_branch.clone(),
        },
    })
    .collect()
}

pub fn fetch(root: &Path, cancelled: &AtomicBool) -> Result<String> {
    let names = inspect(root, &["remote"], cancelled)?;
    let mut warnings = vec![];
    for name in names.lines() {
        if let Err(error) = git(
            root,
            &["fetch", "--no-tags", "--", name],
            Duration::from_secs(60),
            cancelled,
        ) {
            ensure!(
                !cancelled.load(Ordering::Relaxed),
                "branch discovery cancelled"
            );
            warnings.push(format!("Fetch {name} failed: {error:#}"));
        }
    }
    Ok(warnings.join("; "))
}

pub fn branches(root: &Path, cancelled: &AtomicBool) -> Result<Vec<Branch>> {
    inspect(root, &["rev-parse", "--verify", "HEAD^{commit}"], cancelled)
        .context("Repository has no commits; create a commit before adding a worktree")?;
    let refs = inspect(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)%09%(symref)",
            "refs/heads",
            "refs/remotes",
        ],
        cancelled,
    )?;
    let mut branches = vec![];
    for line in refs.lines() {
        let (reference, symbolic) = line.split_once('\t').unwrap_or((line, ""));
        if !symbolic.is_empty() {
            continue;
        }
        let (name, remote) = if let Some(name) = reference.strip_prefix("refs/heads/") {
            (name, None)
        } else if let Some((remote, name)) = reference
            .strip_prefix("refs/remotes/")
            .and_then(|value| value.split_once('/'))
        {
            (name, Some(remote.to_owned()))
        } else {
            continue;
        };
        if name != crate::repository::GITBUTLER_WORKSPACE {
            branches.push(Branch {
                reference: reference.into(),
                name: name.into(),
                remote,
            });
        }
    }
    let locals: HashSet<_> = branches
        .iter()
        .filter(|branch| branch.remote.is_none())
        .map(|branch| branch.name.clone())
        .collect();
    branches.retain(|branch| branch.remote.is_none() || !locals.contains(&branch.name));
    branches.sort_by_key(Branch::label);
    Ok(branches)
}

pub fn branch_items(
    root: &Path,
    branches: &[Branch],
    new: bool,
    preferred: Option<&str>,
) -> Vec<Item> {
    let preferred = preferred.and_then(|name| preferred_branch(branches, name));
    let mut items: Vec<_> = branches
        .iter()
        .map(|branch| Item {
            id: format!("worktree:{}", branch.reference),
            title: branch.label(),
            source: if branch.remote.is_some() {
                "Remote"
            } else {
                "Local"
            }
            .into(),
            description: branch.reference.clone(),
            command: Command::WorktreeBranch {
                root: root.into(),
                branch: branch.clone(),
                new,
            },
        })
        .collect();
    if let Some(preferred) = preferred {
        items.sort_by_key(|item| item.id != format!("worktree:{}", preferred.reference));
    }
    items
}

fn preferred_branch<'a>(branches: &'a [Branch], name: &str) -> Option<&'a Branch> {
    if let Some(local) = branches
        .iter()
        .find(|branch| branch.name == name && branch.remote.is_none())
    {
        return Some(local);
    }
    let mut candidates = branches.iter().filter(|branch| branch.name == name);
    let candidate = candidates.next()?;
    candidates.next().is_none().then_some(candidate)
}

pub fn load(
    root: &Path,
    new: bool,
    invocation_branch: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<(Vec<Item>, Option<String>)> {
    let mut warning = fetch(root, cancelled)?;
    let current = invocation_branch.unwrap_or_default().to_owned();
    let preferred =
        if new && (current.is_empty() || current == crate::repository::GITBUTLER_WORKSPACE) {
            match crate::pr::github::default_branch(root, cancelled) {
                Ok(default) => Some(if current.is_empty() {
                    default
                } else {
                    crate::repository::effective_branch(&current, &default).to_owned()
                }),
                Err(error) => {
                    warning.push_str(&format!(
                        " Default base unavailable; choose a branch: {error:#}"
                    ));
                    None
                }
            }
        } else {
            new.then_some(current)
        };
    let branches = branches(root, cancelled)?;
    if let Some(preferred) = &preferred
        && preferred_branch(&branches, preferred).is_none()
    {
        warning.push_str(" Preferred base is unavailable or ambiguous; choose a branch.");
    }
    let items = branch_items(root, &branches, new, preferred.as_deref());
    Ok((items, (!warning.is_empty()).then_some(warning)))
}

pub fn validate_name(root: &Path, name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name == name.trim()
            && !name.chars().any(char::is_control)
            && !name.contains("@{")
            && name != crate::repository::GITBUTLER_WORKSPACE,
        "Enter a valid new branch name"
    );
    let cancelled = AtomicBool::new(false);
    inspect(root, &["check-ref-format", "--branch", name], &cancelled)
        .context("Enter a valid new branch name")?;
    ensure!(
        !local_branch_exists(root, name, &cancelled)?,
        "Branch already exists; use Add Existing Branch as Worktree"
    );
    Ok(())
}

fn local_branch_exists(root: &Path, name: &str, cancelled: &AtomicBool) -> Result<bool> {
    let reference = format!("refs/heads/{name}");
    Ok(inspect(
        root,
        &["for-each-ref", "--format=%(refname)", &reference],
        cancelled,
    )?
    .lines()
    .any(|candidate| candidate == reference))
}

pub fn request_for(
    root: &Path,
    branch: &Branch,
    new_name: Option<&str>,
    context: &Context,
    checkouts: &Value,
) -> Result<(String, Value)> {
    let cancelled = AtomicBool::new(false);
    inspect(
        root,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", branch.reference),
        ],
        &cancelled,
    )
    .context("Selected branch is no longer available")?;
    let workspace_id = context
        .workspace_id
        .as_deref()
        .context("command target is unavailable")?;
    if let Some(name) = new_name {
        validate_name(root, name)?;
        return Ok((
            "worktree.create".into(),
            json!({"workspace_id":workspace_id,"branch":name,"base":branch.reference,"focus":true}),
        ));
    }
    let name = &branch.name;
    let checkouts = checkouts["worktrees"]
        .as_array()
        .context("missing Herdr worktree list")?;
    if checkouts.iter().any(|checkout| {
        checkout["branch"]
            .as_str()
            .is_some_and(|value| value == name || value == format!("refs/heads/{name}"))
    }) {
        return Ok((
            "worktree.open".into(),
            json!({"workspace_id":workspace_id,"branch":name,"focus":true}),
        ));
    }
    let base = (!local_branch_exists(root, name, &cancelled)?).then_some(&branch.reference);
    Ok((
        "worktree.create".into(),
        json!({"workspace_id":workspace_id,"branch":name,"base":base,"focus":true}),
    ))
}

pub fn execute(
    socket: &Path,
    root: &Path,
    branch: &Branch,
    new: bool,
    context: &Context,
    input: Option<&str>,
    snapshot: &Value,
) -> Result<()> {
    let directory = super::repository::directory_from_snapshot(snapshot, context)
        .context("original repository is no longer available")?;
    ensure!(
        self::root(&directory, &AtomicBool::new(false))? == root,
        "original workspace repository changed"
    );
    let mut source = context.clone();
    let workspace = snapshot["workspaces"]
        .as_array()
        .context("missing workspaces")?
        .iter()
        .find(|workspace| workspace["workspace_id"].as_str() == context.workspace_id.as_deref())
        .context("original workspace is no longer available")?;
    if workspace["worktree"]["is_linked_worktree"].as_bool() == Some(true) {
        let snapshot: crate::picker::model::Snapshot = serde_json::from_value(snapshot.clone())?;
        source.workspace_id = Some(
            snapshot
                .worktree_source(
                    context
                        .workspace_id
                        .as_deref()
                        .context("missing source workspace")?,
                )
                .context("repository parent workspace is unavailable or ambiguous")?
                .into(),
        );
    }
    let checkouts = api::request(
        socket,
        "worktree.list",
        json!({"workspace_id":source.workspace_id}),
    )?;
    let name = if new {
        Some(input.context("new worktree requires a branch name")?)
    } else {
        None
    };
    let (method, params) = request_for(root, branch, name, &source, &checkouts)?;
    api::request(socket, &method, params)?;
    Ok(())
}
