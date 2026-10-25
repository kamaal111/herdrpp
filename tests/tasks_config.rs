use herdrpp::tasks::config::{Format, parse, parse_with_format, shell_command};
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
fn malformed_json5_has_line_column_and_object_requires_tasks() {
    let errors = parse("[\n{").unwrap_err().join("\n");
    assert!(errors.contains("line 2 column"));
    assert!(parse("{}").unwrap_err()[0].contains("tasks: expected an array"));
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

#[test]
fn formats_produce_the_same_task_with_schema_metadata() {
    let expected = parse(r#"[{"label":"test","command":"cargo test","new_tab":false}]"#).unwrap();
    for (format, text) in [
        (
            Format::Json5,
            "{ '$schema': '../schemas/tasks.schema.json', // comment\n tasks: [{label: 'test', command: 'cargo test', new_tab: false,},],}",
        ),
        (
            Format::Yaml,
            "$schema: ../schemas/tasks.schema.json\ntasks:\n  - label: test\n    command: cargo test\n    new_tab: false\n",
        ),
        (
            Format::Yaml,
            "- label: test\n  command: cargo test\n  new_tab: false\n",
        ),
        (
            Format::Toml,
            "\"$schema\" = '../schemas/tasks.schema.json'\n[[tasks]]\nlabel = 'test'\ncommand = 'cargo test'\nnew_tab = false\n",
        ),
    ] {
        assert_eq!(
            parse_with_format(text, format).unwrap(),
            expected,
            "{format:?}"
        );
    }
}

#[test]
fn json5_string_extensions_decode_without_changing_shell_literals() {
    let tasks = parse("\u{feff}[/* block */{l\\u0061bel: 'test', command: 'pri\\\nntf', args: ['\\x41', '\\v', '\\0x',],},]\u{a0}").unwrap_err().join("\n");
    assert!(tasks.contains("args[2]: NUL"));
    let tasks = parse("\u{feff}[/* block */{l\\u0061bel: 'test', command: 'pri\\\nntf', args: ['\\x41', '\\v',],},]\u{a0}").unwrap();
    assert_eq!(tasks[0].label, "test");
    assert_eq!(tasks[0].command, "printf");
    assert_eq!(tasks[0].args, ["A", "\u{b}"]);
}

#[test]
fn json5_numeric_extensions_receive_field_type_errors_without_coercion() {
    for number in [
        "0x10",
        "-0x10",
        "+1",
        ".5",
        "1.",
        "1e2",
        "NaN",
        "Infinity",
        "-Infinity",
        "18446744073709551616",
        "-9223372036854775809",
    ] {
        let text = format!("[{{label:'test', command:'true', args:[{number}], new_tab:{number}}}]");
        let errors = parse(&text).unwrap_err().join("\n");
        assert!(
            errors.contains("args[0]: expected a string, received number"),
            "{number}: {errors}"
        );
        assert!(
            errors.contains("new_tab: expected true or false, received number"),
            "{number}: {errors}"
        );
    }
}

#[test]
fn object_roots_reject_unknown_duplicate_missing_and_invalid_metadata() {
    for (text, expected) in [
        ("{tasks: [], task: []}", "unknown field"),
        ("{tasks: [], tasks: []}", "tasks: duplicate field"),
        (
            "{'$schema':'a', '$schema':'b', tasks:[]}",
            "$schema: duplicate field",
        ),
        ("{}", "tasks: expected an array"),
        ("{tasks:null}", "tasks: expected an array"),
        (
            "{tasks:[], '$schema':null}",
            "$schema: expected a nonblank string",
        ),
        ("{tasks:[], '$schema':'  '}", "enter visible text"),
        ("{tasks:[], '$schema':'\\u0000'}", "NUL"),
        (
            "[{label:'test', command:'true', command:'false'}]",
            "command: duplicate field",
        ),
    ] {
        assert!(
            parse(text).unwrap_err().join("\n").contains(expected),
            "{text}"
        );
    }
    assert!(parse("{tasks:[]}").unwrap().is_empty());
}

#[test]
fn yaml_and_toml_keep_validation_defaults_and_reject_invalid_fields() {
    for (format, valid, invalid) in [
        (
            Format::Yaml,
            "tasks: [{label: test, command: 'true'}]",
            "tasks: [{label: test, command: true, new_tab: 'false', args: [1], extra: 1}, {label: test, command: 'true'}]",
        ),
        (
            Format::Toml,
            "[[tasks]]\nlabel='test'\ncommand='true'",
            "[[tasks]]\nlabel='test'\ncommand=true\nnew_tab='false'\nargs=[1]\nextra=1\n[[tasks]]\nlabel='test'\ncommand='true'",
        ),
    ] {
        let tasks = parse_with_format(valid, format).unwrap();
        assert!(tasks[0].new_tab);
        assert!(tasks[0].args.is_empty());
        let errors = parse_with_format(invalid, format).unwrap_err().join("\n");
        for expected in [
            "command: expected a nonblank string",
            "Remove the quotes",
            "args[0]",
            "unknown field",
            "duplicates Task 1",
        ] {
            assert!(errors.contains(expected), "{format:?}: {errors}");
        }
    }
    assert!(
        parse_with_format("tasks = []", Format::Toml)
            .unwrap()
            .is_empty()
    );
    assert!(
        parse_with_format("tasks: []", Format::Yaml)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn malformed_formats_report_locations_and_yaml_rejects_multiple_documents() {
    for (format, text) in [
        (Format::Json5, "[\n{"),
        (Format::Json5, "[/* unterminated"),
        (Format::Json5, "[{label:'unterminated"),
        (Format::Yaml, "tasks: [\n{"),
        (Format::Toml, "[[tasks]\n"),
    ] {
        let errors = parse_with_format(text, format).unwrap_err().join("\n");
        assert!(errors.contains("line"), "{format:?}: {errors}");
        assert!(errors.contains("column"), "{format:?}: {errors}");
    }
    assert!(parse_with_format("tasks: []\n---\ntasks: []", Format::Yaml).is_err());
}

#[test]
fn yaml_and_toml_reject_duplicate_keys_and_wrong_root_types() {
    for (format, text) in [
        (Format::Yaml, "tasks: []\ntasks: []"),
        (
            Format::Yaml,
            "tasks: [{label: test, command: a, command: b}]",
        ),
        (Format::Toml, "tasks=[]\ntasks=[]"),
        (
            Format::Toml,
            "[[tasks]]\nlabel='test'\ncommand='a'\ncommand='b'",
        ),
        (Format::Toml, "tasks='test'"),
        (Format::Yaml, "tasks: test"),
    ] {
        assert!(
            parse_with_format(text, format).is_err(),
            "{format:?}: {text}"
        );
    }
}

#[test]
fn yaml_aliases_are_bounded_and_custom_tags_are_rejected() {
    let tasks = parse_with_format(
        "tasks: [{label: test, command: &cmd 'true', args: [*cmd]}]",
        Format::Yaml,
    )
    .unwrap();
    assert_eq!(tasks[0].args, ["true"]);
    assert!(parse_with_format("tasks: &tasks [*tasks]", Format::Yaml).is_err());
    assert!(parse_with_format("tasks: !include other.yaml", Format::Yaml).is_err());
    assert!(
        parse_with_format(
            "tasks: [{label: test, command: !custom 'true'}]",
            Format::Yaml
        )
        .is_err()
    );
}
