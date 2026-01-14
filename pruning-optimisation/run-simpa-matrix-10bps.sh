#!/usr/bin/env bash
set -euo pipefail

# Top-5 simpa runs at 10 BPS / 12k blocks for validation.
# Optional: RUN_FILTER to run only labels that contain the filter string.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

: "${SIMPA_LOGLEVEL:=info,simpa::simulator::miner=trace,consensus=trace,pruning_processor=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace}"
: "${SIMPA_TPB:=200}"
: "${SIMPA_LONG_PAYLOAD:=0}"
: "${SIMPA_BPS:=10}"
: "${SIMPA_MINERS:=4}"
: "${SIMPA_RETENTION_DAYS:=0.0046}"
: "${SIMPA_TARGET_BLOCKS:=12000}"
: "${SIMPA_FORCE_REBUILD:=1}"
: "${SIMPA_RUN_LABEL_SUFFIX:=}"

CONTINUE_ON_ERROR="${CONTINUE_ON_ERROR:-0}"
RUN_FILTER="${RUN_FILTER:-}"

should_run() {
    local label="$1"
    if [[ -z "${RUN_FILTER}" ]]; then
        return 0
    fi
    [[ "${label}" == *"${RUN_FILTER}"* ]]
}

run_case() {
    local label="$1"
    shift
    local label_with_suffix="${label}${SIMPA_RUN_LABEL_SUFFIX}"

    if ! should_run "${label_with_suffix}"; then
        echo ">> Skip ${label_with_suffix} (RUN_FILTER=${RUN_FILTER})"
        return 0
    fi

    echo ">> Run ${label_with_suffix}"
    if ! env SIMPA_RUN_LABEL="${label_with_suffix}" \
        SIMPA_LOGLEVEL="${SIMPA_LOGLEVEL}" \
        SIMPA_TPB="${SIMPA_TPB}" \
        SIMPA_LONG_PAYLOAD="${SIMPA_LONG_PAYLOAD}" \
        SIMPA_BPS="${SIMPA_BPS}" \
        SIMPA_MINERS="${SIMPA_MINERS}" \
        SIMPA_RETENTION_DAYS="${SIMPA_RETENTION_DAYS}" \
        SIMPA_TARGET_BLOCKS="${SIMPA_TARGET_BLOCKS}" \
        SIMPA_FORCE_REBUILD="${SIMPA_FORCE_REBUILD}" \
        "$@" \
        "${ROOT_DIR}/pruning-optimisation/run-simpa-mainnetish.sh"; then
        echo "!! Run failed: ${label}" >&2
        if [[ "${CONTINUE_ON_ERROR}" == "1" ]]; then
            return 0
        fi
        return 1
    fi
}

# Top-5 (ranked by duration_ms_avg at 5 BPS):
# - reach-lock25 (reach-batched only)
# - maxblocks128 (full-batch)
# - maxops50k (full-batch)
# - maxblocks64 (full-batch)
# - reach-maxbytes1m (reach-batched only)

run_case "control-bps10" \
    PRUNE_BATCH_BODIES=1
run_case "reach-lock25-10bps" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=25 PRUNE_BATCH_MAX_DURATION_MS=50
run_case "maxblocks128-10bps" \
    PRUNE_BATCH_MAX_BLOCKS=128
run_case "maxops50k-10bps" \
    PRUNE_BATCH_MAX_OPS=50000
run_case "maxblocks64-10bps" \
    PRUNE_BATCH_MAX_BLOCKS=64
run_case "reach-maxbytes1m-10bps" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BYTES=1048576

# Additional validation cases (reach-batched only)
run_case "reach-maxblocks10-10bps" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BLOCKS=10
run_case "reach-maxblocks15-10bps" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BLOCKS=15
run_case "reach-lock5-10bps" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=5 PRUNE_BATCH_MAX_DURATION_MS=10
run_case "reach-lock10-10bps" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=10 PRUNE_BATCH_MAX_DURATION_MS=20
run_case "reach-lock15-10bps" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=15 PRUNE_BATCH_MAX_DURATION_MS=30
