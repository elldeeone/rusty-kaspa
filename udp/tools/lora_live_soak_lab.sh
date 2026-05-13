#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TX_SERIAL="/dev/lora-left"
RX_SERIAL="/dev/lora-right"
BAUD="9600"
DURATION_SECONDS="1800"
COUNT=""
INTERVAL_MS="500"
INTER_FRAME_DELAY_MS="2500"
RETRY_COUNT="8"
ACK_TIMEOUT_MS="6000"
SNAPSHOT_EVERY="50"
SESSION_ID="17"
REPORT_PATH="/tmp/lora-live-soak-report.md"
WORKDIR=""
NETWORK="devnet"
PRODUCER_RPC="127.0.0.1:16121"
RECEIVER_RPC="127.0.0.1:16120"
RECEIVER_UDP="127.0.0.1:28520"
BRIDGE_UDP="127.0.0.1:39020"

usage() {
  cat <<USAGE
Usage: $0 [options]

Runs an unattended devnet live DigestV1 LoRa soak:
producer kaspad -> live producer -> lora-bridge tx -> SX126X RF ->
lora-bridge rx -> receiver kaspad UDP ingest -> RPC polling.

Options:
  --tx-serial PATH            TX serial device (default: /dev/lora-left)
  --rx-serial PATH            RX serial device (default: /dev/lora-right)
  --baud BAUD                 UART baud (default: 9600)
  --duration-seconds N        Target soak duration (default: 1800)
  --count N                   Produced datagram count (default: derived from duration)
  --interval-ms N             Producer interval (default: 500)
  --inter-frame-delay-ms N    LoRa TX delay (default: 2500)
  --retry-count N             Reliable fragment retry count (default: 8)
  --ack-timeout-ms N          Reliable fragment ACK timeout (default: 6000)
  --snapshot-every N          Emit a snapshot every N datagrams after first; 0 disables (default: 50)
  --session-id N              Reliable fragment session id (default: 17)
  --report PATH               Markdown report path (default: /tmp/lora-live-soak-report.md)
  --workdir DIR               Scratch directory (default: mktemp)
  --network NAME              Producer network tag (default: devnet)
  -h, --help                  Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tx-serial) TX_SERIAL="${2:-}"; shift 2 ;;
    --rx-serial) RX_SERIAL="${2:-}"; shift 2 ;;
    --baud) BAUD="${2:-}"; shift 2 ;;
    --duration-seconds) DURATION_SECONDS="${2:-}"; shift 2 ;;
    --count) COUNT="${2:-}"; shift 2 ;;
    --interval-ms) INTERVAL_MS="${2:-}"; shift 2 ;;
    --inter-frame-delay-ms) INTER_FRAME_DELAY_MS="${2:-}"; shift 2 ;;
    --retry-count) RETRY_COUNT="${2:-}"; shift 2 ;;
    --ack-timeout-ms) ACK_TIMEOUT_MS="${2:-}"; shift 2 ;;
    --snapshot-every) SNAPSHOT_EVERY="${2:-}"; shift 2 ;;
    --session-id) SESSION_ID="${2:-}"; shift 2 ;;
    --report) REPORT_PATH="${2:-}"; shift 2 ;;
    --workdir) WORKDIR="${2:-}"; shift 2 ;;
    --network) NETWORK="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

if [[ -z "${WORKDIR}" ]]; then
  WORKDIR="$(mktemp -d /tmp/lora-live-soak.XXXXXX)"
fi
mkdir -p "${WORKDIR}" "$(dirname "${REPORT_PATH}")"

if [[ -z "${COUNT}" ]]; then
  # Keep LoRa airtime dominant but derive a count that keeps the run near target duration.
  COUNT=$((DURATION_SECONDS * 1000 / (INTER_FRAME_DELAY_MS + INTERVAL_MS)))
  if [[ "${COUNT}" -lt 1 ]]; then
    COUNT=1
  fi
fi

PRODUCER_APP="${WORKDIR}/producer-app"
RECEIVER_APP="${WORKDIR}/receiver-app"
mkdir -p "${PRODUCER_APP}" "${RECEIVER_APP}"

PRODUCER_KASPAD_LOG="${WORKDIR}/producer-kaspad.log"
RECEIVER_KASPAD_LOG="${WORKDIR}/receiver-kaspad.log"
PRODUCER_LOG="${WORKDIR}/live-producer.log"
TX_LOG="${WORKDIR}/lora-tx.log"
RX_LOG="${WORKDIR}/lora-rx.log"
POLL_LOG="${WORKDIR}/rpc-poll.log"
INFO_JSON="${WORKDIR}/final-info.json"
DIGESTS_JSON="${WORKDIR}/final-digests.json"

PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "${pid}" 2>/dev/null || true
  done
}
trap cleanup EXIT

wait_for_rpc() {
  local rpc_url=$1
  local label=$2
  for _ in $(seq 1 60); do
    if "${ROOT_DIR}/target/debug/udp-rpc-digests" --rpc-url "${rpc_url}" info >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for ${label} RPC at ${rpc_url}" >&2
  return 1
}

echo "starting producer kaspad"
(
  cd "${ROOT_DIR}"
  cargo run -p kaspad --features devnet-prealloc -- \
    --devnet \
    --rpclisten="${PRODUCER_RPC}" \
    --listen=127.0.0.1:0 \
    --appdir="${PRODUCER_APP}" \
    --reset-db --yes \
    --nologfiles \
    --disable-upnp \
    --connect=127.0.0.1:1
) >"${PRODUCER_KASPAD_LOG}" 2>&1 &
PIDS+=("$!")

