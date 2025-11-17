use crate::{block_injector::BlockInjectionFlow, flow_context::FlowContext, flow_trait::Flow};
use kaspa_connectionmanager::{InjectError, PeerMessageInjector};
use kaspa_core::{trace, warn};
use kaspa_p2p_lib::{common::ProtocolError, pb::KaspadMessage, Router};
use kaspa_utils::triggers::Listener;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tokio::sync::mpsc::{self, error::TrySendError};

const SAT_INJECT_QUEUE: usize = 32;

pub struct SatVirtualPeerInjector {
    tx: mpsc::Sender<KaspadMessage>,
    shutdown: Listener,
}

impl SatVirtualPeerInjector {
    pub fn start(ctx: FlowContext) -> Result<Arc<Self>, ProtocolError> {
        let router = Router::new_virtual(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
        ctx.register_virtual_router(router.clone())?;
        Box::new(BlockInjectionFlow::new(ctx.clone(), router.clone())).launch();
        let (tx, mut rx) = mpsc::channel(SAT_INJECT_QUEUE);
        let shutdown = ctx.flow_shutdown_listener();
        let shutdown_for_loop = shutdown.clone();
        tokio::spawn(async move {
            trace!("sat_virtual_peer injector loop started ({})", router);
            loop {
                tokio::select! {
                    _ = shutdown_for_loop.clone() => {
                        trace!("sat_virtual_peer injector shutdown signal ({})", router);
                        break;
                    }
                    msg = rx.recv() => match msg {
                        Some(msg) => {
                            if let Err(err) = router.route_to_flow(msg) {
                                warn!("sat_virtual_peer route error: {}", err);
                            }
                        }
                        None => {
                            trace!("sat_virtual_peer injector channel closed ({})", router);
                            break;
                        }
                    }
                }
            }
            trace!("sat_virtual_peer injector closing router ({})", router);
            router.close().await;
            trace!("sat_virtual_peer injector loop stopped ({})", router);
        });
        Ok(Arc::new(Self { tx, shutdown }))
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

    fn shutdown_listener(&self) -> Option<Listener> {
        Some(self.shutdown.clone())
    }
}
