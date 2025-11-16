#!/usr/bin/env bash
set -euo pipefail

# Defaults
DURATION_ARG="180s"
RATE_ARG="10"
KASPAD_BIN="target/release/kaspad"
GEN_BIN="target/release/udp-generator"
SIGNER_HEX="4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa"
WORKDIR=""

usage() {
    cat <<USAGE
Usage: $0 [--duration 180s] [--rate 10] [--kaspad PATH] [--generator PATH]
           [--workdir DIR]

  --duration       Run length per session (e.g. 180s, 3m). Defaults to 180s.
  --rate           UDP generator bitrate in kbps (numeric). Defaults to 10.
  --kaspad         Path to kaspad binary (built in release mode if missing).
  --generator      Path to udp-generator binary (built in release mode if missing).
  --workdir        Optional scratch directory (defaults to mktemp).
USAGE
}

parse_duration() {
    local raw=$1
    if [[ $raw =~ ^([0-9]+)([smh]?)$ ]]; then
        local value=${BASH_REMATCH[1]}
        local unit=${BASH_REMATCH[2]}
        case "$unit" in
            s|"") echo "$value" ;;
            m) echo $((value * 60)) ;;
            h) echo $((value * 3600)) ;;
            *) return 1 ;;
        esac
    else
        echo "invalid duration: $raw" >&2
        exit 1
    fi
}

parse_rate() {
    local raw=$1
    raw=${raw%kbps}
    if [[ $raw =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
        echo "$raw"
    else
        echo "invalid rate: $1" >&2
        exit 1
    fi
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --duration)
            DURATION_ARG=${2:-}
            shift 2
            ;;
        --rate)
            RATE_ARG=${2:-}
            shift 2
            ;;
        --kaspad)
            KASPAD_BIN=${2:-}
            shift 2
            ;;
        --generator)
            GEN_BIN=${2:-}
            shift 2
            ;;
        --workdir)
            WORKDIR=${2:-}
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage
            exit 1
            ;;
    esac
done

DURATION_SECS=$(parse_duration "$DURATION_ARG")
RATE_KBPS=$(parse_rate "$RATE_ARG")

build_binary() {
    local target=$1
    local crate=$2
    if [[ ! -x $target ]]; then
        echo "building $crate (release)"
        cargo build --release -p "$crate"
    fi
}

build_binary "$KASPAD_BIN" kaspad
build_binary "$GEN_BIN" udp-generator

SCRATCH=${WORKDIR:-$(mktemp -d -t udp-soak.XXXXXX)}
mkdir -p "$SCRATCH"

BASE_CPU_PCT="0.0"
BASE_RSS_DELTA="0"
BASE_PANIC="0"
EN_CPU_PCT="0.0"
EN_RSS_DELTA="0"
EN_PANIC="0"
fail_reasons=()

cleanup_process() {
    local pid=$1
    if kill -0 "$pid" >/dev/null 2>&1; then
        kill "$pid" >/dev/null 2>&1 || true
        wait "$pid" 2>/dev/null || true
    fi
}

read_metrics() {
    local pid=$1
    python3 - "$pid" <<'PY'
import json, os, platform, sys
pid = int(sys.argv[1])
system = platform.system()
if system == "Linux":
    stat_path = f"/proc/{pid}/stat"
    try:
        parts = open(stat_path, 'r').read().split()
    except FileNotFoundError:
        sys.exit(1)
    hz = os.sysconf(os.sysconf_names['SC_CLK_TCK'])
    page_size = os.sysconf('SC_PAGE_SIZE')
    utime = int(parts[13])
    stime = int(parts[14])
    rss = int(parts[23]) * page_size
    cpu = (utime + stime) / hz
else:
    import subprocess
    result = subprocess.run(["ps", "-p", str(pid), "-o", "rss=", "-o", "time="], capture_output=True, text=True)
    line = result.stdout.strip()
    if not line:
        sys.exit(1)
    rss_str, time_str = line.split()
    rss = int(float(rss_str)) * 1024
    day_split = time_str.split('-')
    if len(day_split) == 2:
        days = int(day_split[0])
        time_str = day_split[1]
    else:
        days = 0
    fields = time_str.split(':')
    if len(fields) == 3:
        hours, minutes, seconds = fields
    elif len(fields) == 2:
        hours = 0
        minutes, seconds = fields
    else:
        hours = 0
        minutes = 0
        seconds = fields[0]
    cpu = days * 86400 + int(hours) * 3600 + int(minutes) * 60 + float(seconds)
print(f"{cpu} {rss}")
PY
}

