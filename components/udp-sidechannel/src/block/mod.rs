use crate::{
    frame::{DropReason, SatFrameHeader},
    metrics::UdpMetrics,
};
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::{mpsc, mpsc::error::TrySendError};

/// Parsed block payload variants.
#[derive(Debug, Clone)]
pub enum BlockPayload {
    /// Full block payload as provided over the UDP side-channel.
    Full(Vec<u8>),
    // TODO(block-mode): compact header/body variant.
}

#[derive(Debug, Clone)]
pub struct BlockFrame {
    header: SatFrameHeader,
    payload: BlockPayload,
}

impl BlockFrame {
    pub fn new(header: SatFrameHeader, payload: BlockPayload) -> Self {
        Self { header, payload }
    }

    pub fn into_parts(self) -> (SatFrameHeader, BlockPayload) {
        (self.header, self.payload)
    }

    pub fn header(&self) -> SatFrameHeader {
        self.header
    }
}

#[derive(Debug)]
pub enum BlockParseError {
    Oversize { len: usize, cap: usize },
    UnsupportedFormat,
}

impl BlockParseError {
    pub fn reason(&self) -> DropReason {
        match self {
            BlockParseError::Oversize { .. } => DropReason::BlockOversize,
            BlockParseError::UnsupportedFormat => DropReason::BlockUnsupportedFormat,
        }
    }
}

/// Lightweight parser that enforces payload caps before the payload enters the queue.
#[derive(Debug, Clone)]
pub struct BlockParser {
    max_bytes: usize,
}

impl BlockParser {
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    pub fn parse(&self, header: SatFrameHeader, payload: Bytes) -> Result<BlockFrame, BlockParseError> {
        // TODO(block-mode): introduce a dedicated flag for compact or alternative encodings.
        if header.flags.digest_snapshot() {
            return Err(BlockParseError::UnsupportedFormat);
        }
        if payload.len() > self.max_bytes {
            return Err(BlockParseError::Oversize { len: payload.len(), cap: self.max_bytes });
        }
        Ok(BlockFrame::new(header, BlockPayload::Full(payload.to_vec())))
    }
}

#[derive(Clone)]
pub struct BlockChannel {
    parser: BlockParser,
    queue: Arc<BlockQueue>,
}

impl BlockChannel {
    pub fn new(parser: BlockParser, queue: Arc<BlockQueue>) -> Self {
        Self { parser, queue }
    }

    pub fn queue(&self) -> Arc<BlockQueue> {
        Arc::clone(&self.queue)
    }

    pub fn enqueue(&self, header: SatFrameHeader, payload: Bytes) -> Result<(), BlockChannelError> {
        let frame = self.parser.parse(header, payload).map_err(BlockChannelError::Parse)?;
        self.queue.try_push(frame).map_err(BlockChannelError::Queue)
    }

    pub fn take_rx(&self) -> Option<mpsc::Receiver<QueuedBlock>> {
        self.queue.take_rx()
    }

    pub fn snapshot(&self) -> (usize, usize) {
        self.queue.snapshot()
    }
}

#[derive(Debug)]
pub enum BlockChannelError {
    Parse(BlockParseError),
    Queue(BlockQueueError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{FrameFlags, FrameKind};
    use bytes::Bytes;

    fn header() -> SatFrameHeader {
        SatFrameHeader {
            kind: FrameKind::Block,
            flags: FrameFlags::from_bits(0),
            seq: 1,
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
    fn parser_rejects_oversize() {
        let parser = BlockParser::new(4);
        let payload = Bytes::from(vec![0u8; 8]);
        let result = parser.parse(header(), payload);
        assert!(matches!(result, Err(BlockParseError::Oversize { .. })));
    }

    #[test]
    fn parser_rejects_unsupported_flag() {
        let parser = BlockParser::new(16);
        let mut hdr = header();
        hdr.flags = FrameFlags::from_bits(0x08);
        let payload = Bytes::from(vec![0u8; 1]);
        let result = parser.parse(hdr, payload);
        assert!(matches!(result, Err(BlockParseError::UnsupportedFormat)));
    }

    #[test]
    fn queue_backpressure_applies() {
        let queue = BlockQueue::new(1, Arc::new(UdpMetrics::new()));
        let frame = BlockFrame::new(header(), BlockPayload::Full(vec![]));
        queue.try_push(frame).expect("first insert ok");
        let err = queue.try_push(BlockFrame::new(header(), BlockPayload::Full(vec![]))).expect_err("queue must be full");
        assert!(matches!(err, BlockQueueError::Full));
    }
}

struct BlockQueueSlot {
    depth: Arc<BlockQueueDepth>,
}

impl Drop for BlockQueueSlot {
    fn drop(&mut self) {
        self.depth.release();
    }
}

pub struct QueuedBlock {
    frame: BlockFrame,
    _slot: BlockQueueSlot,
}

impl QueuedBlock {
    fn new(frame: BlockFrame, slot: BlockQueueSlot) -> Self {
        Self { frame, _slot: slot }
    }

    pub fn into_parts(self) -> (SatFrameHeader, BlockPayload) {
        self.frame.into_parts()
    }

    pub fn header(&self) -> SatFrameHeader {
        self.frame.header()
    }
}

struct BlockQueueDepth {
    capacity: usize,
    value: Mutex<usize>,
    metrics: Arc<UdpMetrics>,
}

impl BlockQueueDepth {
    fn new(capacity: usize, metrics: Arc<UdpMetrics>) -> Self {
        Self { capacity, value: Mutex::new(0), metrics }
    }

    fn try_acquire(self: &Arc<Self>) -> Option<BlockQueueSlot> {
        let mut guard = self.value.lock();
        if *guard >= self.capacity {
            return None;
        }
        *guard += 1;
        self.metrics.set_block_queue_depth(*guard as u64);
        drop(guard);
        Some(BlockQueueSlot { depth: Arc::clone(self) })
    }

    fn release(&self) {
        let mut guard = self.value.lock();
        if *guard > 0 {
            *guard -= 1;
            self.metrics.set_block_queue_depth(*guard as u64);
        }
    }

    fn stats(&self) -> (usize, usize) {
        let guard = self.value.lock();
        (self.capacity, *guard)
    }
}

#[derive(Debug)]
pub enum BlockQueueError {
    Full,
    Closed,
}

pub struct BlockQueue {
    tx: mpsc::Sender<QueuedBlock>,
    rx: Mutex<Option<mpsc::Receiver<QueuedBlock>>>,
    depth: Arc<BlockQueueDepth>,
}

impl BlockQueue {
    pub fn new(capacity: usize, metrics: Arc<UdpMetrics>) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self { tx, rx: Mutex::new(Some(rx)), depth: Arc::new(BlockQueueDepth::new(capacity, metrics)) }
    }

    pub fn try_push(&self, frame: BlockFrame) -> Result<(), BlockQueueError> {
        let Some(slot) = self.depth.try_acquire() else {
            return Err(BlockQueueError::Full);
        };
        let queued = QueuedBlock::new(frame, slot);
        match self.tx.try_send(queued) {
            Ok(()) => Ok(()),
            Err(TrySendError::Closed(_)) => Err(BlockQueueError::Closed),
            Err(TrySendError::Full(queued)) => {
                drop(queued);
                Err(BlockQueueError::Full)
            }
        }
    }

    pub fn take_rx(&self) -> Option<mpsc::Receiver<QueuedBlock>> {
        let mut guard = self.rx.lock();
        guard.take()
    }

    pub fn snapshot(&self) -> (usize, usize) {
        self.depth.stats()
    }
}
