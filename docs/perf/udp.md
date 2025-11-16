# UDP Soak & Performance Guard

The UDP ingest service ships with a repeatable soak harness that compares a baseline "kaspad without UDP" run against the digest ingress path operating around 10 kbps. The harness is automated via `udp/tools/soak.sh` and produces a structured JSON summary so the results can be dropped into plan updates or CI logs.

## Running the soak locally

```
./udp/tools/soak.sh \
  --duration 3m \
  --rate 10 \
  --kaspad target/release/kaspad \
  --generator target/release/udp-generator
```

The script performs two back‑to‑back sessions (baseline + UDP enabled). Each session boots a `--simnet` node inside a throwaway `--appdir`, samples CPU and RSS via `/proc` (on Linux) or `ps` (macOS), replays the bundled fuzz corpus through the UDP generator, and finally emits a JSON payload similar to:

```json
{
  "baseline": { "cpu_pct": 2.1, "rss_delta_bytes": -1048576, "panic_drop_count": 0 },
  "enabled": { "cpu_pct": 2.3, "rss_delta_bytes": 6291456, "panic_drop_count": 0 },
  "overhead_pct": 9.5,
  "rate_kbps": 10.0,
  "duration_seconds": 180
}
```

The soak fails (non-zero exit) whenever:

- CPU overhead exceeds **15%** relative to the baseline.
- RSS grows by more than **50 MB** during the UDP-enabled run.
- Any `udp.event` log mentions `reason=panic` (this mirrors the planned `udp_frames_dropped_total{reason="panic"}` guard).

## Interpreting the results

- **cpu_pct** is derived from `process_cpu_seconds_total / wall_seconds`. A 10 kbps digest stream should stay well below the 15% headroom when running in release mode.
- **rss_delta_bytes** tracks high-water RSS deltas inside the soak window. Minor negative values (memory returning to the OS) are normal.
- **panic_drop_count** indicates whether the service logged a panic drop reason; the soak guards block deployment whenever this rises above zero.

## Remediation checklist

1. Re-run the soak with `--loglevel info` (set `KASPAD_EXTRA_ARGS="--loglevel info,udp-ingest=trace"`) to collect detailed traces.
2. Inspect `udp.metrics` via `kaspa-cli rpc getmetrics` (or `curl $RPC/metrics` when the exporter lands) to pinpoint which drop reason is dominating.
3. Validate generator behavior by running `udp/tools/generator --rate 10 --snapshot-interval 30 --network simnet --target 127.0.0.1:28515` directly while watching `kaspad` logs for `udp.event` lines.
4. Tune queue sizes / rate caps via `--udp.digest_queue`, `--udp.max_kbps`, and `--udp.log_verbosity` as needed.
5. File an incident if `panic_drop_count > 0`; this indicates an invariant violation inside the ingest path and requires immediate attention.
