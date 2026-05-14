#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TX_SERIAL="/dev/lora-left"
RX_SERIAL="/dev/lora-right"
BAUD="9600"
DURATION_SECONDS="600"
COUNT=""
INTERVAL_MS="5000"
INTER_FRAME_DELAY_MS="2500"
RETRY_COUNT="8"
ACK_TIMEOUT_MS="6000"
SNAPSHOT_EVERY="10"
EXPECTED_DATAGRAM_MS="6500"
SESSION_ID="17"
SIGNER_ID="0"
REPORT_PATH="/tmp/lora-testnet-live-node-lab.md"
WORKDIR=""
NETWORK="testnet-10"
PRODUCER_RPC="127.0.0.1:16221"
RECEIVER_RPC="127.0.0.1:16220"
RECEIVER_UDP="127.0.0.1:28620"
BRIDGE_UDP="127.0.0.1:39120"
EXTERNAL_PRODUCER_RPC=""
START_RECEIVER="1"
START_PRODUCER="1"
PRODUCER_EXTRA_ARGS=()
RECEIVER_EXTRA_ARGS=()
POST_RX_WAIT_SECONDS="8"
RPC_WAIT_SECONDS="90"
SYNC_WAIT_SECONDS="900"
REQUIRE_SYNCED="1"

usage() {
  cat <<USAGE
Usage: $0 [options]

Runs a real testnet-10 live DigestV1 LoRa lab:
synced producer kaspad RPC -> live producer -> lora-bridge tx -> SX126X RF ->
lora-bridge rx -> receiver kaspad UDP ingest -> RPC/reporting.

Fresh testnet nodes can take hours to sync. For the intended real-network
validation, prefer --external-producer-rpc pointing at an already-synced
testnet-10 kaspad gRPC endpoint. If you need to sync locally, pass a persistent
--workdir and a long --sync-wait-seconds so the appdir can be reused.

Options:
  --producer-rpc HOST:PORT       Producer kaspad gRPC host:port (default: 127.0.0.1:16221)
  --external-producer-rpc URL    Use existing producer gRPC URL and do not start producer kaspad
  --receiver-rpc HOST:PORT       Receiver gRPC host:port (default: 127.0.0.1:16220)
  --receiver-udp HOST:PORT       Receiver UDP ingest bind (default: 127.0.0.1:28620)
  --bridge-udp HOST:PORT         Local UDP socket used by lora-bridge tx (default: 127.0.0.1:39120)
  --duration-seconds N           Target soak duration (default: 600)
  --count N                      Produced datagram count (default: derived from duration)
  --interval-ms N                Producer interval (default: 5000)
  --snapshot-every N             Emit snapshot every N datagrams after first; 0 disables (default: 10)
  --report PATH                  Markdown report path
  --workdir DIR                  Scratch/appdir directory; use a persistent path for long local sync
  --rpc-wait-seconds N           Wait for producer/receiver RPC startup (default: 90)
  --sync-wait-seconds N          Wait for producer sync before failing (default: 900)
  --no-require-synced            Continue even if producer is not synced; report will mark this
  --no-start-receiver            Use existing receiver RPC/UDP instead of starting kaspad
  --producer-extra-arg ARG       Extra arg for started producer kaspad; may repeat
  --receiver-extra-arg ARG       Extra arg for started receiver kaspad; may repeat
  --tx-serial PATH               TX serial device (default: /dev/lora-left)
  --rx-serial PATH               RX serial device (default: /dev/lora-right)
  --baud BAUD                    UART baud (default: 9600)
  -h, --help                     Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --producer-rpc) PRODUCER_RPC="${2:-}"; shift 2 ;;
    --external-producer-rpc) EXTERNAL_PRODUCER_RPC="${2:-}"; START_PRODUCER="0"; shift 2 ;;
    --receiver-rpc) RECEIVER_RPC="${2:-}"; shift 2 ;;
    --receiver-udp) RECEIVER_UDP="${2:-}"; shift 2 ;;
    --bridge-udp) BRIDGE_UDP="${2:-}"; shift 2 ;;
    --duration-seconds) DURATION_SECONDS="${2:-}"; shift 2 ;;
    --count) COUNT="${2:-}"; shift 2 ;;
    --interval-ms) INTERVAL_MS="${2:-}"; shift 2 ;;
    --snapshot-every) SNAPSHOT_EVERY="${2:-}"; shift 2 ;;
    --report) REPORT_PATH="${2:-}"; shift 2 ;;
    --workdir) WORKDIR="${2:-}"; shift 2 ;;
    --rpc-wait-seconds) RPC_WAIT_SECONDS="${2:-}"; shift 2 ;;
    --sync-wait-seconds) SYNC_WAIT_SECONDS="${2:-}"; shift 2 ;;
    --no-require-synced) REQUIRE_SYNCED="0"; shift ;;
    --no-start-receiver) START_RECEIVER="0"; shift ;;
    --producer-extra-arg) PRODUCER_EXTRA_ARGS+=("${2:-}"); shift 2 ;;
    --receiver-extra-arg) RECEIVER_EXTRA_ARGS+=("${2:-}"); shift 2 ;;
    --tx-serial) TX_SERIAL="${2:-}"; shift 2 ;;
    --rx-serial) RX_SERIAL="${2:-}"; shift 2 ;;
    --baud) BAUD="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

