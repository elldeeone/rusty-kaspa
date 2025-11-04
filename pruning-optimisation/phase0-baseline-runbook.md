# Phase 0 Baseline – Pruning Metrics Runbook

## Purpose

Gather reproducible measurements for the current pruning workflow before making behavioural changes. New instrumentation in `consensus/src/pipeline/pruning_processor/processor.rs` logs per-commit statistics and lock behaviour under the `[PRUNING METRICS]` tag.

## Prerequisites

- Build profile: `cargo build --release` (debug symbols optional but useful for profiling).
- Runtime flags: enable consensus trace logging so pruning progress appears in the log stream.

## Execution Steps

1. **Start a node** on `master`/`pruning-optimisation` with metrics enabled:
   ```bash
   cargo run --release --bin kaspad -- --testnet --loglevel info,consensus=trace 2>&1 | tee pruning-baseline.log
   ```
   - Adjust network/flags as needed if you want mainnet or different retention settings (`--retention-period-days`).
   - Allow the node to reach steady state and wait for at least one pruning cycle to complete (≈24h cadence on default retention; shorten by lowering retention depth on a throwaway network if you need quicker turnaround).

2. **Capture metrics output.** Each pruning run now emits a summary plus per-commit statistics, for example:
   ```
   [PRUNING METRICS] duration_ms=... traversed=... pruned=... lock_hold_ms=... lock_yields=... lock_reacquires=...
   [PRUNING METRICS] commit_type=ghostdag_adjust ...
   [PRUNING METRICS] commit_type=tips_and_selected_chain ...
   [PRUNING METRICS] commit_type=per_block ...
   [PRUNING METRICS] commit_type=retention_checkpoint ...
   ```
   Use `rg "\[PRUNING METRICS\]" pruning-baseline.log` to extract the relevant lines.

3. **Record baseline figures.** Track at minimum:
   - `per_block` commit count, average/max ops, average/max duration (primary bottleneck indicator).
   - Total prune duration (`duration_ms`) and block counts (`traversed`, `pruned`).
   - Lock behaviour (`lock_hold_ms`, `lock_yields`, `lock_reacquires`) to establish acceptable contention windows.

4. **Optional profiling hooks.** With the same build, you can attach `cargo flamegraph --root --bin kaspad ...` during pruning to corroborate database hot spots without further code changes.

## Fast Iteration via Simpa Harness

For rapid local testing, the Simpa simulator can generate a deterministic DAG and rerun pruning in a few minutes. A helper script is now available:

```bash
./pruning-optimisation/run-simpa-pruning.sh
```

Behaviour:

- On the first run it generates a synthetic history (`cargo run --bin simpa`) and stores the RocksDB under `pruning-optimisation/baseline/experiments/simpa-db`.
- Subsequent runs reuse the DB and only execute the pruning/validation phase, so each test cycle completes quickly.
- Logs are written to `pruning-optimisation/baseline/experiments/simpa-prune-<timestamp>.log`.
- `[PRUNING METRICS]` entries are automatically exported to `*.csv` via `scripts/collect_pruning_metrics.py`.

Environment overrides allow further tuning without editing the script, for example:

```bash
SIMPA_BPS=4 SIMPA_TARGET_BLOCKS=4000 ./pruning-optimisation/run-simpa-pruning.sh --long-payload
```

Any additional arguments after the script name are forwarded directly to Simpa, enabling experimentation with RocksDB stats, logging, etc.

## Output Handling

Persist extracted metrics (CSV or spreadsheet) alongside environment details:

- Commit hash / branch.
- Hardware (CPU, storage type).
- Retention configuration and network (testnet/mainnet).
- Observed pruning cadence and any anomalies (e.g., concurrent IBD, manual shutdown).

These baselines form the reference set for evaluating batching optimisations in subsequent phases.
