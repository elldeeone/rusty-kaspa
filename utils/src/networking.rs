// #![allow(dead_code)]
use borsh::{BorshDeserialize, BorshSerialize};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::{
    fmt::Display,
    net::{AddrParseError, IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    ops::Deref,
    str::FromStr,
};
use uuid::Uuid;
use wasm_bindgen::prelude::*;

// A network address serialization of [`ContextualNetAddress`].
#[wasm_bindgen(typescript_custom_section)]
const TS_IP_ADDRESS: &'static str = r#"
    /**
     * Generic network address representation.
     * 
     * @category General
     */
    export interface INetworkAddress {
        /**
         * IPv4 or IPv6 address.
         */
        ip: string;
        /**
         * Optional port number.
         */
        port?: number;
    }
"#;

/// A bucket based on an ip's prefix bytes.
/// for ipv4 it consists of 6 leading zero bytes, and the first two octets,
/// for ipv6 it consists of the first 8 octets,
/// encoded into a big endian u64.
#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
pub struct PrefixBucket(u64);

impl PrefixBucket {
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl From<&IpAddress> for PrefixBucket {
    fn from(ip_address: &IpAddress) -> Self {
        match ip_address.0 {
            IpAddr::V4(ipv4) => {
                let prefix_bytes = ipv4.octets();
                Self(u64::from_be_bytes([0u8, 0u8, 0u8, 0u8, 0u8, 0u8, prefix_bytes[0], prefix_bytes[1]]))
            }
            IpAddr::V6(ipv6) => {
                if let Some(ipv4) = ipv6.to_ipv4() {
                    let prefix_bytes = ipv4.octets();
                    Self(u64::from_be_bytes([0u8, 0u8, 0u8, 0u8, 0u8, 0u8, prefix_bytes[0], prefix_bytes[1]]))
                } else {
                    // Else use first 8 bytes (routing prefix + subnetwork id) of ipv6
                    Self(u64::from_be_bytes(ipv6.octets().as_slice()[..8].try_into().expect("Slice with incorrect length")))
                }
            }
        }
    }
}

impl From<&NetAddress> for PrefixBucket {
    fn from(net_address: &NetAddress) -> Self {
        Self::from(&net_address.ip)
    }
}

/// An IP address, newtype of [IpAddr].
#[derive(PartialEq, Eq, Hash, Copy, Clone, Serialize, Deserialize, Debug)]
#[repr(transparent)]
pub struct IpAddress(pub IpAddr);

impl IpAddress {
    pub fn new(ip: IpAddr) -> Self {
        Self(ip)
    }

    pub fn is_publicly_routable(&self) -> bool {
        if self.is_loopback() || self.is_unspecified() {
            return false;
        }

        match self.0 {
            IpAddr::V4(ip) => {
                // RFC 1918 is covered by is_private
                // RFC 5737 is covered by is_documentation
                // RFC 3927 is covered by is_link_local
                // RFC  919 is covered by is_broadcast (wasn't originally covered in go code)
                if ip.is_broadcast() || ip.is_private() || ip.is_documentation() || ip.is_link_local() {
                    return false;
                }
            }
            IpAddr::V6(_ip) => {
                // All of the is_ helper functions for ipv6 are currently marked unstable
            }
        }

        // Based on values from network.go
        let unroutable_nets = [
            "198.18.0.0/15",   // RFC 2544
            "2001:DB8::/32",   // RFC 3849
            "2002::/16",       // RFC 3964
            "FC00::/7",        // RFC 4193
            "2001::/32",       // RFC 4380
            "2001:10::/28",    // RFC 4843
            "FE80::/64",       // RFC 4862
            "64:FF9B::/96",    // RFC 6052
            "::FFFF:0:0:0/96", // RFC 6145
            "100.64.0.0/10",   // RFC 6598
            "0.0.0.0/8",       // Zero Net
            "2001:470::/32",   // Hurricane Electric IPv6 address block.
        ];

        for curr_net in unroutable_nets {
            if IpNet::from_str(curr_net).unwrap().contains(&self.0) {
                return false;
            }
        }

        true
    }

    pub fn prefix_bucket(&self) -> PrefixBucket {
        PrefixBucket::from(self)
    }
}

impl From<IpAddr> for IpAddress {
    fn from(ip: IpAddr) -> Self {
        Self(ip)
    }
}
impl From<Ipv4Addr> for IpAddress {
    fn from(value: Ipv4Addr) -> Self {
        Self(value.into())
    }
}
impl From<Ipv6Addr> for IpAddress {
    fn from(value: Ipv6Addr) -> Self {
        Self(value.into())
    }
}
impl From<IpAddress> for IpAddr {
    fn from(value: IpAddress) -> Self {
        value.0
    }
}

impl FromStr for IpAddress {
    type Err = AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        IpAddr::from_str(s).map(IpAddress::from)
    }
}

impl Display for IpAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Deref for IpAddress {
    type Target = IpAddr;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

//
// Borsh serializers need to be manually implemented for `IpAddress` since
// IpAddr does not currently support Borsh
//

impl BorshSerialize for IpAddress {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> ::core::result::Result<(), std::io::Error> {
        let variant_idx: u8 = match self.0 {
            IpAddr::V4(..) => 0u8,
            IpAddr::V6(..) => 1u8,
        };
        writer.write_all(&variant_idx.to_le_bytes())?;
        match self.0 {
            IpAddr::V4(id0) => {
                borsh::BorshSerialize::serialize(&id0.octets(), writer)?;
            }
            IpAddr::V6(id0) => {
                borsh::BorshSerialize::serialize(&id0.octets(), writer)?;
            }
        }
        Ok(())
    }
}

impl BorshDeserialize for IpAddress {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> ::core::result::Result<Self, borsh::io::Error> {
        let variant_idx: u8 = BorshDeserialize::deserialize_reader(reader)?;
        let ip = match variant_idx {
            0u8 => {
                let octets: [u8; 4] = BorshDeserialize::deserialize_reader(reader)?;
                IpAddr::V4(Ipv4Addr::from(octets))
            }
            1u8 => {
                let octets: [u8; 16] = BorshDeserialize::deserialize_reader(reader)?;
                IpAddr::V6(Ipv6Addr::from(octets))
            }
            _ => {
                let msg = format!("Unexpected variant index: {:?}", variant_idx);
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, msg));
            }
        };
        Ok(Self(ip))
    }
}

