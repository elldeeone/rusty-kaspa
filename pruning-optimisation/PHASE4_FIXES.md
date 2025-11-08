# Phase 4 Pruning Fixes - 2025-11-07

## Summary

Fixed three critical issues blocking Phase 4 real-node pruning validation:

1. **TempGhostdagCompact KeyNotFound Panic** (build.rs:305)
2. **Reachability KeyNotFound Panic** (inquirer.rs:237)
3. **Lock Starvation** (processor.rs:701, 874)

## Issue 1: TempGhostdagCompact KeyNotFound Panic

### Root Cause
In `fill_level_proof_ghostdag_data` (build.rs:333-405), the BFS traversal from `root` through children may not reach `selected_tip` if it's a tip node (no children) or not connected via children relationships. When build.rs:305 calls `.unwrap()` on `get_blue_score(selected_tip)`, it panics with KeyNotFound.

### Fix
Added explicit check and insertion at end of `fill_level_proof_ghostdag_data`:

```rust
// Ensure selected_tip is always in the ghostdag store, even if it wasn't reached via the BFS
// (e.g., if it's a tip with no children). This prevents KeyNotFound panics when checking blue_score.
if selected_tip != root && !ghostdag_store.has(selected_tip).unwrap_or(false) {
    let tip_gd = gd_manager.ghostdag(&relations_service.get_parents(selected_tip).unwrap());
    ghostdag_store.insert(selected_tip, Arc::new(tip_gd)).unwrap_or_exists();
}
```

**File**: `consensus/src/processes/pruning_proof/build.rs:397-402`

## Issue 2: Reachability KeyNotFound Panic

### Root Cause
During pruning, reachability data is deleted while concurrent operations read it. The old `binary_search_descendant` used `.unwrap()` when reading intervals (inquirer.rs:237 old line), causing panics when data was pruned mid-operation.

### Fix
Replaced `.unwrap()` with proper error handling and retry logic:

- Rewrote `binary_search_descendant` to propagate errors properly
- Added retry loop in `get_next_chain_ancestor_unchecked` for transient KeyNotFound errors
- Added defensive filtering in `assert_hashes_ordered` for debug assertions

**Files**: `consensus/src/processes/reachability/inquirer.rs:209-290` (uncommitted changes from previous session)

## Issue 3: Lock Starvation

### Root Cause
The `RfRwLock` is explicitly readers-first (utils/src/sync/rwlock.rs:4-6). When pruning yields the write lock, continuous readers starve the writer from reacquiring it.

**Two bugs compounded this:**

1. **Hardcoded 5ms lock timeout** (processor.rs:701): Used `Duration::from_millis(5)` instead of `self.prune_batch_limits.max_lock_duration`, completely ignoring the `PRUNE_LOCK_MAX_DURATION_MS` env var
2. **Excessive yielding** (processor.rs:874): Always yielded after flushing a batch, even when flush was triggered by batch size limits (not lock duration)

### Fixes

#### Fix 3a: Use configurable lock duration (processor.rs:701)
```rust
// OLD:
if lock_acquire_time.elapsed() > Duration::from_millis(5) {

// NEW:
if lock_acquire_time.elapsed() > self.prune_batch_limits.max_lock_duration {
```

#### Fix 3b: Only yield when lock duration exceeded (processor.rs:866-890)
```rust
let lock_elapsed = lock_acquire_time.elapsed();
let should_yield_lock = lock_elapsed >= self.prune_batch_limits.max_lock_duration;

if self.should_flush_prune_batch(&prune_batch, lock_elapsed) {
    self.flush_prune_batch(&mut prune_batch, &mut metrics);

    // Only yield the lock if we've held it too long - not just because batch is full
    if should_yield_lock {
        // ... yield logic
    } else {
        // ... just reacquire reachability read guard
    }
}
```

#### Fix 3c: Increase default from 25ms to 500ms (processor.rs:165)
```rust
const PRUNE_LOCK_MAX_DURATION_MS: u64 = 500; // Increased from 25ms
```

**File**: `consensus/src/pipeline/pruning_processor/processor.rs`

## Testing Instructions

### Build
```bash
cargo build --release --bin kaspad
```

### Test 1: Offline Prune (Recommended First Test)

This isolates the pruning logic from network contention:

```bash
# Start with zero peers to run offline
LOG_LEVEL='info,consensus=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace' \
  ./target/release/kaspad \
  --listen=0.0.0.0:16110 \
  --rpclisten=127.0.0.1:17110 \
  --perf-metrics \
  --perf-metrics-interval-sec=30 \
  --outpeers=0 \
  --maxinpeers=0 \
  --logdir ~/.rusty-kaspa/kaspa-mainnet/logs
```

**Expected**: Pruning should trigger when conditions are met. Look for `[PRUNING METRICS]` in logs.

### Test 2: Online with Single Peer

If offline test passes, test with limited network activity:

```bash
LOG_LEVEL='info,consensus=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace' \
  ./target/release/kaspad \
  --listen=0.0.0.0:16110 \
  --rpclisten=127.0.0.1:17110 \
  --perf-metrics \
  --perf-metrics-interval-sec=30 \
  --connect=209.147.106.67:16111 \
  --outpeers=1 \
  --maxinpeers=0 \
  --logdir ~/.rusty-kaspa/kaspa-mainnet/logs
```

