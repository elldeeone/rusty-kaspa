#!/usr/bin/env bash
set -euo pipefail

# Probe the highest SIMPA_TPB that completes without crashing (full blocks).
# Uses SIMPA_LONG_PAYLOAD=1 by default and stops on first failure.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

: "${SIMPA_LOGLEVEL:=info,simpa::simulator::miner=trace,consensus=trace,pruning_processor=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace}"
: "${SIMPA_BPS:=10}"
: "${SIMPA_MINERS:=4}"
: "${SIMPA_LONG_PAYLOAD:=1}"
: "${SIMPA_RETENTION_DAYS:=0.0046}"
: "${SIMPA_TARGET_BLOCKS:=3000}"
: "${SIMPA_FORCE_REBUILD:=1}"

: "${SIMPA_TPB_START:=200}"
: "${SIMPA_TPB_STEP:=100}"
: "${SIMPA_TPB_MAX:=2000}"
: "${SIMPA_TPB_DESC:=1}"
: "${SIMPA_TPB_LIST:=}"

: "${STOP_ON_FAIL:=0}"

run_case() {
    local tpb="$1"
    local label="fullblocks-tpb${tpb}"

    echo ">> Probe ${label}"
    set +e
    env SIMPA_RUN_LABEL="${label}" \
        SIMPA_LOGLEVEL="${SIMPA_LOGLEVEL}" \
        SIMPA_BPS="${SIMPA_BPS}" \
        SIMPA_MINERS="${SIMPA_MINERS}" \
        SIMPA_TPB="${tpb}" \
        SIMPA_LONG_PAYLOAD="${SIMPA_LONG_PAYLOAD}" \
        SIMPA_RETENTION_DAYS="${SIMPA_RETENTION_DAYS}" \
        SIMPA_TARGET_BLOCKS="${SIMPA_TARGET_BLOCKS}" \
        SIMPA_FORCE_REBUILD="${SIMPA_FORCE_REBUILD}" \
        "${ROOT_DIR}/pruning-optimisation/run-simpa-mainnetish.sh"
    local status=$?
    set -e
    return "${status}"
}

last_ok=""
first_fail=""
best_ok=""

if [[ -n "${SIMPA_TPB_LIST}" ]]; then
    IFS=',' read -r -a tpb_list <<< "${SIMPA_TPB_LIST}"
else
    tpb_list=()
    if [[ "${SIMPA_TPB_DESC}" == "1" ]]; then
        for ((tpb=SIMPA_TPB_MAX; tpb>=SIMPA_TPB_START; tpb-=SIMPA_TPB_STEP)); do
            tpb_list+=("${tpb}")
        done
    else
        for ((tpb=SIMPA_TPB_START; tpb<=SIMPA_TPB_MAX; tpb+=SIMPA_TPB_STEP)); do
            tpb_list+=("${tpb}")
        done
    fi
fi

for tpb in "${tpb_list[@]}"; do
    if run_case "${tpb}"; then
        last_ok="${tpb}"
        if [[ -z "${best_ok}" || "${tpb}" -gt "${best_ok}" ]]; then
            best_ok="${tpb}"
        fi
        echo ">> OK: ${tpb}"
        continue
    fi
    first_fail="${tpb}"
    echo "!! FAIL: ${tpb}"
    if [[ "${STOP_ON_FAIL}" == "1" ]]; then
        break
    fi
done

echo ">> Probe done. last_ok=${last_ok:-none} best_ok=${best_ok:-none} first_fail=${first_fail:-none}"
