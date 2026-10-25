//! Task destinations live across short-lived palette action processes.
use super::config::{Task, shell_command};
use crate::{
    palette::{
        api::{request, validate_context},
        catalog::Context,
    },
    picker::api::Herdr,
    pr::watcher::Session,
};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Destination {
    workspace: String,
    root: PathBuf,
    label: String,
    tab: String,
    pane: String,
    terminal: String,
}

fn panes(snapshot: &Value) -> Result<&Vec<Value>> {
    snapshot["panes"].as_array().context("missing panes")
}

fn valid(snapshot: &Value, dest: &Destination) -> bool {
    snapshot["tabs"].as_array().is_some_and(|tabs| {
        tabs.iter().any(|t| {
            t["tab_id"].as_str() == Some(&dest.tab)
                && t["workspace_id"].as_str() == Some(&dest.workspace)
        })
    }) && snapshot["panes"].as_array().is_some_and(|panes| {
        panes.iter().any(|p| {
            p["pane_id"].as_str() == Some(&dest.pane)
                && p["tab_id"].as_str() == Some(&dest.tab)
                && p["workspace_id"].as_str() == Some(&dest.workspace)
                && p["terminal_id"].as_str() == Some(&dest.terminal)
        })
    })
}

/// Missing/partial process data must never authorize sending text to a busy pane.
pub fn idle(info: &Value) -> bool {
    let Some(shell) = info["shell_pid"].as_u64().filter(|pid| *pid > 0) else {
        return false;
    };
    let Some(processes) = info["foreground_processes"].as_array() else {
        return false;
    };
    processes.len() == 1
        && processes[0]["pid"].as_u64() == Some(shell)
        && processes[0]["name"].as_str().is_some_and(|name| {
            matches!(
                name.trim_start_matches('-'),
                "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "nu"
            )
        })
        && info["foreground_process_group_id"].as_u64() == Some(shell)
}

fn pane_idle(socket: &Path, pane: &str) -> Result<bool> {
    let result = request(socket, "pane.process_info", json!({"pane_id": pane}))?;
    ensure!(
        result["process_info"]["pane_id"].as_str() == Some(pane),
        "invalid pane process information"
    );
    Ok(idle(&result["process_info"]))
}

fn wait_idle(socket: &Path, pane: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if pane_idle(socket, pane)? {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Task pane {pane} did not become idle within 5 seconds. Stop the process in that pane, then run the task again. No replacement was started."
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn destination(pane: &Value, root: &Path, task: &Task) -> Result<Destination> {
    let field = |name: &str| -> Result<String> {
        Ok(pane[name]
            .as_str()
            .with_context(|| format!("missing pane {name}"))?
            .to_owned())
    };
    Ok(Destination {
        workspace: field("workspace_id")?,
        root: root.into(),
        label: task.label.clone(),
        tab: field("tab_id")?,
        pane: field("pane_id")?,
        terminal: field("terminal_id")?,
    })
}

fn allocate(
    api: &Herdr,
    socket: &Path,
    context: &Context,
    snapshot: &Value,
    root: &Path,
    task: &Task,
) -> Result<Destination> {
    let workspace = context
        .workspace_id
        .as_deref()
        .context("missing opening workspace")?;
    if task.new_tab {
        let result = request(
            socket,
            "tab.create",
            json!({"workspace_id":workspace, "cwd":root, "label":task.label, "focus":true}),
        )?;
        let tab = result["tab"]["tab_id"]
            .as_str()
            .context("tab creation did not return a tab ID")?;
        let fresh = api.raw_snapshot()?;
        let pane = panes(&fresh)?
            .iter()
            .find(|p| {
                p["workspace_id"].as_str() == Some(workspace) && p["tab_id"].as_str() == Some(tab)
            })
            .context("new task tab has no pane")?;
        return destination(pane, root, task);
    }
    let tab = context.tab_id.as_deref().context("missing opening tab")?;
    let opening = context
        .focused_pane_id
        .as_deref()
        .context("missing opening pane")?;
    let mut candidates: Vec<_> = panes(snapshot)?
        .iter()
        .filter(|p| {
            p["workspace_id"].as_str() == Some(workspace) && p["tab_id"].as_str() == Some(tab)
        })
        .collect();
    candidates.sort_by_key(|p| p["pane_id"].as_str() != Some(opening));
    for pane in candidates {
        let id = pane["pane_id"].as_str().context("missing pane ID")?;
        if pane_idle(socket, id).unwrap_or(false) {
            return destination(pane, root, task);
        }
    }
    let result = request(
        socket,
        "pane.split",
        json!({"target_pane_id":opening,"workspace_id":workspace,"direction":"right","cwd":root,"focus":true}),
    )?;
    destination(&result["pane"], root, task)
}

pub fn execute(
    api: &Herdr,
    socket: &Path,
    context: &Context,
    root: &Path,
    task: &Task,
) -> Result<()> {
    let session = Session::load()?;
    let directory = session.state_directory();
    fs::create_dir_all(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("tasks.lock"))?;
    lock.try_lock()
        .context("another task launch is in progress; try again when it finishes")?;
    ensure!(
        session.same_session(),
        "Herdr session changed; reopen tasks"
    );
    let current = api.raw_snapshot()?;
    validate_context(&current, context)?;
    ensure!(
        super::root(api, context)? == root,
        "Workspace checkout changed; reopen tasks"
    );
    let path = directory.join("tasks.json");
    let mut records: Vec<Destination> = match fs::metadata(&path) {
        Ok(_) => crate::palette::cli::read_json(&path).context("read task destination state")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(error) => return Err(error.into()),
    };
    records.retain(|d| valid(&current, d));
    let workspace = context
        .workspace_id
        .as_deref()
        .context("missing workspace")?;
    let existing = records
        .iter()
        .position(|d| d.workspace == workspace && d.root == root && d.label == task.label);
    let index = match existing {
        Some(index) => index,
        None => {
            let dest = allocate(api, socket, context, &current, root, task)?;
            records.retain(|d| d.pane != dest.pane);
            records.push(dest);
            records.len() - 1
        }
    };
    crate::palette::cli::write_json(&path, &records)?;
    let dest = &records[index];
    let fresh = api.raw_snapshot()?;
    ensure!(
        session.same_session() && valid(&fresh, dest),
        "Task destination closed or changed; run the task again"
    );
    request(socket, "tab.focus", json!({"tab_id":dest.tab}))?;
    request(socket, "pane.focus", json!({"pane_id":dest.pane}))?;
    if existing.is_some() && !pane_idle(socket, &dest.pane)? {
        request(
            socket,
            "pane.send_keys",
            json!({"pane_id":dest.pane,"keys":["ctrl+c"]}),
        )?;
    }
    wait_idle(socket, &dest.pane)?;
    ensure!(
        session.same_session() && valid(&api.raw_snapshot()?, dest),
        "Task destination closed or changed; run the task again"
    );
    // A shell-only foreground group can still be loading startup files.
    // Clear pending input without interrupting that initialization.
    request(
        socket,
        "pane.send_keys",
        json!({"pane_id":dest.pane,"keys":["ctrl+u"]}),
    )?;
    request(
        socket,
        "pane.send_text",
        json!({"pane_id":dest.pane,"text":shell_command(task, root)}),
    )?;
    request(
        socket,
        "pane.send_keys",
        json!({"pane_id":dest.pane,"keys":["enter"]}),
    )?;
    Ok(())
}
