use crate::{block_injector::BlockInjectionFlow, flow_context::FlowContext, flow_trait::Flow};
use kaspa_connectionmanager::{InjectError, PeerMessageInjector};
use kaspa_core::warn;
use kaspa_p2p_lib::{common::ProtocolError, pb::KaspadMessage, Router};
use std::{
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
        Box::new(BlockInjectionFlow::new(ctx.clone(), router.clone())).launch();
        let (tx, mut rx) = mpsc::channel(SAT_INJECT_QUEUE);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
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
