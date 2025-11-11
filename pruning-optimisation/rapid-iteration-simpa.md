# Rapid Pruning Iteration via Simpa

Use Simpa to reproduce mainnet-scale pruning workloads in a few hours instead of waiting for the live node’s 12 h cadence. This workflow mirrors today’s mainnet setup closely enough to evaluate lock caps, batch sizes, and RocksDB tuning before running a full soak.

## 1. Snapshot the live datadir (optional but recommended)

```
SNAP=pruning-optimisation/baseline/experiments/simpa-db-mainnet-20251111
rsync -a --delete ~/.rusty-kaspa/kaspa-mainnet/datadir/ "${SNAP}/"
```

Point `run-simpa-pruning.sh` at this directory to reuse the current reachability/UTXO state. Even though the snapshot is post-prune, it preserves the present horizon and lets Simpa advance immediately. (If the live node keeps running, `rsync` may complain about missing `.sst` files while RocksDB compacts; either stop the node briefly or skip this step and let Simpa regenerate a fresh database.)

## 2. Run Simpa with mainnet-like load

Use the convenience wrapper (all env vars still override the defaults):

```
PRUNE_LOCK_MAX_DURATION_MS=30 \
PRUNE_BATCH_MAX_BYTES=524288 \
./pruning-optimisation/run-simpa-mainnetish.sh
```

Defaults baked into the wrapper:

- 10 BPS, 4 miners, long payloads (~1.4 k tx/block) to mimic current mainnet throughput.
- Target 120 k blocks with a 0.5‑day retention window so pruning engages almost immediately.
- RocksDB stats sampled every 30 s; pipelined writes stay off unless `SIMPA_ROCKSDB_PIPELINED=1` is set.
- Database lives under `baseline/experiments/simpa-db-mainnetish` unless `SIMPA_DB_DIR` is overridden.

If no snapshot exists the wrapper will generate the workload automatically; toggle `SIMPA_FORCE_REBUILD=1` to wipe and re-seed between experiments.

## 3. Sweep lock/batch overrides quickly

Simpa forwards environment overrides to the pruning processor, so you can emulate different strategies without rebuilding:

```
PRUNE_LOCK_MAX_DURATION_MS=25 \
PRUNE_BATCH_MAX_BYTES=524288 \
PRUNE_BATCH_MAX_OPS=1500 \
./pruning-optimisation/run-simpa-mainnetish.sh
```

- `PRUNE_LOCK_MAX_DURATION_MS` keeps the reachability lock within Michael’s 10–30 ms target range.
- `PRUNE_BATCH_MAX_{BYTES,OPS}` prevents staging from growing beyond the SSD’s sweet spot while still reducing commit counts.
- Pair these with `SIMPA_ROCKSDB_STATS{,_PERIOD_SEC}` to see how disk throughput responds.

## 4. Harvest metrics after each run

Both wrappers drop logs/CSVs as `simpa-prune-YYYYmmdd-HHMMSS.{log,csv}` under `pruning-optimisation/baseline/experiments/`. Use the same tooling as the live node:

```
python3 pruning-optimisation/scripts/collect_pruning_metrics.py <log> > <csv>
```

Compare the resulting CSV against the mainnet artifacts to validate improvements before restarting the real node with new settings.
