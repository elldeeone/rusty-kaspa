# Pruning Optimisation Plan

## Phase 0 – Confirm Baseline via Simpa
1. Capture two harness runs:
   - `SIMPA_BPS=2 SIMPA_TARGET_BLOCKS=2000 SIMPA_RETENTION_DAYS=0.05`
   - `SIMPA_BPS=10 SIMPA_TARGET_BLOCKS=6000 SIMPA_RETENTION_DAYS=0.05`
2. Archive the logs/CSVs under `pruning-optimisation/baseline/experiments/` (`simpa-phase0-baseline-{2bps,10bps}.{log,csv}`).
3. Summarise commit/lock metrics so every future pruning change can be compared against these references.

## Phase 1 – Implement Batched Pruning
1. Add `PruneBatchContext` to `consensus/src/pipeline/pruning_processor/processor.rs` per design doc.
   - Shared staging stores + WriteBatch.
   - Threshold constants (blocks, ops, bytes, elapsed ms).
2. Include pruning-point update (and optional in-progress marker) in final batch for atomicity.
3. Maintain yield logic by flushing when lock held > threshold.
4. Update instrumentation to emit new `commit_type=batched` stats without breaking existing log format.
5. Add targeted unit/integration coverage (e.g., synthetic DAG prune test) to guard basic invariants.

## Phase 2 – Rapid Feedback Loop (Simpa)
1. Rebuild and rerun `run-simpa-pruning.sh` (DB reuse).
2. Collect `simpa-batched.csv` and diff vs baseline.
3. Adjust thresholds based on:
   - Target ≥10× reduction in `per_block` commits.
   - Keep lock hold durations within acceptable bounds.
   - Monitor memory via RocksDB stats if needed (use `--rocksdb-stats`).

## Phase 3 – Stress & Edge Testing
1. Simpa scenarios:
   - Higher BPS (`SIMPA_BPS=4`), long payloads, multiple miners.
   - Reduced retention (force frequent pruning) and larger retention (stress memory).
2. Crash safety drills: send SIGKILL during pruning; ensure restart resumes cleanly.
3. Optional: enable RocksDB options (pipelined writes, DeleteRange) in isolated experiments once batching is stable.

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
