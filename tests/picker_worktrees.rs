use herdrpp::picker::{
    api::parse_snapshot, config::SearchMode, model::Snapshot, ordering, search::Search,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/worktrees.json")).unwrap()
}

fn snapshot(value: Value) -> Snapshot {
    parse_snapshot(&serde_json::to_vec(&value).unwrap()).unwrap()
}

fn groups(snapshot: &Snapshot) -> Vec<(&str, Vec<&str>)> {
    ordering::groups(snapshot)
        .into_iter()
        .map(|group| {
            (
                group.root.workspace_id.as_str(),
                group
                    .worktrees
                    .iter()
                    .map(|w| w.workspace_id.as_str())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn provenance_is_retained_and_focused_worktree_moves_its_whole_group_first() {
    let snapshot = snapshot(fixture());
    let provenance = snapshot.workspaces[4].worktree.as_ref().unwrap();
    assert_eq!(provenance.repo_key.as_deref(), Some("/repos/herdrpp/.git"));
    assert_eq!(provenance.is_linked_worktree, Some(true));
    assert_eq!(
        groups(&snapshot),
        vec![("w1", vec!["w2", "w3", "w4", "w5"]), ("w0", vec![])]
    );
    assert_eq!(ordering::initial_target(&snapshot), Some("w1"));
}

#[test]
fn native_worktree_source_reuses_unique_picker_group_parent() {
    let snapshot = snapshot(fixture());
    assert_eq!(snapshot.worktree_source("w2"), Some("w1"));
    assert_eq!(snapshot.worktree_source("w1"), Some("w1"));
    assert_eq!(snapshot.worktree_source("w0"), Some("w0"));
    assert_eq!(snapshot.worktree_source("missing"), None);
}

#[test]
fn native_worktree_source_rejects_ambiguous_repository_parents() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"][0]["worktree"] =
        fixture["result"]["snapshot"]["workspaces"][1]["worktree"].clone();
    assert_eq!(snapshot(fixture).worktree_source("w2"), None);
}

#[test]
fn unfocused_groups_and_worktree_siblings_keep_ui_order() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["focused_workspace_id"] = json!("w0");
    fixture["result"]["snapshot"]["focused_tab_id"] = json!("w0:t1");
    fixture["result"]["snapshot"]["workspaces"]
        .as_array_mut()
        .unwrap()
        .reverse();
    fixture["result"]["snapshot"]["tabs"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let snapshot = snapshot(fixture);
    assert_eq!(
        groups(&snapshot),
        vec![("w0", vec![]), ("w1", vec!["w2", "w3", "w4", "w5"])]
    );
    assert_eq!(ordering::initial_target(&snapshot), Some("w1"));
    let rows = Search::default().rows(&snapshot, "multimachine", SearchMode::Hierarchical);
    assert_eq!(
        rows.iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["herdrpp", "multimachine", "Codex", "Terminal"]
    );
}

#[test]
fn renamed_root_still_owns_worktrees_with_same_repository_key() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"][1]["label"] = json!("Renamed project");
    let snapshot = snapshot(fixture);
    assert_eq!(groups(&snapshot)[0], ("w1", vec!["w2", "w3", "w4", "w5"]));
    let rows = Search::default().rows(&snapshot, "renamed just terminal", SearchMode::Hierarchical);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].label, "Terminal");
    assert_eq!(rows[0].label, "Renamed project");
    assert_eq!(rows[1].label, "just");
}

#[test]
fn matching_display_names_do_not_join_different_repositories() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"][0]["label"] = json!("herdrpp");
    fixture["result"]["snapshot"]["workspaces"][0]["worktree"] =
        json!({"repo_key":"/repos/other/.git","is_linked_worktree":false});
    let snapshot = snapshot(fixture);
    assert_eq!(
        groups(&snapshot),
        vec![("w1", vec!["w2", "w3", "w4", "w5"]), ("w0", vec![])]
    );
}

#[test]
fn worktrees_with_closed_root_stay_standalone_and_selectable() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    fixture["result"]["snapshot"]["tabs"]
        .as_array_mut()
        .unwrap()
        .retain(|tab| tab["workspace_id"] != "w1");
    let snapshot = snapshot(fixture);
    assert_eq!(
        groups(&snapshot),
        vec![
            ("w4", vec![]),
            ("w0", vec![]),
            ("w2", vec![]),
            ("w3", vec![]),
            ("w5", vec![])
        ]
    );
    let rows = Search::default().rows(&snapshot, "just", SearchMode::Hierarchical);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].depth, 0);
    assert!(rows[0].ancestors.is_empty());
    assert_eq!(rows[1].label, "Terminal");
    assert_eq!(rows[1].workspace_id, rows[0].workspace_id);
}

#[test]
fn missing_worktree_key_leaves_only_that_worktree_standalone() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"][2]["worktree"]
        .as_object_mut()
        .unwrap()
        .remove("repo_key");
    let snapshot = snapshot(fixture);
    assert_eq!(
        groups(&snapshot),
        vec![
            ("w1", vec!["w3", "w4", "w5"]),
            ("w0", vec![]),
            ("w2", vec![])
        ]
    );
}

#[test]
fn blank_repository_key_never_groups_worktrees() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"][1]["worktree"]["repo_key"] = json!("  ");
    fixture["result"]["snapshot"]["workspaces"][4]["worktree"]["repo_key"] = json!("  ");
    let snapshot = snapshot(fixture);
    assert_eq!(
        groups(&snapshot),
        vec![
            ("w4", vec![]),
            ("w0", vec![]),
            ("w1", vec![]),
            ("w2", vec![]),
            ("w3", vec![]),
            ("w5", vec![])
        ]
    );
}

#[test]
fn missing_root_linked_flag_never_infers_a_parent() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"][1]["worktree"]
        .as_object_mut()
        .unwrap()
        .remove("is_linked_worktree");
    let snapshot = snapshot(fixture);
    assert!(
        groups(&snapshot)
            .iter()
            .all(|(_, worktrees)| worktrees.is_empty())
    );
    assert_eq!(groups(&snapshot).len(), 6);
}

#[test]
fn ambiguous_open_roots_leave_linked_worktrees_standalone() {
    let mut fixture = fixture();
    fixture["result"]["snapshot"]["workspaces"][0]["worktree"] =
        json!({"repo_key":"/repos/herdrpp/.git","is_linked_worktree":false});
    let snapshot = snapshot(fixture);
    assert!(
        groups(&snapshot)
            .iter()
            .all(|(_, worktrees)| worktrees.is_empty())
    );
    assert_eq!(groups(&snapshot).len(), 6);
}
