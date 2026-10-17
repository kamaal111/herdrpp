#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
output=${NAVIGATOR_GATE_OUTPUT_DIR:-"$PWD/target/navigator-gate"}
image=${NAVIGATOR_GATE_IMAGE:-herdrpp-navigator-gate}
mkdir -p "$output"
if [ "${NAVIGATOR_GATE_SKIP_BUILD:-0}" != 1 ]; then
    docker build --platform linux/amd64 -t herdrpp-navigator-base -f tests/snapshots/picker.Dockerfile .
    docker build --platform linux/amd64 -t "$image" -f experiments/machines/Dockerfile .
fi
mode=${1:-collision}
case "$mode" in
    collision) selector=same_chord ;;
    public-chord) selector=public_reserved_control_chord ;;
    passing) selector='private_client_binding or sigwinch or ancestry or fragmented' ;;
    *) echo 'Usage: experiments/machines/run-gate.sh [collision|passing|public-chord]' >&2; exit 2 ;;
esac
docker run --rm --init --platform linux/amd64 \
    -v "$PWD:/workspace:ro" -v "$output:/output" "$image" \
    python3 -m pytest experiments/machines/test_navigator_gate.py -vv -k "$selector" \
    -o cache_dir=/tmp/pytest-cache --junitxml="/output/$mode.xml"
