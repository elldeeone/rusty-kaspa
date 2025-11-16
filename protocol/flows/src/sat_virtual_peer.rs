use crate::flow_context::FlowContext;
use kaspa_connectionmanager::{InjectError, PeerMessageInjector};
use kaspa_consensus_core::block::Block;
use kaspa_core::warn;
use kaspa_p2p_lib::{
    common::ProtocolError,
    pb::{kaspad_message::Payload, KaspadMessage},
    Router,
};
use std::{
    convert::TryFrom,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tokio::sync::mpsc::{self, error::TrySendError};

const SAT_INJECT_QUEUE: usize = 32;

pub struct SatVirtualPeerInjector {
    tx: mpsc::Sender<KaspadMessage>,
}

impl SatVirtualPeerInjector {
    pub fn start(ctx: FlowContext) -> Result<Arc<Self>, ProtocolError> {
        let router = Router::new_virtual(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
        ctx.register_virtual_router(router.clone())?;
        let ctx_clone = ctx.clone();
        let (tx, mut rx) = mpsc::channel::<KaspadMessage>(SAT_INJECT_QUEUE);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Some(Payload::Block(block_msg)) = msg.payload.clone() {
                    match Block::try_from(block_msg) {
                        Ok(block) => {
                            let session = ctx_clone.consensus().session().await;
                            if let Err(err) = ctx_clone.submit_rpc_block(&session, block).await {
                                warn!("sat_virtual_peer block submit error: {}", err);
                            }
                        }
                        Err(err) => warn!("sat_virtual_peer block decode error: {}", err),
                    }
                    continue;
                }
                if let Err(err) = router.route_to_flow(msg) {
                    warn!("sat_virtual_peer route error: {}", err);
                }
            }
        });
        Ok(Arc::new(Self { tx }))
    }
}

impl PeerMessageInjector for SatVirtualPeerInjector {
    fn inject(&self, msg: KaspadMessage) -> Result<(), InjectError> {
        match self.tx.try_send(msg) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(InjectError::QueueFull),
            Err(TrySendError::Closed(_)) => Err(InjectError::Disconnected),
        }
    }
}
