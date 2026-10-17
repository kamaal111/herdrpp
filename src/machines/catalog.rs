use std::{ffi::OsStr, process::Command, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::command;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub target: String,
    pub session: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    Local { session: String },
    Remote(Profile),
}

impl Destination {
    pub fn id(&self) -> &str {
        match self {
            Self::Local { .. } => "local",
            Self::Remote(profile) => &profile.id,
        }
    }

    pub fn command(&self, binary: &OsStr) -> Command {
        let mut command = Command::new(binary);
        for key in [
            "HERDR_CLIENT_SOCKET_PATH",
            "HERDR_REMOTE_PREFERENCES_IDENTITY",
            "HERDR_REATTACH_COMMAND",
            "HERDR_REMOTE_KEYBINDINGS",
            "HERDR_PANE_ID",
            "HERDR_TAB_ID",
            "HERDR_WORKSPACE_ID",
        ] {
            command.env_remove(key);
        }
        match self {
            Self::Local { session } => {
                command.args(["--session", session]);
            }
            Self::Remote(profile) => {
                command.env_remove("HERDR_SOCKET_PATH");
                command.args(["--remote", &profile.target, "--session", &profile.session]);
            }
        }
        command
    }
}

fn command_at(binary: &OsStr, state_home: Option<&std::path::Path>) -> Command {
    let mut command = Command::new(binary);
    if let Some(state_home) = state_home {
        command.env("XDG_STATE_HOME", state_home);
    }
    command
}

pub fn load(binary: &OsStr) -> Result<Vec<Profile>> {
    load_in(binary, None)
}

pub fn load_in(binary: &OsStr, state_home: Option<&std::path::Path>) -> Result<Vec<Profile>> {
    let output = command::run(
        command_at(binary, state_home).args(["machine", "list", "--json"]),
        Duration::from_secs(10),
    )?;
    let profiles: Vec<Profile> =
        serde_json::from_slice(&output).context("read Herdr machine catalog")?;
    let mut ids = std::collections::HashSet::new();
    for profile in &profiles {
        if profile.id.is_empty()
            || profile.id == "local"
            || profile.id.starts_with('-')
            || profile.id.chars().any(char::is_control)
            || profile.label.is_empty()
            || profile.label.chars().any(char::is_control)
            || profile.target.is_empty()
            || profile.target.starts_with('-')
            || profile.target.chars().any(char::is_control)
            || profile.session.is_empty()
            || profile.session.starts_with('-')
            || profile.session.chars().any(char::is_control)
            || !ids.insert(&profile.id)
        {
            bail!("invalid or duplicate machine profile in Herdr catalog");
        }
    }
    Ok(profiles
        .into_iter()
        .filter(|profile| profile.enabled)
        .collect())
}

pub fn preflight(binary: &OsStr, destination: &Destination) -> Result<()> {
    preflight_cancellable(
        binary,
        destination,
        &std::sync::atomic::AtomicBool::new(false),
    )
}

pub fn preflight_cancellable(
    binary: &OsStr,
    destination: &Destination,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    preflight_in(binary, destination, cancelled, None)
}

pub fn preflight_in(
    binary: &OsStr,
    destination: &Destination,
    cancelled: &std::sync::atomic::AtomicBool,
    state_home: Option<&std::path::Path>,
) -> Result<()> {
    if let Destination::Remote(profile) = destination {
        let output = command::run_cancellable(
            command_at(binary, state_home).args(["machine", "status", &profile.id, "--json"]),
            Duration::from_secs(30),
            1024 * 1024,
            cancelled,
            "check machine status",
        )?;
        #[derive(Deserialize)]
        struct Status {
            id: String,
            status: String,
            error: Option<String>,
        }
        let statuses: Vec<Status> = serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "read machine status ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )
        })?;
        let status = statuses
            .iter()
            .find(|status| status.id == profile.id)
            .context("selected machine disappeared from Herdr status")?;
        if status.status != "reachable" {
            bail!(
                "{}: {}{}",
                profile.label,
                status.status,
                status
                    .error
                    .as_ref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            );
        }
        if !output.status.success() {
            bail!(
                "machine status failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    Ok(())
}