echo "starting receiver kaspad"
(
  cd "${ROOT_DIR}"
  RUST_LOG='info,kaspa_udp_sidechannel=debug' cargo run -p kaspad --features devnet-prealloc -- \
    --devnet \
    --udp.enable \
    --udp.listen="${RECEIVER_UDP}" \
    --udp.require_signature=true \
    --udp.allowed_signers=4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa \
    --udp.db_migrate=true \
    --udp.retention_count=2000 \
    --udp.retention_days=1 \
    --rpclisten="${RECEIVER_RPC}" \
    --listen=127.0.0.1:0 \
    --appdir="${RECEIVER_APP}" \
    --reset-db --yes \
    --nologfiles \
    --disable-upnp \
    --connect=127.0.0.1:1
) >"${RECEIVER_KASPAD_LOG}" 2>&1 &
PIDS+=("$!")

wait_for_rpc "grpc://${PRODUCER_RPC}" "producer"
wait_for_rpc "grpc://${RECEIVER_RPC}" "receiver"

echo "starting LoRa RX"
"${ROOT_DIR}/target/debug/lora-bridge" rx \
  --serial "${RX_SERIAL}" \
  --baud "${BAUD}" \
  --output udp \
  --udp-target "${RECEIVER_UDP}" \
  --count "${COUNT}" \
  --timeout-ms $((DURATION_SECONDS * 1000 + 120000)) \
  --packet-idle-ms 600 \
  --session-id "${SESSION_ID}" \
  >"${RX_LOG}" 2>&1 &
RX_PID=$!
PIDS+=("${RX_PID}")

sleep 1
echo "starting LoRa TX"
"${ROOT_DIR}/target/debug/lora-bridge" tx \
  --serial "${TX_SERIAL}" \
  --baud "${BAUD}" \
  --input udp \
  --udp-bind "${BRIDGE_UDP}" \
  --count "${COUNT}" \
  --reliable-fragments \
  --retry-count "${RETRY_COUNT}" \
  --ack-timeout-ms "${ACK_TIMEOUT_MS}" \
  --session-id "${SESSION_ID}" \
  --inter-frame-delay-ms "${INTER_FRAME_DELAY_MS}" \
  >"${TX_LOG}" 2>&1 &
TX_PID=$!
PIDS+=("${TX_PID}")

sleep 1
echo "starting live producer count=${COUNT}"
"${ROOT_DIR}/target/debug/udp-live-digest-producer" \
  --rpc-url "grpc://${PRODUCER_RPC}" \
  --network "${NETWORK}" \
  --output udp \
  --udp-target "${BRIDGE_UDP}" \
  --count "${COUNT}" \
  --interval-ms "${INTERVAL_MS}" \
  --snapshot-first \
  --snapshot-every "${SNAPSHOT_EVERY}" \
  --lab-progress-counter \
  >"${PRODUCER_LOG}" 2>&1 &
PRODUCER_PID=$!
PIDS+=("${PRODUCER_PID}")

while kill -0 "${RX_PID}" 2>/dev/null; do
  date -u '+%Y-%m-%dT%H:%M:%SZ' >>"${POLL_LOG}"
  "${ROOT_DIR}/target/debug/udp-rpc-digests" --rpc-url "grpc://${RECEIVER_RPC}" info >>"${POLL_LOG}" 2>&1 || true
  sleep 30
done

wait "${PRODUCER_PID}"
wait "${TX_PID}"
wait "${RX_PID}"

"${ROOT_DIR}/target/debug/udp-rpc-digests" --rpc-url "grpc://${RECEIVER_RPC}" info >"${INFO_JSON}"
"${ROOT_DIR}/target/debug/udp-rpc-digests" --rpc-url "grpc://${RECEIVER_RPC}" digests --limit 10 >"${DIGESTS_JSON}"

{
  echo "# LoRa Live Soak Report"
  echo
  echo "Generated: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo
  echo "## Configuration"
  echo
  echo "- Duration target: \`${DURATION_SECONDS}\` seconds"
  echo "- Produced datagrams: \`${COUNT}\`"
  echo "- TX serial: \`${TX_SERIAL}\`"
  echo "- RX serial: \`${RX_SERIAL}\`"
  echo "- Inter-frame delay: \`${INTER_FRAME_DELAY_MS}\` ms"
  echo "- Retry count: \`${RETRY_COUNT}\`"
  echo "- ACK timeout: \`${ACK_TIMEOUT_MS}\` ms"
  echo "- Snapshot every: \`${SNAPSHOT_EVERY}\`"
  echo "- Session id: \`${SESSION_ID}\`"
  echo "- Workdir: \`${WORKDIR}\`"
  echo
  echo "## Bridge Summary"
  echo
  rg 'bridge_summary' "${TX_LOG}" "${RX_LOG}" || true
  echo
  echo "## Kaspad Ingest Summary"
  echo
  rg '\"framesReceived\"|\"bytesTotal\"|\"lastDigest\"|\"signatureFailures\"|\"sourceCount\"' "${INFO_JSON}" || true
  echo
  echo "## Recent Digests"
  echo
  sed -n '1,160p' "${DIGESTS_JSON}"
  echo
  echo "## Logs"
  echo
  echo "- Producer kaspad: \`${PRODUCER_KASPAD_LOG}\`"
  echo "- Receiver kaspad: \`${RECEIVER_KASPAD_LOG}\`"
  echo "- Live producer: \`${PRODUCER_LOG}\`"
  echo "- LoRa TX: \`${TX_LOG}\`"
  echo "- LoRa RX: \`${RX_LOG}\`"
  echo "- RPC poll: \`${POLL_LOG}\`"
} >"${REPORT_PATH}"

echo "wrote report: ${REPORT_PATH}"
