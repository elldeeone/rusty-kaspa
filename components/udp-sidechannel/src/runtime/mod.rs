use crate::frame::{header::HEADER_LEN, DropReason, FrameKind, SatFrameHeader};
use blake3::hash;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub max_kbps: u32,
    pub dedup_window: u64,
    pub dedup_max_entries: usize,
    pub dedup_retention: Duration,
    pub snapshot_overdraft_factor: f64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_kbps: 10,
            dedup_window: 4096,
            dedup_max_entries: 4096,
            dedup_retention: Duration::from_secs(30),
            snapshot_overdraft_factor: 2.0,
        }
    }
}

#[derive(Debug)]
pub struct FrameRuntime {
    digest_window: DedupWindow,
    block_window: DedupWindow,
    digest_bucket: TokenBucket,
    block_bucket: TokenBucket,
}

#[derive(Debug, Clone)]
struct DedupWindow {
    entries: VecDeque<Entry>,
    index: HashSet<u128>,
    retention: Duration,
    window_span: u64,
    max_entries: usize,
    highest_seq: u64,
}

#[derive(Debug, Clone)]
struct Entry {
    key: u128,
    timestamp: Instant,
}

#[derive(Debug)]
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    fill_rate: f64,
    overdraft: f64,
    last: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropClass {
    Delta,
    Snapshot,
}

impl DropClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DropClass::Delta => "delta",
            DropClass::Snapshot => "snapshot",
        }
    }
}

pub enum RuntimeDecision {
    Accept,
    Drop { reason: DropReason, drop_class: Option<DropClass> },
}

impl FrameRuntime {
    pub fn new(config: RuntimeConfig) -> Self {
        let bytes_per_sec = bytes_per_sec(config.max_kbps);
        let digest_bucket = TokenBucket::new(bytes_per_sec, config.snapshot_overdraft_factor);
        let block_bucket = TokenBucket::new(bytes_per_sec, 1.0);
        Self {
            digest_window: DedupWindow::new(config.dedup_window, config.dedup_max_entries, config.dedup_retention),
            block_window: DedupWindow::new(config.dedup_window, config.dedup_max_entries, config.dedup_retention),
            digest_bucket,
            block_bucket,
        }
    }

