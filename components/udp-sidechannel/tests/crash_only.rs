use bytes::Bytes;
use kaspa_udp_sidechannel::{
    frame::{
        assembler::{FrameAssembler, FrameAssemblerConfig, ReassembledFrame},
        header::{HeaderParseContext, SatFrameHeader, HEADER_LEN},
        DropEvent, PayloadCaps,
    },
    metrics::UdpMetrics,
    runtime::{FrameRuntime, RuntimeConfig, RuntimeDecision},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
fn crash_only_corpus_replay() {
    let seeds = load_seed_datagrams();
    assert!(!seeds.is_empty(), "fuzz corpus missing");

    let mut harness = Harness::new();
    let mut now = Instant::now();

    for frame in &seeds {
        harness.process(frame, now);
        now += Duration::from_millis(5);
    }

    // Simulate graceful shutdown by advancing the clock to flush fragments.
    now += Duration::from_secs(1);
    harness.advance(now);

    // Replay the corpus again while "shutdown" is in progress.
    for frame in &seeds {
        harness.process(frame, now);
        now += Duration::from_millis(5);
    }
    harness.advance(now + Duration::from_secs(1));
    assert!(harness.drop_events.is_empty());

    let drops = harness.metrics.drops_snapshot();
    let panic_count = drops.iter().find(|(reason, _)| *reason == "panic").map(|(_, count)| *count).unwrap_or(0);
    assert_eq!(panic_count, 0, "panic drop reason observed: {drops:?}");
}

fn load_seed_datagrams() -> Vec<Vec<u8>> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz").join("corpus").join("frame_header");
    let mut frames = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    // For header fuzzing seeds the payload may be zero-length; pad to HEADER_LEN.
                    if bytes.len() >= HEADER_LEN {
                        frames.push(bytes);
                    }
                }
            }
        }
    }
    frames
}

struct Harness {
    assembler: FrameAssembler,
    runtime: FrameRuntime,
    ctx: HeaderParseContext,
    metrics: UdpMetrics,
    drop_events: Vec<DropEvent>,
}

impl Harness {
    fn new() -> Self {
        let assembler = FrameAssembler::new(FrameAssemblerConfig {
            max_groups: 16,
            max_buffer_bytes: 128 * 1024,
            fragment_ttl: Duration::from_secs(1),
        });
        let runtime = FrameRuntime::new(RuntimeConfig {
            max_kbps: 25,
            dedup_window: 16,
            dedup_max_entries: 64,
            dedup_retention: Duration::from_secs(2),
            snapshot_overdraft_factor: 2.0,
        });
        let ctx = HeaderParseContext { network_tag: 0x01, payload_caps: PayloadCaps { digest: 4096, block: 0, tx: 0 } };
        Self { assembler, runtime, ctx, metrics: UdpMetrics::new(), drop_events: Vec::new() }
    }

    fn process(&mut self, datagram: &[u8], now: Instant) {
        self.assembler.collect_expired(now, &mut self.drop_events);
        self.consume_drop_events();
        match SatFrameHeader::parse(datagram, &self.ctx) {
            Ok(parsed) => {
                let payload = Bytes::copy_from_slice(parsed.payload);
                if let Some(frame) = self.assembler.ingest(parsed.header, payload, now, &mut self.drop_events) {
                    self.consume_drop_events();
                    self.handle_frame(frame, now);
                }
            }
            Err(err) => self.metrics.record_drop(err.reason),
        }
    }

    fn advance(&mut self, now: Instant) {
        self.assembler.collect_expired(now, &mut self.drop_events);
        self.consume_drop_events();
    }

    fn handle_frame(&mut self, frame: ReassembledFrame, now: Instant) {
        match self.runtime.evaluate(&frame.header, &frame.payload, now) {
            RuntimeDecision::Accept => {
                self.metrics.record_frame(frame.header.kind, frame.payload.len() + HEADER_LEN);
            }
            RuntimeDecision::Drop { reason, .. } => {
                self.metrics.record_drop(reason);
            }
        }
    }

    fn consume_drop_events(&mut self) {
        for event in self.drop_events.drain(..) {
            self.metrics.record_drop(event.reason);
        }
    }
}
