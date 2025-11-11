# Pruning Test Run - 2025-11-12 - Prune-Batching Branch (25ms)

## Run Details
- **Branch**: prune-batching (commit 783b30ce)
- **Version**: kaspad v1.0.1-783b30ce
- **Started**: 2025-11-11 19:08:04
- **IBD Completed**: 2025-11-12 00:37:39 (5h 29m)
- **Pruning Completed**: 2025-11-12 04:14:32 (3h 37m after IBD)
- **Total Duration**: ~9 hours 6 minutes

## Configuration
- `config_lock_max_ms`: **25**
- `config_batch_max_ms`: 50
- `config_batch_max_ops`: 50000
- `config_batch_max_bytes`: 4194304 (4MB)

## Pruning Metrics Results

### Main Metrics
- **Total Pruning Duration**: 5,562,865 ms (92.7 minutes / 1.55 hours)
- **Blocks Traversed**: 473,871
- **Blocks Pruned**: 431,296
- **Lock Hold Time**: 4,715,216 ms (78.6 minutes, 84.8% of total time)
- **Lock Yields**: 129,190
- **Lock Reacquires**: 129,191

### Commit Statistics

#### Batched Commits
- **Count**: 129,188
- **Avg ops per commit**: 1,028.16
- **Max ops**: 3,698
- **Avg bytes per commit**: 116,987.74
- **Max bytes**: 610,032
- **Avg commit time**: 0.688 ms
- **Max commit time**: 443.794 ms
- **Total commit time estimate**: 129,188 × 0.688ms = 88.9 seconds (1.6% of pruning time)

#### Ghostdag Adjust Commit
- Count: 1
- Avg ops: 66.00
- Avg commit time: 0.280 ms

#### Tips and Selected Chain Commit
- Count: 1
- Ops: 145,658
- Bytes: 3,350,146
- Commit time: 52.425 ms

## Performance Metrics Summary

### Around Pruning Time (04:10-04:14)
- **RAM usage**: 1.5-1.9 GB
- **CPU usage**: 0.3-1.0 (30-100% of single core)
- **Disk read rate**: 146-178 MB/s
- **Disk write rate**: 2-66 MB/s (spiky during commits)
- **Total data read**: ~3.3 TB
- **Total data written**: ~1.06 TB

### Sync Phase Performance (normal operation)
- **Block processing**: ~100-110 blocks per 10s
- **Transaction throughput**: ~11-12 u-tps
- **Parents**: ~6-7 average
- **Mergeset**: ~6-8 average

## Analysis

### Batching Efficiency
- **Commits reduced**: From ~473K (per-block) to 129K (batched) = 72.7% reduction
- **Avg batch size**: 1,028 ops - very close to optimal
- **Lock yields**: 129,190 - one yield per ~3.7 blocks
- **Commit overhead**: 88.9s / 5562.9s = 1.6% of total pruning time

### Comparison to Baseline (from previous analysis)
This run compared to 20251110 25ms batched run:
- Duration: 92.7 min vs 89.9 min (3.1% slower, within noise)
- Batched commits: 129,188 vs 128,548 (very similar)
- Avg ops: 1,028 vs 993 (3.5% larger batches)
- Avg commit time: 0.688ms vs 0.665ms (3.5% slower, consistent with batch size)

### Configuration Validation
The new metrics output successfully shows config values:
```
[PRUNING METRICS] config_lock_max_ms=25 config_batch_max_ms=50 config_batch_max_ops=50000 config_batch_max_bytes=4194304 ...
```

This makes it easy to identify which settings produced the results when reviewing logs later.

## Conclusions

1. **25ms lock duration is optimal** - Produces consistent ~90 minute pruning times on fast NVMe SSD
2. **Batching is working correctly** - 72.7% reduction in commits, 1,028 ops/batch average
3. **Metrics enhancement successful** - Config values now visible in output
4. **Performance is stable** - Results match previous 25ms runs
5. **Commit overhead minimal on fast storage** - Only 1.6% of pruning time

## Next Steps

- Run non-batched baseline for direct comparison on same hardware/network conditions
- Consider testing on HDD to validate the 40-50% improvement hypothesis