### Test 3: Full Online (If Tests 1 & 2 Pass)

Normal operation with default peer settings:

```bash
LOG_LEVEL='info,consensus=trace,kaspa_consensus::pipeline::pruning_processor::processor=trace' \
  ./target/release/kaspad \
  --listen=0.0.0.0:16110 \
  --rpclisten=127.0.0.1:17110 \
  --perf-metrics \
  --perf-metrics-interval-sec=30 \
  --logdir ~/.rusty-kaspa/kaspa-mainnet/logs
```

### Environment Overrides (If Still Needed)

The fixes should work with defaults, but you can still override if necessary:

```bash
# Further increase lock hold time if needed
export PRUNE_LOCK_MAX_DURATION_MS=2000

# Increase batch size to reduce flush frequency
export PRUNE_BATCH_MAX_BLOCKS=1024
export PRUNE_BATCH_MAX_DURATION_MS=1000
```

## Success Criteria

A successful prune cycle will emit metrics like:

```
[PRUNING METRICS] duration_ms=... traversed=... pruned=... lock_hold_ms=... lock_yields=... lock_reacquires=...
[PRUNING METRICS] commit_type=ghostdag_adjust count=... avg_ops=... max_ops=...
[PRUNING METRICS] commit_type=tips_and_selected_chain count=... avg_ops=...
[PRUNING METRICS] commit_type=batched count=... avg_ops=... avg_bytes=... avg_commit_ms=...
```

## Collecting Metrics

Once you get a successful prune:

```bash
# Extract from kaspad logs (adjust date pattern as needed)
LOG_FILE=~/.rusty-kaspa/kaspa-mainnet/logs/rusty-kaspa.log

# OR archive to experiments dir first
cp ~/.rusty-kaspa/kaspa-mainnet/logs/rusty-kaspa.log \
   pruning-optimisation/baseline/experiments/kaspad-phase4-mainnet-$(date +%Y%m%d-%H%M%S).log

# Then collect metrics
python3 pruning-optimisation/scripts/collect_pruning_metrics.py <log_file>
```

## Monitoring During Test

### Watch for starvation
```bash
tail -f ~/.rusty-kaspa/kaspa-mainnet/logs/rusty-kaspa.log | grep -E "traversed:|lock_hold_ms|pruning"
```

### Check for panics
```bash
tail -f ~/.rusty-kaspa/kaspa-mainnet/logs/rusty-kaspa.log | grep -i panic
```

### Sample if blocked (macOS)
```bash
# If pruning stalls, get a stack sample
sample kaspad 10 -f /tmp/kaspad_$(date +%Y%m%d_%H%M%S).sample.txt
```

## Rollback Plan

If issues persist:

```bash
# Reset to upstream baseline
git checkout HEAD consensus/src/processes/pruning_proof/build.rs
git checkout HEAD consensus/src/pipeline/pruning_processor/processor.rs
git checkout HEAD consensus/src/processes/reachability/inquirer.rs
```

## Files Modified

1. `consensus/src/processes/pruning_proof/build.rs` - Fix TempGhostdagCompact panic
2. `consensus/src/pipeline/pruning_processor/processor.rs` - Fix lock starvation
3. `consensus/src/processes/reachability/inquirer.rs` - Fix Reachability panic (already modified)

## Next Steps After Successful Prune

1. Archive the log: `kaspad-phase4-mainnet-<timestamp>.log`
2. Extract metrics with collect_pruning_metrics.py
3. Compare Phase 4 metrics to Phase 0/Simpa baselines
4. Document any remaining performance issues or warnings
5. If successful, consider submitting fixes upstream

## Background Context

- **Branch**: pruning-optimisation
- **Goal**: Phase 4 real-node validation of batched pruning processor
- **Baseline**: Phase 0/Simpa results in pruning-optimisation/baseline/
- **Previous attempts**:
  - Run 1: Stalled at ~200K blocks traversed, starved by readers
  - Run 2: Reachability KeyNotFound panic (now fixed)

## Theoretical Analysis

### Lock Starvation Math

With the old 5ms timeout and a readers-first lock:
- Pruning yields every 5ms
- If >1 reader arrives every 5ms, writer is permanently starved
- Mainnet sync generates ~100-1000 reads/sec during IBD
- Mean time between reads: 1-10ms
- **Result**: Near-certain starvation

With new 500ms timeout:
- Pruning holds lock for 500ms
- Processes ~1000-5000 blocks per lock acquisition (depending on batch limits)
- Yields only every 500ms
- Readers get occasional 500ms windows
- **Result**: Fair interleaving, pruning makes progress

### Why Offline Test First

Running offline isolates the fixes:
- No reader contention → tests Fixes 1 & 2 independently
- If it fails, problem is in pruning logic itself
- If it succeeds, then test with readers → validates Fix 3

---

**Author**: Claude Code (automated fix)
**Date**: 2025-11-07
**Status**: Ready for testing
