use std::{
    env, mem,
    time::{Duration, Instant},
};

use rocksdb::WriteBatch;

use kaspa_core::{info, warn};
use kaspa_database::prelude::DB;

#[derive(Default)]
struct CommitStats {
    commits: usize,
    total_ops: usize,
    max_ops: usize,
    total_bytes: usize,
    max_bytes: usize,
    total_duration: Duration,
    max_duration: Duration,
}

impl CommitStats {
    fn record(&mut self, ops: usize, bytes: usize, duration: Duration) {
        self.commits += 1;
        self.total_ops += ops;
        self.max_ops = self.max_ops.max(ops);
        self.total_bytes += bytes;
        self.max_bytes = self.max_bytes.max(bytes);
        self.total_duration += duration;
        self.max_duration = self.max_duration.max(duration);
    }
}

pub(super) struct PruningPhaseMetrics {
    started: Instant,
    config: PruneBatchLimits,
    batch_commit: CommitStats,
    body_commit: CommitStats,
    total_traversed: usize,
    total_pruned: usize,
}

impl PruningPhaseMetrics {
    pub(super) fn new(config: PruneBatchLimits) -> Self {
        Self {
            started: Instant::now(),
            config,
            batch_commit: CommitStats::default(),
            body_commit: CommitStats::default(),
            total_traversed: 0,
            total_pruned: 0,
        }
    }

    pub(super) fn record_batch_commit(&mut self, ops: usize, bytes: usize, duration: Duration) {
        self.batch_commit.record(ops, bytes, duration);
    }

    pub(super) fn record_body_commit(&mut self, ops: usize, bytes: usize, duration: Duration) {
        self.body_commit.record(ops, bytes, duration);
    }

    pub(super) fn set_traversed(&mut self, traversed: usize, pruned: usize) {
        self.total_traversed = traversed;
        self.total_pruned = pruned;
    }

    pub(super) fn log_summary(&self) {
        let elapsed_ms = self.started.elapsed().as_millis();
        info!(
            "[PRUNING METRICS] config_lock_max_ms={} config_batch_max_ms={} config_batch_max_ops={} config_batch_max_bytes={} config_batch_max_blocks={} config_batch_bodies={} duration_ms={} traversed={} pruned={}",
            self.config.max_lock_duration.as_millis(),
            self.config.max_duration.as_millis(),
            self.config.max_ops,
            self.config.max_bytes,
            self.config.max_blocks,
            self.config.batch_bodies as u8,
            elapsed_ms,
            self.total_traversed,
            self.total_pruned
        );
        if self.batch_commit.commits == 0 {
            return;
        }
        let avg_ops = self.batch_commit.total_ops as f64 / self.batch_commit.commits as f64;
        let avg_bytes = self.batch_commit.total_bytes as f64 / self.batch_commit.commits as f64;
        let avg_duration_ms = self.batch_commit.total_duration.as_secs_f64() * 1000.0 / self.batch_commit.commits as f64;
        info!(
            "[PRUNING METRICS] commit_type=batched count={} avg_ops={:.2} max_ops={} avg_bytes={:.2} max_bytes={} avg_commit_ms={:.3} max_commit_ms={:.3}",
            self.batch_commit.commits,
            avg_ops,
            self.batch_commit.max_ops,
            avg_bytes,
            self.batch_commit.max_bytes,
            avg_duration_ms,
            self.batch_commit.max_duration.as_secs_f64() * 1000.0
        );
        if self.body_commit.commits > 0 {
            let avg_ops = self.body_commit.total_ops as f64 / self.body_commit.commits as f64;
            let avg_bytes = self.body_commit.total_bytes as f64 / self.body_commit.commits as f64;
            let avg_duration_ms = self.body_commit.total_duration.as_secs_f64() * 1000.0 / self.body_commit.commits as f64;
            info!(
                "[PRUNING METRICS] commit_type=body_per_block count={} avg_ops={:.2} max_ops={} avg_bytes={:.2} max_bytes={} avg_commit_ms={:.3} max_commit_ms={:.3}",
                self.body_commit.commits,
                avg_ops,
                self.body_commit.max_ops,
                avg_bytes,
                self.body_commit.max_bytes,
                avg_duration_ms,
                self.body_commit.max_duration.as_secs_f64() * 1000.0
            );
        }
    }
}

const DEFAULT_PRUNE_BATCH_MAX_BLOCKS: usize = 256;
const DEFAULT_PRUNE_BATCH_MAX_OPS: usize = 50_000;
const DEFAULT_PRUNE_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_PRUNE_BATCH_MAX_DURATION_MS: u64 = 50;
const DEFAULT_PRUNE_LOCK_MAX_DURATION_MS: u64 = 25;
const DEFAULT_PRUNE_BATCH_BODIES: bool = true;

