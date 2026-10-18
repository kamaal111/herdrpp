use herdrpp::palette::{
    catalog::{Context, Scope},
    naming::Naming,
};
use serde_json::json;
use std::{process::Command, sync::atomic::AtomicBool};

#[test]
fn snapshot_names_and_tab_count_follow_original_ids_instead_of_current_focus() {
    let context: Context = serde_json::from_value(
        json!({"workspace_id":"w2","tab_id":"w2:t7","focused_pane_id":"w2:p8","tab_label":"stale"}),
    )
    .unwrap();
    let snapshot = json!({
        "focused_workspace_id":"w1",
        "workspaces":[{"workspace_id":"w1","label":"other"},{"workspace_id":"w2","label":"herdrpp","new_workspace_cwd":"/projects/herdrpp"}],
        "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"other"},{"tab_id":"w2:t7","workspace_id":"w2","label":"Codex","custom_label":false},{"tab_id":"w2:t9","workspace_id":"w2","label":"Terminal"}],
        "panes":[{"pane_id":"w2:p8","label":"Tests"}]
    });
    let names = Naming::from_snapshot(&snapshot, &context);
    assert_eq!(names.rename_name(Scope::Workspace), "herdrpp");
    assert_eq!(names.rename_name(Scope::Tab), "Codex");
    assert_eq!(names.rename_name(Scope::Pane), "Tests");
    assert_eq!(names.suggestion(Scope::Tab), "3");
    assert!(names.tab_auto_named);
    assert_eq!(names.workspace_cwd.as_deref(), Some("/projects/herdrpp"));
}

#[test]
fn absent_optional_metadata_falls_back_to_context_labels_and_keeps_pane_empty() {
    let context: Context = serde_json::from_value(json!({"workspace_id":"w2","tab_id":"w2:t7","focused_pane_id":"w2:p8","workspace_label":"Project","tab_label":"Codex"})).unwrap();
    let names = Naming::from_snapshot(
        &json!({"panes":[{"pane_id":"w2:p8","label":null}]}),
        &context,
    );
    assert_eq!(names.rename_name(Scope::Workspace), "Project");
    assert_eq!(names.rename_name(Scope::Tab), "Codex");
    assert_eq!(names.rename_name(Scope::Pane), "");
    assert!(!names.tab_auto_named);
    assert_eq!(names.workspace_cwd, None);
    assert_eq!(names.suggestion(Scope::Workspace), "workspace");
}

#[test]
fn workspace_suggestion_uses_repository_root_from_a_subdirectory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    let cwd = root.join("src");
    std::fs::create_dir_all(&cwd).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let mut names = Naming {
        workspace_cwd: Some(cwd.to_str().unwrap().into()),
        ..Naming::default()
    };
    names.resolve_workspace_suggestion(&AtomicBool::new(false));
    assert_eq!(names.suggestion(Scope::Workspace), "project");
}

#[test]
fn workspace_suggestion_falls_back_to_directory_without_a_repository() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().join("notes");
    std::fs::create_dir(&cwd).unwrap();
    let mut names = Naming {
        workspace_cwd: Some(cwd.to_str().unwrap().into()),
        ..Naming::default()
    };
    names.resolve_workspace_suggestion(&AtomicBool::new(false));
    assert_eq!(names.suggestion(Scope::Workspace), "notes");
}
