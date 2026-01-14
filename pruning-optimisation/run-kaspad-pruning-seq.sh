#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_DIR="${LOG_DIR:-${ROOT_DIR}/pruning-optimisation/baseline/experiments/kaspad}"
mkdir -p "${LOG_DIR}"

: "${KASPAD_LOGLEVEL:=info,consensus=trace}"
: "${PRUNE_WAIT_PRUNED_MIN:=1}"
: "${KASPAD_EXTRA_ARGS:=}"
: "${RESET_DB:=1}"
: "${KASPAD_SIGINT_WAIT_SEC:=60}"
: "${KASPAD_SIGTERM_WAIT_SEC:=30}"

if [[ -x "${ROOT_DIR}/target/release/kaspad" ]]; then
    KASPAD_CMD=("${ROOT_DIR}/target/release/kaspad")
else
    KASPAD_CMD=(cargo run --release --bin kaspad --)
fi

run_kaspad() {
    local label="$1"
    local network_flag="$2"
    shift 2
    local -a envs=("$@")

    local ts
    ts="$(date +%Y%m%d-%H%M%S)"
    local log_file="${LOG_DIR}/kaspad-${label}-${ts}.log"
    local csv_file="${LOG_DIR}/kaspad-${label}-${ts}.csv"

    local -a args=("--loglevel" "${KASPAD_LOGLEVEL}")
    if [[ "${RESET_DB}" == "1" ]]; then
        args+=("--reset-db")
    fi
    if [[ -n "${network_flag}" ]]; then
        args+=("${network_flag}")
    fi
    if [[ -n "${KASPAD_EXTRA_ARGS}" ]]; then
        # shellcheck disable=SC2206
        args+=(${KASPAD_EXTRA_ARGS})
    fi

    echo ">> Run ${label}"
    env "${envs[@]}" "${KASPAD_CMD[@]}" "${args[@]}" >"${log_file}" 2>&1 &
    local kaspad_pid=$!

    python3 - "${log_file}" "${kaspad_pid}" "${PRUNE_WAIT_PRUNED_MIN}" <<'PY'
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

    if kill -0 "${kaspad_pid}" 2>/dev/null; then
        if ! wait_for_exit "${kaspad_pid}" "${KASPAD_SIGINT_WAIT_SEC}"; then
            kill -TERM "${kaspad_pid}" 2>/dev/null || true
            if ! wait_for_exit "${kaspad_pid}" "${KASPAD_SIGTERM_WAIT_SEC}"; then
                kill -KILL "${kaspad_pid}" 2>/dev/null || true
                wait_for_exit "${kaspad_pid}" 10 || true
            fi
        fi
    fi
    wait "${kaspad_pid}" 2>/dev/null || true
    python3 "${ROOT_DIR}/pruning-optimisation/scripts/collect_pruning_metrics.py" "${log_file}" >"${csv_file}" || true
    echo ">> Done ${label}. Log: ${log_file}"
    echo ">> Metrics: ${csv_file}"
}

UNBATCHED_ENV=(
    PRUNE_BATCH_MAX_BLOCKS=1
    PRUNE_BATCH_MAX_OPS=1
    PRUNE_BATCH_MAX_BYTES=1
    PRUNE_BATCH_MAX_DURATION_MS=0
    PRUNE_BATCH_BODIES=1
)

REACH_LOCK5_ENV=(
    PRUNE_BATCH_BODIES=0
    PRUNE_LOCK_MAX_DURATION_MS=5
    PRUNE_BATCH_MAX_DURATION_MS=10
)

run_kaspad "mainnet-unbatched" "" "${UNBATCHED_ENV[@]}"
run_kaspad "mainnet-reach-lock5" "" "${REACH_LOCK5_ENV[@]}"
run_kaspad "testnet-unbatched" "--testnet" "${UNBATCHED_ENV[@]}"
run_kaspad "testnet-reach-lock5" "--testnet" "${REACH_LOCK5_ENV[@]}"