/// Service flag indicating that an address supports acting as a libp2p relay bridge.
pub const NET_ADDRESS_SERVICE_LIBP2P_RELAY: u64 = 1 << 0;

/// A network address, equivalent of a [SocketAddr], enriched with service metadata.
#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub struct NetAddress {
    pub ip: IpAddress,
    pub port: u16,
    #[serde(default)]
    pub services: u64,
    #[serde(default)]
    pub relay_port: Option<u16>,
}

/// Versioned Borsh wire encoding for `NetAddress`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum NetAddressWire {
    V0V4 { octets: [u8; 4], port: u16 } = 0,
    V0V6 { octets: [u8; 16], port: u16 } = 1,
    V1V4 { octets: [u8; 4], port: u16, services: u64, relay_port: Option<u16> } = 2,
    V1V6 { octets: [u8; 16], port: u16, services: u64, relay_port: Option<u16> } = 3,
}

impl NetAddressWire {
    pub fn from_net_address_v0(addr: &NetAddress) -> Self {
        match addr.ip.0 {
            IpAddr::V4(ip) => Self::V0V4 { octets: ip.octets(), port: addr.port },
            IpAddr::V6(ip) => Self::V0V6 { octets: ip.octets(), port: addr.port },
        }
    }

    pub fn from_net_address_v1(addr: &NetAddress) -> Self {
        match addr.ip.0 {
            IpAddr::V4(ip) => {
                Self::V1V4 { octets: ip.octets(), port: addr.port, services: addr.services, relay_port: addr.relay_port }
            }
            IpAddr::V6(ip) => {
                Self::V1V6 { octets: ip.octets(), port: addr.port, services: addr.services, relay_port: addr.relay_port }
            }
        }
    }