if [[ -z "${WORKDIR}" ]]; then
  WORKDIR="$(mktemp -d /tmp/lora-testnet-live.XXXXXX)"
fi
mkdir -p "${WORKDIR}" "$(dirname "${REPORT_PATH}")"

if [[ -z "${COUNT}" ]]; then
  COUNT=$((DURATION_SECONDS * 1000 / EXPECTED_DATAGRAM_MS))
  if [[ "${COUNT}" -lt 1 ]]; then
    COUNT=1
  fi
fi

RX_TIMEOUT_MS=$((COUNT * EXPECTED_DATAGRAM_MS + 180000))
MIN_RX_TIMEOUT_MS=$((DURATION_SECONDS * 1000 + 180000))
if [[ "${RX_TIMEOUT_MS}" -lt "${MIN_RX_TIMEOUT_MS}" ]]; then
  RX_TIMEOUT_MS="${MIN_RX_TIMEOUT_MS}"
fi

PRODUCER_APP="${WORKDIR}/producer-app"
RECEIVER_APP="${WORKDIR}/receiver-app"
mkdir -p "${PRODUCER_APP}" "${RECEIVER_APP}"

PRODUCER_RPC_URL="${EXTERNAL_PRODUCER_RPC:-grpc://${PRODUCER_RPC}}"
RECEIVER_RPC_URL="grpc://${RECEIVER_RPC}"
PRODUCER_KASPAD_LOG="${WORKDIR}/producer-kaspad.log"
RECEIVER_KASPAD_LOG="${WORKDIR}/receiver-kaspad.log"
PRODUCER_LOG="${WORKDIR}/live-producer.log"
TX_LOG="${WORKDIR}/lora-tx.log"
RX_LOG="${WORKDIR}/lora-rx.log"
POLL_LOG="${WORKDIR}/rpc-poll.log"
PRODUCER_INFO_BEFORE="${WORKDIR}/producer-info-before.json"
PRODUCER_INFO_AFTER="${WORKDIR}/producer-info-after.json"
RECEIVER_INFO_FINAL="${WORKDIR}/receiver-info-final.json"
INGEST_INFO_JSON="${WORKDIR}/final-ingest-info.json"
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
  for _ in $(seq 1 "${RPC_WAIT_SECONDS}"); do
    if "${ROOT_DIR}/target/debug/udp-rpc-node-info" --rpc-url "${rpc_url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for ${label} RPC at ${rpc_url}" >&2
  return 1
}

