#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

APPDIR="${APPDIR:-/tmp/udp-lab-app}"
LOG_FILE="${LOG_FILE:-/tmp/kaspad-udp-lab.log}"
TX_FILE="${TX_FILE:-/tmp/udp-orphan-tx.borsh}"
UDP_TARGET="${UDP_TARGET:-127.0.0.1:28515}"
RPC_LISTEN="${RPC_LISTEN:-127.0.0.1:16110}"
UDP_NETWORK="${UDP_NETWORK:-devnet}"
RUST_LOG="${RUST_LOG:-info,kaspa_udp_sidechannel=debug,kaspad::udp=debug}"
KASPAD_BIN="${KASPAD_BIN:-cargo}"
KASPAD_EXTRA_ARGS="${KASPAD_EXTRA_ARGS:-}"

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
  --appdir="${APPDIR}"
)

if [[ "${UDP_NETWORK}" != "devnet" ]]; then
  echo "only devnet supported by this script (UDP_NETWORK=${UDP_NETWORK})" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 missing; needed to build test tx bytes" >&2
  exit 1
fi

mkdir -p "${APPDIR}"
: > "${LOG_FILE}"

if [[ "${KASPAD_BIN}" == "cargo" ]]; then
  KASPAD_CMD=(cargo run -p kaspad --)
else
  KASPAD_CMD=("${KASPAD_BIN}")
fi

cleanup() {
  if [[ -n "${KASPAD_PID:-}" ]] && kill -0 "${KASPAD_PID}" >/dev/null 2>&1; then
    kill "${KASPAD_PID}" >/dev/null 2>&1 || true
    wait "${KASPAD_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

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

for _ in $(seq 1 80); do
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

python3 - <<'PY'
import struct

out_path = "/tmp/udp-orphan-tx.borsh"

u16 = lambda x: struct.pack("<H", x)
u32 = lambda x: struct.pack("<I", x)
u64 = lambda x: struct.pack("<Q", x)

def vec_u8(data: bytes) -> bytes:
    return u32(len(data)) + data

TX_VERSION = 0
MAX_SCRIPT_PUBLIC_KEY_VERSION = 0
MAX_TX_IN_SEQUENCE_NUM = (1 << 64) - 1
SUBNETWORK_ID_NATIVE = bytes([0]) + bytes(19)

pubkey = bytes([0x11] * 32)
script = bytes([0x20]) + pubkey + bytes([0xAC])

prev_txid = bytes([0x22] * 32)
prev_index = 0
signature_script = b""
sequence = MAX_TX_IN_SEQUENCE_NUM
sig_op_count = 1

input_bytes = b"".join(
    [
        prev_txid,
        u32(prev_index),
        vec_u8(signature_script),
        u64(sequence),
        struct.pack("<B", sig_op_count),
    ]
)

value = 100_000_000
output_bytes = b"".join(
    [
        u64(value),
        u16(MAX_SCRIPT_PUBLIC_KEY_VERSION),
        vec_u8(script),
    ]
)

inputs_vec = u32(1) + input_bytes
outputs_vec = u32(1) + output_bytes
lock_time = 0
gas = 0
payload = b""
mass = 0
tx_id = bytes(32)

blob = b"".join(
    [
        u16(TX_VERSION),
        inputs_vec,
        outputs_vec,
        u64(lock_time),
        SUBNETWORK_ID_NATIVE,
        u64(gas),
        vec_u8(payload),
        u64(mass),
        tx_id,
    ]
)

with open(out_path, "wb") as f:
    f.write(blob)

print(out_path, len(blob))
PY

(
  cd "${ROOT_DIR}"
  cargo run -p udp-generator --bin udp-tx-generator -- \
    --tx-file "${TX_FILE}" \
    --target "${UDP_TARGET}" \
    --network "${UDP_NETWORK}" \
    --allow-orphan
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
