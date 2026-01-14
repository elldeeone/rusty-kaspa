#!/usr/bin/env bash
set -euo pipefail

# 10 BPS matrix with fuller blocks (long payload + higher TPB).

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

: "${SIMPA_TPB:=1400}"
: "${SIMPA_LONG_PAYLOAD:=1}"
: "${SIMPA_RUN_LABEL_SUFFIX:=-fullblocks}"

exec env SIMPA_TPB="${SIMPA_TPB}" \
    SIMPA_LONG_PAYLOAD="${SIMPA_LONG_PAYLOAD}" \
    SIMPA_RUN_LABEL_SUFFIX="${SIMPA_RUN_LABEL_SUFFIX}" \
    "${ROOT_DIR}/pruning-optimisation/run-simpa-matrix-10bps.sh"
