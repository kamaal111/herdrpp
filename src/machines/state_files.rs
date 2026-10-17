//! Shared client-state I/O and three-way reconciliation for both viewing strategies.
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub(super) type Files = BTreeMap<PathBuf, Vec<u8>>;

#[derive(Clone, Copy)]
pub(super) enum Scope {
    Preferences,
    CatalogAndPreferences,
}

pub(super) fn synchronize(
    user_root: &Path,
    private_root: &Path,
    base: &Files,
    scope: Scope,
) -> Result<Files> {
    let user = read_files(user_root, scope)?;
    let private = read_files(private_root, scope)?;
    let paths = base
        .keys()
        .chain(user.keys())
        .chain(private.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut merged = Files::new();
    for path in paths {
        let before = base.get(&path);
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
    if read_files(user_root, scope)? != user || read_files(private_root, scope)? != private {
        bail!("client state changed during synchronization; both directories were preserved");
    }
    let paths = base
        .keys()
        .chain(merged.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for path in paths {
        let content = merged.get(&path).map(Vec::as_slice);
        if user.get(&path) != merged.get(&path) {
            write_file(&user_root.join(&path), content)?;
        }
        if private.get(&path) != merged.get(&path) {
            write_file(&private_root.join(&path), content)?;
        }
    }
    Ok(merged)
}

pub(super) fn read_files(root: &Path, scope: Scope) -> Result<Files> {
    let mut files = Files::new();
    if matches!(scope, Scope::CatalogAndPreferences) {
        read_one(root, Path::new("client/endpoints.json"), &mut files)?;
    }
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

pub(super) fn write_file(path: &Path, content: Option<&[u8]>) -> Result<()> {
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
