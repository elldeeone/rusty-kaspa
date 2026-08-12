use crate::{flow_context::FlowContext, flow_trait::Flow};
use itertools::Itertools;
use kaspa_addressmanager::NetAddress;
use kaspa_p2p_lib::{
    IncomingRoute, Router,
    common::ProtocolError,
    dequeue, dequeue_with_timeout, make_message,
    pb::{AddressesMessage, RequestAddressesMessage, kaspad_message::Payload},
};
use kaspa_utils::networking::{RelayRole, synthetic_relay_endpoint};
use rand::seq::SliceRandom;
use std::{collections::HashMap, sync::Arc};
use tokio::time::{Duration, sleep};

/// The maximum number of addresses that are sent in a single kaspa Addresses message.
const MAX_ADDRESSES_SEND: usize = 1000;

/// The maximum number of addresses that can be received in a single kaspa Addresses response.
/// If a peer exceeds this value we consider it a protocol error.
const MAX_ADDRESSES_RECEIVE: usize = 2500;
/// Periodic refresh for address gossip to avoid startup race windows.
const ADDRESS_REFRESH_INTERVAL: Duration = Duration::from_secs(45);

type Libp2pAdvertisement = (u64, Option<u16>, Option<u32>, Option<u64>, Option<RelayRole>, Option<String>, Option<String>);

fn merge_duplicate_addresses(addresses: Vec<NetAddress>) -> Vec<NetAddress> {
    let mut merged: Vec<NetAddress> = Vec::with_capacity(addresses.len());
    let mut index_by_endpoint: HashMap<NetAddress, usize> = HashMap::with_capacity(addresses.len());
    for address in addresses {
        if let Some(index) = index_by_endpoint.get(&address).copied() {
            merged[index].merge_metadata(&address);
        } else {
            index_by_endpoint.insert(address.clone(), merged.len());
            merged.push(address);
        }
    }
    merged
}

fn select_addresses_for_gossip(mut addresses: Vec<NetAddress>, local_address: Option<NetAddress>) -> Vec<NetAddress> {
    let guarantee_local = local_address.as_ref().is_some_and(NetAddress::has_libp2p_discovery_metadata);
    if let Some(local_address) = local_address.as_ref() {
        if guarantee_local {
            addresses.retain(|address| address != local_address);
        } else {
            addresses.push(local_address.clone());
        }
    }

    let unique_addresses = merge_duplicate_addresses(addresses);
    let sample_size = MAX_ADDRESSES_SEND.saturating_sub(usize::from(guarantee_local));
    let mut selected = unique_addresses.choose_multiple(&mut rand::thread_rng(), sample_size).cloned().collect_vec();
    if guarantee_local {
        selected.push(local_address.expect("guaranteed local address is present"));
    }
    selected
}

