//! Dedicated single-endpoint viewing state with reconciled native preferences.
use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::state_files::{self, Files, Scope, read_files, write_file};

pub struct StateMirror {
    user: PathBuf,
    private: PathBuf,
    base: Files,
    temporary: Option<tempfile::TempDir>,
    private_home: PathBuf,
}

impl StateMirror {
    /// `user_home` is the original XDG_STATE_HOME, not its Herdr child.
    pub fn create(user_home: &Path) -> Result<Self> {
        let temporary = tempfile::Builder::new()
            .prefix("herdrpp-machines-")
            .tempdir()?;
        let private_home = temporary.path().to_owned();
        let user = user_home.join("herdr");
        let private = private_home.join("herdr");
        let base = read_files(&user, Scope::Preferences)?;
        for (path, content) in &base {
            write_file(&private.join(path), Some(content))?;
        }
        let state = Self {
            user,
            private,
            base,
            temporary: Some(temporary),
            private_home,
        };
        write_file(
            &state.private.join("client/endpoints.json"),
            Some(br#"{"version":1,"ssh":[]}"#),
        )?;
        state.select_local()?;
        Ok(state)
    }

    pub fn private_home(&self) -> &Path {
        &self.private_home
    }

    fn select_local(&self) -> Result<()> {
        write_file(
            &self.private.join("client/endpoint-selection.json"),
            Some(&serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "selected_profile": serde_json::Value::Null
            }))?),
        )
    }

    pub fn synchronize(&mut self) -> Result<bool> {
        self.synchronize_inner().map_err(|error| {
            if let Some(temporary) = self.temporary.take() {
                let _ = temporary.keep();
            }
            error.context(format!(
                "private client state preserved at {}",
                self.private_home.display()
            ))
        })
    }

    fn synchronize_inner(&mut self) -> Result<bool> {
        let catalog: serde_json::Value =
            serde_json::from_slice(&fs::read(self.private.join("client/endpoints.json"))?)?;
        if !catalog
            .get("ssh")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            bail!(
                "the owned viewing client gained machine profiles; manage profiles with the regular Herdr CLI and switch with the launcher picker"
            );
        }
        let merged =
            state_files::synchronize(&self.user, &self.private, &self.base, Scope::Preferences)?;
        let changed = merged != self.base;
        self.base = merged;
        Ok(changed)
    }
}

impl Drop for StateMirror {
    fn drop(&mut self) {
        if let Err(error) = self.synchronize() {
            eprintln!("herdrpp: {error:#}");
        }
    }
}
