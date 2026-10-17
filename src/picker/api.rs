use std::{ffi::OsString, process::Command, sync::atomic::AtomicBool, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

use crate::picker::model::{Snapshot, Target};

const TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 32 * 1024 * 1024;

pub struct Herdr {
    pub binary: OsString,
}

impl Default for Herdr {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()),
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<Value> {
    let response: Value = serde_json::from_slice(bytes).context("invalid Herdr JSON response")?;
    if let Some(error) = response.get("error") {
        bail!(
            "Herdr {}: {}",
            error["code"].as_str().unwrap_or("error"),
            error["message"].as_str().unwrap_or("request failed")
        );
    }
    response
        .get("result")
        .filter(|result| result.is_object())
        .cloned()
        .context("Herdr response has no object result")
}

pub fn parse_snapshot(bytes: &[u8]) -> Result<Snapshot> {
    let result = decode(bytes)?;
    let mut snapshot: Snapshot = serde_json::from_value(
        result
            .get("snapshot")
            .cloned()
            .context("Herdr response has no snapshot")?,
    )
    .context("invalid Herdr snapshot")?;
    snapshot.validate()?;
    Ok(snapshot)
}

pub fn focus_args(target: &Target) -> Vec<&str> {
    match target {
        Target::Workspace(id) => vec!["workspace", "focus", id],
        Target::Tab(id) => vec!["tab", "focus", id],
    }
}

fn validate_focus(result: &Value, target: &Target) -> Result<()> {
    let (kind, record, field, expected) = match target {
        Target::Workspace(id) => ("workspace_info", "workspace", "workspace_id", id),
        Target::Tab(id) => ("tab_info", "tab", "tab_id", id),
    };
    ensure!(
        result["type"].as_str() == Some(kind),
        "invalid Herdr focus response type"
    );
    ensure!(
        result[record][field].as_str() == Some(expected.as_str()),
        "Herdr focus response has an unexpected destination ID"
    );
    Ok(())
}

impl Herdr {
    pub fn call(&self, args: &[&str]) -> Result<Vec<u8>> {
        self.call_cancellable(args, &AtomicBool::new(false))
    }

    pub(crate) fn call_cancellable(
        &self,
        args: &[&str],
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>> {
        let output = crate::command::run_cancellable(
            Command::new(&self.binary).args(args),
            TIMEOUT,
            OUTPUT_LIMIT,
            cancelled,
            "Herdr request",
        )
        .map_err(|error| {
            // Keep launch diagnostics distinct from errors returned by a running CLI.
            if error.to_string().starts_with("start ") {
                error.context("start Herdr CLI (check HERDR_BIN_PATH)")
            } else {
                error
            }
        })?;
        if !output.status.success() {
            if !output.stdout.is_empty() {
                decode(&output.stdout)?;
            }
            bail!(
                "Herdr request failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output.stdout)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        parse_snapshot(&self.call(&["api", "snapshot"])?)
    }

    pub fn snapshot_cancellable(&self, cancelled: &AtomicBool) -> Result<Snapshot> {
        parse_snapshot(&self.call_cancellable(&["api", "snapshot"], cancelled)?)
    }

    pub fn open(&self) -> Result<()> {
        self.open_popup("picker", None)
    }

    pub(crate) fn open_popup(&self, entrypoint: &str, environment: Option<&str>) -> Result<()> {
        let mut args = vec![
            "plugin",
            "pane",
            "open",
            "--plugin",
            crate::PLUGIN_ID,
            "--entrypoint",
            entrypoint,
        ];
        if let Some(environment) = environment {
            args.extend(["--env", environment]);
        }
        decode(&self.call(&args)?)?;
        Ok(())
    }

    pub fn focus(&self, target: &Target) -> Result<()> {
        let result = decode(&self.call(&focus_args(target))?)?;
        validate_focus(&result, target)
    }

    pub fn notify(&self, message: &str) -> Result<()> {
        decode(&self.call(&[
            "notification",
            "show",
            crate::DISPLAY_NAME,
            "--body",
            message,
        ])?)?;
        Ok(())
    }
}

/// Public CLI routing for a launcher-owned machine, independent of inherited pane context.
pub struct MachineApi<'a> {
    pub binary: &'a std::ffi::OsStr,
    pub machine: &'a str,
    pub local_session: &'a str,
    pub profile: Option<&'a crate::machines::catalog::Profile>,
}
impl MachineApi<'_> {
    fn call(&self, args: &[&str], cancelled: &AtomicBool) -> Result<Vec<u8>> {
        let mut command = Command::new(self.binary);
        // Freeze the browsed profile for each public CLI request. Concurrent catalog
        // edits must not redirect a snapshot or focus request to another SSH target.
        let catalog = tempfile::tempdir()?;
        for key in [
            "HERDR_SOCKET_PATH",
            "HERDR_CLIENT_SOCKET_PATH",
            "HERDR_SESSION",
            "HERDR_PANE_ID",
            "HERDR_TAB_ID",
            "HERDR_WORKSPACE_ID",
        ] {
            command.env_remove(key);
        }
        if self.machine == "local" {
            command.args(["--session", self.local_session]);
        } else {
            let profile = self.profile.context("missing browsed machine identity")?;
            ensure!(profile.id == self.machine, "machine identity mismatch");
            let directory = catalog.path().join("herdr/client");
            std::fs::create_dir_all(&directory)?;
            std::fs::write(
                directory.join("endpoints.json"),
                serde_json::to_vec(&serde_json::json!({"version": 1, "ssh": [profile]}))?,
            )?;
            command
                .env("XDG_STATE_HOME", catalog.path())
                .args(["--machine", self.machine]);
        }
        let output = crate::command::run_cancellable(
            command.args(args),
            Duration::from_secs(20),
            OUTPUT_LIMIT,
            cancelled,
            "machine request",
        )?;
        if !output.status.success() {
            if !output.stdout.is_empty() {
                decode(&output.stdout)?;
            }
            bail!(
                "machine request failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output.stdout)
    }
    pub fn snapshot(&self, cancelled: &AtomicBool) -> Result<Snapshot> {
        parse_snapshot(&self.call(&["api", "snapshot"], cancelled)?)
    }
    pub fn validate_target(&self, target: &Target, cancelled: &AtomicBool) -> Result<()> {
        let snapshot = self.snapshot(cancelled)?;
        let exists = match target {
            Target::Workspace(id) => snapshot.workspaces.iter().any(|w| &w.workspace_id == id),
            Target::Tab(id) => snapshot.tabs.iter().any(|t| &t.tab_id == id),
        };
        ensure!(exists, "selected destination no longer exists");
        Ok(())
    }
    pub fn focus_cancellable(&self, target: &Target, cancelled: &AtomicBool) -> Result<()> {
        self.validate_target(target, cancelled)?;
        let response = decode(&self.call(&focus_args(target), cancelled)?)?;
        validate_focus(&response, target)
    }
}
