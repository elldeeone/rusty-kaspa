# Phase 1 – Pruning Batching Architecture (Draft)

Date: 2025-11-01  
Branch: `pruning-optimisation` (commit `08018e7925830c7d8fe8bc1185782eb53dab3da6`)

## 1. Baseline Observations

Source: `pruning-optimisation/baseline/phase0-testnet-2025-11-01.log`

- Per prune run, the loop invoked `db.write` **52 327** times (`commit_type=per_block`), averaging **62.8** operations and **3.9 KB** per batch, with spikes up to **313 ops / 21 KB** and **15.6 ms** latency.
- Total prune duration ≈ **8.8 minutes** (`duration_ms=527 146`), traversing **52 327** blocks and fully pruning **30 913**.
- Lock churn is extreme: **31 790** yields and **84 118** reacquires, while cumulative hold time still reaches **244 s**.
- One-off phases (`ghostdag_adjust`, `tips_and_selected_chain`, `retention_checkpoint`) issue a single tiny batch each; the hot path is the per-block section at `consensus/src/pipeline/pruning_processor/processor.rs:438-641`.
- Each per-block iteration creates fresh staging stores (`StagingReachabilityStore`, `StagingRelationsStore`) and commits immediately, even though the operations are deletions that could be merged safely.

Takeaway: Remarkably fine-grained commits dominate disk I/O. Reducing commit count by 5–20× should materially improve pruning throughput and reduce lock churn.

## 2. Design Objectives

1. **Amortize RocksDB write cost** by batching multiple block deletions per commit without inflating memory or lock hold times beyond safe limits.
2. **Preserve correctness and crash safety**: either all staged changes for a batch land atomically or none do; application-level checkpoints must remain in sync.
3. **Maintain lock fairness** comparable to current behaviour, i.e. continue yielding frequently enough that consensus ingress is not starved.
4. **Avoid large in-memory payloads** (block bodies, UTXO diffs) in unbounded batches; support chunking or separate handling for large data.
5. **Keep instrumentation hooks** so we can compare Phase 2 metrics directly to Phase 0.

## 3. Proposed Architecture

### 3.1 PruneBatchContext

Introduce a struct that encapsulates all staging state spanning multiple blocks:

- Shared `WriteBatch`.
- Reusable `StagingRelationsStore` instances (level 0..N) plus `StagingReachabilityStore`.
- Cached `statuses_write`, `reachability_relations_write`, etc., reinitialised only when a batch commits.
- Counters: blocks staged, op count, estimated bytes, elapsed time since last commit.

Thresholds (tunables, validated via metrics):

- `max_blocks_per_batch` (e.g. 256–1024).
- `max_ops_per_batch` (e.g. 10 000).
- `max_bytes_per_batch` (e.g. 2–4 MB).
- `max_time_ms` since last yield (to bound lock hold).

When any threshold is hit:

1. Convert staging stores into actual writes (`commit()`).
2. `db.write(batch)` once.
3. Release expensive guards, yield the pruning lock (`prune_guard.blocking_yield()`), and recreate the context.

### 3.2 Metadata vs Block Bodies

Block body deletions (`block_transactions_store`, `utxo_diffs_store`, `acceptance_data_store`) currently enqueue deletions into the same batch and are not large (deletes carry only keys). The risk is staging maps that mirror those keys. Two options:

- **Default behaviour**: include bodies in the main batch (since they are tombstones, not payloads).
- **Optional guardrail**: track key count/bytes per store; if a batch accumulates too many body deletions, flush early or split into sub-batches to bound memory.

We do not plan to maintain a separate body batch unless profiling shows RAM pressure.

### 3.3 Reachability / Relations Staging

`StagingReachabilityStore::new` currently consumes an upgradable read guard and, on `commit`, returns a write guard. For multi-block staging:

- Keep a single `reachability_read` guard per batch.
- Accumulate operations in the staging store.
- On `flush`, call `commit()` once, drop the returned write guard immediately after writing, then reopen a fresh upgradable guard for the next batch.

This avoids repeated reallocation while still freeing resources between batches.

### 3.4 Lock Management

Current code yields whenever the lock has been held for >5 ms. With batching, we need a similar mechanism:

- Track `batch_start = Instant::now()` when acquiring `reachability_read`.
- On each block, if `batch_start.elapsed()` exceeds the configured limit (e.g. 20–30 ms), flush early even if thresholds aren’t hit.
- After `db.write`, call `prune_guard.blocking_yield()` and reestablish the context. The metrics instrumentation already counts yields; we can monitor whether yield frequency drops too low.

### 3.5 Crash Safety & Checkpoints

Current implementation updates the retention checkpoint in a separate batch when pruning completes (`processor.rs:662-671`). Phase 1 should:

- Stage the retention checkpoint update in the **same final batch** as the last deletions.
- Optionally write a “pruning_in_progress” marker before the loop and clear it inside the final commit, so restart logic knows whether an incomplete batch needs cleanup (aligns with Gemini’s recommendation).

If the node crashes mid-batch, the last committed batch remains fully applied, and any staged-but-uncommitted blocks can be retried safely (their deletes are idempotent).

### 3.6 Instrumentation & Validation

Keep the Phase 0 logging helpers but adapt them to record new batch sizes. Add counters for:

- Blocks / ops / bytes per flush (post-batching).
- Lock hold durations per batch.
- Total batches per prune cycle.

These metrics will confirm batching effectiveness and help tune thresholds.

## 4. Implementation Plan (High Level)

1. **Refactor loop skeleton**: wrap per-block logic with `PruneBatchContext` responsible for staging and flushing.
2. **Integrate thresholds**; start with conservative values (e.g. 128 blocks, 8k ops, 1 MB) and adjust after observing Phase 2 metrics.
3. **Unify checkpoint update** into final batch; add optional in-progress marker if needed for recovery clarity.
4. **Retain yield logic** but gate by cumulative batch time. Ensure we still re-open staging structures cleanly after each flush.
5. **Guard rails**: add sanity asserts in debug mode (e.g. staging ops count equals tracked counter).
6. **Documentation/tests**: update runbook to gather new metrics; build targeted integration test that prunes a synthetic DAG with >threshold blocks to ensure multi-block batches exercise flush logic.

## 5. Open Questions

- **Optimal thresholds**: need Phase 2 experiments to balance throughput vs. lock latency across SSD vs. HDD environments.
- **DeleteRange**: still optional—should revisit once multi-block batching is stable to see if contiguous block IDs make range deletes worthwhile.
- **Recovery marker semantics**: define precise expectations with consensus team before writing DB flags (avoid conflicts with existing recovery flow).
- **Memory telemetry**: consider tracking peak staging map size to detect unexpected growth during batch staging.

## 6. Next Steps

1. Prototype `PruneBatchContext` and restructure the per-block loop (Phase 2 implementation).
2. Run the same testnet baseline with new code, collect `[PRUNING METRICS]`, and compare commit counts/latency.
3. Iterate on thresholds and lock timing; document tuning results.
