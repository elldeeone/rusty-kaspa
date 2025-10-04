use crate::config::ProbingConfig;
use crate::metrics::SensorMetrics;
use crate::models::PeerClassification;
use log::debug;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
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

    #[error("Rate limit exceeded")]
    RateLimitExceeded,
}

/// Active prober with rate limiting and metrics
pub struct ActiveProber {
    config: ProbingConfig,
    semaphore: Arc<Semaphore>,
    metrics: Option<Arc<SensorMetrics>>,
}

impl ActiveProber {
    /// Create a new prober with configuration
    pub fn new(config: ProbingConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_probes));
        Self {
            config,
            semaphore,
            metrics: None,
        }
    }

    /// Create prober with metrics
    pub fn with_metrics(config: ProbingConfig, metrics: Arc<SensorMetrics>) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_probes));
        Self {
            config,
            semaphore,
            metrics: Some(metrics),
        }
    }

    /// Probe a peer to determine if it's publicly accessible
    pub async fn probe_peer(&self, address: &str) -> Result<(PeerClassification, u64), ProbeError> {
        // Acquire semaphore permit for rate limiting
        let _permit = self.semaphore.try_acquire()
            .map_err(|_| ProbeError::RateLimitExceeded)?;

        let start = Instant::now();

        // Parse the address
        let socket_addr: SocketAddr = address
            .parse()
            .map_err(|_| ProbeError::InvalidAddress(address.to_string()))?;

        // Skip private addresses if configured
        if self.config.skip_private_ips && self.is_private_address(&socket_addr) {
            debug!("Skipping probe for private address: {}", address);
            let duration_ms = start.elapsed().as_millis() as u64;
            return Ok((PeerClassification::Private, duration_ms));
        }

        // Add delay before probing if configured
        if self.config.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.config.delay_ms)).await;
        }

        // Attempt to connect with timeout
        let probe_timeout = Duration::from_millis(self.config.timeout_ms);

        let classification = match timeout(probe_timeout, TcpStream::connect(socket_addr)).await {
            Ok(Ok(_stream)) => {
                // Successfully connected - peer is public
                debug!("Successfully probed peer {} - classified as Public", address);
                PeerClassification::Public
            }
            Ok(Err(e)) => {
                // Connection failed - peer is private or unreachable
                debug!("Failed to probe peer {}: {} - classified as Private", address, e);

                // Track specific error types in metrics
                if let Some(ref metrics) = self.metrics {
                    let error_type = match e.kind() {
                        std::io::ErrorKind::ConnectionRefused => "connection_refused",
                        std::io::ErrorKind::TimedOut => "timeout",
                        std::io::ErrorKind::ConnectionReset => "connection_reset",
                        _ => "other",
                    };
                    metrics.record_probe_error(error_type);
                }

                PeerClassification::Private
            }
            Err(_) => {
                // Timeout elapsed - peer is not reachable
                debug!("Probe timeout for peer {} - classified as Private", address);

                if let Some(ref metrics) = self.metrics {
                    metrics.record_probe_error("timeout");
                }

                PeerClassification::Private
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        // Record metrics
        if let Some(ref metrics) = self.metrics {
            metrics.record_probe(classification, duration_ms as f64 / 1000.0);
        }

        Ok((classification, duration_ms))
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

    /// Get current number of active probes
    pub fn active_probes(&self) -> usize {
        self.config.max_concurrent_probes - self.semaphore.available_permits()
    }

    /// Get maximum concurrent probes
    pub fn max_probes(&self) -> usize {
        self.config.max_concurrent_probes
    }
}

impl Clone for ActiveProber {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            semaphore: self.semaphore.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_private_address_detection() {
        let config = ProbingConfig {
            enabled: true,
            timeout_ms: 1000,
            delay_ms: 0,
            max_concurrent_probes: 10,
            skip_private_ips: true,
        };

        let prober = ActiveProber::new(config);

        // Test localhost
        let (classification, _) = prober.probe_peer("127.0.0.1:16111").await.unwrap();
        assert_eq!(classification, PeerClassification::Private);

        // Test private IPv4
        let (classification, _) = prober.probe_peer("192.168.1.1:16111").await.unwrap();
        assert_eq!(classification, PeerClassification::Private);

        // Test private IPv4 (10.x.x.x)
        let (classification, _) = prober.probe_peer("10.0.0.1:16111").await.unwrap();
        assert_eq!(classification, PeerClassification::Private);
    }

    #[test]
    fn test_is_private_address() {
        let config = ProbingConfig {
            enabled: true,
            timeout_ms: 1000,
            delay_ms: 0,
            max_concurrent_probes: 10,
            skip_private_ips: true,
        };

        let prober = ActiveProber::new(config);

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

    #[tokio::test]
    async fn test_rate_limiting() {
        let config = ProbingConfig {
            enabled: true,
            timeout_ms: 1000,
            delay_ms: 0,
            max_concurrent_probes: 2,
            skip_private_ips: false,
        };

        let prober = ActiveProber::new(config);

        // These should succeed (within limit)
        let _permit1 = prober.semaphore.try_acquire().unwrap();
        let _permit2 = prober.semaphore.try_acquire().unwrap();

        // This should fail (over limit)
        assert!(prober.semaphore.try_acquire().is_err());
    }
}
