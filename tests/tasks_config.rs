use herdrpp::tasks::config::{parse, shell_command};
use std::path::Path;

#[test]
fn defaults_and_literal_arguments_preserve_shell_command_syntax() {
    let tasks = parse(r#"[{"label":"dev", "command":"printf '%s\\n'", "args":["", "a b", "it's", "$HOME", "x; false"]}]"#).unwrap();
    assert!(tasks[0].new_tab);
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &shell_command(&tasks[0], Path::new("/tmp"))])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "\na b\nit's\n$HOME\nx; false\n"
    );
    let defaults = parse(r#"[{"label":"test", "command":"true"}]"#).unwrap();
    assert!(defaults[0].args.is_empty());
}

#[test]
fn aggregates_actionable_field_errors_and_rejects_entire_file() {
    let errors = parse(
        r#"[
        {"label":"test", "command":"", "new_tab":"false", "arg":[]},
        {"label":"test", "args":[1, null], "new_tab":null},
        false
    ]"#,
    )
    .unwrap_err()
    .join("\n");
    assert!(errors.contains("Task 1 (\"test\"), command"));
    assert!(errors.contains("nonblank string"));
    assert!(errors.contains("Remove the quotes"));
    assert!(errors.contains("unknown field"));
    assert!(errors.contains("label, command, args, new_tab"));
    assert!(errors.contains("duplicates Task 1"));
    assert!(errors.contains("args[0]"));
    assert!(errors.contains("args[1]"));
    assert!(errors.contains("Task 3: expected an object"));
}

#[test]
fn duplicate_keys_are_not_silently_overwritten() {
    let errors =
        parse(r#"[{"label":"dev", "command":"true", "command":"false", "args":[], "args":[]}]"#)
            .unwrap_err()
            .join("\n");
    assert!(errors.contains("command: duplicate field"));
    assert!(errors.contains("args: duplicate field"));
}

#[test]
fn malformed_json_has_line_column_and_top_level_must_be_array() {
    let errors = parse("[\n{").unwrap_err().join("\n");
    assert!(errors.contains("line 2 column"));
    assert!(parse("{}").unwrap_err()[0].contains("expected a JSON array"));
}

#[test]
fn invalid_strings_and_nulls_are_rejected_without_coercion() {
    let errors = parse(r#"[{"label":"bad\nlabel", "command":"x\u0000", "args":["\u0000"]}, {"label":null,"command":4,"args":null,"new_tab":null}]"#).unwrap_err().join("\n");
    assert!(errors.contains("control characters"));
    assert!(errors.contains("NUL"));
    assert!(errors.contains("Task 2, label"));
    assert!(errors.contains("Task 2, args"));
    assert!(errors.contains("Task 2, new_tab"));
}

#[test]
fn explicit_false_and_empty_array_are_valid() {
    assert!(!parse(r#"[{"label":"test", "command":"true", "new_tab":false}]"#).unwrap()[0].new_tab);
    assert!(parse("[]").unwrap().is_empty());
}
