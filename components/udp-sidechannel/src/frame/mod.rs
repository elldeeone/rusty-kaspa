pub mod assembler;
pub mod header;

pub use assembler::{FrameAssembler, FrameAssemblerConfig, ReassembledFrame};
pub use header::{
    FrameFlags, FrameKind, HeaderParseContext, HeaderParseError, ParsedFrame, PayloadCaps, SatFrameHeader, HEADER_LEN, KUDP_MAGIC,
};

/// Drop reasons exposed via metrics and structured logs. Keep values in sync with plan.md requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropReason {
    Crc,
    Version,
    NetworkMismatch,
    PayloadCap,
    FragmentTimeout,
    FecIncomplete,
    QueueFull,
    Duplicate,
    StaleSeq,
    RateCap,
    Signature,
    SourceSignerMismatch,
}

impl DropReason {
    pub const ALL: [DropReason; 12] = [
        DropReason::Crc,
        DropReason::Version,
        DropReason::NetworkMismatch,
        DropReason::PayloadCap,
        DropReason::FragmentTimeout,
        DropReason::FecIncomplete,
        DropReason::QueueFull,
        DropReason::Duplicate,
        DropReason::StaleSeq,
        DropReason::RateCap,
        DropReason::Signature,
        DropReason::SourceSignerMismatch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DropReason::Crc => "crc",
            DropReason::Version => "version",
            DropReason::NetworkMismatch => "network_mismatch",
            DropReason::PayloadCap => "payload_cap",
            DropReason::FragmentTimeout => "fragment_timeout",
            DropReason::FecIncomplete => "fec_incomplete",
            DropReason::QueueFull => "queue_full",
            DropReason::Duplicate => "duplicate",
            DropReason::StaleSeq => "stale_seq",
            DropReason::RateCap => "rate_cap",
            DropReason::Signature => "signature",
            DropReason::SourceSignerMismatch => "source_signer_mismatch",
        }
    }

    pub fn index(self) -> usize {
        match self {
            DropReason::Crc => 0,
            DropReason::Version => 1,
            DropReason::NetworkMismatch => 2,
            DropReason::PayloadCap => 3,
            DropReason::FragmentTimeout => 4,
            DropReason::FecIncomplete => 5,
            DropReason::QueueFull => 6,
            DropReason::Duplicate => 7,
            DropReason::StaleSeq => 8,
            DropReason::RateCap => 9,
            DropReason::Signature => 10,
            DropReason::SourceSignerMismatch => 11,
        }
    }
}

/// Lightweight description of a dropped frame, including optional header context.
#[derive(Debug, Clone, Copy)]
pub struct DropContext {
    pub kind: Option<FrameKind>,
    pub seq: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DropEvent {
    pub reason: DropReason,
    pub context: DropContext,
    pub bytes: usize,
}

impl DropEvent {
    pub fn new(reason: DropReason, context: DropContext, bytes: usize) -> Self {
        Self { reason, context, bytes }
    }
}
