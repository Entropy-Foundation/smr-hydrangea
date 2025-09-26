#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

show_usage() {
    cat <<'USAGE'
Usage: start_local.sh [N]

Runs N local Hydrangea nodes bound to localhost ports.

Arguments:
  N   Optional. Number of nodes to launch (default: 4).

Environment:
  BASE_PORT       Starting port number for the first node (default: 3000).
                  Each node consumes three consecutive ports: consensus,
                  primary, and worker-to-primary.
  PROFILE         Cargo profile to build/run (default: debug). Set to
                  "release" for an optimized binary or any custom cargo
                  profile name for advanced use.
  BLS_THRESHOLD   Threshold value for BLS key generation (default: N).
  WORKERS         Number of workers to configure per authority (default: 1).

The script stores generated data under scripts/.local and keeps track of
spawned process IDs in scripts/.local/node.pids so they can be stopped with
`xargs kill < scripts/.local/node.pids`.
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    show_usage
    exit 0
fi

N_VALUE="${1:-4}"
if ! [[ "$N_VALUE" =~ ^[0-9]+$ ]] || [[ "$N_VALUE" == "0" ]]; then
    echo "Error: N must be a positive integer." >&2
    exit 1
fi
N=$((N_VALUE))

BASE_PORT_VALUE="${BASE_PORT:-3000}"
if ! [[ "$BASE_PORT_VALUE" =~ ^[0-9]+$ ]] || (( BASE_PORT_VALUE <= 1024 )); then
    echo "Error: BASE_PORT must be an integer greater than 1024 (got '$BASE_PORT_VALUE')." >&2
    exit 1
fi
BASE_PORT=$((BASE_PORT_VALUE))

PROFILE="${PROFILE:-debug}"
BLS_THRESHOLD_VALUE="${BLS_THRESHOLD:-$N}"
if ! [[ "$BLS_THRESHOLD_VALUE" =~ ^[0-9]+$ ]] || (( BLS_THRESHOLD_VALUE <= 0 )); then
    echo "Error: BLS_THRESHOLD must be a positive integer (got '$BLS_THRESHOLD_VALUE')." >&2
    exit 1
fi
if (( BLS_THRESHOLD_VALUE > N )); then
    echo "Error: BLS_THRESHOLD ($BLS_THRESHOLD_VALUE) cannot exceed N ($N)." >&2
    exit 1
fi
BLS_THRESHOLD=$((BLS_THRESHOLD_VALUE))

WORKERS_VALUE="${WORKERS:-1}"
if ! [[ "$WORKERS_VALUE" =~ ^[0-9]+$ ]] || (( WORKERS_VALUE <= 0 )); then
    echo "Error: WORKERS must be a positive integer (got '$WORKERS_VALUE')." >&2
    exit 1
fi
WORKERS_PER_NODE=$((WORKERS_VALUE))
NODE_PORT_STRIDE=$((3 + WORKERS_PER_NODE * 3))

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is required but not found in PATH." >&2
    exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "Error: python3 is required but not found in PATH." >&2
    exit 1
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

declare -a BUILD_ARGS=()
TARGET_DIR=""

case "$PROFILE" in
    release)
        BUILD_ARGS=(--release)
        TARGET_DIR="release"
        ;;
    debug)
        BUILD_ARGS=()
        TARGET_DIR="debug"
        ;;
    *)
        BUILD_ARGS=(--profile "$PROFILE")
        TARGET_DIR="$PROFILE"
        ;;
esac
NODE_BIN="$REPO_ROOT/target/$TARGET_DIR/node"

DATA_DIR="$SCRIPT_DIR/.local"
CONFIG_DIR="$DATA_DIR/config"
KEY_DIR="$DATA_DIR/keys"
BLS_DIR="$DATA_DIR/bls"
STORE_DIR="$DATA_DIR/stores"
LOG_DIR="$DATA_DIR/logs"
PIDS_FILE="$DATA_DIR/node.pids"
COMMITTEE_FILE="$CONFIG_DIR/committee.json"
PARAMS_FILE="$CONFIG_DIR/parameters.json"
BLS_TEMPLATE="$BLS_DIR/node-bls-x.json"

mkdir -p "$CONFIG_DIR" "$KEY_DIR" "$BLS_DIR" "$STORE_DIR" "$LOG_DIR"

if [[ -f "$PIDS_FILE" ]]; then
    while IFS=: read -r pid _; do
        if [[ -n "${pid:-}" && "$pid" =~ ^[0-9]+$ ]]; then
            if kill -0 "$pid" >/dev/null 2>&1; then
                echo "Stopping existing node process $pid"
                kill "$pid" >/dev/null 2>&1 || true
            fi
        fi
    done < "$PIDS_FILE"
    rm -f "$PIDS_FILE"
fi

