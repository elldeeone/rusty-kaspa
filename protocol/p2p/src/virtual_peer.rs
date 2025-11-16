use crate::{common::ProtocolError, pb::KaspadMessage};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("inject queue full")]
    QueueFull,
    #[error("injector disconnected")]
    Disconnected,
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
}

/// Trait implemented by in-process peers that can enqueue messages into the router flows.
pub trait PeerMessageInjector: Send + Sync {
    fn inject(&self, msg: KaspadMessage) -> Result<(), InjectError>;
}
