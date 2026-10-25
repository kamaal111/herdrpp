use herdrpp::palette::{
    catalog::{self, Command, Context, Item},
    history::{History, UsageKey},
};
use herdrpp::tasks::config::Task;
use std::path::Path;

fn task(root: &str, label: &str, id: &str) -> Item {
    Item::new(
        id.into(),
        label.into(),
        "Task".into(),
        "true".into(),
        Command::Task {
            root: root.into(),
            task: Task {
                label: label.into(),
                command: "true".into(),
                args: vec![],
                new_tab: true,
            },
        },
    )
    .unwrap()
}

#[test]
fn history_persists_deduplicates_and_resolves_only_available_commands() {
    let directory = tempfile::tempdir().unwrap();
    let items = catalog::builtins(&Context::default()).unwrap();
    let search = UsageKey::for_item(&items[0]);
    History::record(directory.path(), search.clone()).unwrap();
    History::record(
        directory.path(),
        UsageKey::for_item(&task("/app", "dev", "task:0")),
    )
    .unwrap();
    History::record(directory.path(), search).unwrap();
    let history = History::load(directory.path()).unwrap();
    assert_eq!(history.recent_ids(&items), vec!["herdr:search_workspaces"]);
    let stored: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("palette-history.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(stored.as_array().unwrap().len(), 2);
}

#[test]
fn tasks_follow_checkout_and_label_after_file_reordering() {
    let directory = tempfile::tempdir().unwrap();
    History::record(
        directory.path(),
        UsageKey::for_item(&task("/app", "dev", "task:0")),
    )
    .unwrap();
    let reordered = task("/app", "dev", "task:2");
    let items = vec![
        task("/other", "dev", "task:0"),
        task("/app", "renamed", "task:1"),
        reordered,
    ];
    assert_eq!(
        History::load(directory.path()).unwrap().recent_ids(&items),
        vec!["task:2"]
    );
}

#[test]
fn missing_history_is_empty_and_malformed_history_can_be_replaced_on_next_launch() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        History::load(directory.path())
            .unwrap()
            .recent_ids(&[])
            .is_empty()
    );
    std::fs::write(directory.path().join("palette-history.json"), "broken").unwrap();
    assert!(History::load(directory.path()).is_err());
    let items = catalog::builtins(&Context::default()).unwrap();
    History::record(directory.path(), UsageKey::for_item(&items[0])).unwrap();
    assert_eq!(
        History::load(directory.path()).unwrap().recent_ids(&items),
        vec!["herdr:search_workspaces"]
    );
}

#[test]
fn concurrent_updates_preserve_each_successful_record() {
    let directory = tempfile::tempdir().unwrap();
    let items: Vec<_> = (0..12)
        .map(|n| task("/app", &format!("task-{n}"), &format!("task:{n}")))
        .collect();
    std::thread::scope(|scope| {
        for item in &items {
            let root = directory.path();
            scope.spawn(move || History::record(root, UsageKey::for_item(item)).unwrap());
        }
    });
    assert_eq!(
        History::load(directory.path())
            .unwrap()
            .recent_ids(&items)
            .len(),
        12
    );
}

#[test]
fn history_retains_only_the_latest_thousand_items() {
    let directory = tempfile::tempdir().unwrap();
    let items: Vec<_> = (0..1001)
        .map(|n| task("/app", &format!("task-{n}"), &format!("task:{n}")))
        .collect();
    let keys: Vec<_> = items.iter().rev().map(UsageKey::for_item).collect();
    std::fs::write(
        directory.path().join("palette-history.json"),
        serde_json::to_vec(&keys).unwrap(),
    )
    .unwrap();
    History::record(directory.path(), UsageKey::for_item(&items[1000])).unwrap();
    let recent = History::load(directory.path()).unwrap().recent_ids(&items);
    assert_eq!(recent.len(), 1000);
    assert_eq!(recent[0], "task:1000");
    assert!(!recent.contains(&"task:0".into()));
}

#[test]
fn unusable_state_directory_returns_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file");
    std::fs::write(&file, "occupied").unwrap();
    let item = task("/app", "dev", "task:0");
    assert!(History::record(Path::new(&file), UsageKey::for_item(&item)).is_err());
}
