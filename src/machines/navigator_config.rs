//! Isolate navigator control bindings while preserving native Settings edits.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table, TableLike};

#[derive(Clone, Debug)]
pub struct ControlBinding {
    pub action: String,
    pub key: String,
    pub defaults: Vec<String>,
}

/// The pinned Herdr release publishes effective action defaults as commented
/// TOML entries in --default-config. Fail closed if that contract changes.
pub fn default_binding(template: &str, action: &str) -> Result<Vec<String>> {
    let mut keys = false;
    for line in template.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            keys = line == "[keys]";
        }
        if !keys {
            continue;
        }
        let entry = line.strip_prefix("# ").unwrap_or(line);
        if let Some((name, _)) = entry.split_once('=')
            && name.trim() == action
        {
            let document: DocumentMut = entry.parse().context("invalid published key default")?;
            return binding_values(document.get(action));
        }
    }
    bail!("Herdr --default-config does not publish keys.{action}")
}

pub struct ConfigMirror {
    user_path: PathBuf,
    private_path: PathBuf,
    base_user: DocumentMut,
    base_private: DocumentMut,
    controls: Vec<ControlBinding>,
    shared_state: PathBuf,
}

impl ConfigMirror {
    pub fn create(
        user_path: &Path,
        private_path: &Path,
        controls: Vec<ControlBinding>,
        shared_state: &Path,
    ) -> Result<Self> {
        let _lock = crate::configure::lock_at(shared_state)?;
        let user_path = crate::configure::resolve(&std::env::current_dir()?.join(user_path), 0)?;
        let base_user = read_document(&user_path)?;
        let base_private = inject(&base_user, &controls)?;
        atomic_write(private_path, &base_private.to_string())?;
        Ok(Self {
            user_path,
            private_path: private_path.into(),
            base_user,
            base_private,
            controls,
            shared_state: shared_state.into(),
        })
    }

    /// Merge disjoint edits; leave both files untouched when native Settings and
    /// an external editor changed the same setting differently.
    pub fn synchronize(&mut self) -> Result<bool> {
        let _lock = crate::configure::lock_at(&self.shared_state)?;
        let mut user = read_document(&self.user_path)?;
        let current_user_text = user.to_string();
        let private = read_document(&self.private_path)?;
        let base_clean = strip_controls(&self.base_private, &self.base_user, &self.controls)?;
        let clean = strip_controls(&private, &self.base_user, &self.controls)?;
        let patches = changed_paths(&semantic(&base_clean)?, &semantic(&clean)?, Vec::new());
        let base_user = semantic(&self.base_user)?;
        let current_user = semantic(&user)?;
        let desired = semantic(&clean)?;
        for path in &patches {
            let current = lookup(&current_user, path);
            let before = lookup(&base_user, path);
            let after = lookup(&desired, path);
            if current != before && current != after {
                self.save_recovery(&private, &user)?;
                bail!(
                    "native Settings conflict at {}; both config files were preserved",
                    path.join(".")
                );
            }
        }
        for path in &patches {
            replace_at(
                user.as_table_mut(),
                path,
                item_at(clean.as_table(), path).cloned(),
            )?;
        }
        let injected = inject(&user, &self.controls)?;
        let user_text = user.to_string();
        let private_text = injected.to_string();
        let changed =
            user_text != self.base_user.to_string() || private_text != private.to_string();
        if read_document(&self.user_path)?.to_string() != current_user_text
            || read_document(&self.private_path)?.to_string() != private.to_string()
        {
            self.save_recovery(&private, &user)?;
            bail!("config changed during synchronization; both config files were preserved");
        }
        if !patches.is_empty() && user_text != current_user_text {
            atomic_write(&self.user_path, &user_text)?;
        }
        if private_text != private.to_string() {
            atomic_write(&self.private_path, &private_text)?;
        }
        self.base_user = user;
        self.base_private = injected;
        Ok(changed)
    }

    fn save_recovery(&self, private: &DocumentMut, current: &DocumentMut) -> Result<()> {
        atomic_write(
            &self.shared_state.join("settings-base.toml"),
            &self.base_user.to_string(),
        )?;
        atomic_write(
            &self.shared_state.join("settings-current.toml"),
            &current.to_string(),
        )?;
        atomic_write(
            &self.shared_state.join("settings-pending.toml"),
            &private.to_string(),
        )
    }
}

fn read_document(path: &Path) -> Result<DocumentMut> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .parse()
            .with_context(|| format!("invalid config {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(error) => Err(error).with_context(|| format!("read config {}", path.display())),
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    fs::create_dir_all(path.parent().context("config has no parent")?)?;
    crate::configure::atomic_write(
        path,
        content.as_bytes(),
        fs::metadata(path)
            .ok()
            .map(|metadata| metadata.permissions()),
    )
}

fn binding_values(item: Option<&Item>) -> Result<Vec<String>> {
    match item {
        None => Ok(Vec::new()),
        Some(item) => {
            if let Some(value) = item.as_str() {
                return Ok(vec![value.to_owned()]);
            }
            let values = item
                .as_array()
                .context("navigator binding must be a string or string array")?;
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .context("navigator binding contains a non-string key")
                })
                .collect()
        }
    }
}

