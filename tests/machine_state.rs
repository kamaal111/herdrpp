use herdrpp::machines::state::StateMirror;
use std::fs;

fn write(root: &std::path::Path, relative: &str, value: &str) {
    let path = root.join("herdr").join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, value).unwrap();
}

#[test]
fn remembered_remote_selection_and_profiles_do_not_change_the_owned_attachment() {
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "client/endpoint-selection.json",
        r#"{"version":1,"selected_profile":"old-remote"}"#,
    );
    write(
        directory.path(),
        "client/endpoints.json",
        r#"{"version":1,"ssh":[{"id":"old-remote"}]}"#,
    );
    let mut state = StateMirror::create(directory.path()).unwrap();
    assert_eq!(
        fs::read(
            state
                .private_home()
                .join("herdr/client/endpoint-selection.json")
        )
        .unwrap(),
        br#"{"selected_profile":null,"version":1}"#
    );
    assert_eq!(
        fs::read(state.private_home().join("herdr/client/endpoints.json")).unwrap(),
        br#"{"version":1,"ssh":[]}"#
    );
    assert!(!state.synchronize().unwrap());
    assert_eq!(
        fs::read_to_string(
            directory
                .path()
                .join("herdr/client/endpoint-selection.json")
        )
        .unwrap(),
        r#"{"version":1,"selected_profile":"old-remote"}"#
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("herdr/client/endpoints.json")).unwrap(),
        r#"{"version":1,"ssh":[{"id":"old-remote"}]}"#
    );
}

#[test]
fn external_profiles_remain_private_to_the_picker_while_native_preferences_are_saved() {
    let directory = tempfile::tempdir().unwrap();
    let mut state = StateMirror::create(directory.path()).unwrap();
    write(
        directory.path(),
        "client/endpoints.json",
        r#"{"version":1,"ssh":[{"id":"new-profile"}]}"#,
    );
    write(
        state.private_home(),
        "client-shell/local.json",
        r#"{"sidebar_collapsed":true}"#,
    );
    assert!(state.synchronize().unwrap());
    assert_eq!(
        fs::read(state.private_home().join("herdr/client/endpoints.json")).unwrap(),
        br#"{"version":1,"ssh":[]}"#
    );
    assert_eq!(
        fs::read(directory.path().join("herdr/client/endpoints.json")).unwrap(),
        br#"{"version":1,"ssh":[{"id":"new-profile"}]}"#
    );
    assert_eq!(
        fs::read(directory.path().join("herdr/client-shell/local.json")).unwrap(),
        br#"{"sidebar_collapsed":true}"#
    );
}

#[test]
fn conflicting_preference_changes_preserve_both_copies_and_private_recovery_directory() {
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), "client-shell/local.json", "{}");
    let mut state = StateMirror::create(directory.path()).unwrap();
    let private = state.private_home().to_owned();
    write(
        directory.path(),
        "client-shell/local.json",
        r#"{"external":1}"#,
    );
    write(&private, "client-shell/local.json", r#"{"native":2}"#);
    let error = state.synchronize().unwrap_err();
    assert!(error.to_string().contains("private client state preserved"));
    drop(state);
    assert_eq!(
        fs::read(directory.path().join("herdr/client-shell/local.json")).unwrap(),
        br#"{"external":1}"#
    );
    assert_eq!(
        fs::read(private.join("herdr/client-shell/local.json")).unwrap(),
        br#"{"native":2}"#
    );
    fs::remove_dir_all(private).unwrap();
}

#[test]
fn native_preference_changes_are_saved_when_launcher_exits() {
    let directory = tempfile::tempdir().unwrap();
    let state = StateMirror::create(directory.path()).unwrap();
    let private = state.private_home().to_owned();
    write(
        &private,
        "client-shell/local.json",
        r#"{"sidebar_width":36}"#,
    );
    drop(state);
    assert_eq!(
        fs::read(directory.path().join("herdr/client-shell/local.json")).unwrap(),
        br#"{"sidebar_width":36}"#
    );
    assert!(!private.exists());
}

#[test]
fn preference_symlinks_remain_links_after_native_settings_changes() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("saved.json");
    fs::write(&target, "{}").unwrap();
    fs::create_dir_all(directory.path().join("herdr/client-shell")).unwrap();
    std::os::unix::fs::symlink(
        &target,
        directory.path().join("herdr/client-shell/local.json"),
    )
    .unwrap();
    let mut state = StateMirror::create(directory.path()).unwrap();
    write(
        state.private_home(),
        "client-shell/local.json",
        r#"{"sidebar_width":36}"#,
    );
    assert!(state.synchronize().unwrap());
    assert!(
        directory
            .path()
            .join("herdr/client-shell/local.json")
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(target).unwrap(),
        r#"{"sidebar_width":36}"#
    );
}

#[test]
fn adding_native_profiles_to_owned_frontend_fails_without_changing_global_catalog() {
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "client/endpoints.json",
        r#"{"version":1,"ssh":[{"id":"original"}]}"#,
    );
    let mut state = StateMirror::create(directory.path()).unwrap();
    let private = state.private_home().to_owned();
    write(
        &private,
        "client/endpoints.json",
        r#"{"version":1,"ssh":[{"id":"native-addition"}]}"#,
    );
    let error = state.synchronize().unwrap_err();
    assert!(format!("{error:#}").contains("owned viewing client gained machine profiles"));
    drop(state);
    assert_eq!(
        fs::read(directory.path().join("herdr/client/endpoints.json")).unwrap(),
        br#"{"version":1,"ssh":[{"id":"original"}]}"#
    );
    assert!(private.exists());
    fs::remove_dir_all(private).unwrap();
}
