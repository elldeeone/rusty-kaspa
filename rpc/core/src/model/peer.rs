use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use kaspa_utils::networking::{ContextualNetAddress, IpAddress, NetAddress, NetAddressWire, PeerId};

pub type RpcNodeId = PeerId;
pub type RpcIpAddress = IpAddress;
pub type RpcPeerAddress = NetAddress;
pub type RpcContextualPeerAddress = ContextualNetAddress;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcPeerInfo {
    pub id: RpcNodeId,
    pub address: RpcPeerAddress,
    pub last_ping_duration: u64, // NOTE: i64 in gRPC protowire

    pub is_outbound: bool,
    pub time_offset: i64,
    pub user_agent: String,

    pub advertised_protocol_version: u32,
    pub time_connected: u64, // NOTE: i64 in gRPC protowire
    pub is_ibd_peer: bool,
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub(crate) struct RpcPeerInfoWire {
    pub id: RpcNodeId,
    pub address: NetAddressWire,
    pub last_ping_duration: u64, // NOTE: i64 in gRPC protowire

    pub is_outbound: bool,
    pub time_offset: i64,
    pub user_agent: String,

    pub advertised_protocol_version: u32,
    pub time_connected: u64, // NOTE: i64 in gRPC protowire
    pub is_ibd_peer: bool,
}

impl RpcPeerInfoWire {
    pub fn from_peer_info(peer: &RpcPeerInfo, wire_version: u16) -> Self {
        let address = if wire_version <= 1 {
            NetAddressWire::from_net_address_v0(&peer.address)
        } else {
            NetAddressWire::from_net_address_v1(&peer.address)
        };

        Self {
            id: peer.id,
            address,
            last_ping_duration: peer.last_ping_duration,
            is_outbound: peer.is_outbound,
            time_offset: peer.time_offset,
            user_agent: peer.user_agent.clone(),
            advertised_protocol_version: peer.advertised_protocol_version,
            time_connected: peer.time_connected,
            is_ibd_peer: peer.is_ibd_peer,
        }
    }

    pub fn into_peer_info(self) -> RpcPeerInfo {
        RpcPeerInfo {
            id: self.id,
            address: self.address.into_net_address(),
            last_ping_duration: self.last_ping_duration,
            is_outbound: self.is_outbound,
            time_offset: self.time_offset,
            user_agent: self.user_agent,
            advertised_protocol_version: self.advertised_protocol_version,
            time_connected: self.time_connected,
            is_ibd_peer: self.is_ibd_peer,
        }
    }
}
