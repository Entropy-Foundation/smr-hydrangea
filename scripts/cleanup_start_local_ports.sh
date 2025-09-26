#!/usr/bin/env bash
set -euo pipefail

show_usage() {
    cat <<'USAGE'
Usage: cleanup_start_local_ports.sh [N]

Lists processes bound to the TCP ports that scripts/start_local.sh uses
and, upon confirmation, terminates them.

Arguments:
  N   Optional. Number of nodes to inspect (default: 4).

Environment:
  BASE_PORT   Starting port number for the first node (default: 3000).
  WORKERS     Number of workers per authority (default: 1).

The script mirrors the port allocation logic in scripts/start_local.sh.
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

WORKERS_VALUE="${WORKERS:-1}"
if ! [[ "$WORKERS_VALUE" =~ ^[0-9]+$ ]] || (( WORKERS_VALUE <= 0 )); then
    echo "Error: WORKERS must be a positive integer (got '$WORKERS_VALUE')." >&2
    exit 1
fi
WORKERS_PER_NODE=$((WORKERS_VALUE))

if ! command -v lsof >/dev/null 2>&1; then
    echo "Error: lsof is required but not found in PATH." >&2
    exit 1
fi

NODE_PORT_STRIDE=$((3 + WORKERS_PER_NODE * 3))
PORTS=()
for (( i=0; i<N; i++ )); do
    node_base=$((BASE_PORT + i * NODE_PORT_STRIDE))
    PORTS+=("$node_base" "$((node_base + 1))" "$((node_base + 2))")

    for (( worker=0; worker<WORKERS_PER_NODE; worker++ )); do
        worker_base=$((node_base + 3 + worker * 3))
        PORTS+=("$worker_base" "$((worker_base + 1))" "$((worker_base + 2))")
    done
done

UNIQUE_PORTS=()
while IFS= read -r port; do
    [[ -n "$port" ]] && UNIQUE_PORTS+=("$port")
done < <(printf '%s\n' "${PORTS[@]}" | sort -n | uniq)

FOUND=0
LISTINGS=()
PIDS=()
PIDS_SET=","  # sentinel for quick membership tests

for port in "${UNIQUE_PORTS[@]}"; do
    while IFS= read -r line; do
        trimmed="${line#${line%%[![:space:]]*}}"
        [[ -z "$trimmed" ]] && continue

        command=$(awk 'NR==1 {print $1}' <<<"$trimmed")
        pid=$(awk 'NR==1 {print $2}' <<<"$trimmed")
        user=$(awk 'NR==1 {print $3}' <<<"$trimmed")
        name=$(awk '{if (NF >= 9) {out = $9; for (i = 10; i <= NF; i++) out = out " " $i; print out}}' <<<"$trimmed")
        if [[ -z "$name" ]]; then
            name="(unknown)"
        fi

        if [[ -z "$pid" ]]; then
            continue
        fi

        FOUND=1
        LISTINGS+=("Port $port  PID $pid  USER $user  CMD $command  NAME $name")

        if [[ "$PIDS_SET" != *",$pid,"* ]]; then
            PIDS+=("$pid")
            PIDS_SET+="$pid,"
        fi
    done < <({ lsof -nP -sTCP:LISTEN -iTCP:"$port" 2>/dev/null || true; } | sed '1d')
done

if (( FOUND == 0 )); then
    echo "No listening processes detected on the target port range."
    exit 0
fi

printf '%s\n' "${LISTINGS[@]}"

printf '\nDetected %d unique PID(s) on the Hydrangea local ports.\n' "${#PIDS[@]}"
read -r -p "Terminate these processes? [y/N]: " reply
case "$reply" in
    [yY][eE][sS]|[yY])
        for pid in "${PIDS[@]}"; do
            if kill "$pid" >/dev/null 2>&1; then
                echo "Sent SIGTERM to PID $pid"
            else
                echo "Failed to terminate PID $pid" >&2
            fi
        done
        ;;
    *)
        echo "Skipped terminating processes."
        ;;
esac
