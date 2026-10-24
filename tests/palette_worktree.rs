use herdrpp::palette::worktree::{self, Branch};
use herdrpp::palette::{
    app::{Action, App, Mode},
    catalog::Context,
    cli::View,
    naming::Naming,
};
use serde_json::json;
use std::{path::Path, process::Command, sync::atomic::AtomicBool};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q", "-b", "feature/palette"]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    );
    root
}

#[test]
fn local_branches_win_and_gitbutler_workspace_is_excluded() {
    let root = project();
    let sha = git(root.path(), &["rev-parse", "HEAD"]);
    git(root.path(), &["branch", "gitbutler/workspace"]);
    git(
        root.path(),
        &["update-ref", "refs/remotes/origin/feature/palette", &sha],
    );
    git(
        root.path(),
        &["update-ref", "refs/remotes/origin/feature/report", &sha],
    );
    git(
        root.path(),
        &["update-ref", "refs/remotes/company/feature/report", &sha],
    );
    git(
        root.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/feature/palette",
        ],
    );
    let branches = worktree::branches(root.path(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        branches.iter().map(Branch::label).collect::<Vec<_>>(),
        vec![
            "company/feature/report",
            "feature/palette",
            "origin/feature/report"
        ]
    );
}

#[test]
fn new_branch_names_reject_existing_invalid_and_previous_checkout_names() {
    let root = project();
    assert!(worktree::validate_name(root.path(), "feature/new").is_ok());
    assert!(worktree::validate_name(root.path(), "feature/palette").is_err());
    assert!(worktree::validate_name(root.path(), " ").is_err());
    assert!(worktree::validate_name(root.path(), "bad name").is_err());
    assert!(worktree::validate_name(root.path(), "@{-1}").is_err());
    assert!(worktree::validate_name(root.path(), "gitbutler/workspace").is_err());
}

