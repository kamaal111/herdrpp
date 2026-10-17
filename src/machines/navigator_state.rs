//! Client state isolation: native inspection never writes shared preferences.
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::state_files::{self, Files, Scope, read_files, write_file};

pub struct StateMirror {
    user: PathBuf,
    private: PathBuf,
    base: Files,
    inspection: Option<Files>,
}

impl StateMirror {
    /// Paths point at the Herdr application directory inside XDG_STATE_HOME.
    pub fn create(user: &Path, private: &Path) -> Result<Self> {
        let base = read_files(user, Scope::CatalogAndPreferences)?;
        for (path, content) in &base {
            write_file(&private.join(path), Some(content))?;
        }
        Ok(Self {
            user: user.into(),
            private: private.into(),
            base,
            inspection: None,
        })
    }

    pub fn begin_inspection(&mut self) -> Result<()> {
        if self.inspection.is_some() {
            bail!("native inspection already active");
        }
        self.synchronize()?;
        self.inspection = Some(read_files(&self.private, Scope::CatalogAndPreferences)?);
        Ok(())
    }

    pub fn end_inspection(&mut self) -> Result<()> {
        let saved = self
            .inspection
            .as_ref()
            .context("native inspection is not active")?;
        let current = read_files(&self.private, Scope::CatalogAndPreferences)?;
        let paths = saved.keys().chain(current.keys()).collect::<BTreeSet<_>>();
        for path in paths {
            write_file(&self.private.join(path), saved.get(path).map(Vec::as_slice))?;
        }
        self.inspection = None;
        self.synchronize()?;
        Ok(())
    }

    pub fn synchronize(&mut self) -> Result<bool> {
        if self.inspection.is_some() {
            return Ok(false);
        }
        let merged = state_files::synchronize(
            &self.user,
            &self.private,
            &self.base,
            Scope::CatalogAndPreferences,
        )?;
        let changed = merged != self.base;
        self.base = merged;
        Ok(changed)
    }
}
