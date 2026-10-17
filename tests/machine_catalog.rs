use herdrpp::machines::catalog::load;
use std::{fs, os::unix::fs::PermissionsExt, sync::Mutex};

static CLI_TEST_LOCK: Mutex<()> = Mutex::new(());

fn catalog(profile: serde_json::Value) -> anyhow::Result<Vec<herdrpp::machines::catalog::Profile>> {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("herdr");
    let json = serde_json::to_string(&vec![profile]).unwrap();
    fs::write(
        &executable,
        format!("#!/bin/sh\ncat <<'CATALOG_JSON'\n{json}\nCATALOG_JSON\n"),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    load(executable.as_os_str())
}

fn profile() -> serde_json::Value {
    serde_json::json!({
        "id": "opaque-profile-id",
        "label": "開発 🐈",
        "target": "alice@build.example",
        "session": "engineering",
        "enabled": true
    })
}

#[test]
fn unicode_labels_and_opaque_ids_are_preserved() {
    let profiles = catalog(profile()).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].label, "開発 🐈");
    assert_eq!(profiles[0].id, "opaque-profile-id");
}

#[test]
fn terminal_control_sequences_in_labels_are_rejected_before_rendering() {
    let mut value = profile();
    value["label"] = "Build\u{1b}]52;c;AAAA\u{7}".into();
    let error = catalog(value).unwrap_err();
    assert!(error.to_string().contains("invalid"), "{error:#}");
}

#[test]
fn control_characters_in_ssh_targets_are_rejected() {
    let mut value = profile();
    value["target"] = "alice@build\nother".into();
    let error = catalog(value).unwrap_err();
    assert!(error.to_string().contains("invalid"), "{error:#}");
}

#[test]
fn option_shaped_sessions_are_rejected_before_launching() {
    let mut value = profile();
    value["session"] = "--help".into();
    let error = catalog(value).unwrap_err();
    assert!(error.to_string().contains("invalid"), "{error:#}");
}

#[test]
fn control_characters_in_profile_ids_are_rejected() {
    let mut value = profile();
    value["id"] = "opaque\rprofile".into();
    let error = catalog(value).unwrap_err();
    assert!(error.to_string().contains("invalid"), "{error:#}");
}
