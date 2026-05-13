#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TX_SERIAL="/dev/lora-left"
RX_SERIAL="/dev/lora-right"
BAUD="9600"
DELAYS="250,500,750,1000,1250,1500"
DELTA_COUNT="50"
SNAPSHOT_COUNT="50"
RX_TIMEOUT_MS="30000"
PACKET_IDLE_MS="600"
REPORT_PATH="/tmp/lora-reliability-report.md"
WORKDIR=""
NETWORK="devnet"
BRIDGE_BIN="${ROOT_DIR}/target/debug/lora-bridge"
FIXTURE_BIN="${ROOT_DIR}/target/debug/udp-digest-fixtures"

usage() {
  cat <<USAGE
Usage: $0 [options]

Runs repeated byte-equality LoRa bridge tests over real SX126X UART hardware.

Options:
  --tx-serial PATH         TX serial device (default: /dev/lora-left)
  --rx-serial PATH         RX serial device (default: /dev/lora-right)
  --baud BAUD             UART baud (default: 9600)
  --delays LIST           Comma-separated inter-frame delays in ms
                           (default: 250,500,750,1000,1250,1500)
  --delta-count N         Delta samples per delay (default: 50)
  --snapshot-count N      Snapshot samples per delay (default: 50)
  --rx-timeout-ms N       Per-sample RX timeout (default: 30000)
  --packet-idle-ms N      RX packet idle boundary (default: 600)
  --report PATH           Markdown report path (default: /tmp/lora-reliability-report.md)
  --workdir DIR           Scratch directory (default: mktemp)
  --network NAME          Fixture network tag (default: devnet)
  --bridge-bin PATH       lora-bridge binary (default: target/debug/lora-bridge)
  --fixture-bin PATH      udp-digest-fixtures binary (default: target/debug/udp-digest-fixtures)
  -h, --help              Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tx-serial) TX_SERIAL="${2:-}"; shift 2 ;;
    --rx-serial) RX_SERIAL="${2:-}"; shift 2 ;;
    --baud) BAUD="${2:-}"; shift 2 ;;
    --delays) DELAYS="${2:-}"; shift 2 ;;
    --delta-count) DELTA_COUNT="${2:-}"; shift 2 ;;
    --snapshot-count) SNAPSHOT_COUNT="${2:-}"; shift 2 ;;
    --rx-timeout-ms) RX_TIMEOUT_MS="${2:-}"; shift 2 ;;
    --packet-idle-ms) PACKET_IDLE_MS="${2:-}"; shift 2 ;;
    --report) REPORT_PATH="${2:-}"; shift 2 ;;
    --workdir) WORKDIR="${2:-}"; shift 2 ;;
    --network) NETWORK="${2:-}"; shift 2 ;;
    --bridge-bin) BRIDGE_BIN="${2:-}"; shift 2 ;;
    --fixture-bin) FIXTURE_BIN="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

if [[ ! -x "${BRIDGE_BIN}" ]]; then
  echo "missing executable bridge binary: ${BRIDGE_BIN}" >&2
  echo "run: cargo build -p lora-bridge" >&2
  exit 1
fi

if [[ ! -x "${FIXTURE_BIN}" ]]; then
  echo "missing executable fixture binary: ${FIXTURE_BIN}" >&2
  echo "run: cargo build -p udp-generator --bin udp-digest-fixtures" >&2
  exit 1
fi

if [[ "${DELTA_COUNT}" -lt 0 || "${SNAPSHOT_COUNT}" -lt 0 ]]; then
  echo "sample counts must be non-negative" >&2
  exit 1
fi

if [[ -z "${WORKDIR}" ]]; then
  WORKDIR="$(mktemp -d /tmp/lora-reliability.XXXXXX)"
fi
mkdir -p "${WORKDIR}"