    pub fn evaluate(&mut self, header: &SatFrameHeader, payload: &[u8], now: Instant) -> RuntimeDecision {
        let hash = fingerprint(payload);
        let window = match header.kind {
            FrameKind::Digest => &mut self.digest_window,
            FrameKind::Block => &mut self.block_window,
        };

        match window.check(header.seq, hash, now) {
            DedupDecision::Duplicate => {
                return RuntimeDecision::Drop { reason: DropReason::Duplicate, drop_class: None };
            }
            DedupDecision::Stale => {
                return RuntimeDecision::Drop { reason: DropReason::StaleSeq, drop_class: None };
            }
            DedupDecision::Accept => {}
        }

        let cost = HEADER_LEN + payload.len();
        let traffic = match (header.kind, header.flags.digest_snapshot()) {
            (FrameKind::Digest, true) => TrafficClass::Snapshot,
            (FrameKind::Digest, false) => TrafficClass::Delta,
            (FrameKind::Block, _) => TrafficClass::Block,
        };

        let bucket = match header.kind {
            FrameKind::Digest => &mut self.digest_bucket,
            FrameKind::Block => &mut self.block_bucket,
        };

        match bucket.try_consume(cost, traffic, now) {
            BucketDecision::Accept => RuntimeDecision::Accept,
            BucketDecision::Drop(drop_class) => RuntimeDecision::Drop { reason: DropReason::RateCap, drop_class },
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TrafficClass {
    Delta,
    Snapshot,
    Block,
}

enum DedupDecision {
    Accept,
    Duplicate,
    Stale,
}

impl DedupWindow {
    fn new(window_span: u64, max_entries: usize, retention: Duration) -> Self {
        Self {
            entries: VecDeque::new(),
            index: HashSet::new(),
            retention,
            window_span,
            max_entries: max_entries.max(1),
            highest_seq: 0,
        }
    }

    fn check(&mut self, seq: u64, hash: u64, now: Instant) -> DedupDecision {
        self.purge(now);
        let floor = self.highest_seq.saturating_sub(self.window_span);
        if seq < floor {
            return DedupDecision::Stale;
        }
        let key = make_key(seq, hash);
        if self.index.contains(&key) {
            return DedupDecision::Duplicate;
        }
        self.insert(key, seq, now);
        DedupDecision::Accept
    }

    fn insert(&mut self, key: u128, seq: u64, now: Instant) {
        self.highest_seq = self.highest_seq.max(seq);
        self.entries.push_back(Entry { key, timestamp: now });
        self.index.insert(key);
        self.purge(now);
    }

    fn purge(&mut self, now: Instant) {
        while let Some(entry) = self.entries.front() {
            if self.entries.len() <= self.max_entries && now.duration_since(entry.timestamp) < self.retention {
                break;
            }
            if let Some(entry) = self.entries.pop_front() {
                self.index.remove(&entry.key);
            }
        }
    }
}

impl TokenBucket {
    fn new(bytes_per_sec: f64, overdraft_factor: f64) -> Self {
        let capacity = if bytes_per_sec <= 0.0 { 0.0 } else { bytes_per_sec * 2.0 };
        let overdraft = if bytes_per_sec <= 0.0 { 0.0 } else { bytes_per_sec * overdraft_factor.max(1.0) };
        Self { capacity, tokens: capacity, fill_rate: bytes_per_sec, overdraft, last: Instant::now() }
    }

    fn try_consume(&mut self, amount: usize, class: TrafficClass, now: Instant) -> BucketDecision {
        self.refill(now);
        let need = amount as f64;
        if self.capacity == 0.0 && self.fill_rate == 0.0 {
            return BucketDecision::Drop(match class {
                TrafficClass::Delta => Some(DropClass::Delta),
                TrafficClass::Snapshot => Some(DropClass::Snapshot),
                TrafficClass::Block => None,
            });
        }

        match class {
            TrafficClass::Snapshot => {
                if self.tokens + self.overdraft >= need {
                    self.tokens -= need;
                    BucketDecision::Accept
                } else {
                    BucketDecision::Drop(Some(DropClass::Snapshot))
                }
            }
            TrafficClass::Delta => {
                if self.tokens >= need {
                    self.tokens -= need;
                    BucketDecision::Accept
                } else {
                    BucketDecision::Drop(Some(DropClass::Delta))
                }
            }
            TrafficClass::Block => {
                if self.tokens >= need {
                    self.tokens -= need;
                    BucketDecision::Accept
                } else {
                    BucketDecision::Drop(None)
                }
            }
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last);
        if elapsed.is_zero() {
            return;
        }
        self.last = now;
        let replenish = self.fill_rate * elapsed.as_secs_f64();
        self.tokens = (self.tokens + replenish).min(self.capacity);
    }
}

enum BucketDecision {
    Accept,
    Drop(Option<DropClass>),
}

fn bytes_per_sec(max_kbps: u32) -> f64 {
    (max_kbps as f64 * 1024.0) / 8.0
}

fn make_key(seq: u64, hash: u64) -> u128 {
    ((seq as u128) << 64) | hash as u128
}

fn fingerprint(payload: &[u8]) -> u64 {
    let digest = hash(payload);
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::header::{FrameFlags, FrameKind, SatFrameHeader};

    fn header(seq: u64, snapshot: bool) -> SatFrameHeader {
        SatFrameHeader {
            kind: FrameKind::Digest,
            flags: if snapshot { FrameFlags::from_bits(0x08) } else { FrameFlags::from_bits(0x00) },
            seq,
            group_id: 0,
            group_k: 0,
            group_n: 0,
            frag_ix: 0,
            frag_cnt: 1,
            payload_len: 0,
            source_id: 0,
        }
    }

    #[test]
    fn dedup_and_ratecap() {
        let mut runtime = FrameRuntime::new(RuntimeConfig {
            max_kbps: 1,
            dedup_window: 4,
            dedup_max_entries: 8,
            dedup_retention: Duration::from_secs(1),
            snapshot_overdraft_factor: 2.0,
        });
        let mut now = Instant::now();
        let payload = vec![1u8; 32];

        // accept first delta
        assert!(matches!(runtime.evaluate(&header(1, false), &payload, now), RuntimeDecision::Accept));

        // duplicate
        assert!(matches!(
            runtime.evaluate(&header(1, false), &payload, now),
            RuntimeDecision::Drop { reason: DropReason::Duplicate, .. }
        ));

        // advance seq to raise floor
        now = now + Duration::from_millis(10);
        assert!(matches!(runtime.evaluate(&header(10, false), &payload, now), RuntimeDecision::Accept));
        // stale seq
        assert!(matches!(
            runtime.evaluate(&header(5, false), &payload, now),
            RuntimeDecision::Drop { reason: DropReason::StaleSeq, .. }
        ));

        // exhaust rate limiter with deltas
        let mut dropped_delta = false;
        for seq in 11..40 {
            let decision = runtime.evaluate(&header(seq, false), &payload, now);
            match decision {
                RuntimeDecision::Drop { reason: DropReason::RateCap, drop_class: Some(DropClass::Delta) } => {
                    dropped_delta = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(dropped_delta);

        // snapshots borrow against overdraft until exhausted
        let mut snapshot_drop = false;
        for seq in 100..140 {
            let decision = runtime.evaluate(&header(seq, true), &payload, now);
            match decision {
                RuntimeDecision::Drop { reason: DropReason::RateCap, drop_class: Some(DropClass::Snapshot) } => {
                    snapshot_drop = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(snapshot_drop);
    }
}
