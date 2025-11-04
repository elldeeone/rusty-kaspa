#!/usr/bin/env bash
set -euo pipefail

# Runs a deterministic Simpa pruning cycle, captures logs, and exports pruning metrics.
#
# Environment overrides:
#   SIMPA_DB_DIR   - directory to store/reuse the simulated RocksDB (default: baseline/experiments/simpa-db)
#   SIMPA_BPS      - virtual blocks per second (default: 2.0)
#   SIMPA_DELAY    - simulated network delay in seconds (default: 2.0)
#   SIMPA_TARGET_BLOCKS - number of blocks to generate when no DB exists (default: 2000)
#   SIMPA_TPB      - target txs per block (default: 64)
#   SIMPA_RETENTION_DAYS - pruning retention window in days (default: 0.05 ~= 72 minutes)
#   SIMPA_LOGLEVEL - log level passed to Simpa/kaspad (default: info,consensus=trace,pruning_processor=trace,simpa=info)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPERIMENT_DIR="${ROOT_DIR}/pruning-optimisation/baseline/experiments"
mkdir -p "${EXPERIMENT_DIR}"

SIMPA_DB_DIR="${SIMPA_DB_DIR:-${EXPERIMENT_DIR}/simpa-db}"
SIMPA_BPS="${SIMPA_BPS:-2.0}"
SIMPA_DELAY="${SIMPA_DELAY:-2.0}"
SIMPA_TARGET_BLOCKS="${SIMPA_TARGET_BLOCKS:-2000}"
SIMPA_TPB="${SIMPA_TPB:-64}"
SIMPA_RETENTION_DAYS="${SIMPA_RETENTION_DAYS:-0.05}"
SIMPA_LOGLEVEL="${SIMPA_LOGLEVEL:-info,consensus=trace,pruning_processor=trace,simpa=info}"

timestamp() {
    date +"%Y%m%d-%H%M%S"
}

LOG_FILE="${EXPERIMENT_DIR}/simpa-prune-$(timestamp).log"
METRICS_FILE="${LOG_FILE%.log}.csv"

COMMON_ARGS=(
    --test-pruning
    --skip-acceptance-validation
    --bps "${SIMPA_BPS}"
    --delay "${SIMPA_DELAY}"
    --tpb "${SIMPA_TPB}"
    --retention-period-days "${SIMPA_RETENTION_DAYS}"
    --loglevel "${SIMPA_LOGLEVEL}"
)

SIMPA_DB_CURRENT="${SIMPA_DB_DIR}/CURRENT"
if [[ -f "${SIMPA_DB_CURRENT}" ]]; then
    echo ">> Reusing existing Simpa DB at ${SIMPA_DB_DIR}"
    IO_ARGS=(--input-dir "${SIMPA_DB_DIR}")
else
    echo ">> No Simpa DB found. Generating ${SIMPA_TARGET_BLOCKS} blocks..."
    if [[ -d "${SIMPA_DB_DIR}" ]]; then
        rm -rf "${SIMPA_DB_DIR}"
    fi
    mkdir -p "$(dirname "${SIMPA_DB_DIR}")"
    IO_ARGS=(--output-dir "${SIMPA_DB_DIR}" --target-blocks "${SIMPA_TARGET_BLOCKS}")
fi

echo ">> Running Simpa. Log: ${LOG_FILE}"
(
    cd "${ROOT_DIR}"
    cargo run --quiet --release --bin simpa -- \
        "${COMMON_ARGS[@]}" \
        "${IO_ARGS[@]}" \
        "$@" \
        2>&1
) | tee "${LOG_FILE}"

if command -v python3 >/dev/null 2>&1; then
    echo ">> Extracting pruning metrics to ${METRICS_FILE}"
    python3 "${ROOT_DIR}/pruning-optimisation/scripts/collect_pruning_metrics.py" "${LOG_FILE}" > "${METRICS_FILE}"
else
    echo "!! python3 not found; skipping metrics extraction" >&2
fi

echo ">> Done. Raw log: ${LOG_FILE}"
echo ">> Metrics CSV (if produced): ${METRICS_FILE}"
