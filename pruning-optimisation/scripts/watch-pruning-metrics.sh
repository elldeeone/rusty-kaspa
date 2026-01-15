#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

LOG_FILE="${1:-}"
KASPAD_PID="${2:-}"
MIN_PRUNED="${3:-1}"

if [[ -z "${LOG_FILE}" || -z "${KASPAD_PID}" ]]; then
    echo "Usage: $0 <log_file> <kaspad_pid> [min_pruned]" >&2
    exit 2
fi

: "${KASPAD_SIGINT_WAIT_SEC:=60}"
: "${KASPAD_SIGTERM_WAIT_SEC:=30}"

CSV_FILE="${LOG_FILE%.log}.csv"

python3 - "${LOG_FILE}" "${KASPAD_PID}" "${MIN_PRUNED}" <<'PY'
import os
import re
import signal
import sys
import time

log_path = sys.argv[1]
pid = int(sys.argv[2])
min_pruned = int(sys.argv[3])

pattern = re.compile(r"\[PRUNING METRICS\].*\bpruned=(\d+)")

while not os.path.exists(log_path):
    time.sleep(0.2)

with open(log_path, "r") as f:
    while True:
        line = f.readline()
        if not line:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                sys.exit(1)
            time.sleep(0.5)
            continue
        if "[PRUNING METRICS]" in line and "duration_ms=" in line:
            match = pattern.search(line)
            if match and int(match.group(1)) >= min_pruned:
                os.kill(pid, signal.SIGINT)
                sys.exit(0)
PY

wait_for_exit() {
    local pid="$1"
    local timeout="$2"
    local start
    start="$(date +%s)"
    while kill -0 "${pid}" 2>/dev/null; do
        if (( $(date +%s) - start >= timeout )); then
            return 1
        fi
        sleep 1
    done
    return 0
}

if kill -0 "${KASPAD_PID}" 2>/dev/null; then
    if ! wait_for_exit "${KASPAD_PID}" "${KASPAD_SIGINT_WAIT_SEC}"; then
        kill -TERM "${KASPAD_PID}" 2>/dev/null || true
        if ! wait_for_exit "${KASPAD_PID}" "${KASPAD_SIGTERM_WAIT_SEC}"; then
            kill -KILL "${KASPAD_PID}" 2>/dev/null || true
            wait_for_exit "${KASPAD_PID}" 10 || true
        fi
    fi
fi

python3 "${ROOT_DIR}/pruning-optimisation/scripts/collect_pruning_metrics.py" "${LOG_FILE}" >"${CSV_FILE}" || true
echo ">> Metrics: ${CSV_FILE}"
