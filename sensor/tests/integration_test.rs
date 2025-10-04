//! Integration tests for the Kaspa Network Sensor
//!
//! These tests verify end-to-end functionality of the sensor components.

use kaspa_sensor::{
    config::{DatabaseConfig, ExportConfig, MetricsConfig, ProbingConfig, SensorConfig, SensorIdentity, NetworkConfig},
    export::EventExporter,
    metrics::SensorMetrics,
    models::{ConnectionDirection, PeerClassification, PeerConnectionEvent},
    prober::ActiveProber,
    storage::EventStorage,
};
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test configuration with temp directories
fn create_test_config(temp_dir: &TempDir) -> SensorConfig {
    SensorConfig {
        sensor: SensorIdentity {
            sensor_id: "test-sensor-001".to_string(),
            description: Some("Integration test sensor".to_string()),
            environment: Some("test".to_string()),
        },
        network: NetworkConfig {
            listen_address: "127.0.0.1:0".to_string(),  // Random port
            network_type: "mainnet".to_string(),
            max_inbound_connections: 10,
            max_outbound_connections: 5,
            dns_seeders: vec![],
            peers_per_seeder: 0,
        },
        probing: ProbingConfig {
            enabled: true,
            timeout_ms: 1000,
            delay_ms: 10,
            max_concurrent_probes: 5,
            skip_private_ips: true,
        },
        export: ExportConfig {
            enabled: false,  // Disabled for tests
            backend: "http".to_string(),
            endpoint: None,
            api_key: None,
            batch_size: 10,
            interval_secs: 60,
            max_retries: 2,
            retry_backoff_secs: 1,
        },
        database: DatabaseConfig {
            path: temp_dir.path().join("test-events.db"),
            pool_size: 5,
            retention_days: 7,
            enable_wal: false,  // Disable WAL for tests
            addressdb_path: temp_dir.path().join("test-addresses.db"),
        },
        metrics: MetricsConfig {
            enabled: false,  // Disabled for tests
            address: "127.0.0.1:0".to_string(),
        },
    }
}

#[test]
fn test_storage_lifecycle() {
    let temp_dir = TempDir::new().unwrap();
    let config = create_test_config(&temp_dir);

    // Create storage
    let storage = EventStorage::new(&config.database).expect("Failed to create storage");

    // Create a test event
    let event = PeerConnectionEvent::new(
        "test-sensor".to_string(),
        IpAddr::from_str("203.0.113.42").unwrap(),
        16111,
        7,
        "kaspad:0.14.0".to_string(),
        "kaspa-mainnet".to_string(),
        ConnectionDirection::Inbound,
    );

    // Insert event
    let id = storage.insert_event(&event).expect("Failed to insert event");
    assert!(id > 0);

    // Verify event count
    let count = storage.get_event_count().expect("Failed to get count");
    assert_eq!(count, 1);

    // Retrieve unexported events
    let events = storage.get_unexported_events(10).expect("Failed to retrieve events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sensor_id, "test-sensor");
    assert_eq!(events[0].peer_port, 16111);

    // Mark as exported
    let event_ids: Vec<String> = events.iter().map(|e| e.event_id.clone()).collect();
    let updated = storage.mark_events_exported(&event_ids).expect("Failed to mark exported");
    assert_eq!(updated, 1);

    // Verify no more unexported events
    let remaining = storage.get_unexported_events(10).expect("Failed to retrieve events");
    assert_eq!(remaining.len(), 0);

    // Get statistics
    let stats = storage.get_statistics().expect("Failed to get statistics");
    assert_eq!(stats.total_events, 1);
    assert_eq!(stats.exported_events, 1);
    assert_eq!(stats.pending_exports, 0);
}

#[tokio::test]
async fn test_prober_public_classification() {
    let config = ProbingConfig {
        enabled: true,
        timeout_ms: 2000,
        delay_ms: 0,
        max_concurrent_probes: 10,
        skip_private_ips: false,
    };

    let prober = ActiveProber::new(config);

    // Test probing localhost (should be classified as private even if connectable)
    let (classification, duration) = prober.probe_peer("127.0.0.1:80").await.expect("Probe failed");

    // Localhost should be classified as private due to IP address
    assert_eq!(classification, PeerClassification::Private);
    assert!(duration < 3000); // Should complete quickly
}

#[tokio::test]
async fn test_prober_private_classification() {
    let config = ProbingConfig {
        enabled: true,
        timeout_ms: 1000,
        delay_ms: 0,
        max_concurrent_probes: 10,
        skip_private_ips: false,
    };

    let prober = ActiveProber::new(config);

    // Test probing unreachable address
    let (classification, _duration) = prober.probe_peer("192.168.255.254:9999").await.expect("Probe failed");

    assert_eq!(classification, PeerClassification::Private);
}

#[tokio::test]
async fn test_prober_skip_private_ips() {
    let config = ProbingConfig {
        enabled: true,
        timeout_ms: 1000,
        delay_ms: 0,
        max_concurrent_probes: 10,
        skip_private_ips: true,  // Skip private IPs
    };

    let prober = ActiveProber::new(config);

    // Private IP should be skipped and immediately classified as private
    let (classification, duration) = prober.probe_peer("10.0.0.1:16111").await.expect("Probe failed");

    assert_eq!(classification, PeerClassification::Private);
    assert!(duration < 100); // Should be instant
}

#[test]
fn test_metrics_creation_and_recording() {
    let metrics = SensorMetrics::new().expect("Failed to create metrics");

    // Test connection recording
    metrics.record_connection(true, true);
    assert_eq!(metrics.connections_active.get(), 1);
    assert_eq!(metrics.connections_inbound.get(), 1);

    metrics.record_connection(false, true);
    assert_eq!(metrics.connections_active.get(), 2);
    assert_eq!(metrics.connections_outbound.get(), 1);

    // Test probe recording
    metrics.record_probe(PeerClassification::Public, 0.5);
    assert_eq!(metrics.peers_classified_public.get(), 1);

    metrics.record_probe(PeerClassification::Private, 1.0);
    assert_eq!(metrics.peers_classified_private.get(), 1);

    // Test metrics gathering
    let output = metrics.gather();
    assert!(output.contains("sensor_connections_active"));
    assert!(output.contains("sensor_peers_classified_public_total"));
}

#[test]
fn test_config_validation() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = create_test_config(&temp_dir);

    // Valid config should pass
    assert!(config.validate().is_ok());

    // Empty sensor ID should fail
    config.sensor.sensor_id = String::new();
    assert!(config.validate().is_err());

    // Reset sensor ID
    config.sensor.sensor_id = "test".to_string();
    assert!(config.validate().is_ok());

    // Export enabled without endpoint should fail
    config.export.enabled = true;
    config.export.endpoint = None;
    assert!(config.validate().is_err());

    // Export with endpoint should pass
    config.export.endpoint = Some("http://example.com".to_string());
    assert!(config.validate().is_ok());
}

