// Kaspa Network Sensor - Lightweight P2P network topology mapper
//
// This library provides all the core components for the Kaspa network sensor,
// which is designed to map the topology of the Kaspa P2P network by:
// 1. Listening for inbound peer connections
// 2. Performing P2P handshakes
// 3. Actively probing peers to classify them as Public or Private
// 4. Logging all interactions to local storage
// 5. Exporting data to centralized services (Firebase, HTTP, etc.)

pub mod address_flows;
pub mod config;
pub mod export;
pub mod metrics;
pub mod models;
pub mod postgres;
pub mod prober;
pub mod storage;

// Re-export commonly used types
pub use config::{SensorConfig, ProbingConfig, ExportConfig, DatabaseConfig, MetricsConfig, PostgresConfig};
pub use export::EventExporter;
pub use metrics::SensorMetrics;
pub use models::{PeerConnectionEvent, PeerClassification, ConnectionDirection, EventBatch};
pub use postgres::{PostgresWriter, PeerEvent};
pub use prober::{ActiveProber, ProbeError};
pub use storage::{EventStorage, StorageStatistics};