#[derive(Clone, Copy)]
pub(super) struct PruneBatchLimits {
    pub(super) max_blocks: usize,
    pub(super) max_ops: usize,
    pub(super) max_bytes: usize,
    pub(super) max_duration: Duration,
    pub(super) max_lock_duration: Duration,
    pub(super) batch_bodies: bool,
}

impl PruneBatchLimits {
    pub(super) fn from_env() -> Self {
        let max_blocks = env_usize("PRUNE_BATCH_MAX_BLOCKS").unwrap_or(DEFAULT_PRUNE_BATCH_MAX_BLOCKS);
        let max_ops = env_usize("PRUNE_BATCH_MAX_OPS").unwrap_or(DEFAULT_PRUNE_BATCH_MAX_OPS);
        let max_bytes = env_usize("PRUNE_BATCH_MAX_BYTES").unwrap_or(DEFAULT_PRUNE_BATCH_MAX_BYTES);
        let max_duration_ms = env_u64("PRUNE_BATCH_MAX_DURATION_MS").unwrap_or(DEFAULT_PRUNE_BATCH_MAX_DURATION_MS);
        let max_lock_duration_ms = env_u64("PRUNE_LOCK_MAX_DURATION_MS").unwrap_or(DEFAULT_PRUNE_LOCK_MAX_DURATION_MS);
        let batch_bodies = env_bool("PRUNE_BATCH_BODIES").unwrap_or(DEFAULT_PRUNE_BATCH_BODIES);

        Self {
            max_blocks,
            max_ops,
            max_bytes,
            max_duration: Duration::from_millis(max_duration_ms),
            max_lock_duration: Duration::from_millis(max_lock_duration_ms),
            batch_bodies,
        }
    }
}

pub(super) struct PruneBatch {
    pub(super) batch: WriteBatch,
    block_count: usize,
    started: Option<Instant>,
    limits: PruneBatchLimits,
}

impl PruneBatch {
    pub(super) fn new(limits: PruneBatchLimits) -> Self {
        Self { batch: WriteBatch::default(), block_count: 0, started: None, limits }
    }

    pub(super) fn on_block_staged(&mut self) {
        if self.block_count == 0 {
            self.started = Some(Instant::now());
        }
        self.block_count += 1;
    }

    pub(super) fn len(&self) -> usize {
        self.batch.len()
    }

    pub(super) fn size_in_bytes(&self) -> usize {
        self.batch.size_in_bytes()
    }

    pub(super) fn blocks(&self) -> usize {
        self.block_count
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.started.map(|t| t.elapsed()).unwrap_or_default()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.batch.len() == 0
    }

    pub(super) fn take(&mut self) -> WriteBatch {
        self.block_count = 0;
        self.started = None;
        mem::take(&mut self.batch)
    }

    pub(super) fn should_flush(&self, lock_elapsed: Duration) -> bool {
        if self.is_empty() {
            return false;
        }

        self.blocks() >= self.limits.max_blocks
            || self.len() >= self.limits.max_ops
            || self.size_in_bytes() >= self.limits.max_bytes
            || self.elapsed() >= self.limits.max_duration
            || lock_elapsed >= self.limits.max_lock_duration
    }

    pub(super) fn flush(&mut self, db: &DB, metrics: &mut PruningPhaseMetrics) {
        if self.is_empty() {
            return;
        }

        let ops = self.len();
        let bytes = self.size_in_bytes();
        let commit_start = Instant::now();
        let write_batch = self.take();
        db.write(write_batch).unwrap();
        metrics.record_batch_commit(ops, bytes, commit_start.elapsed());
    }
}

fn env_u64(name: &str) -> Option<u64> {
    match env::var(name) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(value) => Some(value),
            Err(_) => {
                warn!("Invalid value for {}: {}", name, raw);
                None
            }
        },
        Err(_) => None,
    }
}

fn env_usize(name: &str) -> Option<usize> {
    match env::var(name) {
        Ok(raw) => match raw.parse::<usize>() {
            Ok(value) => Some(value),
            Err(_) => {
                warn!("Invalid value for {}: {}", name, raw);
                None
            }
        },
        Err(_) => None,
    }
}

fn env_bool(name: &str) -> Option<bool> {
    match env::var(name) {
        Ok(raw) => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => {
                warn!("Invalid value for {}: {}", name, raw);
                None
            }
        },
        Err(_) => None,
    }
}