#[test]
fn fetch_discovers_new_remote_branches_and_keeps_refs_after_failure() {
    let root = project();
    let remote = project();
    git(remote.path(), &["branch", "feature/report"]);
    git(
        root.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    assert!(
        worktree::fetch(root.path(), &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
    assert!(
        worktree::branches(root.path(), &AtomicBool::new(false))
            .unwrap()
            .iter()
            .any(|branch| branch.label() == "origin/feature/report")
    );
    git(
        root.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "/nonexistent/herdrpp-test-remote",
        ],
    );
    assert!(
        worktree::fetch(root.path(), &AtomicBool::new(false))
            .unwrap()
            .contains("origin")
    );
    assert!(
        worktree::branches(root.path(), &AtomicBool::new(false))
            .unwrap()
            .iter()
            .any(|branch| branch.label() == "origin/feature/report")
    );
}

#[test]
fn branch_search_uses_the_palette_matcher_and_highlights() {
    let root = project();
    git(root.path(), &["branch", "feature/report"]);
    let branches = worktree::branches(root.path(), &AtomicBool::new(false)).unwrap();
    let items = worktree::branch_items(root.path(), &branches, true, Some("feature/report"));
    assert_eq!(items[0].title, "feature/report");
    let hits = herdrpp::palette::catalog::search(&items, "FEAT REPORT", &mut Default::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(items[hits[0].item].title, "feature/report");
    assert!(!hits[0].matched.indices[3].is_empty());
}

fn local_branch(root: &Path) -> Branch {
    worktree::branches(root, &AtomicBool::new(false))
        .unwrap()
        .remove(0)
}

#[test]
fn native_requests_keep_source_workspace_and_reuse_existing_checkouts() {
    let root = project();
    let branch = local_branch(root.path());
    let context = Context {
        workspace_id: Some("w2".into()),
        ..Context::default()
    };
    assert_eq!(
        worktree::request_for(
            root.path(),
            &branch,
            Some("feature/new"),
            &context,
            &json!({"worktrees":[]})
        )
        .unwrap(),
        (
            "worktree.create".into(),
            json!({"workspace_id":"w2","branch":"feature/new","base":"refs/heads/feature/palette","focus":true})
        )
    );
    assert_eq!(
        worktree::request_for(
            root.path(),
            &branch,
            None,
            &context,
            &json!({"worktrees":[{"branch":"feature/palette"}]})
        )
        .unwrap(),
        (
            "worktree.open".into(),
            json!({"workspace_id":"w2","branch":"feature/palette","focus":true})
        )
    );
    assert_eq!(
        worktree::request_for(
            root.path(),
            &branch,
            None,
            &context,
            &json!({"worktrees":[]})
        )
        .unwrap(),
        (
            "worktree.create".into(),
            json!({"workspace_id":"w2","branch":"feature/palette","base":null,"focus":true})
        )
    );
}

#[test]
fn remote_selection_rechecks_local_precedence_before_creating() {
    let root = project();
    let sha = git(root.path(), &["rev-parse", "HEAD"]);
    git(
        root.path(),
        &["update-ref", "refs/remotes/origin/feature/report", &sha],
    );
    let branch = worktree::branches(root.path(), &AtomicBool::new(false))
        .unwrap()
        .pop()
        .unwrap();
    let context = Context {
        workspace_id: Some("w2".into()),
        ..Context::default()
    };
    assert_eq!(
        worktree::request_for(
            root.path(),
            &branch,
            None,
            &context,
            &json!({"worktrees":[]})
        )
        .unwrap()
        .1["base"],
        "refs/remotes/origin/feature/report"
    );
    git(root.path(), &["branch", "feature/report"]);
    assert_eq!(
        worktree::request_for(
            root.path(),
            &branch,
            None,
            &context,
            &json!({"worktrees":[]})
        )
        .unwrap()
        .1["base"],
        serde_json::Value::Null
    );
}

#[test]
fn disappeared_refs_and_unborn_repositories_are_rejected() {
    let root = project();
    git(root.path(), &["branch", "feature/report"]);
    let branch = worktree::branches(root.path(), &AtomicBool::new(false))
        .unwrap()
        .pop()
        .unwrap();
    git(root.path(), &["branch", "-D", "feature/report"]);
    assert!(
        worktree::request_for(
            root.path(),
            &branch,
            None,
            &Context::default(),
            &json!({"worktrees":[]})
        )
        .is_err()
    );
    let unborn = tempfile::tempdir().unwrap();
    git(unborn.path(), &["init", "-q"]);
    assert!(
        worktree::branches(unborn.path(), &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("no commits")
    );
}

#[test]
fn base_chooser_prompts_for_name_validates_and_returns_to_selected_base() {
    use terminal_input::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let root = project();
    let items = worktree::branch_items(
        root.path(),
        &[local_branch(root.path())],
        true,
        Some("feature/palette"),
    );
    let mut app = App::new(
        Context::default(),
        items,
        Naming::default(),
        Default::default(),
        None,
    );
    app.view = View::BaseBranches {
        root: root.path().into(),
        invocation_branch: "feature/palette".into(),
    };
    assert_eq!(app.selected, Some(0));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Action::Continue
    );
    assert!(matches!(app.mode, Mode::Name { .. }));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Action::Continue
    );
    assert!(app.prompt_error.is_some());
    app.handle(Event::Paste("feature/new".into()));
    assert!(app.prompt_error.is_none());
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Action::Execute(herdrpp::palette::app::Selection {
            id: "worktree:refs/heads/feature/palette".into(),
            input: Some("feature/new".into()),
            confirmed: false,
        })
    );
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
        Action::Continue
    );
    assert_eq!(app.mode, Mode::Search);
    assert_eq!(app.selected, Some(0));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        ))),
        Action::Abort
    );
}

#[test]
fn gitbutler_and_detached_head_use_shared_remote_default_fallback() {
    let root = project();
    let remote = project();
    git(remote.path(), &["branch", "-m", "trunk"]);
    git(
        root.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(root.path(), &["fetch", "-q", "origin"]);
    git(
        root.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    git(root.path(), &["branch", "-m", "gitbutler/workspace"]);
    let (items, warning) = worktree::load(
        root.path(),
        true,
        Some("gitbutler/workspace"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(items[0].title, "origin/trunk");
    assert!(warning.is_none());
    assert_eq!(
        herdrpp::repository::effective_branch("gitbutler/workspace", "trunk"),
        "trunk"
    );
    assert_eq!(
        herdrpp::pr::github::default_branch(root.path(), &AtomicBool::new(false)).unwrap(),
        "trunk"
    );
    git(root.path(), &["checkout", "--detach", "-q"]);
    assert_eq!(
        worktree::load(root.path(), true, Some(""), &AtomicBool::new(false))
            .unwrap()
            .0[0]
            .title,
        "origin/trunk"
    );
}

#[test]
fn invocation_branch_is_pinned_even_after_checkout_changes() {
    let root = project();
    git(root.path(), &["checkout", "-qb", "aaa"]);
    let (items, warning) = worktree::load(
        root.path(),
        true,
        Some("feature/palette"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(items[0].title, "feature/palette");
    assert!(warning.is_none());
}
