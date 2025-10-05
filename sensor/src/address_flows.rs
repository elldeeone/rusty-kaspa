// Address gossip flows - extracted from kaspa-p2p-flows/src/v5/address.rs
// This is the battle-tested mainnet implementation for P2P address exchange

use itertools::Itertools;
use kaspa_addressmanager::{AddressManager, NetAddress};
use kaspa_p2p_lib::{
    common::ProtocolError,
    dequeue, dequeue_with_timeout, make_message,
    pb::{kaspad_message::Payload, AddressesMessage, RequestAddressesMessage},
    IncomingRoute, KaspadMessagePayloadType, Router,
};
use kaspa_utils::networking::IpAddress;
use log::{debug, info, warn};
use parking_lot::Mutex;
use rand::seq::SliceRandom;
use std::sync::Arc;

/// The maximum number of addresses that are sent in a single kaspa Addresses message.
const MAX_ADDRESSES_SEND: usize = 1000;

/// The maximum number of addresses that can be received in a single kaspa Addresses response.
/// If a peer exceeds this value we consider it a protocol error.
const MAX_ADDRESSES_RECEIVE: usize = 2500;

/// Flow that receives addresses from a peer
///
/// This flow:
/// 1. Sends a RequestAddresses message to the peer upon connection
/// 2. Waits for the Addresses response (with timeout)
/// 3. Validates and stores received addresses in the address manager
///
/// This is a one-shot flow - it runs once per connection and then exits.
pub struct ReceiveAddressesFlow {
    address_manager: Arc<Mutex<AddressManager>>,
    router: Arc<Router>,
    incoming_route: IncomingRoute,
}

impl ReceiveAddressesFlow {
    pub fn new(
        address_manager: Arc<Mutex<AddressManager>>,
        router: Arc<Router>,
        incoming_route: IncomingRoute,
    ) -> Self {
        Self { address_manager, router, incoming_route }
    }

    /// Launch the flow in a background task
    pub fn launch(mut self) {
        tokio::spawn(async move {
            if let Err(err) = self.start().await {
                if !err.is_connection_closed_error() {
                    warn!("ReceiveAddressesFlow error for peer {}: {}", self.router, err);
                }
                self.router.try_sending_reject_message(&err).await;
                let _ = self.router.close().await;
            } else {
                debug!("ReceiveAddressesFlow completed successfully for peer {}", self.router);
            }
        });
    }

    async fn start(&mut self) -> Result<(), ProtocolError> {
        // Send RequestAddresses message to peer
        debug!("Sending RequestAddresses to peer {}", self.router);
        self.router
            .enqueue(make_message!(
                Payload::RequestAddresses,
                RequestAddressesMessage { include_all_subnetworks: false, subnetwork_id: None }
            ))
            .await?;

        // Wait for Addresses response (with 120 second timeout from DEFAULT_TIMEOUT)
        let msg = dequeue_with_timeout!(self.incoming_route, Payload::Addresses)?;
        let address_list: Vec<(IpAddress, u16)> = msg.try_into()?;

        // Validate address count
        if address_list.len() > MAX_ADDRESSES_RECEIVE {
            return Err(ProtocolError::OtherOwned(format!(
                "address count {} exceeded {}",
                address_list.len(),
                MAX_ADDRESSES_RECEIVE
            )));
        }

        // Store addresses in address manager
        info!("Received {} addresses from peer {}", address_list.len(), self.router);
        let mut amgr_lock = self.address_manager.lock();
        for (ip, port) in address_list {
            amgr_lock.add_address(NetAddress::new(ip, port))
        }

        Ok(())
    }
}

/// Flow that sends addresses to peers upon request
///
/// This flow:
/// 1. Listens for RequestAddresses messages from the peer
/// 2. Responds with up to MAX_ADDRESSES_SEND (1000) random addresses from the address manager
/// 3. Runs in a loop until the connection closes
///
/// This is a long-running flow that handles multiple requests per connection.
pub struct SendAddressesFlow {
    address_manager: Arc<Mutex<AddressManager>>,
    router: Arc<Router>,
    incoming_route: IncomingRoute,
}

impl SendAddressesFlow {
    pub fn new(
        address_manager: Arc<Mutex<AddressManager>>,
        router: Arc<Router>,
        incoming_route: IncomingRoute,
    ) -> Self {
        Self { address_manager, router, incoming_route }
    }

    /// Launch the flow in a background task
    pub fn launch(mut self) {
        tokio::spawn(async move {
            if let Err(err) = self.start().await {
                if !err.is_connection_closed_error() {
                    warn!("SendAddressesFlow error for peer {}: {}", self.router, err);
                }
                self.router.try_sending_reject_message(&err).await;
                let _ = self.router.close().await;
            } else {
                debug!("SendAddressesFlow completed for peer {}", self.router);
            }
        });
    }

    async fn start(&mut self) -> Result<(), ProtocolError> {
        loop {
            // Wait for RequestAddresses message from peer
            dequeue!(self.incoming_route, Payload::RequestAddresses)?;

            // Get addresses from address manager
            let addresses = self.address_manager.lock().iterate_addresses().collect_vec();

            // Select up to MAX_ADDRESSES_SEND random addresses
            let address_list = addresses
                .choose_multiple(&mut rand::thread_rng(), MAX_ADDRESSES_SEND)
                .map(|addr| (addr.ip, addr.port).into())
                .collect();

            debug!("Sending {} addresses to peer {}", addresses.len().min(MAX_ADDRESSES_SEND), self.router);

            // Send Addresses response
            self.router.enqueue(make_message!(Payload::Addresses, AddressesMessage { address_list })).await?;
        }
    }
}

/// Launch both address flows for a peer connection
///
/// This function should be called after the handshake is complete.
/// It starts both the receive flow (which requests addresses) and
/// the send flow (which responds to address requests).
pub fn launch_address_flows(address_manager: Arc<Mutex<AddressManager>>, router: Arc<Router>) {
    // Subscribe to Addresses messages (for receiving)
    let receive_route = router.subscribe(vec![KaspadMessagePayloadType::Addresses]);
    let receive_flow = ReceiveAddressesFlow::new(address_manager.clone(), router.clone(), receive_route);
    receive_flow.launch();

    // Subscribe to RequestAddresses messages (for sending)
    let send_route = router.subscribe(vec![KaspadMessagePayloadType::RequestAddresses]);
    let send_flow = SendAddressesFlow::new(address_manager, router, send_route);
    send_flow.launch();
}