if [[ -d "$STORE_DIR" ]]; then
    for dir in "$STORE_DIR"/*; do
        [[ -d "$dir" ]] && rm -rf "$dir"
    done
fi

rm -f "$KEY_DIR"/node-*.json
rm -f "$BLS_DIR"/node-bls-*.json

mkdir -p "$STORE_DIR"

printf 'Compiling node binary (%s profile)...\n' "$PROFILE"
if (( ${#BUILD_ARGS[@]} )); then
    cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" --bin node "${BUILD_ARGS[@]}"
else
    cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" --bin node
fi

printf 'Generating %d ed25519 key pairs...\n' "$N"
for (( i=0; i<N; i++ )); do
    "$NODE_BIN" generate_keys --filename "$KEY_DIR/node-$i.json"
done

printf 'Generating %d BLS key shares (threshold=%d)...\n' "$N" "$BLS_THRESHOLD"
"$NODE_BIN" generate_bls_keys --nodes "$N" --threshold "$BLS_THRESHOLD" --path "$BLS_TEMPLATE"

printf 'Writing committee and parameter files...\n'
python3 - "$N" "$BASE_PORT" "$CONFIG_DIR" "$KEY_DIR" "$BLS_DIR" "$COMMITTEE_FILE" "$PARAMS_FILE" "$WORKERS_PER_NODE" <<'PY'
import json
import sys
from pathlib import Path

n = int(sys.argv[1])
base_port = int(sys.argv[2])
config_dir = Path(sys.argv[3])
key_dir = Path(sys.argv[4])
bls_dir = Path(sys.argv[5])
committee_path = Path(sys.argv[6])
params_path = Path(sys.argv[7])
workers_per_node = int(sys.argv[8])

names = []
bls_g1 = []
bls_g2 = []
for idx in range(n):
    with open(key_dir / f"node-{idx}.json", 'r', encoding='utf-8') as f:
        names.append(json.load(f)["name"])
    with open(bls_dir / f"node-bls-{idx}.json", 'r', encoding='utf-8') as f:
        data = json.load(f)
        bls_g1.append(data["nameg1"])
        bls_g2.append(data["nameg2"])

authorities = {}
port_cursor = base_port
for idx, name in enumerate(names):
    consensus_port = port_cursor
    primary_port = port_cursor + 1
    worker_primary_port = port_cursor + 2

    workers = {}
    worker_port_base = port_cursor + 3
    for worker_id in range(workers_per_node):
        base = worker_port_base + worker_id * 3
        workers[worker_id] = {
            "primary_to_worker": f"127.0.0.1:{base}",
            "transactions": f"127.0.0.1:{base + 1}",
            "worker_to_worker": f"127.0.0.1:{base + 2}"
        }

    authorities[name] = {
        "id": idx,
        "bls_pubkey_g1": bls_g1[idx],
        "bls_pubkey_g2": bls_g2[idx],
        "is_honest": True,
        "stake": 1,
        "consensus": {
            "consensus_to_consensus": f"127.0.0.1:{consensus_port}"
        },
        "primary": {
            "primary_to_primary": f"127.0.0.1:{primary_port}",
            "worker_to_primary": f"127.0.0.1:{worker_primary_port}"
        },
        "workers": workers
    }

    port_cursor += 3 + workers_per_node * 3

committee = {"authorities": authorities}
with open(committee_path, 'w', encoding='utf-8') as f:
    json.dump(committee, f, indent=4)
    f.write('\n')

default_params = {
    "consensus_only": False,
    "timeout_delay": 5000,
    "header_size": 1000,
    "max_block_size": 1,
    "max_header_delay": 100,
    "gc_depth": 50,
    "sync_retry_delay": 5000,
    "sync_retry_nodes": 3,
    "batch_size": 500000,
    "max_batch_delay": 100,
    "use_vote_aggregator": False,
    "leader_elector": "Simple",
}

f = 0 if n <= 1 else (n - 1) // 3
remaining = max(n - 1 - 3 * f, 0)
c = 0
k = remaining

parameters = {
    **default_params,
    "n": n,
    "f": f,
    "c": c,
    "k": k,
}
with open(params_path, 'w', encoding='utf-8') as f:
    json.dump(parameters, f, indent=4)
    f.write('\n')
PY

: > "$PIDS_FILE"

printf 'Launching %d nodes (workers per node: %d)...\n' "$N" "$WORKERS_PER_NODE"
for (( i=0; i<N; i++ )); do
    store_dir="$STORE_DIR/node-$i"
    mkdir -p "$store_dir"
    log_file="$LOG_DIR/node-$i.log"
    node_base=$((BASE_PORT + i * NODE_PORT_STRIDE))
    consensus_port=$node_base
    primary_port=$((node_base + 1))
    worker_primary_port=$((node_base + 2))

    printf '  - Node %d: consensus %d, primary %d, worker-to-primary %d (log: %s)\n' \
        "$i" "$consensus_port" "$primary_port" "$worker_primary_port" "$log_file"

    "$NODE_BIN" -vv run \
        --edkeys "$KEY_DIR/node-$i.json" \
        --blskeys "$BLS_DIR/node-bls-$i.json" \
        --committee "$COMMITTEE_FILE" \
        --parameters "$PARAMS_FILE" \
        --store "$store_dir" \
        primary \
        >> "$log_file" 2>&1 &
    pid=$!
    echo "$pid" >> "$PIDS_FILE"
done

printf '\nAll nodes launched. Active PIDs stored in %s.\n' "$PIDS_FILE"
printf 'Stop them with: xargs kill < %s\n' "$PIDS_FILE"
printf 'Logs are under %s.\n' "$LOG_DIR"
