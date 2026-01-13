# 2025-11-12 Pruning Runs – Codex Cross-Check

Codex review of the two archived mainnet pruning runs captured on 2025‑11‑12. No existing files were modified; this document lives alongside the original artifacts under `pruning-optimisation/testing-runs/`.

## Source Files
- Baseline (no batching): `20251112-baseline-no-batch/full-run.log`, `perf-metrics.txt`, `pruning-metrics.txt`
- Batched (25 ms lock window): `20251112-prune-batching-branch/full-run.log`, `perf-metrics.txt`, `pruning-metrics.txt`

## Pruning Metrics Snapshot

| Metric | Baseline (per-block commits) | Batched (25 ms lock) | Delta |
| --- | --- | --- | --- |
| Pruning duration | 6 340 177 ms = 105.67 min = 1.76 h | 5 562 865 ms = 92.71 min = 1.55 h | −12.96 min (−12.3 %) |
| Blocks traversed / pruned | 474 895 / 432 859 | 473 871 / 431 296 | −1 024 traversed / −1 563 pruned (≈0.2 %) |
| Lock hold time | 5 340 010 ms (84.2 % of pruning) | 4 715 216 ms (84.8 %) | −624 794 ms (−11.7 %) |
| Lock yields | 417 863 | 129 190 | −288 673 (−69.1 %) |
| Commit count | 474 882 per-block + 3 specials | 129 188 batched + 2 specials | −345 694 commits (−72.8 %) |
| Avg ops / commit | 282.30 | 1 028.16 | +745.86 (+265 %) |
| Avg bytes / commit | 31 622.66 B (31.6 KB) | 116 987.74 B (117 KB) | +85 365.08 B (+270 %) |
| Avg commit latency | 0.246 ms (max 426.607 ms) | 0.688 ms (max 443.794 ms) | +0.442 ms (+180 %) |
| Total commit time | 116.8 s = 1.84 % of duration | 88.9 s = 1.60 % of duration | −27.9 s |

Source lines: `20251112-baseline-no-batch/pruning-metrics.txt:1-5`, `20251112-prune-batching-branch/pruning-metrics.txt:1-4`.

## Performance Monitor Summary (pruning windows)

Statistics are computed from every `perf-monitor` sample taken between the “Starting Header and Block pruning…” log entry and the corresponding `[PRUNING METRICS]` line (613 samples for the baseline window at 14:46:12–16:28:22, 543 samples for the batched window at 02:44:04–04:14:32).

| Metric | Baseline avg (min↔max) | Batched avg (min↔max) |
| --- | --- | --- |
| CPU usage (fraction of one core) | 0.47 (0.23↔1.20) | 0.43 (0.20↔1.03) |
| RAM resident set | 1.33 GiB (0.19↔2.03 GiB) | 1.49 GiB (0.82↔2.23 GiB) |
| Disk read rate | 140 MB/s (16↔205 MB/s) | 154 MB/s (17↔203 MB/s) |
| Disk write rate | 19.5 MB/s (0↔88 MB/s) | 21.9 MB/s (0↔94 MB/s) |

Underlying samples: `20251112-baseline-no-batch/perf-metrics.txt:3899-5136`, `20251112-prune-batching-branch/perf-metrics.txt:5465-6554`.

## Baseline Run (per-block commits)

### Timeline
- 09:20:33 — `kaspad v1.0.1-cf01f294` started (`full-run.log:1`).
- 13:08:48 — Final IBD handshake completed (`full-run.log:5479-5489`).
- 14:46:12 — “Starting Header and Block pruning…” (`full-run.log:13147-13153`).
- 16:28:22 — Pruning metrics emitted plus trusted-data rebuild notice (`full-run.log:21622-21666` and `pruning-metrics.txt:1-5`).

### Pruning behaviour
- Traversal advanced 474 895 blocks and deleted 432 859, matching the final counter message (`pruning-metrics.txt:1`).
- Lock contention stayed high: 417 863 yields and virtually identical reacquires, one per per-block commit (`pruning-metrics.txt:1`).
- Commit mix remained vanilla: 474 882 per-block commits plus single ghostdag/retention/tips commits (`pruning-metrics.txt:2-5`).
- Each commit held ~282 ops / 31.6 KB and completed in 0.246 ms on average; worst-case commit latency hit 426.607 ms when the tips/selected-chain flush wrote 3.39 MB (`pruning-metrics.txt:3,5`).

### Perf monitor around pruning

