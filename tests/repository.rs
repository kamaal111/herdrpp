use herdrpp::repository::{parse_remote, repository_url};
use std::{path::Path, process::Command, sync::atomic::AtomicBool};

fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
            .status
            .success()
    );
}

fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q", "-b", "feature"]);
    root
}

fn url(root: &Path) -> Option<String> {
    repository_url(root, &AtomicBool::new(false)).unwrap()
}

#[test]
fn normalizes_browser_urls_across_hosts_and_transports() {
    for (remote, expected) in [
        (
            "git@github.com:kamaal111/herdrpp.git",
            "https://github.com/kamaal111/herdrpp",
        ),
        (
            "ssh://git@gitlab.com:2222/team/subgroup/app.git/",
            "https://gitlab.com/team/subgroup/app",
        ),
        (
            "http://user:secret@git.example:8080/team/app.git",
            "http://git.example:8080/team/app",
        ),
        (
            "https://git.example/team/app/",
            "https://git.example/team/app",
        ),
    ] {
        assert_eq!(parse_remote(remote).unwrap().url(), expected);
    }
}

#[test]
fn rejects_local_and_malformed_remotes() {
    for remote in [
        "/tmp/team/app.git",
        "../app.git",
        "file:///tmp/app.git",
        "https:///team/app",
        "https://host/team//app",
        "https://host/team/app?token=secret",
        "host:../app",
        "https://host/team/app#readme",
    ] {
        assert!(parse_remote(remote).is_none(), "{remote}");
    }
}

#[test]
fn sole_remote_needs_no_pr_or_commit_and_duplicates_are_not_ambiguous() {
    let root = project();
    git(
        root.path(),
        &[
            "remote",
            "add",
            "company",
            "git@github.com:kamaal111/herdrpp.git",
        ],
    );
    git(
        root.path(),
        &[
            "remote",
            "add",
            "alias",
            "https://github.com/kamaal111/herdrpp.git",
        ],
    );
    assert_eq!(
        url(root.path()).as_deref(),
        Some("https://github.com/kamaal111/herdrpp")
    );
}

#[test]
fn tracking_remote_wins_over_other_candidates() {
    let root = project();
    git(
        root.path(),
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/team/app.git",
        ],
    );
    git(
        root.path(),
        &[
            "remote",
            "add",
            "fork",
            "https://gitlab.com/team/subgroup/app.git",
        ],
    );
    assert_eq!(url(root.path()), None);
    git(root.path(), &["config", "branch.feature.remote", "fork"]);
    assert_eq!(
        url(root.path()).as_deref(),
        Some("https://gitlab.com/team/subgroup/app")
    );
}

#[test]
fn explicit_ineligible_tracking_remote_prevents_fallback() {
    let root = project();
    git(
        root.path(),
        &["remote", "add", "origin", "https://github.com/team/app.git"],
    );
    for tracking in [".", "missing"] {
        git(root.path(), &["config", "branch.feature.remote", tracking]);
        assert_eq!(url(root.path()), None);
    }
    git(root.path(), &["remote", "add", "local", "/tmp/app.git"]);
    git(root.path(), &["config", "branch.feature.remote", "local"]);
    assert_eq!(url(root.path()), None);
}

#[test]
fn detached_head_uses_sole_remote() {
    let root = project();
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
    git(root.path(), &["checkout", "--detach", "-q"]);
    git(
        root.path(),
        &["remote", "add", "origin", "https://github.com/team/app.git"],
    );
    assert_eq!(
        url(root.path()).as_deref(),
        Some("https://github.com/team/app")
    );
}

#[test]
fn unavailable_directories_and_local_only_repositories_have_no_url() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(url(root.path()), None);
    assert_eq!(url(&root.path().join("missing")), None);
    assert_eq!(url(Path::new("relative")), None);
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["remote", "add", "local", "/tmp/app.git"]);
    assert_eq!(url(root.path()), None);
}

#[test]
fn cancellation_prevents_git_discovery() {
    let root = project();
    assert!(repository_url(root.path(), &AtomicBool::new(true)).is_err());
}