node_network_matches() {
  local info_path=$1
  rg -q "\"networkId\": \"${NETWORK}\"" "${info_path}" && rg -q "\"dagNetwork\": \"${NETWORK}\"" "${info_path}"
}

wait_for_synced() {
  local rpc_url=$1
  local out=$2
  local deadline=$((SECONDS + SYNC_WAIT_SECONDS))
  while [[ "${SECONDS}" -le "${deadline}" ]]; do
    if "${ROOT_DIR}/target/debug/udp-rpc-node-info" --rpc-url "${rpc_url}" >"${out}.tmp" 2>"${out}.err"; then
      mv "${out}.tmp" "${out}"
      if node_network_matches "${out}" && rg -q '"serverInfoSynced": true|"syncStatusSynced": true' "${out}"; then
        return 0
      fi
    fi
    local remaining=$((deadline - SECONDS))
    if [[ "${remaining}" -le 0 ]]; then
      break
    fi
    if [[ "${remaining}" -lt 10 ]]; then
      sleep "${remaining}"
    else
      sleep 10
    fi
  done
  [[ -f "${out}.tmp" ]] && mv "${out}.tmp" "${out}" || true
  return 1
}

if [[ "${START_PRODUCER}" == "1" ]]; then
  echo "starting producer testnet kaspad"
  (
    cd "${ROOT_DIR}"
    cargo run -p kaspad -- \
      --testnet \
      --netsuffix=10 \
      --rpclisten="${PRODUCER_RPC}" \
      --listen=127.0.0.1:0 \
      --appdir="${PRODUCER_APP}" \
      --nologfiles \
      --disable-upnp \
      "${PRODUCER_EXTRA_ARGS[@]}"
  ) >"${PRODUCER_KASPAD_LOG}" 2>&1 &
  PIDS+=("$!")
fi

wait_for_rpc "${PRODUCER_RPC_URL}" "producer"

echo "waiting for producer sync evidence"
PRODUCER_SYNC_OK="0"
if wait_for_synced "${PRODUCER_RPC_URL}" "${PRODUCER_INFO_BEFORE}"; then
  PRODUCER_SYNC_OK="1"
elif [[ "${REQUIRE_SYNCED}" == "1" ]]; then
  echo "producer did not report synced ${NETWORK} state within ${SYNC_WAIT_SECONDS}s" >&2
  exit 1
fi
if [[ -f "${PRODUCER_INFO_BEFORE}" ]] && ! node_network_matches "${PRODUCER_INFO_BEFORE}"; then
  echo "producer RPC is not on ${NETWORK}" >&2
  exit 1
fi

if [[ "${START_RECEIVER}" == "1" ]]; then
  echo "starting receiver testnet kaspad"
  (
    cd "${ROOT_DIR}"
    RUST_LOG='info,kaspa_udp_sidechannel=debug' cargo run -p kaspad -- \
      --testnet \
      --netsuffix=10 \
      --udp.enable \
      --udp.listen="${RECEIVER_UDP}" \
      --udp.require_signature=true \
      --udp.allowed_signers=4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa \
      --udp.db_migrate=true \
      --udp.retention_count=5000 \
      --udp.retention_days=1 \
      --rpclisten="${RECEIVER_RPC}" \
      --listen=127.0.0.1:0 \
      --appdir="${RECEIVER_APP}" \
      --nologfiles \
      --disable-upnp \
      "${RECEIVER_EXTRA_ARGS[@]}"
  ) >"${RECEIVER_KASPAD_LOG}" 2>&1 &
  PIDS+=("$!")
fi

wait_for_rpc "${RECEIVER_RPC_URL}" "receiver"

"${ROOT_DIR}/target/debug/udp-rpc-node-info" --rpc-url "${RECEIVER_RPC_URL}" >"${RECEIVER_INFO_FINAL}" 2>&1 || true
if ! node_network_matches "${RECEIVER_INFO_FINAL}"; then
  echo "receiver RPC is not on ${NETWORK}" >&2
  exit 1
