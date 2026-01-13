# Phase 0 – Simpa Harness Baselines (2025-11-05)

To make pruning experiments repeatable we locked in two Simpa scenarios: a fast “inner loop” profile at 2 BPS and a high-throughput profile at 10 BPS. Both rely on the pruning instrumentation in `pruning_processor.rs`, emit `[PRUNING METRICS] …` lines, and capture their logs/CSVs under `pruning-optimisation/baseline/experiments/`.

| Profile | Purpose | Command (abridged) | Artifacts |
| --- | --- | --- | --- |
| 2 BPS · 2 000 blocks | Quick sanity check for day-to-day development | `SIMPA_BPS=2 SIMPA_TARGET_BLOCKS=2000 SIMPA_RETENTION_DAYS=0.05 ./pruning-optimisation/run-simpa-pruning.sh` | `simpa-phase0-baseline-2bps.{log,csv}` |
| 10 BPS · 6 000 blocks | Represents the new network throughput | `SIMPA_BPS=10 SIMPA_TARGET_BLOCKS=6000 SIMPA_RETENTION_DAYS=0.05 ./pruning-optimisation/run-simpa-pruning.sh` | `simpa-phase0-baseline-10bps.{log,csv}` |

Both runs finish in <15 s on the dev machine yet exercise every stage (pruning-point advance, proof rebuild, retention checkpoint). The shorter block horizons keep turnaround low; when we start deleting per-block data we can always spin a longer scenario.

## Metrics snapshot (averages per cycle)

| Profile | Cycles | Avg duration (ms) | Avg traversed | Avg pruned | `ghostdag_adjust` ops | `tips_and_selected_chain` ops | `retention_checkpoint` ops |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 2 BPS | 2 | 12 | 0 | 0 | 0 (12 B) | 3.0 (max 5) · 117 B | 1 (48 B) |
| 10 BPS | 2 | 12 | 0 | 0 | 0 (12 B) | 10.0 (max 13) · 362 B | 1 (48 B) |

Key takeaways:

1. **Instrumentation works without behaviour change.** Commit statistics capture each RocksDB write (none of them aggregate multiple blocks yet), lock telemetry shows one reacquire / one yield per cycle, and the trusted-data sanity checks pass in both profiles.
2. **Retention is intentionally shallow.** With only a few thousand blocks the traversal loop short-circuits (`traversed=0`). That is fine for Phase 0; Phase 1 can bump `SIMPA_TARGET_BLOCKS` or shrink retention further to produce non-zero deletions for benchmarking.
3. **Artifacts are stable.** Re-running the same command simply regenerates the DB and new timestamped logs, which we then copy to the canonical filenames above. These files now serve as the “before” reference any future batching patch must improve upon.