    pub fn into_net_address(self) -> NetAddress {
        match self {
            NetAddressWire::V0V4 { octets, port } => NetAddress::new(IpAddr::V4(Ipv4Addr::from(octets)).into(), port),
            NetAddressWire::V0V6 { octets, port } => NetAddress::new(IpAddr::V6(Ipv6Addr::from(octets)).into(), port),
            NetAddressWire::V1V4 { octets, port, services, relay_port } => {
                NetAddress { ip: IpAddr::V4(Ipv4Addr::from(octets)).into(), port, services, relay_port }
            }
            NetAddressWire::V1V6 { octets, port, services, relay_port } => {
                NetAddress { ip: IpAddr::V6(Ipv6Addr::from(octets)).into(), port, services, relay_port }
            }
        }
    }
}

impl NetAddress {
    pub fn new(ip: IpAddress, port: u16) -> Self {
        Self { ip, port, services: 0, relay_port: None }
    }

    pub fn prefix_bucket(&self) -> PrefixBucket {
        PrefixBucket::from(self)
    }

    pub fn with_services(mut self, services: u64) -> Self {
        self.services = services;
        self
    }

    pub fn with_relay_port(mut self, relay_port: Option<u16>) -> Self {
        self.relay_port = relay_port;
        self
    }

    pub fn set_services(&mut self, services: u64) {
        self.services = services;
    }

    pub fn set_relay_port(&mut self, relay_port: Option<u16>) {
        self.relay_port = relay_port;
    }

    /// Merge capability metadata from another address that refers to the same endpoint.
    /// Services are ORed; relay port is upgraded if provided by the other address.
    pub fn merge_metadata(&mut self, other: &NetAddress) {
        self.services |= other.services;
        if let Some(port) = other.relay_port {
            self.relay_port = Some(port);
        }
    }

    pub fn has_services(&self, mask: u64) -> bool {
        (self.services & mask) == mask
    }

    pub fn is_libp2p_relay(&self) -> bool {
        self.has_services(NET_ADDRESS_SERVICE_LIBP2P_RELAY)
    }
}

impl From<SocketAddr> for NetAddress {
    fn from(value: SocketAddr) -> Self {
        Self::new(value.ip().into(), value.port())
    }
}

impl From<NetAddress> for SocketAddr {
    fn from(value: NetAddress) -> Self {
        Self::new(value.ip.0, value.port)
    }
}

impl FromStr for NetAddress {
    type Err = AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        SocketAddr::from_str(s).map(NetAddress::from)
    }
}

impl Display for NetAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        SocketAddr::from(self.to_owned()).fmt(f)
    }
}

impl PartialEq for NetAddress {
    fn eq(&self, other: &Self) -> bool {
        self.ip == other.ip && self.port == other.port
    }
}

impl Eq for NetAddress {}

impl std::hash::Hash for NetAddress {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ip.hash(state);
        self.port.hash(state);
    }
}

/// A network address possibly without explicit port.
///
/// Use `normalize` to get a fully determined address.
#[derive(PartialEq, Eq, Hash, Copy, Clone, Serialize, Deserialize, Debug, BorshSerialize, BorshDeserialize)]
pub struct ContextualNetAddress {
    ip: IpAddress,
    port: Option<u16>,
}

impl ContextualNetAddress {
    pub fn new(ip: IpAddress, port: Option<u16>) -> Self {
        Self { ip, port }
    }

    pub fn has_port(&self) -> bool {
        self.port.is_some()
    }

    pub fn normalize(&self, default_port: u16) -> NetAddress {
        NetAddress::new(self.ip, self.port.unwrap_or(default_port))
    }

    pub fn unspecified() -> Self {
        Self { ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)).into(), port: None }
    }

    pub fn loopback() -> Self {
        Self { ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)).into(), port: None }
    }

    pub fn port_not_specified(&self) -> bool {
        self.port.is_none()
    }

    pub fn with_port(&self, port: u16) -> Self {
        Self { ip: self.ip, port: Some(port) }
    }
}

impl From<NetAddress> for ContextualNetAddress {
    fn from(value: NetAddress) -> Self {
        Self::new(value.ip, Some(value.port))
    }
}

