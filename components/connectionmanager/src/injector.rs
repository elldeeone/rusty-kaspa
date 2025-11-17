use kaspa_p2p_lib::{common::ProtocolError, pb::KaspadMessage};
use kaspa_utils::triggers::Listener;
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

pub trait PeerMessageInjector: Send + Sync {
    fn inject(&self, msg: KaspadMessage) -> Result<(), InjectError>;

    fn shutdown_listener(&self) -> Option<Listener> {
        None
    }
}
