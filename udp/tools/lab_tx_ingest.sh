#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

APPDIR="${APPDIR:-}"
LOG_FILE="${LOG_FILE:-/tmp/kaspad-udp-lab.log}"
TX_FILE="${TX_FILE:-/tmp/udp-signed-tx.borsh}"
UDP_TARGET="${UDP_TARGET:-127.0.0.1:28515}"
RPC_LISTEN="${RPC_LISTEN:-127.0.0.1:16110}"
UDP_NETWORK="${UDP_NETWORK:-devnet}"
RUST_LOG="${RUST_LOG:-info,kaspa_udp_sidechannel=debug,kaspad::udp=debug}"
KASPAD_BIN="${KASPAD_BIN:-cargo}"
KASPAD_FEATURES="${KASPAD_FEATURES:-devnet-prealloc}"
KASPAD_EXTRA_ARGS="${KASPAD_EXTRA_ARGS:-}"
PRIVKEY_HEX="${PRIVKEY_HEX:-0101010101010101010101010101010101010101010101010101010101010101}"
PREALLOC_AMOUNT="${PREALLOC_AMOUNT:-10000000000}"
NUM_PREALLOC_UTXOS="${NUM_PREALLOC_UTXOS:-1}"
PREALLOC_TXID="${PREALLOC_TXID:-1}"
FEE_SOMPI="${FEE_SOMPI:-1000000}"
BIND_WAIT_SECS="${BIND_WAIT_SECS:-60}"
RESET_DB="${RESET_DB:-true}"
CLEAN_APPDIR="${CLEAN_APPDIR:-false}"

KASPAD_FLAGS=(
  --devnet
  --unsaferpc
  --udp.enable
  --udp.tx_enable
  --udp.listen="${UDP_TARGET}"
  --udp.require_signature=false
  --udp.db_migrate=true
  --rpclisten="${RPC_LISTEN}"
  --nologfiles
  --disable-upnp
  --connect=127.0.0.1:1
)

if [[ "${UDP_NETWORK}" != "devnet" ]]; then
  echo "only devnet supported by this script (UDP_NETWORK=${UDP_NETWORK})" >&2
  exit 1
fi

APPDIR_CREATED=false
if [[ -z "${APPDIR}" ]]; then
  APPDIR="$(mktemp -d /tmp/udp-lab-app.XXXXXX)"
  APPDIR_CREATED=true
fi
mkdir -p "${APPDIR}"
: > "${LOG_FILE}"

KASPAD_FLAGS+=(--appdir="${APPDIR}")

if [[ "${KASPAD_BIN}" == "cargo" ]]; then
  KASPAD_CMD=(cargo run -p kaspad --features "${KASPAD_FEATURES}" --)
else
  KASPAD_CMD=("${KASPAD_BIN}")
fi

cleanup() {
  if [[ -n "${KASPAD_PID:-}" ]] && kill -0 "${KASPAD_PID}" >/dev/null 2>&1; then
    kill "${KASPAD_PID}" >/dev/null 2>&1 || true
    wait "${KASPAD_PID}" >/dev/null 2>&1 || true
  fi
  if [[ "${APPDIR_CREATED}" == "true" && "${CLEAN_APPDIR}" == "true" ]]; then
    if command -v trash >/dev/null 2>&1; then
      trash "${APPDIR}" >/dev/null 2>&1 || true
    fi
  fi
}
trap cleanup EXIT

PREALLOC_ADDRESS="$(
  cd "${ROOT_DIR}"
  cargo run -p udp-generator --bin udp-tx-build -- \
    --privkey-hex "${PRIVKEY_HEX}" \
    --network "${UDP_NETWORK}" \
    --print-address
)"

if [[ -z "${PREALLOC_ADDRESS}" ]]; then
  echo "failed to derive prealloc address" >&2
  exit 1
fi

KASPAD_FLAGS+=(
  --num-prealloc-utxos="${NUM_PREALLOC_UTXOS}"
  --prealloc-address="${PREALLOC_ADDRESS}"
  --prealloc-amount="${PREALLOC_AMOUNT}"
)

if [[ "${RESET_DB}" == "true" ]]; then
  KASPAD_FLAGS+=(--reset-db --yes)
fi

(
  cd "${ROOT_DIR}"
  if [[ -n "${KASPAD_EXTRA_ARGS}" ]]; then
    read -r -a EXTRA_ARGS_EXPANDED <<< "${KASPAD_EXTRA_ARGS}"
    RUST_LOG="${RUST_LOG}" "${KASPAD_CMD[@]}" "${KASPAD_FLAGS[@]}" "${EXTRA_ARGS_EXPANDED[@]}" >"${LOG_FILE}" 2>&1 &
  else
    RUST_LOG="${RUST_LOG}" "${KASPAD_CMD[@]}" "${KASPAD_FLAGS[@]}" >"${LOG_FILE}" 2>&1 &
  fi
  KASPAD_PID=$!
  echo "${KASPAD_PID}" > /tmp/kaspad-udp-lab.pid
  disown "${KASPAD_PID}"
)

for _ in $(seq 1 $((BIND_WAIT_SECS * 4))); do
  if grep -q "udp.event=bind_ok" "${LOG_FILE}"; then
    break
  fi
  if grep -q "udp.event=bind_fail" "${LOG_FILE}"; then
    echo "udp bind failed" >&2
    tail -n 80 "${LOG_FILE}" >&2
    exit 1
  fi
  sleep 0.25
done

if ! grep -q "udp.event=bind_ok" "${LOG_FILE}"; then
  echo "udp bind not observed" >&2
  tail -n 80 "${LOG_FILE}" >&2
  exit 1
fi

(
  cd "${ROOT_DIR}"
  cargo run -p udp-generator --bin udp-tx-build -- \
    --privkey-hex "${PRIVKEY_HEX}" \
    --network "${UDP_NETWORK}" \
    --prev-utxo-id "${PREALLOC_TXID}" \
    --prev-utxo-index 0 \
    --prev-amount "${PREALLOC_AMOUNT}" \
    --fee-sompi "${FEE_SOMPI}" \
    --dest-address "${PREALLOC_ADDRESS}" \
    --out "${TX_FILE}"

  cargo run -p udp-generator --bin udp-tx-generator -- \
    --tx-file "${TX_FILE}" \
    --target "${UDP_TARGET}" \
    --network "${UDP_NETWORK}"
)

for _ in $(seq 1 80); do
  if grep -q "udp.event=tx_submit_ok" "${LOG_FILE}"; then
    echo "tx submit ok"
    tail -n 30 "${LOG_FILE}"
    exit 0
  fi
  if grep -q "udp.event=tx_submit_fail" "${LOG_FILE}"; then
    echo "tx submit failed" >&2
    tail -n 80 "${LOG_FILE}" >&2
    exit 1
  fi
  sleep 0.25
done

echo "tx submit not observed" >&2
tail -n 80 "${LOG_FILE}" >&2
exit 1
