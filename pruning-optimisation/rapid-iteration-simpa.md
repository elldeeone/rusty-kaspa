# Rapid Pruning Iteration via Simpa

Use Simpa to reproduce mainnet-scale pruning workloads in a few hours instead of waiting for the live node’s 12 h cadence. This workflow mirrors today’s mainnet setup closely enough to evaluate lock caps, batch sizes, and RocksDB tuning before running a full soak.

## 1. Snapshot the live datadir (optional but recommended)

```
SNAP=pruning-optimisation/baseline/experiments/simpa-db-mainnet-20251111
rsync -a --delete ~/.rusty-kaspa/kaspa-mainnet/datadir/ "${SNAP}/"
```

Point `run-simpa-pruning.sh` at this directory to reuse the current reachability/UTXO state. Even though the snapshot is post-prune, it preserves the present horizon and lets Simpa advance immediately.

## 2. Run Simpa with mainnet-like load

```
SIMPA_DB_DIR="${SNAP}" \
SIMPA_BPS=10 \
SIMPA_MINERS=4 \
SIMPA_LONG_PAYLOAD=1 \
SIMPA_RETENTION_DAYS=0.5 \
SIMPA_ROCKSDB_STATS=1 \
./pruning-optimisation/run-simpa-pruning.sh
```

- If the snapshot does not exist, add `SIMPA_FORCE_REBUILD=1 SIMPA_TARGET_BLOCKS=120000` to generate ~120 k blocks in one shot. With `SIMPA_BPS=10` this finishes in ~3 h on a laptop but produces the same pruning depth as the multi-hour mainnet run.
- Enable pipelined writes via `SIMPA_ROCKSDB_PIPELINED=1` when you want to test RocksDB’s pipelined WAL path.

## 3. Sweep lock/batch overrides quickly

Simpa forwards environment overrides to the pruning processor, so you can emulate different strategies without rebuilding:

```
PRUNE_LOCK_MAX_DURATION_MS=25 \
PRUNE_BATCH_MAX_BYTES=524288 \
PRUNE_BATCH_MAX_OPS=1500 \
SIMPA_DB_DIR="${SNAP}" \
./pruning-optimisation/run-simpa-pruning.sh
```

- `PRUNE_LOCK_MAX_DURATION_MS` keeps the reachability lock within Michael’s 10–30 ms target range.
- `PRUNE_BATCH_MAX_{BYTES,OPS}` prevents staging from growing beyond the SSD’s sweet spot while still reducing commit counts.
- Pair these with `SIMPA_ROCKSDB_STATS{,_PERIOD_SEC}` to see how disk throughput responds.

## 4. Harvest metrics after each run

`run-simpa-pruning.sh` automatically creates `simpa-prune-YYYYmmdd-HHMMSS.{log,csv}` under `pruning-optimisation/baseline/experiments/`. Use the same tooling as the live node:

```
python3 pruning-optimisation/scripts/collect_pruning_metrics.py <log> > <csv>
```

Compare the resulting CSV against the mainnet artifacts to validate improvements before restarting the real node with new settings.
