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
