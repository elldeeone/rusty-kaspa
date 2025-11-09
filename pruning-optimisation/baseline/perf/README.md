# Perf Metrics Capture

This folder stores perf-monitor snapshots that back Phase 4 of `pruning-optimisation/plan.md` and the “Perf snapshot plan” called out in `design/phase1-batching-architecture.md`. Each run should ship with:

1. The raw kaspad log segment with `--perf-metrics` enabled.
2. A CSV/JSON export of the `[PRUNING METRICS]` lines (via `scripts/collect_pruning_metrics.py`).
3. A short metadata note describing the environment, retention settings, and relevant overrides.

## Capture Workflow

1. Build `kaspad` in release mode (`cargo build --release --bin kaspad`).
2. Launch the node with perf monitoring enabled:
   ```bash
   LOG_LEVEL='info,consensus=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace' \
     target/release/kaspad \
     --perf-metrics --perf-metrics-interval-sec=10 \
     --logdir ~/.rusty-kaspa/kaspa-mainnet/logs \
     --<network flags>
   ```
3. Once pruning completes, copy the relevant log slice here as `YYYYMMDD-perf.log` (or similar) and run:
   ```bash
   python3 pruning-optimisation/scripts/collect_pruning_metrics.py ~/.rusty-kaspa/kaspa-mainnet/logs/rusty-kaspa.log \
     > pruning-optimisation/baseline/perf/YYYYMMDD-pruning-metrics.csv
   ```
4. Record run metadata in a companion `.md` file following the template below.

## Metadata Template

```
# <Run Title>

- Commit: <hash>
- Command: <kaspad invocation>
- Retention / network: <details>
- Perf interval: <seconds>
- Log file: <relative path>
- Notes: <interesting observations>
```

## Quick Parsing Aids

- `rg "\[perf-metrics\]" perf.log` – isolate perf monitor samples.
- `rg "\[PRUNING METRICS\]" perf.log` – correlate prune batches with perf samples.
- `python3 pruning-optimisation/scripts/collect_pruning_metrics.py perf.log` – regenerate the pruning CSV from the perf-enabled log if needed.
