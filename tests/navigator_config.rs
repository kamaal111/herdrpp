use herdrpp::machines::navigator_config::{ConfigMirror, ControlBinding};
use std::fs;

fn controls() -> Vec<ControlBinding> {
    vec![ControlBinding {
        action: "goto".into(),
        key: "ctrl+f12".into(),
        defaults: vec!["prefix+g".into()],
    }]
}

#[test]
fn creating_a_private_binding_preserves_disabled_original_and_comments() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    let original = "# user's config\n[keys]\ngoto = []\n[ui]\nmouse_capture = false\n";
    fs::write(&user, original).unwrap();
    let mut mirror = ConfigMirror::create(
        &user,
        &private,
        controls(),
        &directory.path().join("shared-state"),
    )
    .unwrap();
    assert!(fs::read_to_string(&private).unwrap().contains("ctrl+f12"));
    assert_eq!(fs::read_to_string(&user).unwrap(), original);
    assert!(!mirror.synchronize().unwrap());
}

#[test]
fn native_settings_and_external_editor_merge_disjoint_changes_without_private_keys() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    let original = "# preserve me\n[keys]\ngoto = 'prefix+g'\n[ui]\nmouse_capture = false\nsidebar_width = 30\n";
    fs::write(&user, original).unwrap();
    let mut mirror = ConfigMirror::create(
        &user,
        &private,
        controls(),
        &directory.path().join("shared-state"),
    )
    .unwrap();
    fs::write(
        &private,
        fs::read_to_string(&private)
            .unwrap()
            .replace("sidebar_width = 30", "sidebar_width = 36"),
    )
    .unwrap();
    fs::write(
        &user,
        original.replace("mouse_capture = false", "mouse_capture = true"),
    )
    .unwrap();
    assert!(mirror.synchronize().unwrap());
    let merged = fs::read_to_string(&user).unwrap();
    assert!(merged.contains("# preserve me"));
    assert!(merged.contains("mouse_capture = true"));
    assert!(merged.contains("sidebar_width = 36"));
    assert!(!merged.contains("ctrl+f12"));
    assert!(fs::read_to_string(&private).unwrap().contains("ctrl+f12"));
    assert!(!mirror.synchronize().unwrap());
}

#[test]
fn conflicting_settings_edits_preserve_both_files_and_report_the_key() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    fs::write(&user, "[ui]\nsidebar_width = 30\n").unwrap();
    let mut mirror = ConfigMirror::create(
        &user,
        &private,
        controls(),
        &directory.path().join("shared-state"),
    )
    .unwrap();
    let pending = fs::read_to_string(&private)
        .unwrap()
        .replace("sidebar_width = 30", "sidebar_width = 34");
    fs::write(&private, &pending).unwrap();
    let edited = "[ui]\nsidebar_width = 36\n";
    fs::write(&user, edited).unwrap();
    let error = mirror.synchronize().unwrap_err();
    assert!(error.to_string().contains("ui.sidebar_width"));
    assert_eq!(fs::read_to_string(&user).unwrap(), edited);
    assert_eq!(fs::read_to_string(&private).unwrap(), pending);
}

#[test]
fn external_reload_preserves_new_user_bindings_and_native_settings_change_to_goto() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    fs::write(&user, "[keys]\ngoto = 'prefix+g'\n").unwrap();
    let mut mirror = ConfigMirror::create(
        &user,
        &private,
        controls(),
        &directory.path().join("shared-state"),
    )
    .unwrap();
    fs::write(&user, "[keys]\ngoto = ['prefix+g', 'ctrl+k']\n").unwrap();
    assert!(mirror.synchronize().unwrap());
    fs::write(
        &private,
        fs::read_to_string(&private)
            .unwrap()
            .replace("ctrl+k", "ctrl+l"),
    )
    .unwrap();
    assert!(mirror.synchronize().unwrap());
    let merged = fs::read_to_string(&user).unwrap();
    assert!(merged.contains("ctrl+l"));
    assert!(!merged.contains("ctrl+k"));
    assert!(!merged.contains("ctrl+f12"));
}

