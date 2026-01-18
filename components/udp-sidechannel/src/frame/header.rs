use crate::frame::{DropContext, DropEvent, DropReason};
use crc32fast::Hasher;

pub const KUDP_MAGIC: [u8; 4] = *b"KUDP";
pub const HEADER_LEN: usize = 38;
const HEADER_CRC_OFFSET: usize = HEADER_LEN - 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameKind {
    Digest = 0x01,
    Block = 0x02,
    Tx = 0x03,
}

impl FrameKind {
    pub const ALL: [FrameKind; 3] = [FrameKind::Digest, FrameKind::Block, FrameKind::Tx];

    pub fn as_str(self) -> &'static str {
        match self {
            FrameKind::Digest => "digest",
            FrameKind::Block => "block",
            FrameKind::Tx => "tx",
        }
    }

    pub fn index(self) -> usize {
        match self {
            FrameKind::Digest => 0,
            FrameKind::Block => 1,
            FrameKind::Tx => 2,
        }
    }
}

impl TryFrom<u8> for FrameKind {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(FrameKind::Digest),
            0x02 => Ok(FrameKind::Block),
            0x03 => Ok(FrameKind::Tx),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFlags(u8);

impl FrameFlags {
    pub fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub fn bits(self) -> u8 {
        self.0
    }

    pub fn fec_present(self) -> bool {
        self.0 & 0x01 != 0
    }

    pub fn fragmented(self) -> bool {
        self.0 & 0x02 != 0
    }

    pub fn signed(self) -> bool {
        self.0 & 0x04 != 0
    }

    pub fn digest_snapshot(self) -> bool {
        self.0 & 0x08 != 0
    }