Representative samples pulled directly from `perf-metrics.txt`:

| Phase | Timestamp | CPU | RAM | Disk read / write |
| --- | --- | --- | --- | --- |
| Kick-off | 14:46:14 | 0.56 cores, 1.87 GB | 120 MB/s read, 28 KB/s write (`perf-metrics.txt:3903-3904`) |
| Mid-pass | 15:30:17 | 0.54 cores, 1.51 GB | 163 MB/s read, 46 MB/s write (`perf-metrics.txt:4431-4432`) |
| Final minute | 16:28:18 | 0.38 cores, 1.79 GB | 158 MB/s read, 0.21 MB/s write (`perf-metrics.txt:5127-5128`) |

Across the entire 613-sample window, averages and ranges are reported in the Performance Monitor Summary table above.

## Batched Run (25 ms lock window)

### Timeline
- 19:08:04 (Nov 11) — `kaspad v1.0.1-783b30ce` started (`full-run.log:1`).
- 00:37:39 — IBD finished after the second handshake (`full-run.log:7967-7989`).
- 02:44:04 — “Starting Header and Block pruning…” (`full-run.log:17941-17946`).
- 04:14:32 — Batched `[PRUNING METRICS]` block, including the exported configuration (`full-run.log:25380-25390`, `pruning-metrics.txt:1-4`).

### Pruning behaviour
- Configuration confirmed at runtime: lock max 25 ms, batch window 50 ms, 50 000 ops, 4 MB cap (`pruning-metrics.txt:1`).
- Traversal/prune counts (473 871 / 431 296) match the completion banner at 04:12:10 before the metrics dump (`full-run.log:25380-25390`, `pruning-metrics.txt:1`).
- Batched commits collapsed to 129 188 entries while maintaining one ghostdag adjust and one tips/selected-chain flush (`pruning-metrics.txt:1-4`).
- Average batch packed 1 028 ops (117 KB payload) and committed in 0.688 ms; the hottest batch reached 3 698 ops / 610 KB / 443.794 ms (`pruning-metrics.txt:2`).
- Lock yields dropped to 129 190 (a yield roughly every 3.7 blocks) and the lock stayed held 4.72 s out of every 5.56 s of pruning time (`pruning-metrics.txt:1`).

### Perf monitor around pruning

Representative samples:

| Phase | Timestamp | CPU | RAM | Disk read / write |
| --- | --- | --- | --- | --- |
| Kick-off | 02:44:09 | 0.54 cores, 1.61 GB | 189 MB/s read, 63 MB/s write (`perf-metrics.txt:5467-5468`) |
| Mid-pass | 03:30:20 | 0.35 cores, 1.55 GB | 153 MB/s read, 20 MB/s write (`perf-metrics.txt:6021-6022`) |
| Final minute | 04:14:10 | 1.00 cores, 2.02 GB | 17 MB/s read, 0 MB/s write (`perf-metrics.txt:6547-6548`) |

Aggregated averages/ranges across the 543-sample window appear in the Performance Monitor Summary table.

## Direct Comparison Notes

1. **Pruning duration** — Batched mode trimmed 12.96 minutes (12.3 %) off the pruning phase while touching essentially the same DAG span (`pruning-metrics.txt` files cited above).
2. **Commit pressure** — Commits dropped by 345 694 (−72.8 %), taking lock yields down by 69.1 % and reducing total commit CPU time by 27.9 s even though each batch commit costs 0.688 ms (`pruning-metrics.txt:1-4` in both runs).
3. **Lock occupancy** — Both modes spent ~84–85 % of pruning time holding the pruning lock, but batching kept progress with far fewer context switches (one yield every ~3.7 blocks vs. every block).
4. **I/O profile** — Batched mode sustained slightly higher average disk throughput (154 MB/s read, 21.9 MB/s write) and a higher RAM working set (1.49 GiB vs. 1.33 GiB), consistent with holding larger RocksDB batches in memory (`perf-metrics.txt:5465-6554` vs. `perf-metrics.txt:3899-5136`).
5. **Responsiveness** — The 25 ms lock limit kept CPU usage within the same 0.2–1.0 core envelope as the baseline while still surfacing the `[PRUNING METRICS]` config stanza for post-run auditing (`pruning-metrics.txt:1` in the batched run).

These findings are fully traceable to the archived logs listed in the Source Files section. To reproduce the aggregated performance numbers, filter the corresponding `perf-metrics.txt` files to the pruning windows described above and compute the arithmetic means and extrema of the reported samples.
