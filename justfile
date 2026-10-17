alias fmt := format
alias fmt-c := format-check

@_default:
    just --list --unsorted

build:
    cargo build --locked

build-release:
    cargo build --locked --release --target-dir target

format:
    cargo fmt --all

format-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

# Rust and isolated Python integration tests, without Docker.
test:
    cargo test --locked --all-features
    uv run --locked pytest -m 'not snapshot'

test-pty: build-release
    python3 tests/pty_smoke.py

quality: format-check lint

ready: quality test test-pty

# Full release verification, including focus, visuals, and machine switching.
ready-all: ready test-focus test-snapshots test-machines

test-focus:
    sh tests/snapshots/run.sh --focus-tests

test-snapshots:
    sh tests/snapshots/run.sh --self-test
    sh tests/snapshots/run.sh --repeat 2
    uv run --locked pytest -m snapshot --repeat 2

record-picker-snapshots *cases:
    sh tests/snapshots/run.sh --record {{cases}}

record-sidebar-snapshots:
    uv run --locked pytest -m snapshot --record

plugin-link: build-release
    "${HERDR_BIN_PATH:-herdr}" plugin link "$PWD"

plugin-check: build-release
    ./target/release/herdrpp check

plugin-check-pr: build-release
    ./target/release/herdrpp check-pr

plugin-open: build-release
    "${HERDR_BIN_PATH:-herdr}" plugin action invoke herdrpp.open

plugin-start: build-release
    "${HERDR_BIN_PATH:-herdr}" plugin action invoke herdrpp.start

plugin-refresh: build-release
    "${HERDR_BIN_PATH:-herdr}" plugin action invoke herdrpp.refresh

plugin-stop:
    "${HERDR_BIN_PATH:-herdr}" plugin action invoke herdrpp.stop

plugin-unlink:
    "${HERDR_BIN_PATH:-herdr}" plugin unlink herdrpp

# Real SSH and rendered graphics gates for the production launcher (Docker required).
test-machines:
    sh tests/machines/run.sh all
