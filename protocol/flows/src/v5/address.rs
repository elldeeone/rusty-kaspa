use crate::{flow_context::FlowContext, flow_trait::Flow};
use itertools::Itertools;
use kaspa_addressmanager::NetAddress;
use kaspa_core::trace;
use kaspa_p2p_lib::{
    common::{ProtocolError, DEFAULT_TIMEOUT},
    make_message,
    pb::{kaspad_message::Payload, AddressesMessage, RequestAddressesMessage},
    IncomingRoute, Router,
};
use kaspa_utils::networking::IpAddress;
use rand::seq::SliceRandom;
use std::sync::Arc;

/// The maximum number of addresses that are sent in a single kaspa Addresses message.
const MAX_ADDRESSES_SEND: usize = 1000;

/// The maximum number of addresses that can be received in a single kaspa Addresses response.
/// If a peer exceeds this value we consider it a protocol error.
const MAX_ADDRESSES_RECEIVE: usize = 2500;

pub struct ReceiveAddressesFlow {
    ctx: FlowContext,
    router: Arc<Router>,
    incoming_route: IncomingRoute,
}

#[async_trait::async_trait]
impl Flow for ReceiveAddressesFlow {
    fn router(&self) -> Option<Arc<Router>> {
        Some(self.router.clone())
    }

    async fn start(&mut self) -> Result<(), ProtocolError> {
        self.start_impl().await
    }
}

impl ReceiveAddressesFlow {
    pub fn new(ctx: FlowContext, router: Arc<Router>, incoming_route: IncomingRoute) -> Self {
        Self { ctx, router, incoming_route }
    }

    async fn start_impl(&mut self) -> Result<(), ProtocolError> {
        trace!("ReceiveAddressesFlow start peer {}", self.router);
        self.router
            .enqueue(make_message!(
                Payload::RequestAddresses,
                RequestAddressesMessage { include_all_subnetworks: false, subnetwork_id: None }
            ))
            .await?;

        let shutdown = self.ctx.flow_shutdown_listener();
        let recv = tokio::select! {
            _ = shutdown => {
                trace!("ReceiveAddressesFlow shutdown before response peer {}", self.router);
                return Err(ProtocolError::ConnectionClosed);
            }
            _ = tokio::time::sleep(DEFAULT_TIMEOUT) => {
                return Err(ProtocolError::Timeout(DEFAULT_TIMEOUT));
            }
            msg = self.incoming_route.recv() => msg,
        };
        let msg = recv.ok_or(ProtocolError::ConnectionClosed)?;
        let payload_type = msg.payload.as_ref().map(|payload| payload.into());
        let addresses = match msg.payload {
            Some(Payload::Addresses(addresses)) => addresses,
            _ => return Err(ProtocolError::UnexpectedMessage("Payload::Addresses", payload_type)),
        };
        let address_list: Vec<(IpAddress, u16)> = addresses.try_into()?;
        if address_list.len() > MAX_ADDRESSES_RECEIVE {
            return Err(ProtocolError::OtherOwned(format!("address count {} exceeded {}", address_list.len(), MAX_ADDRESSES_RECEIVE)));
        }
        let mut amgr_lock = self.ctx.address_manager.lock();
        for (ip, port) in address_list {
            amgr_lock.add_address(NetAddress::new(ip, port))
        }
        trace!("ReceiveAddressesFlow completed for peer {}", self.router);

        Ok(())
    }
}

pub struct SendAddressesFlow {
    ctx: FlowContext,
    router: Arc<Router>,
    incoming_route: IncomingRoute,
}

#[async_trait::async_trait]
impl Flow for SendAddressesFlow {
    fn router(&self) -> Option<Arc<Router>> {
        Some(self.router.clone())
    }

    async fn start(&mut self) -> Result<(), ProtocolError> {
        self.start_impl().await
    }
}

impl SendAddressesFlow {
    pub fn new(ctx: FlowContext, router: Arc<Router>, incoming_route: IncomingRoute) -> Self {
        Self { ctx, router, incoming_route }
    }

    async fn start_impl(&mut self) -> Result<(), ProtocolError> {
        let shutdown = self.ctx.flow_shutdown_listener();
        loop {
            let msg = tokio::select! {
                _ = shutdown.clone() => {
                    trace!("SendAddressesFlow shutdown for peer {}", self.router);
                    return Err(ProtocolError::ConnectionClosed);
                }
                msg = self.incoming_route.recv() => msg,
            };
            let msg = msg.ok_or(ProtocolError::ConnectionClosed)?;
            let payload = msg.payload;
            let payload_type = payload.as_ref().map(|payload| payload.into());
            match payload {
                Some(Payload::RequestAddresses(_)) => {}
                _ => return Err(ProtocolError::UnexpectedMessage("Payload::RequestAddresses", payload_type)),
            }
            let addresses = self.ctx.address_manager.lock().iterate_addresses().collect_vec();
            let address_list = addresses
                .choose_multiple(&mut rand::thread_rng(), MAX_ADDRESSES_SEND)
                .map(|addr| (addr.ip, addr.port).into())
                .collect();
            self.router.enqueue(make_message!(Payload::Addresses, AddressesMessage { address_list })).await?;
        }
    }
}