fn apply_libp2p_advertisement(base_address: Option<NetAddress>, advertisement: Libp2pAdvertisement) -> Option<NetAddress> {
    let (services, relay_port, relay_capacity, relay_ttl_ms, relay_role, libp2p_peer_id, relay_hint) = advertisement;
    let mut local = base_address.or_else(|| {
        let peer_id = libp2p_peer_id.as_deref()?;
        relay_hint.as_ref()?;
        let (ip, port) = synthetic_relay_endpoint(peer_id);
        Some(NetAddress::new(ip, port))
    })?;
    local.services |= services;
    if let Some(port) = relay_port {
        local.set_relay_port(Some(port));
    }
    if let Some(capacity) = relay_capacity {
        local.set_relay_capacity(Some(capacity));
    }
    if let Some(ttl_ms) = relay_ttl_ms {
        local.set_relay_ttl_ms(Some(ttl_ms));
    }
    if let Some(role) = relay_role {
        local.set_relay_role(Some(role));
    }
    if let Some(peer_id) = libp2p_peer_id {
        local.set_libp2p_peer_id(Some(peer_id));
    }
    if let Some(hint) = relay_hint {
        local.set_relay_circuit_hint(Some(hint));
    }
    Some(local)
}

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
        loop {
            self.router
                .enqueue(make_message!(
                    Payload::RequestAddresses,
                    RequestAddressesMessage { include_all_subnetworks: false, subnetwork_id: None }
                ))
                .await?;

            let msg = dequeue_with_timeout!(self.incoming_route, Payload::Addresses)?;
            let address_list: Vec<NetAddress> = msg.try_into()?;
            if address_list.len() > MAX_ADDRESSES_RECEIVE {
                return Err(ProtocolError::OtherOwned(format!(
                    "address count {} exceeded {}",
                    address_list.len(),
                    MAX_ADDRESSES_RECEIVE
                )));
            }
            {
                let mut amgr_lock = self.ctx.address_manager.lock();
                for addr in address_list {
                    if self.ctx.is_local_libp2p_peer_id(addr.libp2p_peer_id.as_deref()) {
                        continue;
                    }
                    amgr_lock.add_address(addr)
                }
            }
            sleep(ADDRESS_REFRESH_INTERVAL).await;
        }
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
        loop {
            dequeue!(self.incoming_route, Payload::RequestAddresses)?;
            let addresses = {
                let address_manager = self.ctx.address_manager.lock();
                address_manager.iterate_addresses().chain(address_manager.libp2p_discovery_addresses()).collect_vec()
            };
            let address_list =
                select_addresses_for_gossip(addresses, self.current_local_address()).into_iter().map(Into::into).collect();
            self.router.enqueue(make_message!(Payload::Addresses, AddressesMessage { address_list })).await?;
        }
    }

    fn current_local_address(&self) -> Option<NetAddress> {
        let base_address = self.ctx.address_manager.lock().best_local_address().or_else(|| self.ctx.libp2p_advertise_address());
        apply_libp2p_advertisement(base_address, self.ctx.libp2p_advertisement())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_utils::networking::{IpAddress, NET_ADDRESS_SERVICE_LIBP2P_RELAY, synthetic_relay_endpoint};
    use std::{net::Ipv4Addr, str::FromStr};

    const PEER_ID: &str = "12D3KooWANUDpDH4cX56NHHs7u7aFPZ63Sdo8GTDpVwGScBht9u7";
    const RELAY_HINT: &str = "/ip4/23.118.8.163/tcp/18111/p2p/12D3KooWK8n2eei2n7MaUvYgCF9Km1unreBEyDndEhgyYdJSbqvo/p2p-circuit";

    fn private_advertisement(peer_id: Option<&str>, relay_hint: Option<&str>) -> Libp2pAdvertisement {
        (0, None, None, None, Some(RelayRole::Private), peer_id.map(str::to_owned), relay_hint.map(str::to_owned))
    }

    #[test]
    fn private_relay_hint_uses_synthetic_address_without_tcp_address() {
        let address = apply_libp2p_advertisement(None, private_advertisement(Some(PEER_ID), Some(RELAY_HINT)))
            .expect("private relay hint should create a gossip address");
        let (expected_ip, expected_port) = synthetic_relay_endpoint(PEER_ID);

        assert_eq!(address.ip, expected_ip);
        assert_eq!(address.port, expected_port);
        assert_eq!(address.relay_role, Some(RelayRole::Private));
        assert_eq!(address.libp2p_peer_id.as_deref(), Some(PEER_ID));
        assert_eq!(address.relay_circuit_hint.as_deref(), Some(RELAY_HINT));
        assert!(address.is_synthetic_relay_hint());
    }

    #[test]
    fn incomplete_private_relay_identity_does_not_create_synthetic_address() {
        assert!(apply_libp2p_advertisement(None, private_advertisement(Some(PEER_ID), None)).is_none());
        assert!(apply_libp2p_advertisement(None, private_advertisement(None, Some(RELAY_HINT))).is_none());
    }

    #[test]
    fn existing_tcp_address_remains_the_gossip_base() {
        let base = NetAddress::new(IpAddress::from_str("203.0.113.9").unwrap(), 16111);
        let address = apply_libp2p_advertisement(Some(base.clone()), private_advertisement(Some(PEER_ID), Some(RELAY_HINT)))
            .expect("existing address should remain usable");

        assert_eq!(address.ip, base.ip);
        assert_eq!(address.port, base.port);
        assert_eq!(address.libp2p_peer_id.as_deref(), Some(PEER_ID));
        assert_eq!(address.relay_circuit_hint.as_deref(), Some(RELAY_HINT));
    }

    #[test]
    fn complete_local_advertisement_is_guaranteed_in_a_full_gossip_response() {
        let local = NetAddress::new(IpAddress::from_str("8.8.8.8").unwrap(), 16111)
            .with_services(NET_ADDRESS_SERVICE_LIBP2P_RELAY)
            .with_relay_port(Some(16112))
            .with_relay_role(Some(RelayRole::Public));
        let mut addresses = vec![NetAddress::new(local.ip, local.port)];
        addresses.extend(
            (0..1_100u16).map(|index| NetAddress::new(Ipv4Addr::new(11, (index / 256) as u8, (index % 256) as u8, 1).into(), 16111)),
        );

        let selected = select_addresses_for_gossip(addresses, Some(local.clone()));

        assert_eq!(selected.len(), MAX_ADDRESSES_SEND);
        let matching = selected.iter().filter(|address| address.ip == local.ip && address.port == local.port).collect_vec();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].relay_port, Some(16112));
        assert!(matching[0].has_services(NET_ADDRESS_SERVICE_LIBP2P_RELAY));
    }

    #[test]
    fn ordinary_local_address_keeps_existing_sampling_behavior() {
        let local = NetAddress::new(IpAddress::from_str("8.8.8.8").unwrap(), 16111);
        let addresses = vec![NetAddress::new(IpAddress::from_str("1.1.1.1").unwrap(), 16111)];

        let selected = select_addresses_for_gossip(addresses, Some(local.clone()));

        assert_eq!(selected.len(), 2);
        assert!(selected.contains(&local));
    }

    #[test]
    fn discovery_metadata_is_merged_into_an_earlier_plain_duplicate() {
        let plain = NetAddress::new(IpAddress::from_str("8.8.8.8").unwrap(), 16111);
        let enriched = plain
            .clone()
            .with_services(NET_ADDRESS_SERVICE_LIBP2P_RELAY)
            .with_relay_port(Some(16112))
            .with_relay_role(Some(RelayRole::Public));

        let selected = select_addresses_for_gossip(vec![plain, enriched], None);

        assert_eq!(selected.len(), 1);
        assert!(selected[0].has_services(NET_ADDRESS_SERVICE_LIBP2P_RELAY));
        assert_eq!(selected[0].relay_port, Some(16112));
    }
}