impl FromStr for ContextualNetAddress {
    type Err = AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match SocketAddr::from_str(s) {
            Ok(socket) => Ok(Self::new(socket.ip().into(), Some(socket.port()))),
            Err(_) => Ok(Self::new(IpAddress::from_str(s)?, None)),
        }
    }
}

impl TryFrom<&str> for ContextualNetAddress {
    type Error = AddrParseError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        ContextualNetAddress::from_str(s)
    }
}

impl TryFrom<String> for ContextualNetAddress {
    type Error = AddrParseError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        ContextualNetAddress::from_str(&s)
    }
}

impl Display for ContextualNetAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.port {
            Some(port) => SocketAddr::new(self.ip.into(), port).fmt(f),
            None => self.ip.fmt(f),
        }
    }
}
#[derive(PartialEq, Eq, Hash, Copy, Clone, Serialize, Deserialize, Debug, Default)]
#[repr(transparent)]
pub struct PeerId(pub Uuid);

impl PeerId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, uuid::Error> {
        Ok(Uuid::from_slice(bytes)?.into())
    }
}
impl From<Uuid> for PeerId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}
impl From<PeerId> for Uuid {
    fn from(value: PeerId) -> Self {
        value.0
    }
}

impl FromStr for PeerId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::from_str(s).map(PeerId::from)
    }
}

impl Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Deref for PeerId {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

//
// Borsh serializers need to be manually implemented for `PeerId` since
// Uuid does not currently support Borsh
//

impl BorshSerialize for PeerId {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> ::core::result::Result<(), std::io::Error> {
        borsh::BorshSerialize::serialize(&self.0.as_bytes(), writer)?;
        Ok(())
    }
}

impl BorshDeserialize for PeerId {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> ::core::result::Result<Self, std::io::Error> {
        let bytes: uuid::Bytes = BorshDeserialize::deserialize_reader(reader)?;
        Ok(Self::new(Uuid::from_bytes(bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_ip_address_borsh() {
        // Tests for IpAddress Borsh ser/deser since we manually implemented them
        let ip: IpAddress = Ipv4Addr::from([44u8; 4]).into();
        let bin = borsh::to_vec(&ip).unwrap();
        let ip2: IpAddress = BorshDeserialize::try_from_slice(&bin).unwrap();
        assert_eq!(ip, ip2);

        let ip: IpAddress = Ipv6Addr::from([66u8; 16]).into();
        let bin = borsh::to_vec(&ip).unwrap();
        let ip2: IpAddress = BorshDeserialize::try_from_slice(&bin).unwrap();
        assert_eq!(ip, ip2);
    }

    #[test]
    fn test_peer_id_borsh() {
        // Tests for PeerId Borsh ser/deser since we manually implemented them
        let id: PeerId = Uuid::new_v4().into();
        let bin = borsh::to_vec(&id).unwrap();
        let id2: PeerId = BorshDeserialize::try_from_slice(&bin).unwrap();
        assert_eq!(id, id2);

        let id: PeerId = Uuid::from_bytes([123u8; 16]).into();
        let bin = borsh::to_vec(&id).unwrap();
        let id2: PeerId = BorshDeserialize::try_from_slice(&bin).unwrap();
        assert_eq!(id, id2);
    }

    /// Legacy layout used by v1 (ip + port only).
    #[derive(PartialEq, Eq, Hash, Copy, Clone, Serialize, Deserialize, Debug, BorshSerialize, BorshDeserialize)]
    pub struct NetAddressLegacy {
        pub ip: IpAddress,
        pub port: u16,
    }

    fn legacy_net_address_bytes(ip: IpAddress, port: u16) -> Vec<u8> {
        borsh::to_vec(&NetAddressLegacy { ip, port }).unwrap()
    }

    #[test]
    fn test_net_address_wire_legacy_single_decodes() {
        let ip = IpAddress::from_str("5.6.7.8").unwrap();
        let port = 5555;
        let legacy = legacy_net_address_bytes(ip, port);

        let wire: NetAddressWire = BorshDeserialize::try_from_slice(&legacy).unwrap();
        let decoded = wire.into_net_address();

        assert_eq!(decoded.ip, ip);
        assert_eq!(decoded.port, port);
        assert_eq!(decoded.services, 0);
        assert_eq!(decoded.relay_port, None);
    }

    #[test]
    fn test_net_address_wire_legacy_vec_decodes() {
        let a_ip = IpAddress::from_str("1.2.3.4").unwrap();
        let a_port = 1111;
        let b_ip = IpAddress::from_str("9.8.7.6").unwrap();
        let b_port = 2222;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(2u32).to_le_bytes());
        bytes.extend_from_slice(&legacy_net_address_bytes(a_ip, a_port));
        bytes.extend_from_slice(&legacy_net_address_bytes(b_ip, b_port));

        let decoded: Vec<NetAddressWire> = BorshDeserialize::try_from_slice(&bytes).unwrap();
        let decoded: Vec<NetAddress> = decoded.into_iter().map(NetAddressWire::into_net_address).collect();

        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].ip, a_ip);
        assert_eq!(decoded[0].port, a_port);
        assert_eq!(decoded[1].ip, b_ip);
        assert_eq!(decoded[1].port, b_port);
    }

