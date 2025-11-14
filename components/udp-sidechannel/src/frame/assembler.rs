use crate::frame::{DropEvent, DropReason, FrameKind, SatFrameHeader};
use bytes::{Bytes, BytesMut};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct FrameAssemblerConfig {
    pub max_groups: usize,
    pub max_buffer_bytes: usize,
    pub fragment_ttl: Duration,
}

impl Default for FrameAssemblerConfig {
    fn default() -> Self {
        Self { max_groups: 128, max_buffer_bytes: 8 * 1024 * 1024, fragment_ttl: Duration::from_secs(5) }
    }
}

#[derive(Debug)]
pub struct FrameAssembler {
    config: FrameAssemblerConfig,
    groups: HashMap<GroupKey, GroupState>,
    order: VecDeque<GroupKey>,
    buffered_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GroupKey {
    kind: FrameKind,
    id: u32,
}

#[derive(Debug)]
struct GroupState {
    header: SatFrameHeader,
    fragments: BTreeMap<u16, Bytes>,
    expected: u16,
    fec_present: bool,
    expires_at: Instant,
    total_bytes: usize,
}

#[derive(Debug)]
pub struct ReassembledFrame {
    pub header: SatFrameHeader,
    pub payload: Bytes,
}

impl FrameAssembler {
    pub fn new(config: FrameAssemblerConfig) -> Self {
        Self { config, groups: HashMap::new(), order: VecDeque::new(), buffered_bytes: 0 }
    }

    pub fn collect_expired(&mut self, now: Instant, drops: &mut Vec<DropEvent>) {
        while let Some(key) = self.order.front().copied() {
            let should_drop = match self.groups.get(&key) {
                Some(state) => state.expires_at <= now,
                None => true,
            };

            if !should_drop {
                break;
            }

            self.order.pop_front();
            if let Some(state) = self.groups.remove(&key) {
                self.buffered_bytes = self.buffered_bytes.saturating_sub(state.total_bytes);
                let reason = if state.fec_present { DropReason::FecIncomplete } else { DropReason::FragmentTimeout };
                drops.push(state.header.as_drop_event(reason, state.total_bytes));
            }
        }
    }

    pub fn ingest(
        &mut self,
        header: SatFrameHeader,
        payload: Bytes,
        now: Instant,
        drops: &mut Vec<DropEvent>,
    ) -> Option<ReassembledFrame> {
        if !header.flags.fragmented() || header.frag_cnt == 1 {
            let payload_len = payload.len();
            return Some(ReassembledFrame { header: header.normalized_for_reassembly(payload_len), payload });
        }

        if header.group_id == 0 {
            drops.push(header.as_drop_event(DropReason::Crc, payload.len()));
            return None;
        }

        let key = GroupKey { kind: header.kind, id: header.group_id };

        let is_new_group = !self.groups.contains_key(&key);
        if !self.evict_for_capacity(is_new_group, payload.len(), drops) {
            drops.push(header.as_drop_event(DropReason::QueueFull, payload.len()));
            return None;
        }

        let state = self.groups.entry(key).or_insert_with(|| GroupState {
            header,
            fragments: BTreeMap::new(),
            expected: header.frag_cnt,
            fec_present: header.flags.fec_present(),
            expires_at: now + self.config.fragment_ttl,
            total_bytes: 0,
        });

        if state.fragments.contains_key(&header.frag_ix) {
            drops.push(header.as_drop_event(DropReason::Duplicate, payload.len()));
            return None;
        }

        state.total_bytes += payload.len();
        self.buffered_bytes += payload.len();
        state.fragments.insert(header.frag_ix, payload);

        if state.fragments.len() == state.expected as usize {
            let mut total = 0usize;
            for chunk in state.fragments.values() {
                total = total.saturating_add(chunk.len());
            }
            let mut buf = BytesMut::with_capacity(total);
            for chunk in state.fragments.values() {
                buf.extend_from_slice(chunk);
            }
            let payload = buf.freeze();
            let header = state.header.normalized_for_reassembly(payload.len());
            self.buffered_bytes = self.buffered_bytes.saturating_sub(state.total_bytes);
            self.groups.remove(&key);
            return Some(ReassembledFrame { header, payload });
        }

        if is_new_group {
            self.order.push_back(key);
        }

        None
    }

    fn evict_for_capacity(&mut self, is_new_group: bool, additional: usize, drops: &mut Vec<DropEvent>) -> bool {
        if self.config.max_groups == 0 || self.config.max_buffer_bytes == 0 {
            return false;
        }

        while (is_new_group && self.groups.len() >= self.config.max_groups)
            || self.buffered_bytes + additional > self.config.max_buffer_bytes
        {
            let key = match self.order.pop_front() {
                Some(k) => k,
                None => break,
            };
            if let Some(state) = self.groups.remove(&key) {
                let reason = DropReason::QueueFull;
                drops.push(state.header.as_drop_event(reason, state.total_bytes));
                self.buffered_bytes = self.buffered_bytes.saturating_sub(state.total_bytes);
            } else {
                continue;
            }
        }

        (!is_new_group || self.groups.len() < self.config.max_groups)
            && self.buffered_bytes + additional <= self.config.max_buffer_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::header::{FrameFlags, FrameKind, SatFrameHeader};

    fn header(kind: FrameKind, group_id: u32, frag_ix: u16, frag_cnt: u16) -> SatFrameHeader {
        SatFrameHeader {
            kind,
            flags: FrameFlags::from_bits(0x02),
            seq: 1,
            group_id,
            group_k: 0,
            group_n: 0,
            frag_ix,
            frag_cnt,
            payload_len: 0,
            source_id: 0,
        }
    }

    #[test]
    fn loss_reorder() {
        let mut assembler =
            FrameAssembler::new(FrameAssemblerConfig { fragment_ttl: Duration::from_millis(100), ..Default::default() });
        let mut drops = Vec::new();
        let now = Instant::now();
        let payloads = [Bytes::from_static(b"foo"), Bytes::from_static(b"bar"), Bytes::from_static(b"baz")];

        assert!(assembler.ingest(header(FrameKind::Digest, 7, 1, 3), payloads[1].clone(), now, &mut drops).is_none());
        assert!(assembler.ingest(header(FrameKind::Digest, 7, 0, 3), payloads[0].clone(), now, &mut drops).is_none());
        let frame = assembler.ingest(header(FrameKind::Digest, 7, 2, 3), payloads[2].clone(), now, &mut drops).expect("assembled");
        assert_eq!(frame.payload, Bytes::from_static(b"foobarbaz"));

        drops.clear();
        let start = Instant::now();
        assert!(assembler.ingest(header(FrameKind::Digest, 42, 0, 2), Bytes::from_static(b"late"), start, &mut drops).is_none());
        assembler.collect_expired(start + Duration::from_millis(250), &mut drops);
        assert!(drops.iter().any(|d| d.reason == DropReason::FragmentTimeout));

        drops.clear();
        let mut fec_header = header(FrameKind::Digest, 100, 0, 2);
        fec_header.flags = FrameFlags::from_bits(0x03);
        let fec_start = Instant::now();
        assert!(assembler.ingest(fec_header, Bytes::from_static(b"fec"), fec_start, &mut drops).is_none());
        assembler.collect_expired(fec_start + Duration::from_millis(250), &mut drops);
        assert!(drops.iter().any(|d| d.reason == DropReason::FecIncomplete));
    }
}