#[test]
fn omitted_goto_keeps_effective_default_and_settings_can_explicitly_disable_it() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    fs::write(&user, "# implicit defaults\n").unwrap();
    let mut mirror = ConfigMirror::create(
        &user,
        &private,
        controls(),
        &directory.path().join("shared-state"),
    )
    .unwrap();
    let injected = fs::read_to_string(&private).unwrap();
    assert!(injected.contains("prefix+g"));
    fs::write(
        &private,
        injected.replace(r#"["prefix+g", "ctrl+f12"]"#, "[]"),
    )
    .unwrap();
    assert!(mirror.synchronize().unwrap());
    assert!(fs::read_to_string(&user).unwrap().contains("goto = []"));
}

#[test]
fn private_binding_collisions_with_other_actions_and_custom_commands_fail_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    fs::write(&user, "[keys]\nnew_tab = 'F12+Control'\n").unwrap();
    assert!(
        ConfigMirror::create(
            &user,
            &private,
            controls(),
            &directory.path().join("shared-state")
        )
        .is_err()
    );
    assert!(!private.exists());
    fs::write(
        &user,
        "[[keys.command]]\nkey = 'ctrl+f12'\ncommand = 'echo preserved'\n",
    )
    .unwrap();
    assert!(
        ConfigMirror::create(
            &user,
            &private,
            controls(),
            &directory.path().join("shared-state")
        )
        .is_err()
    );
    assert!(!private.exists());
}

#[test]
fn inline_keys_table_preserves_custom_goto_and_merges_native_settings_edits() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    fs::write(
        &user,
        "keys = { goto = 'ctrl+k' }\nui = { mouse_capture = false }\n",
    )
    .unwrap();
    let mut mirror = ConfigMirror::create(
        &user,
        &private,
        controls(),
        &directory.path().join("shared-state"),
    )
    .unwrap();
    fs::write(
        &private,
        fs::read_to_string(&private)
            .unwrap()
            .replace("mouse_capture = false", "mouse_capture = true"),
    )
    .unwrap();
    assert!(mirror.synchronize().unwrap());
    let original = fs::read_to_string(&user).unwrap();
    assert!(original.contains("ctrl+k"));
    assert!(original.contains("mouse_capture = true"));
    assert!(!original.contains("ctrl+f12"));
}

#[test]
fn published_default_bindings_are_read_within_the_keys_section_and_validate_types() {
    use herdrpp::machines::navigator_config::default_binding;
    let template = "[keys]\n# goto = \"prefix+g\"\n# reload_config = [\"prefix+shift+r\", \"alt+r\"]\n[other]\ngoto = \"ignored\"\n";
    assert_eq!(default_binding(template, "goto").unwrap(), vec!["prefix+g"]);
    assert_eq!(
        default_binding(template, "reload_config").unwrap(),
        vec!["prefix+shift+r", "alt+r"]
    );
    assert!(default_binding("[other]\n# goto = \"bad\"", "goto").is_err());
    assert!(default_binding("[keys]\n# goto = 3", "goto").is_err());
}

#[test]
fn native_settings_create_a_missing_config_target_without_replacing_its_symlink() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("missing/config.toml");
    let user = directory.path().join("user.toml");
    let private = directory.path().join("private.toml");
    std::os::unix::fs::symlink(&target, &user).unwrap();
    let mut mirror = ConfigMirror::create(
        &user,
        &private,
        controls(),
        &directory.path().join("shared-state"),
    )
    .unwrap();
    let edited = fs::read_to_string(&private).unwrap() + "\n[ui]\nmouse_capture = false\n";
    fs::write(&private, edited).unwrap();
    assert!(mirror.synchronize().unwrap());
    assert!(
        fs::symlink_metadata(&user)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("mouse_capture = false")
    );
    assert!(!fs::read_to_string(&target).unwrap().contains("ctrl+f12"));
}

#[test]
fn literal_reserved_backslash_and_duplicate_control_modifiers_cannot_shadow_the_controller() {
    for (occupied, action, reserved) in [
        (r"ctrl+\", "reload_config", "ctrl+backslash"),
        ("CONTROL+ctrl+BACKSLASH", "reload_config", "ctrl+backslash"),
        ("control + ctrl + ]", "goto", "ctrl+]"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let user = directory.path().join("user.toml");
        let private = directory.path().join("private.toml");
        let original = format!("[keys]\nclose_workspace = '{occupied}'\n");
        fs::write(&user, &original).unwrap();
        let binding = ControlBinding {
            action: action.into(),
            key: reserved.into(),
            defaults: vec![],
        };
        assert!(
            ConfigMirror::create(
                &user,
                &private,
                vec![binding],
                &directory.path().join("state")
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&user).unwrap(), original);
        assert!(!private.exists());
    }
}