    #[test]
    fn test_net_address_wire_v1_roundtrip() {
        let addr = NetAddress::new(IpAddress::from_str("1.2.3.4").unwrap(), 1234).with_services(0b101).with_relay_port(Some(1818));

        let wire = NetAddressWire::from_net_address_v1(&addr);
        let bin = borsh::to_vec(&wire).unwrap();
        let decoded: NetAddressWire = BorshDeserialize::try_from_slice(&bin).unwrap();
        let addr2 = decoded.into_net_address();

        assert_eq!(addr.ip, addr2.ip);
        assert_eq!(addr.port, addr2.port);
        assert_eq!(addr.services, addr2.services);
        assert_eq!(addr.relay_port, addr2.relay_port);
    }

    #[test]
    fn test_net_address_from_str() {
        let addr_v4 = NetAddress::from_str("1.2.3.4:5678");
        assert!(addr_v4.is_ok());
        let addr_v6 = NetAddress::from_str("[2a01:4f8:191:1143::2]:5678");
        assert!(addr_v6.is_ok());
    }

    #[test]
    fn net_address_merges_services_and_relay_port() {
        let mut base = NetAddress::new(IpAddress::from_str("1.2.3.4").unwrap(), 1234).with_services(0b001);
        let other = NetAddress::new(base.ip, base.port).with_services(0b100).with_relay_port(Some(1818));

        base.merge_metadata(&other);
        assert_eq!(base.services, 0b101);
        assert_eq!(base.relay_port, Some(1818));
    }

    #[test]
    fn net_address_backwards_compatible() {
        let net_v1 = NetAddressLegacy { ip: IpAddress(IpAddr::V4(Ipv4Addr::from_bits(0))), port: 0 };

        let net_v1_encoded = borsh::to_vec(&net_v1).unwrap();

        let netwire_with_v1: NetAddressWire = BorshDeserialize::try_from_slice(&net_v1_encoded).unwrap();

        let decoded = netwire_with_v1.into_net_address();
        assert_eq!(decoded.ip.0, IpAddr::V4(Ipv4Addr::from_bits(0)));
        assert_eq!(decoded.port, 0);
        assert_eq!(decoded.services, 0);
        assert!(decoded.relay_port.is_none());
    }

