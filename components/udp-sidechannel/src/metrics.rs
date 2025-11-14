use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct UdpMetrics {
    frames_total: AtomicU64,
    frames_dropped: AtomicU64,
    bytes_total: AtomicU64,
}

impl Default for UdpMetrics {
    fn default() -> Self {
        Self { frames_total: AtomicU64::new(0), frames_dropped: AtomicU64::new(0), bytes_total: AtomicU64::new(0) }
    }
}

impl UdpMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_frame(&self, bytes: usize) {
        self.frames_total.fetch_add(1, Ordering::Relaxed);
        self.bytes_total.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_drop(&self) {
        self.frames_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.frames_total.load(Ordering::Relaxed),
            self.frames_dropped.load(Ordering::Relaxed),
            self.bytes_total.load(Ordering::Relaxed),
        )
    }
}
