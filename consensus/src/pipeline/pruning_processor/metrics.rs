use std::time::{Duration, Instant};

use kaspa_core::info;

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

pub(super) struct PruningMetrics {
    started: Instant,
    lock_hold: Duration,
    lock_yields: usize,
    lock_reacquires: usize,
    total_traversed: usize,
    total_pruned: usize,
    commit: CommitStats,
}

impl PruningMetrics {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            lock_hold: Duration::ZERO,
            lock_yields: 0,
            lock_reacquires: 1,
            total_traversed: 0,
            total_pruned: 0,
            commit: CommitStats::default(),
        }
    }

    pub(super) fn record_commit(&mut self, ops: usize, bytes: usize, duration: Duration) {
        self.commit.record(ops, bytes, duration);
    }

    pub(super) fn record_yield(&mut self, lock_elapsed: Duration) {
        self.lock_hold += lock_elapsed;
        self.lock_yields += 1;
        self.lock_reacquires += 1;
    }

    pub(super) fn record_final_lock_hold(&mut self, lock_elapsed: Duration) {
        self.lock_hold += lock_elapsed;
    }

    pub(super) fn set_traversed(&mut self, traversed: usize, pruned: usize) {
        self.total_traversed = traversed;
        self.total_pruned = pruned;
    }

    pub(super) fn log_summary(&self) {
        let elapsed_ms = self.started.elapsed().as_millis();
        info!(
            "[PRUNING METRICS] duration_ms={} traversed={} pruned={} lock_hold_ms={} lock_yields={} lock_reacquires={} config_lock_max_ms=5 config_batch_max_ms=0 config_batch_max_ops=0 config_batch_max_bytes=0 config_batch_max_blocks=0 config_batch_bodies=1",
            elapsed_ms,
            self.total_traversed,
            self.total_pruned,
            self.lock_hold.as_millis(),
            self.lock_yields,
            self.lock_reacquires
        );
        if self.commit.commits == 0 {
            return;
        }
        let avg_ops = self.commit.total_ops as f64 / self.commit.commits as f64;
        let avg_bytes = self.commit.total_bytes as f64 / self.commit.commits as f64;
        let avg_duration_ms = self.commit.total_duration.as_secs_f64() * 1000.0 / self.commit.commits as f64;
        info!(
            "[PRUNING METRICS] commit_type=unbatched count={} avg_ops={:.2} max_ops={} avg_bytes={:.2} max_bytes={} avg_commit_ms={:.3} max_commit_ms={:.3}",
            self.commit.commits,
            avg_ops,
            self.commit.max_ops,
            avg_bytes,
            self.commit.max_bytes,
            avg_duration_ms,
            self.commit.max_duration.as_secs_f64() * 1000.0
        );
    }
}
