use super::{
    catalog::{Command, Context, Direction, Scope},
    naming::Naming,
};
use crate::picker::api::{Herdr, decode};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

const LIMIT: u64 = 4 * 1024 * 1024;

/// One request per connection; mutation requests are never retried.
pub fn request(socket: &Path, method: &str, params: Value) -> Result<Value> {
    let mut stream = UnixStream::connect(socket).context("connect to Herdr session socket")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let id = format!("herdrpp-palette-{}", std::process::id());
    let mut bytes = serde_json::to_vec(&json!({"id":id,"method":method,"params":params}))?;
    ensure!(
        bytes.len() < LIMIT as usize,
        "Herdr request exceeds output limit"
    );
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let mut line = Vec::new();
    BufReader::new(stream)
        .take(LIMIT + 1)
        .read_until(b'\n', &mut line)?;
    ensure!(
        line.len() <= LIMIT as usize && line.last() == Some(&b'\n'),
        "invalid or oversized Herdr response"
    );
    let response: Value = serde_json::from_slice(&line).context("invalid Herdr JSON response")?;
    ensure!(
        response["id"].as_str() == Some(&id),
        "Herdr response ID mismatch"
    );
    decode(&line)
}

pub fn validate_context(snapshot: &Value, context: &Context) -> Result<()> {
    if let Some(id) = &context.workspace_id {
        ensure!(
            snapshot["workspaces"]
                .as_array()
                .context("missing workspaces")?
                .iter()
                .any(|workspace| workspace["workspace_id"].as_str() == Some(id)),
            "original workspace is no longer available"
        );
    }
    if let Some(id) = &context.tab_id {
        ensure!(
            snapshot["tabs"]
                .as_array()
                .context("missing tabs")?
                .iter()
                .any(|tab| tab["tab_id"].as_str() == Some(id)
                    && tab["workspace_id"].as_str() == context.workspace_id.as_deref()),
            "original tab is no longer available"
        );
    }
    if let Some(id) = &context.focused_pane_id {
        ensure!(
            snapshot["panes"]
                .as_array()
                .context("missing panes")?
                .iter()
                .any(|pane| pane["pane_id"].as_str() == Some(id)
                    && pane["workspace_id"].as_str() == context.workspace_id.as_deref()
                    && pane["tab_id"].as_str() == context.tab_id.as_deref()),
            "original pane is no longer available"
        );
    }
    Ok(())
}

pub fn arguments(command: &Command, context: &Context) -> Result<Option<Vec<String>>> {
    let required = |scope: Scope| scope.id(context).context("command target is unavailable");
    let args: Vec<String> = match command {
        Command::SearchWorkspaces => return Ok(None),
        Command::Plugin { .. } => return Ok(None),
        Command::Create(_) | Command::Rename(_) => return Ok(None),
        Command::Close(scope) => vec![scope.cli().into(), "close".into(), required(*scope)?.into()],
        Command::Split(direction @ (Direction::Right | Direction::Down)) => vec![
            "pane".into(),
            "split".into(),
            "--pane".into(),
            required(Scope::Pane)?.into(),
            "--direction".into(),
            direction.cli().into(),
            "--focus".into(),
        ],
        Command::Split(_) => anyhow::bail!("unsupported split direction"),
        Command::Focus(direction) => vec![
            "pane".into(),
            "focus".into(),
            "--pane".into(),
            required(Scope::Pane)?.into(),
            "--direction".into(),
            direction.cli().into(),
        ],
        Command::Zoom => vec![
            "pane".into(),
            "zoom".into(),
            "--pane".into(),
            required(Scope::Pane)?.into(),
            "--toggle".into(),
        ],
    };
    Ok(Some(args))
}

pub fn rename_request(
    scope: Scope,
    context: &Context,
    input: Option<&str>,
) -> Result<(String, Value)> {
    let label = input.context("rename requires a name")?;
    ensure!(
        !label.trim().is_empty() && !label.chars().any(char::is_control),
        "name must contain visible text without control characters"
    );
    let id = scope.id(context).context("command target is unavailable")?;
    let mut params = serde_json::Map::new();
    params.insert(format!("{}_id", scope.cli()), json!(id));
    params.insert("label".into(), json!(label));
    Ok((format!("{}.rename", scope.cli()), Value::Object(params)))
}

pub fn create_request(
    scope: Scope,
    context: &Context,
    naming: &Naming,
    input: Option<&str>,
) -> Result<(String, Value)> {
    let input = input.context("creation requires a name prompt")?;
    ensure!(
        !input.chars().any(char::is_control),
        "name must not contain control characters"
    );
    let trimmed = input.trim();
    let label = (!trimmed.is_empty() && trimmed != naming.suggestion(scope)).then_some(trimmed);
    let workspace_id = context
        .workspace_id
        .as_deref()
        .context("command target is unavailable")?;
    let params = match scope {
        Scope::Workspace => {
            json!({"source_workspace_id":workspace_id,"cwd":naming.workspace_cwd,"focus":true,"label":label})
        }
        Scope::Tab => json!({"workspace_id":workspace_id,"focus":true,"label":label}),
        Scope::Pane => anyhow::bail!("pane creation requires a split direction"),
    };
    Ok((format!("{}.create", scope.cli()), params))
}

pub fn execute(
    api: &Herdr,
    socket: &Path,
    command: &Command,
    context: &Context,
    naming: &Naming,
    input: Option<&str>,
) -> Result<()> {
    let snapshot = decode(&api.call(&["api", "snapshot"])?)?;
    validate_context(
        snapshot.get("snapshot").context("missing Herdr snapshot")?,
        context,
    )?;
    match command {
        Command::SearchWorkspaces => api.open(),
        Command::Create(scope) => {
            let (method, params) = create_request(*scope, context, naming, input)?;
            request(socket, &method, params)?;
            Ok(())
        }
        Command::Rename(scope) => {
            if *scope == Scope::Tab
                && naming.tab_auto_named
                && input == Some(naming.tab_name.as_str())
            {
                return Ok(());
            }
            let (method, params) = rename_request(*scope, context, input)?;
            request(socket, &method, params)?;
            Ok(())
        }
        Command::Plugin {
            plugin_id,
            action_id,
        } => {
            request(
                socket,
                "plugin.action.invoke",
                json!({"plugin_id":plugin_id,"action_id":action_id,"context":context}),
            )?;
            Ok(())
        }
        _ => {
            let args = arguments(command, context)?.context("missing command arguments")?;
            decode(&api.call(&args.iter().map(String::as_str).collect::<Vec<_>>())?)?;
            Ok(())
        }
    }
}
