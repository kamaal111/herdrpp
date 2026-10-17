//! Dedicated single-endpoint viewing state with reconciled native preferences.
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

type Files = BTreeMap<PathBuf, Vec<u8>>;

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
        let base = read_files(&user)?;
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
        let user = read_files(&self.user)?;
        let private = read_files(&self.private)?;
        let paths = self
            .base
            .keys()
            .chain(user.keys())
            .chain(private.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut merged = Files::new();
        for path in paths {
            let before = self.base.get(&path);
            let user_now = user.get(&path);
            let private_now = private.get(&path);
            if user_now != before && private_now != before && user_now != private_now {
                bail!(
                    "client state conflict at {}; both files were preserved",
                    path.display()
                );
            }
            let content = if private_now != before {
                private_now
            } else {
                user_now
            };
            if let Some(content) = content {
                merged.insert(path, content.clone());
            }
        }
        if read_files(&self.user)? != user || read_files(&self.private)? != private {
            bail!("client state changed during synchronization; both directories were preserved");
        }
        let changed = merged != self.base;
        let paths = self
            .base
            .keys()
            .chain(merged.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for path in paths {
            let content = merged.get(&path).map(Vec::as_slice);
            if user.get(&path) != merged.get(&path) {
                write_file(&self.user.join(&path), content)?;
            }
            if private.get(&path) != merged.get(&path) {
                write_file(&self.private.join(&path), content)?;
            }
        }
        self.base = merged;
        Ok(changed)
    }
}

fn read_files(root: &Path) -> Result<Files> {
    let mut files = Files::new();
    match fs::read_dir(root.join("client-shell")) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if fs::metadata(entry.path())?.is_file()
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "json")
                {
                    read_one(
                        root,
                        &PathBuf::from("client-shell").join(entry.file_name()),
                        &mut files,
                    )?;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(files)
}

fn read_one(root: &Path, path: &Path, files: &mut Files) -> Result<()> {
    match fs::read(root.join(path)) {
        Ok(content) => {
            serde_json::from_slice::<serde_json::Value>(&content)
                .with_context(|| format!("invalid client state {}", root.join(path).display()))?;
            files.insert(path.into(), content);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn write_file(path: &Path, content: Option<&[u8]>) -> Result<()> {
    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_owned(),
        Err(error) => return Err(error.into()),
    };
    match content {
        Some(content) => {
            fs::create_dir_all(target.parent().context("client state path has no parent")?)?;
            crate::configure::atomic_write(
                &target,
                content,
                fs::metadata(&target)
                    .ok()
                    .map(|metadata| metadata.permissions()),
            )
        }
        None => match fs::remove_file(target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

impl Drop for StateMirror {
    fn drop(&mut self) {
        if let Err(error) = self.synchronize() {
            eprintln!("herdrpp: {error:#}");
        }
    }
}
