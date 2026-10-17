use herdrpp::command;
use herdrpp::pr::{
    github::{Checkout, newest_pr, parse_remote},
    herdr::Snapshot,
};
use serde_json::json;
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

fn checkout() -> Checkout {
    Checkout {
        root: PathBuf::from("/tmp/project"),
        branch: "feature".into(),
        head_repo: "alice/project".into(),
        head_branch: "feature".into(),
        host: "github.com".into(),
    }
}

fn pr(number: u64, date: &str, owner: &str, branch: &str) -> serde_json::Value {
    json!({"number":number,"created_at":date,"head":{"ref":branch,"repo":{"full_name":owner}}})
}

#[test]
fn newest_pr_includes_closed_and_merged_across_paginated_results() {
    let first = json!([
        pr(123, "2026-10-01T00:00:00Z", "alice/project", "feature"),
        pr(124, "2026-10-02T00:00:00Z", "alice/project", "feature")
    ]);
    let second = json!([pr(125, "2026-10-03T00:00:00Z", "alice/project", "feature")]);
    assert_eq!(
        newest_pr(format!("{first}\n{second}").as_bytes(), &checkout()).unwrap(),
        Some(125)
    );
}

#[test]
fn same_named_branch_in_another_fork_does_not_match() {
    let response = json!([
        pr(999, "2026-10-04T00:00:00Z", "bob/project", "feature"),
        pr(123, "2026-10-01T00:00:00Z", "alice/project", "feature")
    ]);
    assert_eq!(
        newest_pr(response.to_string().as_bytes(), &checkout()).unwrap(),
        Some(123)
    );
}

#[test]
fn another_branch_does_not_match() {
    let response = json!([pr(123, "2026-10-01T00:00:00Z", "alice/project", "other")]);
    assert_eq!(
        newest_pr(response.to_string().as_bytes(), &checkout()).unwrap(),
        None
    );
}

#[test]
fn empty_list_has_no_pr() {
    assert_eq!(newest_pr(b"[]", &checkout()).unwrap(), None);
}

#[test]
fn malformed_response_is_an_error() {
    assert!(newest_pr(b"{not-json}", &checkout()).is_err());
}

#[test]
fn empty_output_is_an_error() {
    assert!(newest_pr(b"", &checkout()).is_err());
}

#[test]
fn zero_pr_number_is_an_error() {
    let response = json!([pr(0, "2026-10-01T00:00:00Z", "alice/project", "feature")]);
    assert!(newest_pr(response.to_string().as_bytes(), &checkout()).is_err());
}

#[test]
fn ssh_and_https_remote_urls_resolve_the_same_repository() {
    assert_eq!(
        parse_remote("git@github.com:alice/project.git"),
        parse_remote("https://github.com/alice/project.git")
    );
    assert_eq!(
        parse_remote("ssh://git@github.com/alice/project.git"),
        parse_remote("https://github.com/alice/project")
    );
}

#[test]
fn local_paths_are_not_github_remotes() {
    assert_eq!(parse_remote("/tmp/project"), None);
}

#[test]
fn worktree_checkout_wins_over_pane_directory() {
    let snapshot: Snapshot = serde_json::from_value(json!({"workspaces":[{"workspace_id":"w1","active_tab_id":"t1","worktree":{"checkout_path":"/tmp/worktree"}}],"panes":[],"layouts":[]})).unwrap();
    assert_eq!(
        snapshot.directory(&snapshot.workspaces[0]),
        Some(std::path::Path::new("/tmp/worktree"))
    );
}

#[test]
fn active_tabs_focused_pane_supplies_foreground_directory() {
    let snapshot: Snapshot = serde_json::from_value(json!({"workspaces":[{"workspace_id":"w1","active_tab_id":"t1"}],"panes":[{"pane_id":"p1","workspace_id":"w1","tab_id":"t1","cwd":"/tmp/original","foreground_cwd":"/tmp/current"}],"layouts":[{"workspace_id":"w1","tab_id":"t1","focused_pane_id":"p1"}]})).unwrap();
    assert_eq!(
        snapshot.directory(&snapshot.workspaces[0]),
        Some(std::path::Path::new("/tmp/current"))
    );
}

#[test]
fn subprocess_timeout_is_bounded_including_inherited_pipes() {
    let started = Instant::now();
    let result = command::run(
        Command::new("sh").args(["-c", "sleep 30 & wait"]),
        Duration::from_millis(100),
    );
    assert!(result.unwrap_err().to_string().contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn subprocess_failure_preserves_error_context() {
    let result = command::run(
        Command::new("sh").args(["-c", "echo failure >&2; exit 7"]),
        Duration::from_secs(2),
    );
    assert!(result.unwrap_err().to_string().contains("failure"));
}

#[test]
fn cancelled_subprocess_reaps_descendants_holding_output_pipes() {
    let started = Instant::now();
    let result = command::run_cancellable(
        Command::new("sh").args(["-c", "sleep 30 & wait"]),
        Duration::from_secs(30),
        1024 * 1024,
        &std::sync::atomic::AtomicBool::new(true),
        "Herdr request",
    );
    assert_eq!(result.unwrap_err().to_string(), "Herdr request cancelled");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn subprocess_output_is_bounded() {
    let result = command::run_cancellable(
        Command::new("sh").args(["-c", "yes x"]),
        Duration::from_secs(2),
        1024 * 1024,
        &std::sync::atomic::AtomicBool::new(false),
        "Herdr request",
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("output exceeds 1 MiB")
    );
}
