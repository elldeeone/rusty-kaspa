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
        peer_id: Option<String>,
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
            peer_id,
            network,
            direction,
            classification: PeerClassification::Unknown,
            probe_duration_ms: None,
            probe_error: None,
        }
    }

    /// Update with probe result
    pub fn with_probe_result(mut self, classification: PeerClassification, duration_ms: u64, error: Option<String>) -> Self {
        self.classification = classification;
        self.probe_duration_ms = Some(duration_ms);
        self.probe_error = error;
        self
    }

    /// Get peer address as string
    pub fn peer_address(&self) -> String {
        format!("{}:{}", self.peer_ip, self.peer_port)
    }
}

impl EventBatch {
    /// Create a new event batch
    pub fn new(sensor_id: String, events: Vec<PeerConnectionEvent>) -> Self {
        let count = events.len();
        Self {
            sensor_id,
            batch_timestamp: Utc::now(),
            events,
            count,
        }
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
            None,
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        assert_eq!(event.sensor_id, "sensor-001");
        assert_eq!(event.peer_port, 16111);
        assert_eq!(event.classification, PeerClassification::Unknown);
        assert_eq!(event.peer_address(), "203.0.113.1:16111");
    }

    #[test]
    fn test_probe_result_integration() {
        let event = PeerConnectionEvent::new(
            "sensor-001".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            None,
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        let event = event.with_probe_result(PeerClassification::Public, 250, None);

        assert_eq!(event.classification, PeerClassification::Public);
        assert_eq!(event.probe_duration_ms, Some(250));
        assert_eq!(event.probe_error, None);
    }

    #[test]
    fn test_event_batch() {
        let events = vec![
            PeerConnectionEvent::new(
                "sensor-001".to_string(),
                IpAddr::from_str("203.0.113.1").unwrap(),
                16111,
                7,
                "kaspad:0.14.0".to_string(),
                None,
                "kaspa-mainnet".to_string(),
                ConnectionDirection::Inbound,
            ),
        ];

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
            None,
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: PeerConnectionEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(event.sensor_id, deserialized.sensor_id);
        assert_eq!(event.peer_ip, deserialized.peer_ip);
    }
}
