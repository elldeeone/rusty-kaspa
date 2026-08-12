use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use kaspa_utils::networking::{IpAddress, NetAddress, RelayRole};

use super::MAX_CONNECTION_FAILED_COUNT;

const MAX_DISCOVERY_ADDRESSES: usize = 512;
const DEFAULT_DISCOVERY_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum DiscoveryKey {
    PublicRelay(IpAddress, u16),
    PrivatePeer(String),
}

#[derive(Clone, Debug)]
struct DiscoveryEntry {
    address: NetAddress,
    expires_at: Instant,
    observed_at: Instant,
    connection_failed_count: u64,
}

/// Bounded, process-local retention for enriched addresses which may be
/// evicted immediately from the normal address store when it is full.
pub(super) struct Libp2pDiscoveryAddressBook {
    entries: HashMap<DiscoveryKey, DiscoveryEntry>,
    max_entries: usize,
    default_ttl: Duration,
}

impl Default for Libp2pDiscoveryAddressBook {
    fn default() -> Self {
        Self { entries: HashMap::new(), max_entries: MAX_DISCOVERY_ADDRESSES, default_ttl: DEFAULT_DISCOVERY_TTL }
    }
}

impl Libp2pDiscoveryAddressBook {
    pub(super) fn observe(&mut self, address: &NetAddress, now: Instant) {
        let Some(key) = Self::key(address) else {
            return;
        };

        self.prune(now);
        if !self.entries.contains_key(&key) && self.entries.len() >= self.max_entries {
            let oldest = self.entries.iter().min_by_key(|(_, entry)| entry.observed_at).map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }

        let ttl = address
            .relay_ttl_ms
            .map(Duration::from_millis)
            .map(|advertised| advertised.min(self.default_ttl))
            .unwrap_or(self.default_ttl);
        let connection_failed_count = self.entries.get(&key).map(|entry| entry.connection_failed_count).unwrap_or(1);
        self.entries.insert(
            key,
            DiscoveryEntry { address: address.clone(), expires_at: now + ttl, observed_at: now, connection_failed_count },
        );
    }

    pub(super) fn mark_connection_failure(&mut self, address: &NetAddress) {
        let Some(key) = Self::key(address) else {
            return;
        };
        let remove = self.entries.get_mut(&key).is_some_and(|entry| {
            entry.connection_failed_count = entry.connection_failed_count.saturating_add(1);
            entry.connection_failed_count > MAX_CONNECTION_FAILED_COUNT
        });
        if remove {
            self.entries.remove(&key);
        }
    }

