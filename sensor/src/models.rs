use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Classification of a peer based on active probing
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PeerClassification {
    /// Peer is publicly accessible (listening on advertised port)
    Public,
    /// Peer is not publicly accessible (behind NAT/firewall)
    Private,
    /// Classification pending or probe not yet performed
    Unknown,
}

/// Why a peer ended up with its final classification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationReason {
    AdvertisedProbeSuccess,
    AdvertisedProbeTimeout,
    AdvertisedProbeRefused,
    AdvertisedProbeIoError,
    MissingAdvertisedAddress,
    AdvertisedPrivateAddress,
    InvalidAdvertisedAddress,
    ProbeRateLimited,
}

impl ClassificationReason {
    pub fn as_metric_label(&self) -> &'static str {
        match self {
            Self::AdvertisedProbeSuccess => "advertised_probe_success",
            Self::AdvertisedProbeTimeout => "advertised_probe_timeout",
            Self::AdvertisedProbeRefused => "advertised_probe_refused",
            Self::AdvertisedProbeIoError => "advertised_probe_io_error",
            Self::MissingAdvertisedAddress => "missing_advertised_address",
            Self::AdvertisedPrivateAddress => "advertised_private_address",
            Self::InvalidAdvertisedAddress => "invalid_advertised_address",
            Self::ProbeRateLimited => "probe_rate_limited",
        }
    }
}

/// A complete peer connection event with all metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConnectionEvent {
    /// Unique event ID
    pub event_id: String,

    /// Timestamp when the connection was established (UTC)
    pub timestamp: DateTime<Utc>,

    /// Sensor instance identifier
    pub sensor_id: String,

    /// Peer's IP address
    pub peer_ip: IpAddr,

    /// Peer's port
    pub peer_port: u16,

    /// Peer's protocol version
    pub protocol_version: u32,

    /// Peer's user agent string
    pub user_agent: String,

    /// Peer's unique ID (from handshake, hex-encoded)
    pub peer_id: Option<String>,

    /// Address the peer advertised in the Kaspa handshake, if any
    pub advertised_address: Option<String>,

    /// Address the sensor actively probed to classify the peer, if any
    pub probe_target_address: Option<String>,

    /// Peer's advertised network
    pub network: String,

    /// Direction of connection (inbound/outbound)
    pub direction: ConnectionDirection,

    /// Peer classification result
    pub classification: PeerClassification,

    /// Time taken to probe peer (milliseconds), if applicable
    pub probe_duration_ms: Option<u64>,

    /// Any error encountered during probing
    pub probe_error: Option<String>,

    /// Why the peer received its final classification
    pub classification_reason: Option<ClassificationReason>,
}

/// Direction of peer connection
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionDirection {
    /// Peer connected to us
    Inbound,
    /// We connected to peer
    Outbound,
}

/// Probe result for a single peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    /// Timestamp when probe was performed
    pub timestamp: DateTime<Utc>,

    /// Peer address that was probed
    pub peer_address: String,

    /// Result of the probe
    pub classification: PeerClassification,

    /// Duration of probe attempt (milliseconds)
    pub duration_ms: u64,

    /// Error message if probe failed
    pub error: Option<String>,

    /// Why this probe landed on its final result
    pub reason: ClassificationReason,
}

/// Batch of events to be exported
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBatch {
    /// Sensor ID for this batch
    pub sensor_id: String,

    /// Batch creation timestamp
    pub batch_timestamp: DateTime<Utc>,

    /// Connection events in this batch
    pub events: Vec<PeerConnectionEvent>,

    /// Number of events in batch
    pub count: usize,
}

