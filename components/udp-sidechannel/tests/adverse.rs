use bytes::Bytes;
use crc32fast::Hasher;
use kaspa_udp_sidechannel::{
    frame::{
        assembler::{FrameAssembler, FrameAssemblerConfig, ReassembledFrame},
        header::{HeaderParseContext, SatFrameHeader, HEADER_LEN},
        DropEvent, FrameKind, PayloadCaps,
    },
    runtime::{FrameRuntime, RuntimeConfig, RuntimeDecision},
    UdpMetrics,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn adverse_stream_is_bounded() {
    let mut harness = Harness::new();
    let mut now = Instant::now();

    let snapshot = build_datagram(FrameKind::Digest, 1, b"SNAPSHOT", |buf| buf[7] |= 0x08);
    harness.process(&snapshot, now);

    now += Duration::from_millis(5);
    let delta = build_datagram(FrameKind::Digest, 2, b"delta", |_| {});
    harness.process(&delta, now);

    now += Duration::from_millis(5);
    let oversize = build_datagram(FrameKind::Digest, 3, &vec![0u8; 4096], |_| {});
    harness.process(&oversize, now);

    now += Duration::from_millis(5);
    let wrong_net = build_datagram(FrameKind::Digest, 4, b"bad", |buf| buf[6] = 0x99);
    harness.process(&wrong_net, now);

    now += Duration::from_millis(5);
    harness.process(&delta, now);

    now += Duration::from_millis(5);
    let frag0 = build_datagram(FrameKind::Digest, 5, b"frag-a", |buf| {
        buf[7] |= 0x02;
        buf[16..20].copy_from_slice(&7u32.to_le_bytes());
        buf[24..26].copy_from_slice(&0u16.to_le_bytes());
        buf[26..28].copy_from_slice(&2u16.to_le_bytes());
    });
    harness.process(&frag0, now);
    now += Duration::from_millis(200);
    harness.advance(now);

    assert_eq!(harness.accepted, 2);

    let frame_counts = harness.metrics.frames_snapshot();
    let digest_frames = frame_counts.iter().find(|(kind, _)| *kind == "digest").map(|(_, v)| *v).unwrap_or(0);
    assert_eq!(digest_frames, 2);

    let drop_counts = harness.metrics.drops_snapshot();
    assert!(count_for(&drop_counts, "payload_cap") >= 1);
    assert!(count_for(&drop_counts, "network_mismatch") >= 1);
    assert!(count_for(&drop_counts, "duplicate") >= 1);
    assert!(count_for(&drop_counts, "fragment_timeout") >= 1);
}

fn count_for(entries: &[(&str, u64)], key: &str) -> u64 {
    entries.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or(0)
}

struct Harness {
    ctx: HeaderParseContext,
    assembler: FrameAssembler,
    runtime: FrameRuntime,
    metrics: Arc<UdpMetrics>,
    drop_events: Vec<DropEvent>,
    accepted: u64,
}

impl Harness {
    fn new() -> Self {
        let ctx = HeaderParseContext { network_tag: 0x11, payload_caps: PayloadCaps { digest: 2048, block: 131_072 } };
        let assembler = FrameAssembler::new(FrameAssemblerConfig {
            max_groups: 8,
            max_buffer_bytes: 64 * 1024,
            fragment_ttl: Duration::from_millis(100),
        });
        let runtime = FrameRuntime::new(RuntimeConfig {
            max_kbps: 10,
            dedup_window: 16,
            dedup_max_entries: 32,
            dedup_retention: Duration::from_secs(1),
            snapshot_overdraft_factor: 2.0,
        });
        Self { ctx, assembler, runtime, metrics: Arc::new(UdpMetrics::new()), drop_events: Vec::new(), accepted: 0 }
    }

    fn process(&mut self, datagram: &[u8], now: Instant) {
        self.assembler.collect_expired(now, &mut self.drop_events);
        self.consume_drop_events();
        match SatFrameHeader::parse(datagram, &self.ctx) {
            Ok(parsed) => {
                let payload = Bytes::copy_from_slice(parsed.payload);
                match self.assembler.ingest(parsed.header, payload, now, &mut self.drop_events) {
                    Some(frame) => {
                        self.consume_drop_events();
                        self.handle_frame(frame, now);
                    }
                    None => self.consume_drop_events(),
                }
            }
            Err(err) => {
                self.metrics.record_drop(err.reason);
            }
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
                self.accepted += 1;
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

fn build_datagram(kind: FrameKind, seq: u64, payload: &[u8], mut tweak: impl FnMut(&mut [u8])) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_LEN + payload.len()];
    buf[..4].copy_from_slice(b"KUDP");
    buf[4] = 1;
    buf[5] = kind as u8;
    buf[6] = 0x11;
    buf[7] = 0;
    buf[8..16].copy_from_slice(&seq.to_le_bytes());
    buf[16..20].copy_from_slice(&0u32.to_le_bytes());
    buf[20..22].copy_from_slice(&0u16.to_le_bytes());
    buf[22..24].copy_from_slice(&0u16.to_le_bytes());
    buf[24..26].copy_from_slice(&0u16.to_le_bytes());
    buf[26..28].copy_from_slice(&1u16.to_le_bytes());
    buf[28..32].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    buf[32..34].copy_from_slice(&0u16.to_le_bytes());
    buf[HEADER_LEN..].copy_from_slice(payload);
    tweak(&mut buf);
    let mut hasher = Hasher::new();
    hasher.update(&buf[..HEADER_LEN - 4]);
    buf[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&hasher.finalize().to_le_bytes());
    buf
}
