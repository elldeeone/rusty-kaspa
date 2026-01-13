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
#   SIMPA_LOGLEVEL - log level passed to Simpa/kaspad (default: info,consensus=trace,pruning_processor=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace,simpa=info)
#   SIMPA_MINERS - number of miners to simulate (default: 1)
#   SIMPA_LONG_PAYLOAD - set to 1 to enable --long-payload (stress larger blocks)
#   SIMPA_ROCKSDB_STATS - set to 1 to enable RocksDB statistics logging
#   SIMPA_ROCKSDB_STATS_PERIOD_SEC - optional period (seconds) for RocksDB stats sampling when SIMPA_ROCKSDB_STATS=1
#   SIMPA_ROCKSDB_PIPELINED - set to 1 to enable RocksDB pipelined writes during the run
#   SIMPA_SKIP_ACCEPTANCE_VALIDATION - set to 1 to skip acceptance validation (default: 1)
#   SIMPA_RUN_LABEL - optional run label (printed only)
#   PRUNE_LOCK_MAX_DURATION_MS - pruning lock max duration override
#   PRUNE_BATCH_MAX_BLOCKS - prune batch max blocks override
#   PRUNE_BATCH_MAX_OPS - prune batch max ops override
#   PRUNE_BATCH_MAX_BYTES - prune batch max bytes override
#   PRUNE_BATCH_MAX_DURATION_MS - prune batch max duration override
#   PRUNE_BATCH_BODIES - set to 0 to disable body/UTXO batching

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPERIMENT_DIR="${ROOT_DIR}/pruning-optimisation/baseline/experiments"
mkdir -p "${EXPERIMENT_DIR}"

SIMPA_DB_DIR="${SIMPA_DB_DIR:-${EXPERIMENT_DIR}/simpa-db}"
SIMPA_BPS="${SIMPA_BPS:-2.0}"
SIMPA_DELAY="${SIMPA_DELAY:-2.0}"
SIMPA_TARGET_BLOCKS="${SIMPA_TARGET_BLOCKS:-2000}"
SIMPA_TPB="${SIMPA_TPB:-64}"
SIMPA_RETENTION_DAYS="${SIMPA_RETENTION_DAYS:-0.05}"
SIMPA_LOGLEVEL="${SIMPA_LOGLEVEL:-info,consensus=trace,pruning_processor=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace,simpa=info}"
SIMPA_FORCE_REBUILD="${SIMPA_FORCE_REBUILD:-0}"
SIMPA_MINERS="${SIMPA_MINERS:-1}"
SIMPA_LONG_PAYLOAD="${SIMPA_LONG_PAYLOAD:-0}"
SIMPA_ROCKSDB_STATS="${SIMPA_ROCKSDB_STATS:-0}"
SIMPA_ROCKSDB_STATS_PERIOD_SEC="${SIMPA_ROCKSDB_STATS_PERIOD_SEC:-}"
SIMPA_ROCKSDB_PIPELINED="${SIMPA_ROCKSDB_PIPELINED:-0}"
SIMPA_SKIP_ACCEPTANCE_VALIDATION="${SIMPA_SKIP_ACCEPTANCE_VALIDATION:-1}"
SIMPA_RUN_LABEL="${SIMPA_RUN_LABEL:-}"

timestamp() {
    date +"%Y%m%d-%H%M%S"
}

label_suffix=""
if [[ -n "${SIMPA_RUN_LABEL}" ]]; then
    safe_label="$(echo "${SIMPA_RUN_LABEL}" | tr -cs 'A-Za-z0-9._-' '-' | sed -e 's/^-//' -e 's/-$//')"
    label_suffix="-${safe_label}"
fi
LOG_FILE="${EXPERIMENT_DIR}/simpa-prune${label_suffix}-$(timestamp).log"
METRICS_FILE="${LOG_FILE%.log}.csv"

COMMON_ARGS=(
    --test-pruning
    --bps "${SIMPA_BPS}"
    --delay "${SIMPA_DELAY}"
    --tpb "${SIMPA_TPB}"
    --retention-period-days "${SIMPA_RETENTION_DAYS}"
    --loglevel "${SIMPA_LOGLEVEL}"
    --miners "${SIMPA_MINERS}"
)

if [[ "${SIMPA_SKIP_ACCEPTANCE_VALIDATION}" == "1" ]]; then
    COMMON_ARGS+=(--skip-acceptance-validation)
fi

if [[ "${SIMPA_ROCKSDB_STATS}" == "1" ]]; then
    COMMON_ARGS+=(--rocksdb-stats)
    if [[ -n "${SIMPA_ROCKSDB_STATS_PERIOD_SEC}" ]]; then
        COMMON_ARGS+=(--rocksdb-stats-period-sec "${SIMPA_ROCKSDB_STATS_PERIOD_SEC}")
    fi