FIXTURE_DIR="${WORKDIR}/fixtures"
RUN_DIR="${WORKDIR}/runs"
CSV_PATH="${REPORT_PATH%.md}.csv"
mkdir -p "${FIXTURE_DIR}" "${RUN_DIR}" "$(dirname "${REPORT_PATH}")"

"${FIXTURE_BIN}" --out-dir "${FIXTURE_DIR}" --network "${NETWORK}" --negatives none >/dev/null

DELTA_FILE="${FIXTURE_DIR}/delta.bin"
SNAPSHOT_FILE="${FIXTURE_DIR}/snapshot.bin"

now_ms() {
  python3 - <<'PY'
import time
print(int(time.time() * 1000))
PY
}

run_one() {
  local kind=$1
  local delay=$2
  local sample=$3
  local input_file expected_size out_file rx_log tx_log start end latency rx_status tx_status result exact timeout fragment_loss corrupt duplicate

  case "${kind}" in
    delta) input_file="${DELTA_FILE}" ;;
    snapshot) input_file="${SNAPSHOT_FILE}" ;;
    *) echo "unknown kind: ${kind}" >&2; exit 1 ;;
  esac

  expected_size="$(wc -c < "${input_file}")"
  out_file="${RUN_DIR}/${kind}-${delay}-${sample}.rx.bin"
  rx_log="${RUN_DIR}/${kind}-${delay}-${sample}.rx.log"
  tx_log="${RUN_DIR}/${kind}-${delay}-${sample}.tx.log"
  rm -f "${out_file}" "${rx_log}" "${tx_log}"

  start="$(now_ms)"
  "${BRIDGE_BIN}" rx \
    --serial "${RX_SERIAL}" \
    --baud "${BAUD}" \
    --output file \
    --file "${out_file}" \
    --count 1 \
    --timeout-ms "${RX_TIMEOUT_MS}" \
    --packet-idle-ms "${PACKET_IDLE_MS}" \
    >"${rx_log}" 2>&1 &
  local rx_pid=$!
  sleep 0.2

  set +e
  "${BRIDGE_BIN}" tx \
    --serial "${TX_SERIAL}" \
    --baud "${BAUD}" \
    --input file \
    --file "${input_file}" \
    --inter-frame-delay-ms "${delay}" \
    >"${tx_log}" 2>&1
  tx_status=$?
  wait "${rx_pid}"
  rx_status=$?
  set -e
  end="$(now_ms)"
  latency=$((end - start))

  exact=0
  timeout=0
  fragment_loss=0
  corrupt=0
  duplicate=0
  result="fail"

  if [[ "${rx_status}" -eq 0 && "${tx_status}" -eq 0 && -f "${out_file}" ]] && cmp -s "${input_file}" "${out_file}"; then
    exact=1
    result="ok"
  else
    if rg -q "timed out|timeout" "${rx_log}"; then
      timeout=1
    fi
    if rg -q "pending fragments|received fragment" "${rx_log}"; then
      fragment_loss=1
    fi
    if [[ -f "${out_file}" ]] && ! cmp -s "${input_file}" "${out_file}"; then
      corrupt=1
    fi
    if rg -q "duplicate" "${rx_log}" "${tx_log}"; then
      duplicate=1
    fi
  fi

  local recovered_count=0
  if [[ "${exact}" -eq 1 || -f "${out_file}" ]]; then
    recovered_count=1
  fi

  local recovered_size=0
  if [[ -f "${out_file}" ]]; then
    recovered_size="$(wc -c < "${out_file}")"
  fi

  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "${kind}" "${delay}" "${sample}" "${expected_size}" "${recovered_size}" "${result}" \
    "${recovered_count}" "${exact}" "${timeout}" "${fragment_loss}" "${corrupt}" "${duplicate}" \
    "${latency}" "${rx_status}" "${tx_status}" >> "${CSV_PATH}"
}

