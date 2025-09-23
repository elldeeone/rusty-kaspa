use kaspa_core::debug;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum PeerClassification {
    Public,
    Private,
}
use std::net::SocketAddr;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("Connection timeout")]
    Timeout,

    #[error("Connection refused")]
    ConnectionRefused,

    #[error("Network unreachable")]
    NetworkUnreachable,

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct ActiveProber {
    timeout_ms: u64,
}

impl ActiveProber {
    pub fn new(timeout_ms: u64) -> Self {
        Self { timeout_ms }
    }

    /// Probe a peer to determine if it's publicly accessible
    pub async fn probe_peer(&self, address: &str) -> Result<PeerClassification, ProbeError> {
        // Parse the address
        let socket_addr: SocketAddr = address
            .parse()
            .map_err(|_| ProbeError::InvalidAddress(address.to_string()))?;

        // Don't probe private/local addresses
        if self.is_private_address(&socket_addr) {
            debug!("Skipping probe for private address: {}", address);
            return Ok(PeerClassification::Private);
        }

        // Attempt to connect with timeout
        let probe_timeout = Duration::from_millis(self.timeout_ms);

        match timeout(probe_timeout, TcpStream::connect(socket_addr)).await {
            Ok(Ok(_stream)) => {
                // Successfully connected - peer is public
                debug!("Successfully probed peer {} - classified as Public", address);
                Ok(PeerClassification::Public)
            }
            Ok(Err(e)) => {
                // Connection failed - peer is private or unreachable
                debug!("Failed to probe peer {}: {} - classified as Private", address, e);

                // Check specific error types for better classification
                match e.kind() {
                    std::io::ErrorKind::ConnectionRefused => {
                        // Port is closed but host is reachable
                        Ok(PeerClassification::Private)
                    }
                    std::io::ErrorKind::TimedOut => {
                        // Likely behind firewall/NAT
                        Ok(PeerClassification::Private)
                    }
                    _ => {
                        // Other errors still classify as private
                        Ok(PeerClassification::Private)
                    }
                }
            }
            Err(_) => {
                // Timeout elapsed - peer is not reachable
                debug!("Probe timeout for peer {} - classified as Private", address);
                Ok(PeerClassification::Private)
            }
        }
    }

    /// Check if an address is private/local
    fn is_private_address(&self, addr: &SocketAddr) -> bool {
        match addr.ip() {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_loopback()
                    || ipv4.is_private()
                    || ipv4.is_link_local()
                    || ipv4.is_broadcast()
                    || ipv4.is_multicast()
            }
            std::net::IpAddr::V6(ipv6) => {
                ipv6.is_loopback()
                    || ipv6.is_multicast()
                    || ipv6.is_unspecified()
                    // Check for IPv6 private ranges
                    || (ipv6.segments()[0] & 0xfe00) == 0xfc00 // Unique local
                    || (ipv6.segments()[0] & 0xffc0) == 0xfe80 // Link local
            }
        }
    }

    /// Batch probe multiple peers concurrently
    pub async fn batch_probe(
        &self,
        addresses: Vec<String>,
    ) -> Vec<(String, Result<PeerClassification, ProbeError>)> {
        let mut results = Vec::new();
        let mut handles = Vec::new();

        for address in addresses {
            let timeout_ms = self.timeout_ms;
            let handle = tokio::spawn(async move {
                let prober = ActiveProber::new(timeout_ms);
                let result = prober.probe_peer(&address).await;
                (address, result)
            });
            handles.push(handle);
        }

        // Collect all results
        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_private_address_detection() {
        let prober = ActiveProber::new(1000);

        // Test localhost
        let result = prober.probe_peer("127.0.0.1:16111").await;
        assert!(matches!(result, Ok(PeerClassification::Private)));

        // Test private IPv4
        let result = prober.probe_peer("192.168.1.1:16111").await;
        assert!(matches!(result, Ok(PeerClassification::Private)));

        // Test private IPv4 (10.x.x.x)
        let result = prober.probe_peer("10.0.0.1:16111").await;
        assert!(matches!(result, Ok(PeerClassification::Private)));
    }

    #[test]
    fn test_is_private_address() {
        let prober = ActiveProber::new(1000);

        // Localhost
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        assert!(prober.is_private_address(&addr));

        // Private range
        let addr: SocketAddr = "192.168.1.100:8080".parse().unwrap();
        assert!(prober.is_private_address(&addr));

        // Public address
        let addr: SocketAddr = "8.8.8.8:8080".parse().unwrap();
        assert!(!prober.is_private_address(&addr));
    }
}