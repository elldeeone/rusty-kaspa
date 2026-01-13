#!/usr/bin/env bash
set -euo pipefail

# Simpa pruning matrix runner (relative comparisons). Defaults to BPS=5 to avoid
# test-pruning DAA window panics at 10 BPS with the modern sampling rules.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

: "${SIMPA_LOGLEVEL:=info,simpa::simulator::miner=trace,consensus=trace,pruning_processor=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace}"
: "${SIMPA_TPB:=200}"
: "${SIMPA_LONG_PAYLOAD:=0}"
: "${SIMPA_BPS:=5}"
: "${SIMPA_MINERS:=4}"
: "${SIMPA_RETENTION_DAYS:=0.0046}"
: "${SIMPA_TARGET_BLOCKS:=6000}"
: "${SIMPA_FORCE_REBUILD:=1}"

RUN_FILTER="${RUN_FILTER:-}"
CONTINUE_ON_ERROR="${CONTINUE_ON_ERROR:-0}"

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

    if ! should_run "${label}"; then
        echo ">> Skip ${label} (RUN_FILTER=${RUN_FILTER})"
        return 0
    fi

    echo ">> Run ${label}"
    if ! env SIMPA_RUN_LABEL="${label}" \
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

# Group A - baselines
run_case "control-bps${SIMPA_BPS}" \
    PRUNE_BATCH_BODIES=1
run_case "reach-only" \
    PRUNE_BATCH_BODIES=0
run_case "unbatched" \
    PRUNE_BATCH_MAX_BLOCKS=1 PRUNE_BATCH_MAX_OPS=1 PRUNE_BATCH_MAX_BYTES=1 PRUNE_BATCH_MAX_DURATION_MS=0

# Group B - lock/batch timing
run_case "lock10" \
    PRUNE_LOCK_MAX_DURATION_MS=10 PRUNE_BATCH_MAX_DURATION_MS=20
run_case "lock25" \
    PRUNE_LOCK_MAX_DURATION_MS=25 PRUNE_BATCH_MAX_DURATION_MS=50
run_case "lock50" \
    PRUNE_LOCK_MAX_DURATION_MS=50 PRUNE_BATCH_MAX_DURATION_MS=100
run_case "lock100" \
    PRUNE_LOCK_MAX_DURATION_MS=100 PRUNE_BATCH_MAX_DURATION_MS=200
run_case "lock500" \
    PRUNE_LOCK_MAX_DURATION_MS=500 PRUNE_BATCH_MAX_DURATION_MS=1000

# Group C - batch max blocks
run_case "maxblocks10" \
    PRUNE_BATCH_MAX_BLOCKS=10
run_case "maxblocks15" \
    PRUNE_BATCH_MAX_BLOCKS=15
run_case "maxblocks64" \
    PRUNE_BATCH_MAX_BLOCKS=64
run_case "maxblocks128" \
    PRUNE_BATCH_MAX_BLOCKS=128
run_case "maxblocks256" \
    PRUNE_BATCH_MAX_BLOCKS=256

# Group D - batch max ops
run_case "maxops10k" \
    PRUNE_BATCH_MAX_OPS=10000
run_case "maxops50k" \
    PRUNE_BATCH_MAX_OPS=50000
run_case "maxops100k" \
    PRUNE_BATCH_MAX_OPS=100000

# Group E - batch max bytes
run_case "maxbytes1m" \
    PRUNE_BATCH_MAX_BYTES=1048576
run_case "maxbytes4m" \
    PRUNE_BATCH_MAX_BYTES=4194304
run_case "maxbytes16m" \
    PRUNE_BATCH_MAX_BYTES=16777216

# Repeat Groups B-E with reachability-only pruning
run_case "reach-lock10" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=10 PRUNE_BATCH_MAX_DURATION_MS=20
run_case "reach-lock25" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=25 PRUNE_BATCH_MAX_DURATION_MS=50
run_case "reach-lock50" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=50 PRUNE_BATCH_MAX_DURATION_MS=100
run_case "reach-lock100" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=100 PRUNE_BATCH_MAX_DURATION_MS=200
run_case "reach-lock500" \
    PRUNE_BATCH_BODIES=0 PRUNE_LOCK_MAX_DURATION_MS=500 PRUNE_BATCH_MAX_DURATION_MS=1000

run_case "reach-maxblocks10" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BLOCKS=10
run_case "reach-maxblocks15" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BLOCKS=15
run_case "reach-maxblocks64" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BLOCKS=64
run_case "reach-maxblocks128" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BLOCKS=128
run_case "reach-maxblocks256" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BLOCKS=256

run_case "reach-maxops10k" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_OPS=10000
run_case "reach-maxops50k" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_OPS=50000
run_case "reach-maxops100k" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_OPS=100000

run_case "reach-maxbytes1m" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BYTES=1048576
run_case "reach-maxbytes4m" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BYTES=4194304
run_case "reach-maxbytes16m" \
    PRUNE_BATCH_BODIES=0 PRUNE_BATCH_MAX_BYTES=16777216