summarize_kind_delay() {
  local kind=$1
  local delay=$2
  awk -F, -v kind="${kind}" -v delay="${delay}" '
    BEGIN {
      sent=0; recovered=0; exact=0; timeout=0; fragment=0; corrupt=0; duplicate=0;
      min=0; max=0; sum=0; bytes=0;
    }
    NR > 1 && $1 == kind && $2 == delay {
      sent++;
      bytes += $4;
      recovered += $7;
      exact += $8;
      timeout += $9;
      fragment += $10;
      corrupt += $11;
      duplicate += $12;
      latency = $13;
      sum += latency;
      if (min == 0 || latency < min) min = latency;
      if (latency > max) max = latency;
    }
    END {
      avg = sent ? int(sum / sent) : 0;
      dpm = sum ? (sent * 60000.0 / sum) : 0;
      bpm = sum ? (bytes * 60000.0 / sum) : 0;
      printf("| %s | %s | %d | %d | %d | %d | %d | %d | %d | %d/%d/%d | %.2f | %.0f |\n",
        kind, delay, sent, recovered, exact, timeout, fragment, corrupt, duplicate, min, avg, max, dpm, bpm);
    }
  ' "${CSV_PATH}"
}

IFS=',' read -r -a DELAY_VALUES <<< "${DELAYS}"

echo "kind,delay_ms,sample,input_bytes,recovered_bytes,result,recovered_count,exact_match,timeout,fragment_loss,corrupt,duplicate,latency_ms,rx_status,tx_status" > "${CSV_PATH}"

for delay in "${DELAY_VALUES[@]}"; do
  for ((i = 1; i <= DELTA_COUNT; i++)); do
    echo "delta delay=${delay} sample=${i}/${DELTA_COUNT}"
    run_one delta "${delay}" "${i}"
  done
  for ((i = 1; i <= SNAPSHOT_COUNT; i++)); do
    echo "snapshot delay=${delay} sample=${i}/${SNAPSHOT_COUNT}"
    run_one snapshot "${delay}" "${i}"
  done
done

{
  echo "# LoRa KUDP Reliability Harness Report"
  echo
  echo "Generated: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo
  echo "## Configuration"
  echo
  echo "- TX serial: \`${TX_SERIAL}\`"
  echo "- RX serial: \`${RX_SERIAL}\`"
  echo "- Baud: \`${BAUD}\`"
  echo "- Network tag source: \`${NETWORK}\` fixtures"
  echo "- Delays tested: \`${DELAYS}\` ms"
  echo "- Delta samples per delay: \`${DELTA_COUNT}\`"
  echo "- Snapshot samples per delay: \`${SNAPSHOT_COUNT}\`"
  echo "- RX timeout: \`${RX_TIMEOUT_MS}\` ms"
  echo "- Packet idle: \`${PACKET_IDLE_MS}\` ms"
  echo "- Workdir: \`${WORKDIR}\`"
  echo "- Raw CSV: \`${CSV_PATH}\`"
  echo
  echo "## Results"
  echo
  echo "| Kind | Delay ms | Sent | Recovered | Exact | Timeouts | Fragment loss | Corrupt | Duplicate | Latency min/avg/max ms | Datagrams/min | Bytes/min |"
  echo "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
  for delay in "${DELAY_VALUES[@]}"; do
    summarize_kind_delay delta "${delay}"
    summarize_kind_delay snapshot "${delay}"
  done
  echo
  echo "## Notes"
  echo
  echo "- \`fragment_loss\` is inferred when RX logs show a pending fragment or partial fragment reception without exact reassembly."
  echo "- \`corrupt\` means a file was recovered but failed byte-for-byte comparison."
  echo "- Latency is wall-clock time from RX start through TX and RX process completion."
  echo "- This harness exercises standalone byte-equality mode. Use \`udp/docs/lora-prototype.md\` for the kaspad ingest checklist."
} > "${REPORT_PATH}"

echo "wrote report: ${REPORT_PATH}"
echo "wrote csv: ${CSV_PATH}"
