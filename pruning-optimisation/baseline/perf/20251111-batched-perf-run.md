# 2025-11-11 Batched Mainnet Perf Snapshot

- Commit: repo `8f735413884b415576c343576c74794bd0f8d271`, kaspad binary `v1.0.1-24c05c39`
- Command: `LOG_LEVEL='info,consensus=trace,kaspad_lib::daemon=debug,kaspa_perf_monitor=debug' target/release/kaspad --reset-db --perf-metrics --perf-metrics-interval-sec=10`
- Network / retention: Mainnet, default pruning window (≈ 2 days)
- Perf interval: 10 s
- Log file: `pruning-optimisation/baseline/perf/20251111-batched-mainnet.log`
- Metrics CSV: `pruning-optimisation/baseline/perf/20251111-batched-pruning-metrics.csv`
- Notes:
  - Pruning completed in 6 019 954 ms while traversing 473 818 blocks and pruning 432 105 (`[PRUNING METRICS]` at lines 25430‑25433 in the log).
  - Batched commits: 67 559 batches averaging 1 944 ops, 220 KB, and 1.56 ms per RocksDB flush; lock yields dropped to 67 559 (‑83 % vs baseline) with the same traversed/pruned totals.
  - Perf monitor recorded 0.42 average CPU, 1.44 GB RSS, and frequent 200 MB/s write bursts, capturing the impact of larger batches for later comparison against the baseline run archived on 2025‑11‑10/11.