fn inject(document: &DocumentMut, controls: &[ControlBinding]) -> Result<DocumentMut> {
    validate_controls(document, controls)?;
    let mut result = document.clone();
    for control in controls {
        let path = ["keys".to_owned(), control.action.clone()];
        let existing = item_at(result.as_table(), &path);
        let mut values = match existing {
            Some(item) => binding_values(Some(item))?,
            None => control.defaults.clone(),
        };
        if values.contains(&control.key) {
            bail!(
                "internal navigator binding {} collides with user keys.{}",
                control.key,
                control.action
            );
        }
        values.push(control.key.clone());
        let mut array = toml_edit::Array::new();
        for value in values {
            array.push(value);
        }
        replace_at(
            result.as_table_mut(),
            &path,
            Some(Item::Value(array.into())),
        )?;
    }
    Ok(result)
}

fn normalized_key(key: &str) -> Vec<String> {
    let mut parts = key
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "control" => "ctrl".to_owned(),
            "command" | "cmd" => "super".to_owned(),
            "option" | "meta" => "alt".to_owned(),
            "\\" => "backslash".to_owned(),
            part => part.to_owned(),
        })
        .collect::<Vec<_>>();
    parts.sort();
    parts.dedup();
    parts
}

fn collect_key_values(value: &toml::Value, keys: &mut Vec<String>) {
    match value {
        toml::Value::String(value) => keys.push(value.clone()),
        toml::Value::Array(values) => {
            for value in values {
                collect_key_values(value, keys);
            }
        }
        toml::Value::Table(values) => {
            for (name, value) in values {
                if name == "command" {
                    if let Some(commands) = value.as_array() {
                        for command in commands {
                            if let Some(key) = command.get("key") {
                                collect_key_values(key, keys);
                            }
                        }
                    }
                } else {
                    collect_key_values(value, keys);
                }
            }
        }
        _ => {}
    }
}

fn validate_controls(document: &DocumentMut, controls: &[ControlBinding]) -> Result<()> {
    let document = semantic(document)?;
    let mut occupied = Vec::new();
    if let Some(keys) = document.get("keys") {
        collect_key_values(keys, &mut occupied);
    }
    for control in controls {
        if occupied
            .iter()
            .chain(control.defaults.iter())
            .any(|key| normalized_key(key) == normalized_key(&control.key))
        {
            bail!(
                "internal navigator binding {} collides with an existing binding",
                control.key
            );
        }
        occupied.push(control.key.clone());
    }
    Ok(())
}

fn strip_controls(
    document: &DocumentMut,
    base_user: &DocumentMut,
    controls: &[ControlBinding],
) -> Result<DocumentMut> {
    let mut result = document.clone();
    for control in controls {
        let path = ["keys".to_owned(), control.action.clone()];
        let original = item_at(base_user.as_table(), &path);
        let mut values = binding_values(item_at(result.as_table(), &path))?;
        values.retain(|value| value != &control.key);
        let replacement = if values
            == match original {
                Some(item) => binding_values(Some(item))?,
                None => control.defaults.clone(),
            } {
            original.cloned()
        } else {
            let mut array = toml_edit::Array::new();
            for value in values {
                array.push(value);
            }
            Some(Item::Value(array.into()))
        };
        replace_at(result.as_table_mut(), &path, replacement)?;
    }
    Ok(result)
}

fn semantic(document: &DocumentMut) -> Result<toml::Value> {
    toml::from_str(&document.to_string()).context("decode config")
}

fn lookup<'a>(value: &'a toml::Value, path: &[String]) -> Option<&'a toml::Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

fn changed_paths(before: &toml::Value, after: &toml::Value, path: Vec<String>) -> Vec<Vec<String>> {
    if before == after {
        return Vec::new();
    }
    if let (Some(left), Some(right)) = (before.as_table(), after.as_table()) {
        let keys = left.keys().chain(right.keys()).collect::<BTreeSet<_>>();
        return keys
            .into_iter()
            .flat_map(|key| {
                let mut next = path.clone();
                next.push(key.clone());
                match (left.get(key), right.get(key)) {
                    (Some(before), Some(after)) => changed_paths(before, after, next),
                    _ => vec![next],
                }
            })
            .collect();
    }
    vec![path]
}

fn item_at<'a>(table: &'a dyn TableLike, path: &[String]) -> Option<&'a Item> {
    let (last, parents) = path.split_last()?;
    parents
        .iter()
        .try_fold(table, |table, key| table.get(key)?.as_table_like())?
        .get(last)
}

fn replace_at(table: &mut dyn TableLike, path: &[String], item: Option<Item>) -> Result<()> {
    let (first, remaining) = path.split_first().context("cannot replace config root")?;
    if remaining.is_empty() {
        match item {
            Some(item) => {
                table.insert(first, item);
            }
            None => {
                table.remove(first);
            }
        }
        return Ok(());
    }
    if !table.contains_key(first) {
        table.insert(first, Item::Table(Table::new()));
    }
    let child = table
        .get_mut(first)
        .and_then(Item::as_table_like_mut)
        .context("config setting is not a table")?;
    replace_at(child, remaining, item)
}
