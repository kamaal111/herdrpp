#!/usr/bin/env bash
set -euo pipefail

mode=combined
dry_run=0
for argument in "$@"; do
  case "$argument" in
    --plugin-only|--config-only)
      if [[ "$mode" != combined ]]; then
        printf 'Error: choose only one installation mode.\n' >&2
        exit 1
      fi
      mode="${argument#--}"
      ;;
    --dry-run) dry_run=1 ;;
    --help|-h)
      printf '%s\n' 'Usage: install.sh [--plugin-only | --config-only] [--dry-run]' \
        'Default: install the latest release through Herdr and initialize local sidebar tokens and the picker shortcut.' \
        '--plugin-only: install on the server without changing viewing config.' \
        '--config-only: configure the viewing computer without registering a plugin.' \
        'Requires curl, Python 3, and Rust/Cargo; plugin installation also needs Herdr, git, and gh.' \
        'Honors HERDR_BIN_PATH, HERDR_CONFIG_PATH, and XDG_STATE_HOME.'
      exit 0
      ;;
    *) printf 'Error: unknown option: %s\n' "$argument" >&2; exit 1 ;;
  esac
done

if [[ "$dry_run" == 1 ]]; then
  printf 'Would resolve the latest published release of kamaal111/herdrpp.\n'
  if [[ "$mode" != config-only ]]; then
    printf 'Would install that exact tag through Herdr and start it if a session is running.\n'
  fi
  if [[ "$mode" != plugin-only ]]; then
    printf 'Would apply pending config migrations to %s; completed migrations leave it untouched.\n' \
      "${HERDR_CONFIG_PATH:-$HOME/.config/herdr/config.toml}"
  fi
  exit 0
fi

case "$(uname -s)" in
  Darwin|Linux) ;;
  *) printf 'Error: this installer supports macOS and Linux.\n' >&2; exit 1 ;;
esac
required=(curl python3 cargo)
herdr_bin="${HERDR_BIN_PATH:-herdr}"
if [[ "$mode" != config-only ]]; then
  required+=(git gh "$herdr_bin")
else
  required+=(tar)
fi
for executable in "${required[@]}"; do
  if ! command -v "$executable" >/dev/null 2>&1; then
    printf 'Error: install %s before running this installer.\n' "$executable" >&2
    exit 1
  fi
done

temporary="$(mktemp -d "${TMPDIR:-/tmp}/herdrpp-install.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT
curl --fail --silent --show-error --location \
  https://api.github.com/repos/kamaal111/herdrpp/releases/latest >"$temporary/release.json"
release_tag="$(python3 - "$temporary/release.json" <<'PY'
import json, sys
with open(sys.argv[1]) as stream:
    release = json.load(stream)
tag = release.get("tag_name")
if release.get("draft") or release.get("prerelease") or not isinstance(tag, str) or not tag or tag.startswith("-"):
    sys.exit("Error: GitHub did not return a published stable release tag.")
print(tag)
PY
)"
printf 'Installing herdr++ release %s (%s).\n' "$release_tag" "$mode"

if [[ "$mode" != config-only ]]; then
  "$herdr_bin" plugin install kamaal111/herdrpp --ref "$release_tag" --yes
fi

if [[ "$mode" != plugin-only ]]; then
  if [[ "$mode" == config-only ]]; then
    encoded_tag="$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$release_tag")"
    curl --fail --silent --show-error --location \
      "https://api.github.com/repos/kamaal111/herdrpp/tarball/$encoded_tag" >"$temporary/source.tar.gz"
    mkdir "$temporary/source"
    tar -xzf "$temporary/source.tar.gz" -C "$temporary/source" --strip-components=1
    cargo build --manifest-path "$temporary/source/Cargo.toml" --locked --release --target-dir "$temporary/target"
    binary="$temporary/target/release/herdrpp"
  else
    "$herdr_bin" plugin list --plugin herdrpp --json >"$temporary/plugins.json"
    plugin_root="$(python3 - "$temporary/plugins.json" <<'PY'
import json, sys
with open(sys.argv[1]) as stream:
    plugins = json.load(stream)["result"]["plugins"]
matches = [plugin for plugin in plugins if plugin["plugin_id"] == "herdrpp"]
if len(matches) != 1:
    sys.exit("Error: Herdr did not return exactly one installed herdrpp plugin.")
print(matches[0]["plugin_root"])
PY
)"
    binary="$plugin_root/target/release/herdrpp"
  fi
  if ! "$binary" --help | grep -q '^configure '; then
    printf 'Error: release %s does not include config setup; choose a release with configure or set up manually.\n' "$release_tag" >&2
    exit 1
  fi
  "$binary" configure
fi

if [[ "$mode" != config-only ]]; then
  if "$herdr_bin" api snapshot >/dev/null 2>&1; then
    "$herdr_bin" plugin action invoke herdrpp.start
    printf 'Start requested; inspect completed Herdr plugin logs to confirm watcher startup.\n'
  else
    printf 'No accessible running session. The startup hook activates the watcher at server startup.\n'
  fi
  printf 'Ensure git and authenticated gh are on the Herdr server PATH (gh auth login).\n'
fi
if [[ "$mode" != plugin-only ]]; then
  printf 'Select Reload config in the viewing Herdr client.\n'
fi
printf 'herdr++ installation completed.\n'
