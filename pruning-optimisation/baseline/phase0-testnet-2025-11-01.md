# Phase 0 Baseline – Testnet (2025-11-01)

- **Command:** `./target/release/kaspad --testnet --retention-period-days=2.0 --loglevel=info,consensus=trace`
- **Repo commit:** `08018e7925830c7d8fe8bc1185782eb53dab3da6`
- **Retention window:** 2 days (protocol minimum)
- **Log file:** `pruning-baseline.log`
- **Start time:** 2025-10-31 15:20:24 +11:00
- **End time:** 2025-11-01 05:15:26 +11:00

## Aggregated Metrics (from `[PRUNING METRICS]` lines)

```
duration_ms=527146
traversed=52327
pruned=30913
lock_hold_ms=244603
lock_yields=31790
lock_reacquires=84118

commit_type=ghostdag_adjust
  count=1
  avg_ops=14.00 (max=14)
  avg_bytes=2698.00 (max=2698)
  avg_commit_ms=0.041 (max=0.041)

commit_type=per_block
  count=52327
  avg_ops=62.82 (max=313)
  avg_bytes=3906.09 (max=21475)
  avg_commit_ms=0.048 (max=15.592)

commit_type=retention_checkpoint
  count=1
  avg_ops=1.00 (max=1)
  avg_bytes=48.00 (max=48)
  avg_commit_ms=0.031 (max=0.031)

commit_type=tips_and_selected_chain
  count=1
  avg_ops=0.00 (max=0)
  avg_bytes=12.00 (max=12)
  avg_commit_ms=0.010 (max=0.010)
```

## Notes

- No pruning-related errors were recorded once the node was running with the valid retention window. The only error observed (`Retention period (0.1) must be at least 2 days`) came from an early failed launch and can be ignored.
- Metrics capture the current per-block commit behaviour: 52 327 individual RocksDB batches with average 62.8 operations and 3.9 KB payload each, confirming the fine-grained write pattern described in research.