compute_cpu_pct() {
    local cpu_start=$1
    local cpu_end=$2
    local wall_start=$3
    local wall_end=$4
    python3 - <<PY
cpu_start = $cpu_start
cpu_end = $cpu_end
wall = $wall_end - $wall_start
print(0.0 if wall <= 0 else (cpu_end - cpu_start) / wall * 100.0)
PY
}

run_session() {
    local mode=$1
    local enable_udp=$2
    local appdir
    appdir=$(mktemp -d "$SCRATCH/${mode}.XXXX")
    local logfile="$appdir/kaspad.log"
    local listen_addr="127.0.0.1:0"
    local udp_listen="127.0.0.1:28515"
    local cmd=(
        "$KASPAD_BIN"
        --simnet
        --disable-upnp
        --nogrpc
        --rpclisten="$listen_addr"
        --rpclisten-borsh="$listen_addr"
        --rpclisten-json="$listen_addr"
        --listen="$listen_addr"
        --appdir="$appdir"
        --logdir="$appdir"
        --loglevel=warn
    )
    if [[ "$enable_udp" == "true" ]]; then
        cmd+=(
            --udp.enable
            --udp.listen="$udp_listen"
            --udp.require_signature=true
            --udp.allowed_signers="$SIGNER_HEX"
            --udp.db_migrate=true
            --udp.retention_count=10
            --udp.retention_days=1
            --udp.max_kbps=50
        )
    fi
    "${cmd[@]}" >"$logfile" 2>&1 &
    local kaspad_pid=$!
    sleep 5
    local start_ts=$(date +%s)
    read cpu_start rss_start < <(read_metrics "$kaspad_pid")
    local gen_pid=""
    if [[ "$enable_udp" == "true" ]]; then
        "$GEN_BIN" --target "$udp_listen" --network simnet --rate "$RATE_KBPS" --source-id 7 >"$appdir/generator.log" 2>&1 &
        gen_pid=$!
        sleep 2
    fi
    sleep "$DURATION_SECS"
    local end_ts=$(date +%s)
    read cpu_end rss_end < <(read_metrics "$kaspad_pid") || true
    cleanup_process "$gen_pid"
    cleanup_process "$kaspad_pid"

    local cpu_pct=$(compute_cpu_pct "$cpu_start" "$cpu_end" "$start_ts" "$end_ts")
    local rss_delta=$((rss_end - rss_start))
    local panic_count=$(grep -c 'reason=panic' "$logfile" || true)

    if [[ "$enable_udp" == "true" ]]; then
        EN_CPU_PCT="$cpu_pct"
        EN_RSS_DELTA="$rss_delta"
        EN_PANIC="$panic_count"
    else
        BASE_CPU_PCT="$cpu_pct"
        BASE_RSS_DELTA="$rss_delta"
        BASE_PANIC="$panic_count"
    fi
    echo "session $mode complete: cpu_pct=${cpu_pct} rss_delta=${rss_delta} panic=${panic_count}"
}

run_session baseline false
run_session enabled true

calc_overhead() {
    python3 - <<PY
base = float(${BASE_CPU_PCT:-0})
enabled = float(${EN_CPU_PCT:-0})
denom = base if base > 0.01 else 0.01
print((enabled - base) / denom * 100.0)
PY
}

overb=$(calc_overhead)
mem_growth=${EN_RSS_DELTA:-0}
panic=${EN_PANIC:-0}

if python3 - <<PY
import sys
if float($overb) > 15.0:
    sys.exit(1)
PY
then :
else
    fail_reasons+=("CPU overhead ${overb}% > 15%")
fi

if [[ $mem_growth -gt 50000000 ]]; then
    fail_reasons+=("RSS growth ${mem_growth} bytes > 50MB")
fi

if [[ ${panic} -gt 0 ]]; then
    fail_reasons+=("udp_frames_dropped_total{reason=\"panic\"} incremented")
fi

python3 - <<PY
import json
summary = {
    "baseline": {
        "cpu_pct": float(${BASE_CPU_PCT:-0}),
        "rss_delta_bytes": int(${BASE_RSS_DELTA:-0}),
        "panic_drop_count": int(${BASE_PANIC:-0}),
    },
    "enabled": {
        "cpu_pct": float(${EN_CPU_PCT:-0}),
        "rss_delta_bytes": int(${EN_RSS_DELTA:-0}),
        "panic_drop_count": int(${EN_PANIC:-0}),
    },
    "overhead_pct": float($overb),
    "rate_kbps": float($RATE_KBPS),
    "duration_seconds": int($DURATION_SECS)
}
print(json.dumps(summary, indent=2))
PY

if [[ ${#fail_reasons[@]} -gt 0 ]]; then
    printf '\nSoak guard failed:\n'
    for reason in "${fail_reasons[@]}"; do
        echo " - $reason"
    done
    exit 1
fi
