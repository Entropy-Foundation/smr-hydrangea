#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

cleanup() {
    "$SCRIPT_DIR/stop_local.sh" --force --purge >/dev/null 2>&1 || true
}
trap cleanup EXIT

LOCAL_DIR="$SCRIPT_DIR/.local"
export HYDRANGEA_LOCAL_DIR="$LOCAL_DIR"
export HYDRANGEA_NODE_LOG="$LOCAL_DIR/logs/node-0.log"

NODES=${1:-4}
if [[ "$NODES" != "4" ]]; then
    echo "Warning: overriding default node count to $NODES" >&2
fi

printf 'Starting %s local nodes...\n' "$NODES"
"$SCRIPT_DIR/start_local.sh" "$NODES"

# Give nodes a moment to bind their ports before submitting transactions.
sleep 5

printf '\nRunning Aptos integration transfer test...\n'
cargo run --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p aptos_executor --bin integration_test

printf '\nIntegration test completed successfully.\n'
