"""Validate editor diagnostics against the published task configuration schema."""
import json
from pathlib import Path

import jsonschema
import pytest

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "schemas/tasks.schema.json").read_text())
VALIDATOR = jsonschema.Draft7Validator(SCHEMA)


def test_schema_is_valid_and_repository_tasks_use_it():
    jsonschema.Draft7Validator.check_schema(SCHEMA)
    document = json.loads((ROOT / ".herdr/tasks.json").read_text())
    assert not list(VALIDATOR.iter_errors(document))
    assert (ROOT / ".herdr" / document["$schema"]).resolve() == ROOT / "schemas/tasks.schema.json"


@pytest.mark.parametrize("document", [
    [],
    {"tasks": []},
    [{"label": "test", "command": "cargo test", "new_tab": False}],
    {"$schema": "../schemas/tasks.schema.json", "tasks": [
        {"label": "test", "command": "cargo test", "args": ["", "a b", "\n"]},
    ]},
    [{"label": "世界", "command": "printf '\\n'", "args": []}],
    [{"label": "\ufeff", "command": "\ufeff"}],
])
def test_schema_accepts_arrays_and_normalized_json_yaml_toml_documents(document):
    assert not list(VALIDATOR.iter_errors(document))


@pytest.mark.parametrize("document", [
    {},
    {"tasks": "test"},
    {"tasks": [], "task": []},
    {"tasks": [], "$schema": None},
    {"tasks": [], "$schema": " "},
    {"tasks": [], "$schema": "\0"},
    [{"label": "test"}],
    [{"label": "test", "command": True}],
    [{"label": "test", "command": "true", "new_tab": "false"}],
    [{"label": "test", "command": "true", "args": [1]}],
    [{"label": "test", "command": "true", "extra": 1}],
    [{"label": "\u0085", "command": "true"}],
    [{"label": "bad\nlabel", "command": "true"}],
    [{"label": "test", "command": "\u2003"}],
    [{"label": "test", "command": "x\0"}],
    [{"label": "test", "command": "true", "args": ["\0"]}],
    [None],
])
def test_schema_rejects_invalid_root_fields_types_and_strings(document):
    assert list(VALIDATOR.iter_errors(document))
