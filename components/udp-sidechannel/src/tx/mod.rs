use crate::{
    frame::{DropReason, SatFrameHeader},
    metrics::UdpMetrics,
    task::spawn_detached,
};
use borsh::BorshDeserialize;
use kaspa_consensus_core::tx::{Transaction, TransactionId};
use kaspa_core::{debug, warn};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use thiserror::Error;
use tokio::sync::{mpsc, mpsc::error::TrySendError};

const TX_CAPSULE_VERSION: u8 = 1;
const TX_CAPSULE_HEADER_LEN: usize = 14;

#[derive(Debug, Clone, Copy)]
pub struct TxFlags(u8);

impl TxFlags {
    pub fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub fn allow_orphan(self) -> bool {
        self.0 & 0x01 != 0
    }
}

#[derive(Debug, Clone)]
pub struct TxCapsule {
    pub flags: TxFlags,
    pub wallet_timestamp_ms: u64,
    pub transaction: Transaction,
}

#[derive(Debug, Error)]
pub enum TxParseError {
    #[error("tx capsule too short")]
    TooShort,
    #[error("unsupported tx capsule version {0}")]
    UnsupportedVersion(u8),
    #[error("tx length mismatch expected={expected} actual={actual}")]
    LengthMismatch { expected: usize, actual: usize },
    #[error("tx payload exceeds cap {0} bytes")]
    Oversize(usize),
    #[error("tx borsh decode failed: {0}")]
    Decode(String),
}

impl TxParseError {
    pub fn reason(&self) -> DropReason {
        match self {
            TxParseError::Oversize(_) => DropReason::PayloadCap,
            _ => DropReason::TxMalformed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TxParser {
    max_bytes: usize,
}

impl TxParser {
    pub fn new(max_bytes: usize) -> Self {
        let cap = if max_bytes == 0 { usize::MAX } else { max_bytes };
        Self { max_bytes: cap }
    }

    pub fn parse(&self, payload: &[u8]) -> Result<TxCapsule, TxParseError> {
        if payload.len() < TX_CAPSULE_HEADER_LEN {
            return Err(TxParseError::TooShort);
        }
        let version = payload[0];
        if version != TX_CAPSULE_VERSION {
            return Err(TxParseError::UnsupportedVersion(version));
        }
        let flags = TxFlags::from_bits(payload[1]);
        let wallet_timestamp_ms = u64::from_le_bytes(payload[2..10].try_into().unwrap());
        let tx_len = u32::from_le_bytes(payload[10..14].try_into().unwrap()) as usize;
        let actual_len = payload.len() - TX_CAPSULE_HEADER_LEN;
        if tx_len == 0 || tx_len != actual_len {
            return Err(TxParseError::LengthMismatch { expected: tx_len, actual: actual_len });
        }
        if tx_len > self.max_bytes {
            return Err(TxParseError::Oversize(tx_len));
        }
        let tx_bytes = &payload[TX_CAPSULE_HEADER_LEN..];
        let mut transaction = Transaction::try_from_slice(tx_bytes).map_err(|err| TxParseError::Decode(err.to_string()))?;
        transaction.finalize();
        Ok(TxCapsule { flags, wallet_timestamp_ms, transaction })
    }
}

pub type TxSubmitFuture = Pin<Box<dyn Future<Output = Result<TransactionId, TxSubmitError>> + Send>>;

pub trait UdpTxSubmitter: Send + Sync {
    fn submit(&self, transaction: Transaction, allow_orphan: bool) -> TxSubmitFuture;
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct TxSubmitError {
    message: String,
}

impl TxSubmitError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TxChannel {
    parser: TxParser,
    queue: Arc<TxQueue>,
}

impl TxChannel {
    pub(crate) fn new(parser: TxParser, queue: Arc<TxQueue>) -> Self {
        Self { parser, queue }
    }

    pub(crate) fn enqueue(&self, header: SatFrameHeader, payload: &[u8]) -> Result<(), TxChannelError> {
        let capsule = self.parser.parse(payload).map_err(TxChannelError::Parse)?;
        let queued = QueuedTx::new(header, capsule);
        self.queue.try_push(queued).map_err(TxChannelError::Queue)
    }

    pub(crate) fn take_rx(&self) -> Option<mpsc::Receiver<QueuedTx>> {
        self.queue.take_rx()
    }
}

#[derive(Debug)]
pub(crate) enum TxChannelError {
    Parse(TxParseError),
    Queue(TxQueueError),
}

#[derive(Debug, Clone)]
pub(crate) struct QueuedTx {
    header: SatFrameHeader,
    capsule: TxCapsule,
}

impl QueuedTx {
    fn new(header: SatFrameHeader, capsule: TxCapsule) -> Self {
        Self { header, capsule }
    }

    pub(crate) fn into_parts(self) -> (SatFrameHeader, TxCapsule) {
        (self.header, self.capsule)
    }
}

#[derive(Debug)]
pub(crate) enum TxQueueError {
    Full,
    Closed,
}

#[derive(Debug)]
pub(crate) struct TxQueue {
    tx: mpsc::Sender<QueuedTx>,
    rx: Mutex<Option<mpsc::Receiver<QueuedTx>>>,
}

impl TxQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        Self { tx, rx: Mutex::new(Some(rx)) }
    }

    pub(crate) fn try_push(&self, task: QueuedTx) -> Result<(), TxQueueError> {
        match self.tx.try_send(task) {
            Ok(()) => Ok(()),
            Err(TrySendError::Closed(_)) => Err(TxQueueError::Closed),
            Err(TrySendError::Full(_)) => Err(TxQueueError::Full),
        }
    }

    pub(crate) fn take_rx(&self) -> Option<mpsc::Receiver<QueuedTx>> {
        self.rx.lock().ok().and_then(|mut guard| guard.take())
    }
}

pub(crate) fn spawn_tx_submitter(mut rx: mpsc::Receiver<QueuedTx>, submitter: Arc<dyn UdpTxSubmitter>, metrics: Arc<UdpMetrics>) {
    spawn_detached("udp-tx-submit", async move {
        while let Some(task) = rx.recv().await {
            let (header, capsule) = task.into_parts();
            let allow_orphan = capsule.flags.allow_orphan();
            let txid = capsule.transaction.id();
            let wallet_ts_ms = capsule.wallet_timestamp_ms;
            match submitter.submit(capsule.transaction, allow_orphan).await {
                Ok(id) => {
                    debug!("udp.event=tx_submit_ok txid={} seq={} source={}", id, header.seq, header.source_id);
                }
                Err(err) => {
                    metrics.record_drop(DropReason::TxRejected);
                    warn!(
                        "udp.event=tx_submit_fail txid={} seq={} source={} wallet_ts_ms={} err={}",
                        txid, header.seq, header.source_id, wallet_ts_ms, err
                    );
                }
            }
        }
        debug!("udp.event=tx_submitter_stopped");
    });
}