    pub fn without_fragmented(self) -> Self {
        Self(self.0 & !0x02)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SatFrameHeader {
    pub kind: FrameKind,
    pub flags: FrameFlags,
    pub seq: u64,
    pub group_id: u32,
    pub group_k: u16,
    pub group_n: u16,
    pub frag_ix: u16,
    pub frag_cnt: u16,
    pub payload_len: u32,
    pub source_id: u16,
}

impl SatFrameHeader {
    pub fn fragment_len(&self) -> usize {
        self.payload_len as usize
    }

    pub fn estimated_total_len(&self) -> u64 {
        (self.payload_len as u64).saturating_mul(self.frag_cnt as u64)
    }

    pub fn as_drop_event(&self, reason: DropReason, bytes: usize) -> DropEvent {
        DropEvent::new(reason, DropContext { kind: Some(self.kind), seq: Some(self.seq) }, bytes)
    }

    pub fn normalized_for_reassembly(mut self, payload_len: usize) -> Self {
        self.frag_ix = 0;
        self.frag_cnt = 1;
        self.flags = self.flags.without_fragmented();
        self.payload_len = payload_len as u32;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PayloadCaps {
    pub digest: u32,
    pub block: u32,
    pub tx: u32,
}

impl PayloadCaps {
    pub fn cap_for(&self, kind: FrameKind) -> u32 {
        match kind {
            FrameKind::Digest => self.digest,
            FrameKind::Block => self.block,
            FrameKind::Tx => self.tx,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HeaderParseContext {
    pub network_tag: u8,
    pub payload_caps: PayloadCaps,
}

#[derive(Debug)]
pub struct HeaderParseError {
    pub reason: DropReason,
    pub message: &'static str,
    pub kind: Option<FrameKind>,
    pub seq: Option<u64>,
    pub future_version: bool,
}

impl HeaderParseError {
    pub fn drop_event(&self) -> DropEvent {
        DropEvent::new(self.reason, DropContext { kind: self.kind, seq: self.seq }, 0)
    }

    fn new(reason: DropReason, message: &'static str) -> Self {
        Self { reason, message, kind: None, seq: None, future_version: false }
    }

    fn with_kind(mut self, kind: FrameKind, seq: u64) -> Self {
        self.kind = Some(kind);
        self.seq = Some(seq);
        self
    }

    fn future_version(mut self) -> Self {
        self.future_version = true;
        self
    }
}

#[derive(Debug)]
pub struct ParsedFrame<'a> {
    pub header: SatFrameHeader,
    pub payload: &'a [u8],
}

impl SatFrameHeader {
    pub fn parse<'a>(buf: &'a [u8], ctx: &HeaderParseContext) -> Result<ParsedFrame<'a>, HeaderParseError> {
        if buf.len() < HEADER_LEN {
            return Err(HeaderParseError::new(DropReason::Crc, "truncated_header"));
        }

        if buf[..4] != KUDP_MAGIC {
            return Err(HeaderParseError::new(DropReason::Crc, "bad_magic"));
        }

        let version = buf[4];
        if version != 1 {
            return Err(HeaderParseError::new(DropReason::Version, "future_version").future_version());
        }

        let raw_kind = buf[5];
        let kind = FrameKind::try_from(raw_kind).map_err(|_| HeaderParseError::new(DropReason::Crc, "unknown_kind"))?;

        let flags = FrameFlags::from_bits(buf[7]);
        let seq = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let network_id = buf[6];
        if network_id != ctx.network_tag {
            return Err(HeaderParseError::new(DropReason::NetworkMismatch, "network_mismatch").with_kind(kind, seq));
        }

        let group_id = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        let group_k = u16::from_le_bytes(buf[20..22].try_into().unwrap());
        let group_n = u16::from_le_bytes(buf[22..24].try_into().unwrap());
        let frag_ix = u16::from_le_bytes(buf[24..26].try_into().unwrap());
        let frag_cnt = u16::from_le_bytes(buf[26..28].try_into().unwrap());
        let payload_len = u32::from_le_bytes(buf[28..32].try_into().unwrap());
        let source_id = u16::from_le_bytes(buf[32..34].try_into().unwrap());

        if frag_cnt == 0 {
            return Err(HeaderParseError::new(DropReason::Crc, "invalid_frag_count").with_kind(kind, seq));
        }
        if frag_ix >= frag_cnt {
            return Err(HeaderParseError::new(DropReason::Crc, "invalid_frag_index").with_kind(kind, seq));
        }

        let cap = ctx.payload_caps.cap_for(kind) as u64;
        if cap > 0 && (payload_len as u64).saturating_mul(frag_cnt as u64) > cap {
            return Err(HeaderParseError::new(DropReason::PayloadCap, "payload_cap").with_kind(kind, seq));
        }

        let mut hasher = Hasher::new();
        hasher.update(&buf[..HEADER_CRC_OFFSET]);
        let crc = hasher.finalize();
        let expected_crc = u32::from_le_bytes(buf[HEADER_CRC_OFFSET..HEADER_LEN].try_into().unwrap());
        if crc != expected_crc {
            return Err(HeaderParseError::new(DropReason::Crc, "crc_mismatch").with_kind(kind, seq));
        }

        let payload_start = HEADER_LEN;
        let payload_end = payload_start + payload_len as usize;
        if payload_end > buf.len() {
            return Err(HeaderParseError::new(DropReason::Crc, "truncated_payload").with_kind(kind, seq));
        }

        let header = SatFrameHeader { kind, flags, seq, group_id, group_k, group_n, frag_ix, frag_cnt, payload_len, source_id };

        Ok(ParsedFrame { header, payload: &buf[payload_start..payload_end] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::DropReason;

    fn ctx() -> HeaderParseContext {
        HeaderParseContext { network_tag: 0x11, payload_caps: PayloadCaps { digest: 2048, block: 131072, tx: 4096 } }
    }

    fn encode_header(kind: FrameKind, payload: &[u8], overrides: impl FnOnce(&mut [u8])) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_LEN + payload.len()];
        buf[..4].copy_from_slice(&KUDP_MAGIC);
        buf[4] = 1;
        buf[5] = kind as u8;
        buf[6] = 0x11;
        buf[7] = 0;
        buf[16..20].copy_from_slice(&0u32.to_le_bytes());
        buf[20..22].copy_from_slice(&0u16.to_le_bytes());
        buf[22..24].copy_from_slice(&0u16.to_le_bytes());
        buf[24..26].copy_from_slice(&0u16.to_le_bytes());
        buf[26..28].copy_from_slice(&1u16.to_le_bytes());
        buf[28..32].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        buf[32..34].copy_from_slice(&0u16.to_le_bytes());
        buf[8..16].copy_from_slice(&7u64.to_le_bytes());
        buf[HEADER_LEN..].copy_from_slice(payload);
        overrides(&mut buf);
        let mut hasher = Hasher::new();
        hasher.update(&buf[..HEADER_CRC_OFFSET]);
        let crc = hasher.finalize();
        buf[HEADER_CRC_OFFSET..HEADER_LEN].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    #[test]
    fn header_bounds() {
        let payload = vec![0u8; 16];
        let good = encode_header(FrameKind::Digest, &payload, |_| {});
        let parsed = SatFrameHeader::parse(&good, &ctx()).expect("parse");
        assert_eq!(parsed.header.kind, FrameKind::Digest);

        let bad_magic = encode_header(FrameKind::Digest, &payload, |buf| buf[0] = b'X');
        assert_eq!(SatFrameHeader::parse(&bad_magic, &ctx()).unwrap_err().reason, DropReason::Crc);

        let mut future = encode_header(FrameKind::Digest, &payload, |buf| buf[4] = 2);
        // need to reapply crc
        let mut hasher = Hasher::new();
        hasher.update(&future[..HEADER_CRC_OFFSET]);
        future[HEADER_CRC_OFFSET..HEADER_LEN].copy_from_slice(&hasher.finalize().to_le_bytes());
        let err = SatFrameHeader::parse(&future, &ctx()).unwrap_err();
        assert_eq!(err.reason, DropReason::Version);
        assert!(err.future_version);

        let wrong_network = encode_header(FrameKind::Digest, &payload, |buf| buf[6] = 0xA0);
        assert_eq!(SatFrameHeader::parse(&wrong_network, &ctx()).unwrap_err().reason, DropReason::NetworkMismatch);

        let mut truncated = encode_header(FrameKind::Digest, &payload, |_| {});
        truncated.pop();
        assert_eq!(SatFrameHeader::parse(&truncated, &ctx()).unwrap_err().reason, DropReason::Crc);

        let oversized = encode_header(FrameKind::Digest, &vec![0u8; 4096], |_| {});
        assert_eq!(SatFrameHeader::parse(&oversized, &ctx()).unwrap_err().reason, DropReason::PayloadCap);
    }
}
