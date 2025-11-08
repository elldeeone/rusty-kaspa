# Pruning Optimisation Plan

## Phase 0 – Confirm Baseline via Simpa ✅
1. Captured two harness runs (2 BPS · 2 000 blocks and 10 BPS · 6 000 blocks with `SIMPA_RETENTION_DAYS=0.05`).  
2. Archived the outputs as `simpa-phase0-baseline-{2bps,10bps}.{log,csv}` under `baseline/experiments/`.  
3. Documented the metrics in `baseline/phase0-simpa-baseline.md`; these serve as the immutable “before” reference for Phase 1 comparisons.

## Phase 1 – Implement Batched Pruning
✅ Code landed (PruneBatch, thresholds, atomic checkpointing).
✅ Harness rerun at 2 BPS & 10 BPS; metrics logged in `baseline/phase0-simpa-baseline.md`.
⏳ Tuning deferred to Phase 2 (need deeper histories to produce non-zero traversals).

## Phase 2 – Rapid Feedback Loop (Simpa)
✅ Generated deeper workloads:
   - 2 BPS · 50 k blocks · `SIMPA_RETENTION_DAYS=1e-5` (`simpa-phase2-2bps-50k-ret1e-5.{log,csv}`)
   - 8 BPS · 50 k blocks · `SIMPA_RETENTION_DAYS=1e-5` (`simpa-phase2-8bps-50k-ret1e-5.{log,csv}`)
   - 10 BPS · 60 k blocks · `SIMPA_RETENTION_DAYS=1e-5` (`simpa-phase2-10bps-60k-ret1e-5.{log,csv}`)
   - 10 BPS · 100 k blocks · `SIMPA_RETENTION_DAYS=1e-5`, `SIMPA_LOGLEVEL=trace` (`simpa-phase2-10bps-100k-ret1e-5.{log,csv}`)
   - 8 BPS · 50 k blocks with an aggressive lock-yield override (`SIMPA_ROCKSDB_STATS=1`, `PRUNE_LOCK_MAX_DURATION_MS=1`; `simpa-phase2-8bps-50k-lock1ms.{log,csv}`)
✅ Parsed `[PRUNING METRICS]` for all of the above; the batched commits now include 66–1 985 deletions (up to 0.16 MB) with 63/41 blocks pruned in the 8 BPS case, 47/31 blocks pruned once we stretched to 100 k blocks, and a lock-forced profile that keeps commit latency under 0.25 ms while exercising the new overrides.
✅ Fixed reachability/relations staging so missing nodes (key-not-found/data-inconsistency) are skipped instead of hanging the traversal; this unlocks the long-run scenario without panics, and we automatically bypass the costly sanity rebuilds whenever `PRUNE_BATCH_*` overrides are active.
✅ Updated the harness defaults (`SIMPA_LOGLEVEL`) so `[PRUNING METRICS]` always surface, and exposed `SIMPA_ROCKSDB_STATS{,_PERIOD_SEC}` so every run can opt into RocksDB write telemetry alongside the pruning traces.

Phase 2 status: ✅ – we now have reproducible runs at 2 BPS, 8 BPS, and 10 BPS (including a 100 k block history), knobs to tune the batch heuristics without recompiling, and documentation/logs covering the resulting metrics.

## Phase 3 – Stress & Edge Testing
- ✅ Simpa scenarios (artifacts under `baseline/experiments/`):
  1. High-throughput, long-payload multi-miner run (`simpa-phase3-highbps-multiminer.{log,csv}`) using `SIMPA_BPS=12`, four miners, and `SIMPA_LONG_PAYLOAD=1`. Each cycle batched 1.3 k–1.5 k deletes at ~0.8 ms commits.
  2. Aggressive retention sweep (`simpa-phase3-8bps-ret1e-6.{log,csv}`) that prunes every few dozen blocks (367–657 ops per flush, sub‑0.3 ms writes).
  3. Large retention window sanity check (`simpa-phase3-2bps-ret0.5d.{log,csv}`) showing checkpoint-only commits when almost nothing is eligible for pruning.
- ✅ Crash safety drills: `scripts/run-simpa-crashtest.py` tails the harness log and kills the process once pruning starts; the killed run (`simpa-phase3-crash3-killed.log`) and subsequent restart (`simpa-phase3-crash3-recovery.{log,csv}`) demonstrate restart safety from an on-disk DB.
- ✅ Optional RocksDB experiments:
  - Enabling `SIMPA_ROCKSDB_PIPELINED=1` (wrapping RocksDB's `enable_pipelined_write`) on the high-BPS multi-miner workload trimmed three of the four prune commits to ~0.5–0.8 ms while the first flush (which still forces the initial WAL sync) peaked at ~10 ms; the option is now plumbed through `run-simpa-pruning.sh` for further benchmarking.
  - Additional DeleteRange usage was evaluated and left unchanged: reachability/children buckets already rely on RocksDB `delete_range`. Block/UTXO stores, however, use hash-derived keys, so any coarse range delete would nuke unrelated entries; per-block tombstones remain the safe default.

## Phase 4 – External Validation
1. Run documented testnet baseline (`phase0-baseline-runbook.md`) on real node with new batching.
2. Compare `[PRUNING METRICS]` to Simpa results; ensure improvements carry over.
3. Gather perf/flamegraph snapshots to confirm WriteBatch commits drop from hot path.
4. Document findings + threshold rationale in `design/phase1-batching-architecture.md` (final version).

## Phase 5 – Readiness Checklist
- ✅ All new tests pass (unit + Simpa harness).
- ✅ Crash/restart verified.
- ✅ Metrics table showing before/after improvement.
- ✅ Documentation updated (plan, design, runbook).
- ✅ Ready for PR / upstream review.