    pub(super) fn mark_connection_success(&mut self, address: &NetAddress) {
        let Some(key) = Self::key(address) else {
            return;
        };
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.connection_failed_count = 0;
        }
    }

    pub(super) fn addresses(&self, now: Instant) -> Vec<NetAddress> {
        self.entries.values().filter(|entry| entry.expires_at > now).map(|entry| entry.address.clone()).collect()
    }

    pub(super) fn private_addresses(&self, now: Instant) -> Vec<NetAddress> {
        self.entries
            .iter()
            .filter(|(key, entry)| entry.expires_at > now && matches!(key, DiscoveryKey::PrivatePeer(_)))
            .map(|(_, entry)| entry.address.clone())
            .collect()
    }

    pub(super) fn remove_by_ip(&mut self, ip: IpAddress) {
        self.entries.retain(|_, entry| entry.address.ip != ip);
    }

    fn key(address: &NetAddress) -> Option<DiscoveryKey> {
        if !address.has_libp2p_discovery_metadata() {
            return None;
        }
        if address.relay_role != Some(RelayRole::Private)
            && let Some(relay_port) = address.relay_port
            && address.is_libp2p_relay()
        {
            return Some(DiscoveryKey::PublicRelay(address.ip, relay_port));
        }
        address.libp2p_peer_id.clone().map(DiscoveryKey::PrivatePeer)
    }

    fn prune(&mut self, now: Instant) {
        self.entries.retain(|_, entry| entry.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_utils::networking::NET_ADDRESS_SERVICE_LIBP2P_RELAY;
    use std::str::FromStr;

    fn public_relay(ip: &str, relay_port: u16) -> NetAddress {
        NetAddress::new(IpAddress::from_str(ip).unwrap(), 16111)
            .with_services(NET_ADDRESS_SERVICE_LIBP2P_RELAY)
            .with_relay_port(Some(relay_port))
    }

    fn private_peer(peer_id: &str) -> NetAddress {
        NetAddress::new(IpAddress::from_str("10.0.0.2").unwrap(), 16111)
            .with_relay_role(Some(RelayRole::Private))
            .with_libp2p_peer_id(Some(peer_id.to_string()))
            .with_relay_circuit_hint(Some("/ip4/8.8.8.8/tcp/16112/p2p/12D3KooWRelay/p2p-circuit".to_string()))
    }

    #[test]
    fn retains_only_complete_discovery_addresses() {
        let now = Instant::now();
        let mut book = Libp2pDiscoveryAddressBook::default();
        book.observe(&NetAddress::new(IpAddress::from_str("1.1.1.1").unwrap(), 16111), now);
        book.observe(&public_relay("8.8.8.8", 16112), now);
        book.observe(&private_peer("12D3KooWPrivate"), now);
        book.observe(&private_peer("12D3KooWMalformed").with_relay_circuit_hint(Some("not-a-relay-address".to_string())), now);

        assert_eq!(book.addresses(now).len(), 2);
        assert_eq!(book.private_addresses(now).len(), 1);
    }

    #[test]
    fn refreshes_duplicates_and_expires_entries() {
        let now = Instant::now();
        let mut book = Libp2pDiscoveryAddressBook { entries: HashMap::new(), max_entries: 2, default_ttl: Duration::from_secs(10) };
        let relay = public_relay("8.8.8.8", 16112);
        book.observe(&relay, now);
        book.observe(&relay.clone().with_relay_capacity(Some(32)), now + Duration::from_secs(5));

        let retained = book.addresses(now + Duration::from_secs(11));
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].relay_capacity, Some(32));
        assert!(book.addresses(now + Duration::from_secs(16)).is_empty());
    }

    #[test]
    fn evicts_the_oldest_entry_at_capacity() {
        let now = Instant::now();
        let mut book = Libp2pDiscoveryAddressBook { entries: HashMap::new(), max_entries: 2, default_ttl: Duration::from_secs(60) };
        book.observe(&public_relay("8.8.8.8", 16112), now);
        book.observe(&public_relay("8.8.4.4", 16112), now + Duration::from_secs(1));
        book.observe(&public_relay("1.1.1.1", 16112), now + Duration::from_secs(2));

        let retained = book.addresses(now + Duration::from_secs(3));
        assert_eq!(retained.len(), 2);
        assert!(!retained.iter().any(|address| address.ip == IpAddress::from_str("8.8.8.8").unwrap()));
    }

    #[test]
    fn refresh_preserves_failures_and_repeated_failures_remove_entry() {
        let now = Instant::now();
        let mut book = Libp2pDiscoveryAddressBook::default();
        let peer = private_peer("12D3KooWPrivate");
        book.observe(&peer, now);
        book.mark_connection_failure(&peer);

        book.observe(&peer, now + Duration::from_secs(1));
        assert_eq!(book.entries.get(&DiscoveryKey::PrivatePeer("12D3KooWPrivate".to_string())).unwrap().connection_failed_count, 2);

        book.mark_connection_failure(&peer);
        book.mark_connection_failure(&peer);
        assert!(book.addresses(now + Duration::from_secs(2)).is_empty());
    }

    #[test]
    fn successful_connection_resets_failure_count() {
        let now = Instant::now();
        let mut book = Libp2pDiscoveryAddressBook::default();
        let peer = private_peer("12D3KooWPrivate");
        book.observe(&peer, now);
        book.mark_connection_failure(&peer);
        book.mark_connection_success(&peer);

        for _ in 0..MAX_CONNECTION_FAILED_COUNT {
            book.mark_connection_failure(&peer);
        }
        assert_eq!(book.addresses(now).len(), 1);

        book.mark_connection_failure(&peer);
        assert!(book.addresses(now).is_empty());
    }
}
