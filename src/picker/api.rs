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

    pub(crate) fn raw_snapshot(&self) -> Result<Value> {
        let result = decode(&self.call(&["api", "snapshot"])?)?;
        result
            .get("snapshot")
            .cloned()
            .context("missing Herdr snapshot")
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
