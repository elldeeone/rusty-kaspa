use crate::frame::{DropReason, FrameKind};
use std::array;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct UdpMetrics {
    frames_total: [AtomicU64; FrameKind::ALL.len()],
    drops_total: [AtomicU64; DropReason::ALL.len()],
    bytes_total: AtomicU64,
}

impl Default for UdpMetrics {
    fn default() -> Self {
        Self {
            frames_total: array::from_fn(|_| AtomicU64::new(0)),
            drops_total: array::from_fn(|_| AtomicU64::new(0)),
            bytes_total: AtomicU64::new(0),
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
    }

    pub fn record_drop(&self, reason: DropReason) {
        self.drops_total[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn bytes_total(&self) -> u64 {
        self.bytes_total.load(Ordering::Relaxed)
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