fi

if [[ "${SIMPA_LONG_PAYLOAD}" == "1" ]]; then
    COMMON_ARGS+=(--long-payload)
fi

if [[ "${SIMPA_ROCKSDB_PIPELINED}" == "1" ]]; then
    COMMON_ARGS+=(--rocksdb-pipelined-write)
fi

SIMPA_DB_CURRENT="${SIMPA_DB_DIR}/CURRENT"
if [[ "${SIMPA_FORCE_REBUILD}" == "1" ]]; then
    DB_ACTION="SIMPA_FORCE_REBUILD=1 — removing existing DB at ${SIMPA_DB_DIR}"
    rm -rf "${SIMPA_DB_DIR}"
fi

if [[ -f "${SIMPA_DB_CURRENT}" ]]; then
    DB_ACTION="${DB_ACTION:-Reusing existing Simpa DB at ${SIMPA_DB_DIR}}"
    IO_ARGS=(--input-dir "${SIMPA_DB_DIR}")
else
    DB_ACTION="${DB_ACTION:-No Simpa DB found. Generating ${SIMPA_TARGET_BLOCKS} blocks...}"
    if [[ -d "${SIMPA_DB_DIR}" ]]; then
        rm -rf "${SIMPA_DB_DIR}"
    fi
    mkdir -p "$(dirname "${SIMPA_DB_DIR}")"
    IO_ARGS=(--output-dir "${SIMPA_DB_DIR}" --target-blocks "${SIMPA_TARGET_BLOCKS}")
fi

{
    echo ">> Simpa pruning run"
    echo "   SIMPA_RUN_LABEL=${SIMPA_RUN_LABEL}"
    echo "   SIMPA_DB_DIR=${SIMPA_DB_DIR}"
    echo "   SIMPA_BPS=${SIMPA_BPS}, SIMPA_MINERS=${SIMPA_MINERS}, SIMPA_TPB=${SIMPA_TPB}, SIMPA_RETENTION_DAYS=${SIMPA_RETENTION_DAYS}"
    echo "   SIMPA_TARGET_BLOCKS=${SIMPA_TARGET_BLOCKS}, SIMPA_LONG_PAYLOAD=${SIMPA_LONG_PAYLOAD}"
    echo "   SIMPA_ROCKSDB_STATS=${SIMPA_ROCKSDB_STATS} (period ${SIMPA_ROCKSDB_STATS_PERIOD_SEC:-default}s)"
    echo "   SIMPA_ROCKSDB_PIPELINED=${SIMPA_ROCKSDB_PIPELINED}"
    echo "   SIMPA_SKIP_ACCEPTANCE_VALIDATION=${SIMPA_SKIP_ACCEPTANCE_VALIDATION}"
    echo "   SIMPA_FORCE_REBUILD=${SIMPA_FORCE_REBUILD}"
    echo "   PRUNE_LOCK_MAX_DURATION_MS=${PRUNE_LOCK_MAX_DURATION_MS:-unset}"
    echo "   PRUNE_BATCH_MAX_BLOCKS=${PRUNE_BATCH_MAX_BLOCKS:-unset}"
    echo "   PRUNE_BATCH_MAX_OPS=${PRUNE_BATCH_MAX_OPS:-unset}"
    echo "   PRUNE_BATCH_MAX_BYTES=${PRUNE_BATCH_MAX_BYTES:-unset}"
    echo "   PRUNE_BATCH_MAX_DURATION_MS=${PRUNE_BATCH_MAX_DURATION_MS:-unset}"
    echo "   PRUNE_BATCH_BODIES=${PRUNE_BATCH_BODIES:-unset}"
    echo ">> ${DB_ACTION}"
    echo ">> Running Simpa. Log: ${LOG_FILE}"
} | tee "${LOG_FILE}"
(
    cd "${ROOT_DIR}"
    cargo run --quiet --release --bin simpa -- \
        "${COMMON_ARGS[@]}" \
        "${IO_ARGS[@]}" \
        "$@" \
        2>&1
) | tee -a "${LOG_FILE}"

if command -v python3 >/dev/null 2>&1; then
    echo ">> Extracting pruning metrics to ${METRICS_FILE}"
    python3 "${ROOT_DIR}/pruning-optimisation/scripts/collect_pruning_metrics.py" "${LOG_FILE}" > "${METRICS_FILE}"
else
    echo "!! python3 not found; skipping metrics extraction" >&2
fi

echo ">> Done. Raw log: ${LOG_FILE}"
echo ">> Metrics CSV (if produced): ${METRICS_FILE}"