fi

echo "starting LoRa RX"
"${ROOT_DIR}/target/debug/lora-bridge" rx \
  --serial "${RX_SERIAL}" \
  --baud "${BAUD}" \
  --output udp \
  --udp-target "${RECEIVER_UDP}" \
  --count "${COUNT}" \
  --timeout-ms "${RX_TIMEOUT_MS}" \
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
  --reliable-all \
  --retry-count "${RETRY_COUNT}" \
  --ack-timeout-ms "${ACK_TIMEOUT_MS}" \
  --session-id "${SESSION_ID}" \
  --inter-frame-delay-ms "${INTER_FRAME_DELAY_MS}" \
  >"${TX_LOG}" 2>&1 &
TX_PID=$!
PIDS+=("${TX_PID}")

sleep 1
echo "starting live testnet producer count=${COUNT}"
"${ROOT_DIR}/target/debug/udp-live-digest-producer" \
  --rpc-url "${PRODUCER_RPC_URL}" \
  --network "${NETWORK}" \
  --output udp \
  --udp-target "${BRIDGE_UDP}" \
  --count "${COUNT}" \
  --interval-ms "${INTERVAL_MS}" \
  --signer-id "${SIGNER_ID}" \
  --snapshot-first \
  --snapshot-every "${SNAPSHOT_EVERY}" \
  --provenance-report \
  >"${PRODUCER_LOG}" 2>&1 &
PRODUCER_PID=$!
PIDS+=("${PRODUCER_PID}")

while kill -0 "${RX_PID}" 2>/dev/null; do
  date -u '+%Y-%m-%dT%H:%M:%SZ' >>"${POLL_LOG}"
  "${ROOT_DIR}/target/debug/udp-rpc-digests" --rpc-url "${RECEIVER_RPC_URL}" info >>"${POLL_LOG}" 2>&1 || true
  sleep 30
done

PRODUCER_STATUS=0
TX_STATUS=0
RX_STATUS=0
wait "${PRODUCER_PID}" || PRODUCER_STATUS=$?
wait "${TX_PID}" || TX_STATUS=$?
wait "${RX_PID}" || RX_STATUS=$?

if [[ "${POST_RX_WAIT_SECONDS}" -gt 0 ]]; then
  sleep "${POST_RX_WAIT_SECONDS}"
fi

"${ROOT_DIR}/target/debug/udp-rpc-node-info" --rpc-url "${PRODUCER_RPC_URL}" >"${PRODUCER_INFO_AFTER}" 2>&1 || true
"${ROOT_DIR}/target/debug/udp-rpc-node-info" --rpc-url "${RECEIVER_RPC_URL}" >"${RECEIVER_INFO_FINAL}" 2>&1 || true
INGEST_STATUS=0
DIGESTS_STATUS=0
"${ROOT_DIR}/target/debug/udp-rpc-digests" --rpc-url "${RECEIVER_RPC_URL}" info >"${INGEST_INFO_JSON}" 2>&1 || INGEST_STATUS=$?
"${ROOT_DIR}/target/debug/udp-rpc-digests" --rpc-url "${RECEIVER_RPC_URL}" digests --limit 10 --check-monotonic --producer-log "${PRODUCER_LOG}" --compare-local >"${DIGESTS_JSON}" 2>&1 || DIGESTS_STATUS=$?

