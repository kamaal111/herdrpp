#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/snapshots/picker tests/snapshots/picker/baselines
docker build --platform linux/amd64 -t herdrpp-picker-snapshots -f tests/snapshots/picker.Dockerfile .
baseline_mode=ro
if [ "${1:-}" = "--record" ]; then
    if [ -n "${CI:-}" ]; then
        echo 'Recording snapshots is disabled in CI' >&2
        exit 1
    fi
    baseline_mode=rw
fi
docker run --rm --platform linux/amd64 \
    -e CI="${CI:-}" \
    -v "$PWD:/workspace:ro" \
    -v "$PWD/target/snapshots/picker:/output" \
    -v "$PWD/tests/snapshots/picker/baselines:/baselines:$baseline_mode" \
    herdrpp-picker-snapshots python3 tests/picker_snapshots.py "$@"
