use crate::frame::{DropReason, FrameKind};
use kaspa_core::time::unix_now;
use std::{
    array,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

const MAX_SKEW_SECONDS: u64 = 600;

#[derive(Debug)]
pub struct UdpMetrics {
    frames_total: [AtomicU64; FrameKind::ALL.len()],
    drops_total: [AtomicU64; DropReason::ALL.len()],
    bytes_total: AtomicU64,
    start_time: Instant,
    last_frame_ts_ms: AtomicU64,
    signature_failures: AtomicU64,
    skew_seconds: AtomicU64,
    divergence_detected: AtomicBool,
    block_injected_total: AtomicU64,
    block_queue_depth: AtomicU64,
}

impl Default for UdpMetrics {
    fn default() -> Self {
        Self {
            frames_total: array::from_fn(|_| AtomicU64::new(0)),
            drops_total: array::from_fn(|_| AtomicU64::new(0)),
            bytes_total: AtomicU64::new(0),
            start_time: Instant::now(),
            last_frame_ts_ms: AtomicU64::new(0),
            signature_failures: AtomicU64::new(0),
            skew_seconds: AtomicU64::new(0),
            divergence_detected: AtomicBool::new(false),
            block_injected_total: AtomicU64::new(0),
            block_queue_depth: AtomicU64::new(0),
        }
    }
}

impl UdpMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_frame(&self, kind: FrameKind, bytes: usize) {
        self.frames_total[kind.index()].fetch_add(1, Ordering::Relaxed);
        self.bytes_total.fetch_add(bytes as u64, Ordering::Relaxed);
        self.last_frame_ts_ms.store(unix_now(), Ordering::Relaxed);
    }

    pub fn record_signature_failure(&self) {
        self.signature_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_skew_seconds(&self, seconds: u64) {
        self.skew_seconds.store(seconds.min(MAX_SKEW_SECONDS), Ordering::Relaxed);
    }

    pub fn set_divergence_detected(&self, detected: bool) {
        self.divergence_detected.store(detected, Ordering::Relaxed);
    }

    pub fn record_block_injected(&self) {
        self.block_injected_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn block_injected_total(&self) -> u64 {
        self.block_injected_total.load(Ordering::Relaxed)
    }

    pub fn set_block_queue_depth(&self, depth: u64) {
        self.block_queue_depth.store(depth, Ordering::Relaxed);
    }

    pub fn block_queue_depth(&self) -> u64 {
        self.block_queue_depth.load(Ordering::Relaxed)
    }

    pub fn record_drop(&self, reason: DropReason) {
        self.drops_total[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn bytes_total(&self) -> u64 {
        self.bytes_total.load(Ordering::Relaxed)
    }

    pub fn rx_kbps(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64().max(1e-6);
        let bits = self.bytes_total() as f64 * 8.0;
        bits / elapsed / 1000.0
    }

    pub fn last_frame_ts_ms(&self) -> Option<u64> {
        let ts = self.last_frame_ts_ms.load(Ordering::Relaxed);
        if ts == 0 {
            None
        } else {
            Some(ts)
        }
    }

    pub fn signature_failures(&self) -> u64 {
        self.signature_failures.load(Ordering::Relaxed)
    }

    pub fn skew_seconds(&self) -> u64 {
        self.skew_seconds.load(Ordering::Relaxed)
    }

    pub fn divergence_detected(&self) -> bool {
        self.divergence_detected.load(Ordering::Relaxed)
    }

    pub fn frames_snapshot(&self) -> Vec<(&'static str, u64)> {
        FrameKind::ALL.iter().enumerate().map(|(idx, kind)| (kind.as_str(), self.frames_total[idx].load(Ordering::Relaxed))).collect()
    }

    pub fn drops_snapshot(&self) -> Vec<(&'static str, u64)> {
        DropReason::ALL
            .iter()
            .enumerate()
            .map(|(idx, reason)| (reason.as_str(), self.drops_total[idx].load(Ordering::Relaxed)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_drop_counts() {
        let metrics = UdpMetrics::new();
        assert_eq!(metrics.signature_failures(), 0);
        metrics.record_signature_failure();
        assert_eq!(metrics.signature_failures(), 1);
    }

    #[test]
    fn skew_metric_updates() {
        let metrics = UdpMetrics::new();
        metrics.record_skew_seconds(25);
        assert_eq!(metrics.skew_seconds(), 25);
    }
}
