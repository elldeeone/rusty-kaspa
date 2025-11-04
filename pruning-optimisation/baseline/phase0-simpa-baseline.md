# Phase 0 – Simpa Harness Baseline (2025-11-05)

## Execution Details

- **Command:**<br/>
  `SIMPA_DB_DIR=pruning-optimisation/baseline/experiments/simpa-db-phase0 SIMPA_TARGET_BLOCKS=20000 SIMPA_RETENTION_DAYS=0.05 ./pruning-optimisation/run-simpa-pruning.sh`
- **Log:** `baseline/experiments/simpa-phase0-baseline.log`
- **Metrics CSV:** `baseline/experiments/simpa-phase0-baseline.csv`
- **Retained run outputs:** `baseline/experiments/simpa-db-phase0/`

The simulator generated 20 000 blocks at 2 BPS with a 0.05‑day retention window and triggered four pruning cycles back‑to‑back (immediately after block generation completed). Even though the traversal stage reported zero fully pruned blocks—because the simulated history barely exceeds the shortened pruning depth—the run exercises the full pruning pipeline and emits the instrumentation we need for future comparisons.

## Metrics Snapshot

| Metric | Mean | Notes |
| --- | --- | --- |
| Prune duration (ms) | 14 | Four cycles, each 14–15 ms |
| Traversed blocks | 0 | Entire keep-set covers generated DAG, so traversal short circuits |
| Blocks pruned | 0 | Same reason as above |
| Lock hold ms | 0 | Pruning lock released immediately after each fast cycle |
| Lock reacquires | 1 per cycle | Matches “flush once” current behavior |

### Commit Statistics (per cycle averages)

| Commit Type | Count | Avg Ops | Max Ops | Avg Bytes | Max Bytes | Avg Commit ms | Max Commit ms |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `batched` | 4 | 1.0 | 1 | 48 B | 48 B | 0.0038 | 0.0040 |
| `ghostdag_adjust` | 4 | 0.0 | 0 | 12 B | 12 B | 0.0033 | 0.0040 |
| `tips_and_selected_chain` | 4 | 1.75 | 4 | 73 B | 152 B | 0.0038 | 0.0060 |

## Observations

1. **Single WriteBatch per run:** With the current infinite thresholds the pruning loop flushes exactly once per cycle (`count=1`), confirming that batching is already wired but lacks guard rails.
2. **Retention root still near genesis:** Because the simulated window is short, `traversed/pruned` remain zero. For future baselines we should either run more blocks or shrink the retention window further so that the traversal stage actually walks & deletes nodes, but the instrumentation pipeline itself is verified.
3. **Harness ready:** The `simpa-phase0-baseline.*` artifacts now provide a reproducible reference point for comparing upcoming batching tweaks.
