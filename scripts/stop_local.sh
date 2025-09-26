#!/usr/bin/env bash
set -euo pipefail

show_usage() {
    cat <<'USAGE'
Usage: stop_local.sh [--force] [--purge]

Stops Hydrangea nodes launched via start_local.sh by signaling the PIDs stored
under scripts/.local.

Options:
  -f, --force   Use SIGKILL instead of SIGTERM.
  -p, --purge   Remove the scripts/.local directory after stopping nodes.
  -h, --help    Show this message and exit.
USAGE
}

SIGNAL="TERM"
PURGE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        -f|--force)
            SIGNAL="KILL"
            shift
            ;;
        -p|--purge)
            PURGE=true
            shift
            ;;
        -h|--help)
            show_usage
            exit 0
            ;;
        *)
            echo "Error: Unknown option '$1'." >&2
            show_usage
            exit 1
            ;;
    esac
done

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DATA_DIR="$SCRIPT_DIR/.local"
PIDS_FILE="$DATA_DIR/node.pids"
LOG_DIR="$DATA_DIR/logs"

if [[ ! -d "$DATA_DIR" ]]; then
    echo "Nothing to stop: '$DATA_DIR' does not exist."
    exit 0
fi

if [[ ! -f "$PIDS_FILE" ]]; then
    echo "No PID file found at $PIDS_FILE."
else
    pids=()
    while IFS= read -r line || [[ -n "$line" ]]; do
        pids+=("$line")
    done < "$PIDS_FILE"

    if (( ${#pids[@]} == 0 )); then
        echo "PID file is empty; no processes to stop."
    else
        echo "Stopping nodes with SIG$SIGNAL..."
        for raw_pid in "${pids[@]}"; do
            pid="${raw_pid//[[:space:]]/}"
            [[ -z "$pid" ]] && continue
            if [[ ! "$pid" =~ ^[0-9]+$ ]]; then
                echo "  Skipping malformed PID entry '$pid'."
                continue
            fi
            if kill -0 "$pid" 2>/dev/null; then
                if kill -"$SIGNAL" "$pid" 2>/dev/null; then
                    echo "  Sent SIG$SIGNAL to $pid."
                else
                    echo "  Failed to signal $pid." >&2
                fi
            else
                echo "  Process $pid is not running."
            fi
        done
    fi
    : > "$PIDS_FILE"
fi

if $PURGE; then
    echo "Purging $DATA_DIR..."
    rm -rf "$DATA_DIR"
else
    # Keep logs unless purge requested.
    if [[ -d "$LOG_DIR" ]]; then
        echo "Logs remain in $LOG_DIR."
    fi
fi

echo "Done."