#[test]
fn test_event_with_probe_result() {
    let event = PeerConnectionEvent::new(
        "test-sensor".to_string(),
        IpAddr::from_str("203.0.113.1").unwrap(),
        16111,
        7,
        "kaspad:0.14.0".to_string(),
        "kaspa-mainnet".to_string(),
        ConnectionDirection::Inbound,
    );

    // Initially unknown classification
    assert_eq!(event.classification, PeerClassification::Unknown);
    assert_eq!(event.probe_duration_ms, None);

    // Update with probe result
    let event = event.with_probe_result(PeerClassification::Public, 250, None);

    assert_eq!(event.classification, PeerClassification::Public);
    assert_eq!(event.probe_duration_ms, Some(250));
    assert_eq!(event.probe_error, None);

    // Test with error
    let event = PeerConnectionEvent::new(
        "test-sensor".to_string(),
        IpAddr::from_str("203.0.113.2").unwrap(),
        16111,
        7,
        "kaspad:0.14.0".to_string(),
        "kaspa-mainnet".to_string(),
        ConnectionDirection::Inbound,
    );

    let event = event.with_probe_result(
        PeerClassification::Private,
        0,
        Some("Connection refused".to_string()),
    );

    assert_eq!(event.classification, PeerClassification::Private);
    assert_eq!(event.probe_error, Some("Connection refused".to_string()));
}

#[tokio::test]
async fn test_full_event_pipeline() {
    let temp_dir = TempDir::new().unwrap();
    let config = create_test_config(&temp_dir);

    // Initialize components
    let storage = Arc::new(EventStorage::new(&config.database).expect("Failed to create storage"));
    let metrics = Arc::new(SensorMetrics::new().expect("Failed to create metrics"));
    let prober = ActiveProber::with_metrics(config.probing.clone(), metrics.clone());

    // Simulate an inbound connection
    let peer_ip = IpAddr::from_str("192.168.1.100").unwrap();
    let peer_port = 16111;

    let mut event = PeerConnectionEvent::new(
        config.sensor.sensor_id.clone(),
        peer_ip,
        peer_port,
        7,
        "kaspad:0.14.0".to_string(),
        "kaspa-mainnet".to_string(),
        ConnectionDirection::Inbound,
    );

    // Record connection in metrics
    metrics.record_connection(true, true);

    // Probe the peer
    let peer_addr = format!("{}:{}", peer_ip, peer_port);
    let (classification, duration_ms) = prober.probe_peer(&peer_addr).await.expect("Probe failed");

    // Update event with probe result
    event = event.with_probe_result(classification, duration_ms, None);

    // Store event
    storage.insert_event(&event).expect("Failed to insert event");

    // Verify the full pipeline
    let stats = storage.get_statistics().expect("Failed to get stats");
    assert_eq!(stats.total_events, 1);
    assert_eq!(stats.pending_exports, 1);

    // Classification should be private (private IP)
    assert_eq!(event.classification, PeerClassification::Private);
    assert_eq!(metrics.peers_classified_private.get(), 1);
}
