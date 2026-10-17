use herdrpp::machines::navigator_state::StateMirror;
use std::fs;

#[test]
fn inspection_restores_private_chrome_without_overwriting_a_competing_clients_edit() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user/herdr");
    let private = directory.path().join("private/herdr");
    fs::create_dir_all(user.join("client-shell")).unwrap();
    fs::write(
        user.join("client-shell/local.json"),
        r#"{"sidebar_collapsed":true}"#,
    )
    .unwrap();
    let mut mirror = StateMirror::create(&user, &private).unwrap();
    mirror.begin_inspection().unwrap();
    fs::write(
        private.join("client-shell/local.json"),
        r#"{"sidebar_collapsed":false}"#,
    )
    .unwrap();
    fs::write(
        user.join("client-shell/local.json"),
        r#"{"sidebar_collapsed":true,"sidebar_width":32}"#,
    )
    .unwrap();
    assert!(!mirror.synchronize().unwrap());
    mirror.end_inspection().unwrap();
    assert_eq!(
        fs::read(user.join("client-shell/local.json")).unwrap(),
        fs::read(private.join("client-shell/local.json")).unwrap()
    );
    assert!(
        fs::read_to_string(user.join("client-shell/local.json"))
            .unwrap()
            .contains("32")
    );
}

#[test]
fn native_user_chrome_changes_and_external_machine_catalog_changes_are_persisted() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user/herdr");
    let private = directory.path().join("private/herdr");
    let mut mirror = StateMirror::create(&user, &private).unwrap();
    fs::create_dir_all(private.join("client-shell")).unwrap();
    fs::write(
        private.join("client-shell/local.json"),
        r#"{"sidebar_collapsed":true}"#,
    )
    .unwrap();
    assert!(mirror.synchronize().unwrap());
    fs::create_dir_all(user.join("client")).unwrap();
    fs::write(
        user.join("client/endpoints.json"),
        r#"{"version":1,"ssh":[]}"#,
    )
    .unwrap();
    assert!(mirror.synchronize().unwrap());
    assert_eq!(
        fs::read(private.join("client/endpoints.json")).unwrap(),
        fs::read(user.join("client/endpoints.json")).unwrap()
    );
    assert!(user.join("client-shell/local.json").exists());
}

#[test]
fn competing_machine_catalog_edits_are_rejected_without_overwriting_either_copy() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user/herdr");
    let private = directory.path().join("private/herdr");
    fs::create_dir_all(user.join("client")).unwrap();
    fs::write(user.join("client/endpoints.json"), "{}").unwrap();
    let mut mirror = StateMirror::create(&user, &private).unwrap();
    fs::write(user.join("client/endpoints.json"), r#"{"user":1}"#).unwrap();
    fs::write(private.join("client/endpoints.json"), r#"{"private":1}"#).unwrap();
    assert!(
        mirror
            .synchronize()
            .unwrap_err()
            .to_string()
            .contains("client/endpoints.json")
    );
    assert_eq!(
        fs::read_to_string(user.join("client/endpoints.json")).unwrap(),
        r#"{"user":1}"#
    );
    assert_eq!(
        fs::read_to_string(private.join("client/endpoints.json")).unwrap(),
        r#"{"private":1}"#
    );
}

#[test]
fn native_preferences_follow_symlinks_without_replacing_the_shared_link() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user/herdr");
    let private = directory.path().join("private/herdr");
    let target = directory.path().join("preferences.json");
    fs::create_dir_all(user.join("client-shell")).unwrap();
    fs::write(&target, r#"{"sidebar_width":30}"#).unwrap();
    let link = user.join("client-shell/local.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let mut mirror = StateMirror::create(&user, &private).unwrap();
    fs::write(
        private.join("client-shell/local.json"),
        r#"{"sidebar_width":42}"#,
    )
    .unwrap();
    assert!(mirror.synchronize().unwrap());
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        r#"{"sidebar_width":42}"#
    );
    assert_eq!(
        fs::read(&link).unwrap(),
        fs::read(private.join("client-shell/local.json")).unwrap()
    );
}

#[test]
fn inspection_restores_alias_catalog_while_preserving_new_external_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user/herdr");
    let private = directory.path().join("private/herdr");
    fs::create_dir_all(user.join("client")).unwrap();
    fs::write(
        user.join("client/endpoints.json"),
        r#"{"version":1,"ssh":[{"id":"alpha","label":"Original"}]}"#,
    )
    .unwrap();
    let mut mirror = StateMirror::create(&user, &private).unwrap();
    mirror.begin_inspection().unwrap();
    fs::write(
        private.join("client/endpoints.json"),
        r#"{"version":1,"ssh":[{"id":"alpha","label":"PrivateAlias"}]}"#,
    )
    .unwrap();
    let external =
        r#"{"version":1,"ssh":[{"id":"alpha","label":"Original"},{"id":"beta","label":"New"}]}"#;
    fs::write(user.join("client/endpoints.json"), external).unwrap();
    mirror.end_inspection().unwrap();
    assert_eq!(
        fs::read_to_string(user.join("client/endpoints.json")).unwrap(),
        external
    );
    assert_eq!(
        fs::read_to_string(private.join("client/endpoints.json")).unwrap(),
        external
    );
}