{
  echo "# LoRa Testnet Live Node Lab"
  echo
  echo "Generated: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo
  echo "Result: $([[ "${PRODUCER_STATUS}" -eq 0 && "${TX_STATUS}" -eq 0 && "${RX_STATUS}" -eq 0 && "${INGEST_STATUS}" -eq 0 && "${DIGESTS_STATUS}" -eq 0 ]] && echo complete || echo failed)"
  echo
  echo "## Configuration"
  echo
  echo "- Network: \`${NETWORK}\`"
  echo "- Producer RPC: \`${PRODUCER_RPC_URL}\`"
  echo "- Producer sync observed: \`${PRODUCER_SYNC_OK}\`"
  echo "- Receiver RPC: \`${RECEIVER_RPC_URL}\`"
  echo "- Receiver UDP: \`${RECEIVER_UDP}\`"
  echo "- Duration target: \`${DURATION_SECONDS}\` seconds"
  echo "- Produced datagrams: \`${COUNT}\`"
  echo "- Snapshot every: \`${SNAPSHOT_EVERY}\`"
  echo "- Lab progress counter: \`0\`"
  echo "- TX serial: \`${TX_SERIAL}\`"
  echo "- RX serial: \`${RX_SERIAL}\`"
  echo "- Inter-frame delay: \`${INTER_FRAME_DELAY_MS}\` ms"
  echo "- Retry count: \`${RETRY_COUNT}\`"
  echo "- ACK timeout: \`${ACK_TIMEOUT_MS}\` ms"
  echo "- Workdir: \`${WORKDIR}\`"
  echo
  echo "## Producer Node State Before"
  echo
  sed -n '1,120p' "${PRODUCER_INFO_BEFORE}" || true
  echo
  echo "## Producer Node State After"
  echo
  sed -n '1,120p' "${PRODUCER_INFO_AFTER}" || true
  echo
  echo "## Receiver Node State"
  echo
  sed -n '1,120p' "${RECEIVER_INFO_FINAL}" || true
  echo
  echo "## Bridge Summary"
  echo
  rg 'bridge_summary' "${TX_LOG}" "${RX_LOG}" || true
  echo
  echo "## Process Status"
  echo
  echo "- live producer exit: \`${PRODUCER_STATUS}\`"
  echo "- lora tx exit: \`${TX_STATUS}\`"
  echo "- lora rx exit: \`${RX_STATUS}\`"
  echo "- ingest RPC check exit: \`${INGEST_STATUS}\`"
  echo "- digest comparison exit: \`${DIGESTS_STATUS}\`"
  echo
  echo "## Receiver Ingest Summary"
  echo
  rg '\"framesReceived\"|\"bytesTotal\"|\"lastDigest\"|\"signatureFailures\"|\"sourceCount\"|\"divergence\"' "${INGEST_INFO_JSON}" || true
  echo
  echo "## Recent Digests And Comparison"
  echo
  sed -n '1,180p' "${DIGESTS_JSON}"
  echo
  echo "## Producer Provenance Sample"
  echo
  rg 'field provenance|udp_live_digest_provenance|produced index=' "${PRODUCER_LOG}" | sed -n '1,40p' || true
  echo
  echo "## Semantic Audit"
  echo
  echo "- Real testnet RPC fields: pruning point, sink/virtual selected parent, sink blue score, virtual DAA score, sink-header blue work, and sink-header UTXO commitment when getBlock(sink) succeeds."
  echo "- Placeholder/lab-derived fields: pruning_proof_commitment remains a deterministic placeholder; frame_timestamp_ms, signer_id, source_id, and signature key are lab/operator derived; kept_headers_mmr_root is omitted."
  echo "- No lab progress counter was used, so epoch, DAA score, and virtual blue score are direct node/RPC state."
  echo "- Receiver divergence/agreement depends on receiver sync state. A receiver at the same sink should agree; an unsynced or differently synced receiver may report divergence while still proving ingest/storage."
  echo "- This remains advisory ingest only and is not a production-readiness or mainnet claim."
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

if [[ "${PRODUCER_STATUS}" -ne 0 || "${TX_STATUS}" -ne 0 || "${RX_STATUS}" -ne 0 || "${INGEST_STATUS}" -ne 0 || "${DIGESTS_STATUS}" -ne 0 ]]; then
  exit 1
fi
