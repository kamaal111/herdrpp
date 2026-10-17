#!/bin/sh
set -eu
mode=${1:-all}
case "$mode" in
    all|ssh|graphics) ;;
    *) printf 'Usage: tests/machines/run.sh [all|ssh|graphics]\n' >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then
    printf 'Usage: tests/machines/run.sh [all|ssh|graphics]\n' >&2
    exit 2
fi
cd "$(dirname "$0")/../.."
output=${MACHINE_OUTPUT_DIR:-"$PWD/target/machine-switching"}
mkdir -p "$output"
output=$(cd "$output" && pwd)
if [ "${MACHINE_SKIP_BUILD:-0}" != 1 ]; then
    docker build --platform linux/amd64 -t herdrpp-machine-base -f tests/snapshots/picker.Dockerfile .
    case "$mode" in
        all|ssh) docker build --platform linux/amd64 -t herdrpp-navigator-ssh -f tests/machines/ssh.Dockerfile . ;;
    esac
    case "$mode" in
        all|graphics) docker build --platform linux/amd64 -t herdrpp-navigator-graphics -f tests/machines/product_graphics.Dockerfile . ;;
    esac
    docker run --rm --init --platform linux/amd64 -v "$PWD:/workspace:ro" -v "$output:/output" \
        herdrpp-machine-base cargo build --locked --release
fi
docker run --rm --platform linux/amd64 -v "$output:/output" herdrpp-machine-base python3 -c '
import hashlib, json, subprocess
from pathlib import Path
binary = Path("/output/build/release/herdrpp")
metadata = {
    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "herdr_version": subprocess.check_output(["herdr", "--version"], text=True).strip(),
}
Path("/output/binary.json").write_text(json.dumps(metadata, indent=2) + "\n")
'
case "$mode" in
    all|ssh)
        docker run --rm --init --platform linux/amd64 -v "$PWD:/workspace:ro" -v "$output:/output" \
            "${MACHINE_SSH_IMAGE:-herdrpp-navigator-ssh}" python3 -m pytest tests/machines/picker_gate.py tests/machines/ssh_gate.py tests/machines/ssh_failures_gate.py tests/machines/navigator_gate.py tests/machines/navigator_lifecycle_gate.py tests/machines/launch_lifecycle_gate.py \
            -vv -o cache_dir=/tmp/pytest-cache --junitxml=/output/ssh.xml
        ;;
esac
case "$mode" in
    all|graphics)
        docker run --rm --init --platform linux/amd64 -v "$PWD:/workspace:ro" -v "$output:/output" \
            -e MACHINE_GRAPHICS_BINARY=/output/build/release/herdrpp \
            -e MACHINE_GRAPHICS_OUTPUT=/output/graphics \
            "${MACHINE_GRAPHICS_IMAGE:-herdrpp-navigator-graphics}" sh -ec '
                run_graphics_gate() {
                    gate_output=$1
                    gate_script=$2
                    python3 -c '\''import sys; from pathlib import Path; p=Path(sys.argv[1]); p.mkdir(parents=True, exist_ok=True); (p/"outcome.txt").write_text("FAIL\n")'\'' "$gate_output"
                    MACHINE_GRAPHICS_OUTPUT="$gate_output" xvfb-run -a -s "-screen 0 1200x800x24" kitty --config NONE \
                        -o linux_display_server=x11 -o confirm_os_window_close=0 \
                        /opt/snapshots/bin/python "/workspace/tests/machines/$gate_script"
                    python3 -c '\''import sys; from pathlib import Path; p=Path(sys.argv[1]); assert (p/"outcome.txt").read_text() == "PASS\n", f"rendered graphics gate failed; inspect {p}"'\'' "$gate_output"
                }
                run_graphics_gate /output/graphics product_graphics_gate.py
                run_graphics_gate /output/graphics-ssh product_graphics_ssh_gate.py
            '
        ;;
esac