    #[test]
    fn net_address_equality_ignores_metadata() {
        let a = NetAddress::new(IpAddress::from_str("5.6.7.8").unwrap(), 5555).with_services(0b001);
        let mut b = NetAddress::new(a.ip, a.port);
        b.relay_port = Some(9999);
        assert_eq!(a, b);
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    #[test]
    fn test_prefix_bucket() {
        let prefix_bytes: [u8; 2] = [42u8, 43u8];
        let addr = NetAddress::from_str(format!("{0}.{1}.3.4:5678", prefix_bytes[0], prefix_bytes[1]).as_str()).unwrap();
        assert!(addr.prefix_bucket() == PrefixBucket(u16::from_be_bytes(prefix_bytes) as u64));
    }

    #[test]
    fn test_contextual_address_ser() {
        let addr = IpAddress::from_str("127.0.0.1").unwrap();
        let port = Some(1234);
        let net_addr = ContextualNetAddress::new(addr, port);
        let s = serde_json::to_string(&net_addr).unwrap();
        assert_eq!(s, r#"{"ip":"127.0.0.1","port":1234}"#);
    }

    #[test]
    fn test_is_publicly_routable() {
        // RFC 2544 tests
        assert!(!IpAddress::from_str("198.18.0.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("198.19.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("198.17.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("198.20.0.0").unwrap().is_publicly_routable());

        // Zero net tests
        assert!(!IpAddress::from_str("0.0.0.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("0.0.0.1").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("0.0.1.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("0.1.0.0").unwrap().is_publicly_routable());

        // RFC 3849
        assert!(!IpAddress::from_str("2001:db8::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("2001:db8:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:db7:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:db9::").unwrap().is_publicly_routable());

        // Localhost
        assert!(!IpAddress::from_str("127.0.0.1").unwrap().is_publicly_routable());

        // Some random routable IP
        assert!(IpAddress::from_str("123.45.67.89").unwrap().is_publicly_routable());

        // RFC 1918
        assert!(!IpAddress::from_str("10.0.0.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("10.255.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("9.255.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("11.0.0.0").unwrap().is_publicly_routable());

        assert!(!IpAddress::from_str("172.16.0.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("172.31.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("172.15.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("172.32.0.0").unwrap().is_publicly_routable());

        assert!(!IpAddress::from_str("192.168.0.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("192.168.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("192.167.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("192.169.0.0").unwrap().is_publicly_routable());

        // RFC 3927
        assert!(!IpAddress::from_str("169.254.0.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("169.254.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("169.253.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("169.255.0.0").unwrap().is_publicly_routable());

        // RFC 3964
        assert!(!IpAddress::from_str("2002::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:ffff:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2003::").unwrap().is_publicly_routable());

        // RFC 4193
        assert!(!IpAddress::from_str("fc00::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("fb00:ffff:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("fe00::").unwrap().is_publicly_routable());

        // RFC 4380
        assert!(!IpAddress::from_str("2001::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("2001:0:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2000:0:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:1::").unwrap().is_publicly_routable());

        // RFC 4843
        assert!(!IpAddress::from_str("2001:10::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("2001:1f:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:f:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:20::").unwrap().is_publicly_routable());

        // RFC 4862
        assert!(!IpAddress::from_str("fe80::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("fe80::ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("fe7f::ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("fe81::").unwrap().is_publicly_routable());

        // RFC 5737
        assert!(!IpAddress::from_str("192.0.2.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("192.0.2.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("192.0.1.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("192.0.3.0").unwrap().is_publicly_routable());

        assert!(!IpAddress::from_str("198.51.100.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("198.51.100.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("198.51.99.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("198.51.101.0").unwrap().is_publicly_routable());

        assert!(!IpAddress::from_str("203.0.113.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("203.0.113.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("203.0.112.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("203.0.114.0").unwrap().is_publicly_routable());

        // RFC 6052
        assert!(!IpAddress::from_str("64:ff9b::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("64:ff9b::ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("64:ff9a::ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("64:ff9b:1::").unwrap().is_publicly_routable());

        // RFC 6145
        assert!(!IpAddress::from_str("::ffff:0:0:0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("::ffff:0:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("::fffe:0:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("::ffff:1:0:0").unwrap().is_publicly_routable());

        // RFC 6598
        assert!(!IpAddress::from_str("100.64.0.0").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("100.127.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("100.63.255.255").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("100.128.0.0").unwrap().is_publicly_routable());

        // Hurricane Electric IPv6 address block.
        assert!(!IpAddress::from_str("2001:470::").unwrap().is_publicly_routable());
        assert!(!IpAddress::from_str("2001:470:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:46f:ffff:ffff:ffff:ffff:ffff:ffff").unwrap().is_publicly_routable());
        assert!(IpAddress::from_str("2001:471::").unwrap().is_publicly_routable());

        // Broadcast ip
        assert!(!IpAddress::from_str("255.255.255.255").unwrap().is_publicly_routable());
    }
}
