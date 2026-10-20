//! Durable usage history, independent of ephemeral palette selection IDs.
use super::{
    catalog::{Command, Item},
    cli::{read_json, write_json},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

const FILE: &str = "palette-history.json";
const MAX_ENTRIES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UsageKey {
    Command(String),
    Task { root: PathBuf, label: String },
}

impl UsageKey {
    pub fn for_item(item: &Item) -> Self {
        match &item.command {
            Command::Task { root, task } => Self::Task {
                root: root.clone(),
                label: task.label.clone(),
            },
            _ => Self::Command(item.id.clone()),
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct History(Vec<UsageKey>);

impl History {
    pub fn load(directory: &Path) -> Result<Self> {
        match read_json(&directory.join(FILE)) {
            Ok(history) => Ok(history),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(Self::default())
            }
            Err(error) => Err(error).context("read palette usage history"),
        }
    }

    pub fn recent_ids(&self, items: &[Item]) -> Vec<String> {
        let available: HashMap<_, _> = items
            .iter()
            .map(|item| (UsageKey::for_item(item), &item.id))
            .collect();
        self.0
            .iter()
            .filter_map(|key| available.get(key).map(|id| (*id).clone()))
            .collect()
    }

    pub fn record(directory: &Path, key: UsageKey) -> Result<()> {
        fs::create_dir_all(directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("palette-history.lock"))?;
        lock.lock().context("lock palette usage history")?;
        let mut history = match Self::load(directory) {
            Ok(history) => history,
            Err(error) => {
                eprintln!("herdr++: {error:#}; starting fresh usage history");
                Self::default()
            }
        };
        history.0.retain(|entry| entry != &key);
        history.0.insert(0, key);
        history.0.truncate(MAX_ENTRIES);
        write_json(&directory.join(FILE), &history).context("save palette usage history")
    }
}

pub fn state_directory() -> Result<PathBuf> {
    let path = PathBuf::from(
        std::env::var_os("HERDR_PLUGIN_STATE_DIR")
            .context("HERDR_PLUGIN_STATE_DIR is required for palette usage history")?,
    );
    ensure!(
        path.is_absolute(),
        "HERDR_PLUGIN_STATE_DIR must be absolute"
    );
    Ok(path)
}