impl PeerConnectionEvent {
    /// Create a new peer connection event
    pub fn new(
        sensor_id: String,
        peer_ip: IpAddr,
        peer_port: u16,
        protocol_version: u32,
        user_agent: String,
        network: String,
        direction: ConnectionDirection,
    ) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            sensor_id,
            peer_ip,
            peer_port,
            protocol_version,
            user_agent,
            peer_id: None,
            advertised_address: None,
            probe_target_address: None,
            network,
            direction,
            classification: PeerClassification::Unknown,
            probe_duration_ms: None,
            probe_error: None,
            classification_reason: None,
        }
    }

    /// Attach the peer ID captured from the handshake.
    pub fn with_peer_id(mut self, peer_id: Option<String>) -> Self {
        self.peer_id = peer_id;
        self
    }

    /// Attach the peer's advertised Kaspa address from the handshake.
    pub fn with_advertised_address(mut self, advertised_address: Option<String>) -> Self {
        self.advertised_address = advertised_address;
        self
    }

    /// Record the address the sensor actually used for the reverse probe.
    pub fn with_probe_target_address(mut self, probe_target_address: Option<String>) -> Self {
        self.probe_target_address = probe_target_address;
        self
    }

    /// Update with a final classification.
    pub fn with_classification(
        mut self,
        classification: PeerClassification,
        reason: ClassificationReason,
        duration_ms: Option<u64>,
        error: Option<String>,
    ) -> Self {
        self.classification = classification;
        self.classification_reason = Some(reason);
        self.probe_duration_ms = duration_ms;
        self.probe_error = error;
        self
    }

    /// Update with probe result
    pub fn with_probe_result(
        self,
        classification: PeerClassification,
        duration_ms: u64,
        reason: ClassificationReason,
        error: Option<String>,
    ) -> Self {
        self.with_classification(classification, reason, Some(duration_ms), error)
    }

    /// Get source address as string.
    pub fn source_address(&self) -> String {
        format!("{}:{}", self.peer_ip, self.peer_port)
    }

    /// Get peer address as string
    pub fn peer_address(&self) -> String {
        self.source_address()
    }
}

impl EventBatch {
    /// Create a new event batch
    pub fn new(sensor_id: String, events: Vec<PeerConnectionEvent>) -> Self {
        let count = events.len();
        Self { sensor_id, batch_timestamp: Utc::now(), events, count }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_peer_connection_event_creation() {
        let event = PeerConnectionEvent::new(
            "sensor-001".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        assert_eq!(event.sensor_id, "sensor-001");
        assert_eq!(event.peer_port, 16111);
        assert_eq!(event.classification, PeerClassification::Unknown);
        assert_eq!(event.source_address(), "203.0.113.1:16111");
        assert_eq!(event.advertised_address, None);
    }

    #[test]
    fn test_probe_result_integration() {
        let event = PeerConnectionEvent::new(
            "sensor-001".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        )
        .with_peer_id(Some("peer-123".to_string()))
        .with_advertised_address(Some("203.0.113.1:16111".to_string()))
        .with_probe_target_address(Some("203.0.113.1:16111".to_string()));

        let event = event.with_probe_result(PeerClassification::Public, 250, ClassificationReason::AdvertisedProbeSuccess, None);

        assert_eq!(event.classification, PeerClassification::Public);
        assert_eq!(event.probe_duration_ms, Some(250));
        assert_eq!(event.probe_error, None);
        assert_eq!(event.classification_reason, Some(ClassificationReason::AdvertisedProbeSuccess));
        assert_eq!(event.peer_id.as_deref(), Some("peer-123"));
    }

    #[test]
    fn test_event_batch() {
        let events = vec![PeerConnectionEvent::new(
            "sensor-001".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        )];

        let batch = EventBatch::new("sensor-001".to_string(), events);
        assert_eq!(batch.count, 1);
        assert_eq!(batch.events.len(), 1);
    }

    #[test]
    fn test_serialization() {
        let event = PeerConnectionEvent::new(
            "sensor-001".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: PeerConnectionEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(event.sensor_id, deserialized.sensor_id);
        assert_eq!(event.peer_ip, deserialized.peer_ip);
    }
}
