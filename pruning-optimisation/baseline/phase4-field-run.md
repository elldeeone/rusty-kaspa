# Phase 4 – Field Validation (Mainnet, 2025‑11‑09)

This document captures the real-node pruning metrics that Phase 4 of `pruning-optimisation/plan.md` requires. The run was performed on mainnet using the batching code in commit `ed28c40611b9fdd841ad08f5c796e355db251881`.

## Environment

| Item | Details |
| --- | --- |
| Host | Darwin 24.6.0 (Apple Silicon MacBook Air, arm64) |
| Build | `cargo build --release --bin kaspad` |
| Runtime command | `RUST_LOG=info,kaspa_consensus::processes::pruning_proof::build=info target/release/kaspad` |
| Network / retention | Mainnet, default retention window |
| Data dir | `~/.rusty-kaspa/kaspa-mainnet` |
| Log source | `~/.rusty-kaspa/kaspa-mainnet/logs/rusty-kaspa.log` (lines 51086‑51469) |
| Window observed | `2025-11-09 15:11` (pruning prep) → `2025-11-09 17:09` (metrics) |

## Metrics Summary

The pruning pass logged the standard completion banner plus the `[PRUNING METRICS]` block:

| Metric | Value |
| --- | --- |
| Total duration | 7 080 695 ms (≈1h 58m) |
| Traversed / pruned blocks | 473 826 traversed / 432 077 pruned |
| Lock stats | 5 711 421 ms cumulative hold · 149 503 yields · 149 504 reacquires |
| Batched commits | 149 500 commits · avg 854 ops · max 3 126 ops · avg 92.3 KB · max 638 KB · avg 0.961 ms · max 781.666 ms |
| Tips & selected-chain commit | 1 commit · 153 038 ops · 3.52 MB · 56.601 ms |
| Ghostdag adjust commit | 1 commit · 50 ops · 11.96 KB · 0.061 ms |

Completion lines:

```
2025-11-09 17:04:20 [INFO ] Header and Block pruning completed: traversed: 473826, pruned 432077
2025-11-09 17:04:20 [INFO ] Header and Block pruning stats: proof size: 93924, pruning point and anticone: 13, unique headers in proof and windows: 41750, pruning points in history: 1629
2025-11-09 17:09:55 [INFO ] Trusted data was rebuilt successfully following pruning
2025-11-09 17:09:56 [INFO ] [PRUNING METRICS] ...
```

## Comparison vs Simpa Harness

Reference harness run: `pruning-optimisation/baseline/experiments/simpa-phase2-10bps-100k-ret1e-5.{log,csv}` (generated with `SIMPA_BPS=10`, `SIMPA_TARGET_BLOCKS=100000`, `SIMPA_RETENTION_DAYS=1e-5`).

| Aspect | Simpa (100 k synthetic DAG) | Field run |
| --- | --- | --- |
| Traversed / pruned | 47 / 31 blocks (small synthetic retention) | 473 826 / 432 077 |
| Batched commit count | 1 (entire prune fit in a single batch) | 149 500 |
| Avg ops per batched commit | 913 | 854 |
| Avg commit latency | 0.617 ms | 0.961 ms |
| Max commit bytes | 80 804 B | 638 247 B |

Observations:

1. The synthetic workload only exposes one flush because the retention window prunes a few dozen blocks. Still, the per-commit latency (0.6 ms) and payload size (≈80 KB) align with the field averages (0.96 ms / 92 KB) despite the 4‑order-of-magnitude difference in commit count.
2. Mainnet exercised the lock-yield logic heavily (149 k yields). This matches expectations: the traversal touched ≈474 k nodes, so batches had to flush aggressively to respect the `PRUNE_LOCK_MAX_DURATION_MS` limit.
3. Tips/selected-chain commit ballooned to 3.5 MB because the pruning point advanced across 1629 historical checkpoints; future tuning should monitor this size, but 56 ms/wAL sync is acceptable on SSD.

## Follow-ups

1. **Perf monitoring:** Re-run the next cycle with the built-in monitor enabled – e.g. `target/release/kaspad --perf-metrics --perf-metrics-interval-sec=10 --loglevel info,consensus=trace ...` – and archive the emitted CPU/memory/disk stats under `pruning-optimisation/baseline/perf/`.
2. **Documentation:** `pruning-optimisation/design/phase1-batching-architecture.md` has been updated (see “Field Validation Snapshot”) to reference the numbers above and describe the perf-monitor plan.
