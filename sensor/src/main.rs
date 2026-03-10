use kaspa_sensor::{
    config::SensorConfig,
    export::EventExporter,
    metrics::SensorMetrics,
    models::{ClassificationReason, ConnectionDirection, PeerClassification, PeerConnectionEvent},
    prober::ActiveProber,
    storage::EventStorage,
};

use clap::Parser;
use kaspa_addressmanager::AddressManager;
use kaspa_connectionmanager::ConnectionManager;
use kaspa_consensus_core::config::Config as ConsensusConfig;
use kaspa_consensus_core::network::NetworkType;
use kaspa_core::task::tick::TickService;
use kaspa_core::{error, info, warn};
use kaspa_database::prelude::ConnBuilder;
use kaspa_p2p_lib::common::ProtocolError;
use kaspa_p2p_lib::pb::VersionMessage;
use kaspa_p2p_lib::{Adaptor, ConnectionInitializer, Hub, KaspadHandshake, Router};
use kaspa_utils::networking::{ContextualNetAddress, NetAddress};
use kaspa_utils_tower::counters::TowerConnectionCounters;
use parking_lot::Mutex;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::sync::RwLock;
use tokio::time::interval;

#[derive(Parser, Debug)]
#[command(name = "kaspa-sensor")]
#[command(version, about = "Kaspa Network Sensor - Maps network topology and classifies peers")]
struct Args {
    /// Path to configuration file
    #[arg(long, short = 'c')]
    config: Option<PathBuf>,

    /// Sensor ID (overrides config file)
    #[arg(long)]
    sensor_id: Option<String>,

    /// Listen address (overrides config file)
    #[arg(long)]
    listen: Option<String>,

    /// Generate default configuration file
    #[arg(long)]
    generate_config: bool,
}

#[tokio::main]
async fn main() {
    kaspa_core::log::init_logger(None, "INFO");
    let args = Args::parse();

    // Handle config generation
    if args.generate_config {
        let path = PathBuf::from("sensor.toml");
        match SensorConfig::create_default_config_file(&path) {
            Ok(_) => {
                info!("Generated default configuration at {:?}", path);
                return;
            }
            Err(e) => {
                error!("Failed to generate config: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Load configuration
    let mut config = match &args.config {
        Some(path) => match SensorConfig::from_file(path) {
            Ok(cfg) => {
                info!("Loaded configuration from {:?}", path);
                cfg
            }
            Err(e) => {
                error!("Failed to load config: {}", e);
                std::process::exit(1);
            }
        },
        None => {
            info!("No config file specified, using defaults");
            SensorConfig::default()
        }
    };

    // Override with CLI args if provided
    if let Some(sensor_id) = args.sensor_id {
        config.sensor.sensor_id = sensor_id;
    }
    if let Some(listen) = args.listen {
        config.network.listen_address = listen;
    }

    info!("=== Kaspa Network Sensor Starting ===");
    info!("Sensor ID: {}", config.sensor.sensor_id);
    info!("Network: {}", config.network.network_type);
    info!("Listen: {}", config.network.listen_address);

    // Initialize metrics
    let metrics = match SensorMetrics::new() {
        Ok(m) => {
            let m = Arc::new(m);
            if config.metrics.enabled {
                if let Err(e) = m.clone().start_server(config.metrics.clone()).await {
                    error!("Failed to start metrics server: {}", e);
                    std::process::exit(1);
                }
                info!("Metrics server started on {}", config.metrics.address);
            }
            m
        }
        Err(e) => {
            error!("Failed to initialize metrics: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize storage (with PostgreSQL if configured)
    let storage = if config.database.postgres.is_some() {
        info!("PostgreSQL configured, initializing dual storage (SQLite + PostgreSQL)");
        match EventStorage::new_with_postgres(&config.database, &config.sensor).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                error!("Failed to initialize storage with PostgreSQL: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        info!("PostgreSQL not configured, using local SQLite storage only");
        match EventStorage::new(&config.database) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                error!("Failed to initialize storage: {}", e);
                std::process::exit(1);
            }
        }
    };

    // Initialize exporter
    let mut exporter = match EventExporter::new(config.export.clone(), storage.clone()) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to initialize exporter: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = exporter.start().await {
        error!("Failed to start exporter: {}", e);
        std::process::exit(1);
    }

    // Create minimal kaspa components
    let network_type = match config.network.network_type.as_str() {
        "mainnet" => NetworkType::Mainnet,
        "testnet" => NetworkType::Testnet,
        "devnet" => NetworkType::Devnet,
        _ => {
            error!("Invalid network type: {}", config.network.network_type);
            std::process::exit(1);
        }
    };

    let consensus_config = Arc::new(ConsensusConfig::new(network_type.into()));
    let tick_service = Arc::new(TickService::default());

    // Create persistent database for address manager
    let addressdb_path = config.database.addressdb_path.clone();
    info!("Address manager database: {:?}", addressdb_path);
    let db = ConnBuilder::default()
        .with_db_path(addressdb_path)
        .with_files_limit(10)
        .build()
        .expect("Failed to create address manager database");

    // Create address manager
    let (address_manager, _) = AddressManager::new(consensus_config.clone(), db.clone(), tick_service.clone());

    // Create connection initializer
    let initializer = Arc::new(SensorConnectionInitializer::new(
        address_manager.clone(),
        consensus_config.clone(),
        config.clone(),
        storage.clone(),
        metrics.clone(),
    ));

    // Create P2P adaptor
    let hub = Hub::new();
    let listen_addr = match ContextualNetAddress::from_str(&config.network.listen_address) {
        Ok(addr) => addr.normalize(consensus_config.default_p2p_port()),
        Err(e) => {
            error!("Invalid listen address: {}", e);
            std::process::exit(1);
        }
    };

    let adaptor = match Adaptor::bidirectional(listen_addr, hub, initializer.clone(), Arc::new(TowerConnectionCounters::default())) {
        Ok(a) => {
            info!("P2P service started on {}", config.network.listen_address);
            a
        }
        Err(e) => {
            error!("Failed to start P2P service: {}", e);
            std::process::exit(1);
        }
    };

    // Start uptime tracker
    let start_time = Instant::now();
    let metrics_clone = metrics.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(10));
        loop {
            ticker.tick().await;
            metrics_clone.update_uptime(start_time.elapsed().as_secs() as i64);
        }
    });

    // Start storage metrics updater
    let storage_clone = storage.clone();
    let metrics_clone = metrics.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            if let Ok(stats) = storage_clone.get_statistics() {
                metrics_clone.update_storage_metrics(
                    stats.total_events as i64,
                    stats.pending_exports as i64,
                    storage_clone.get_database_size().unwrap_or(0),
                );
            }
        }
    });

    // Start database cleanup task
    let storage_clone = storage.clone();
    let retention_days = config.database.retention_days;
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(86400)); // Daily
        loop {
            ticker.tick().await;
            match storage_clone.cleanup_old_events(retention_days) {
                Ok(deleted) if deleted > 0 => {
                    info!("Cleaned up {} old events", deleted);
                }
                Err(e) => {
                    error!("Failed to cleanup old events: {}", e);
                }
                _ => {}
            }
        }
    });

    // Initialize ConnectionManager for continuous peer discovery
    // This handles DNS seeders, address manager, and automatic connection rotation
    let connection_manager = ConnectionManager::new(
        adaptor.clone(),
        config.network.max_outbound_connections, // outbound target
        config.network.max_inbound_connections,  // inbound limit
        consensus_config.dns_seeders,            // static DNS seeder list
        16111,                                   // default port
        address_manager.clone(),                 // for peer discovery
    );

    info!("Connection manager started - will maintain {} outbound connections", config.network.max_outbound_connections);

    // Wait for shutdown signal
    info!("Sensor running. Press Ctrl+C to shutdown.");
    signal::ctrl_c().await.expect("Failed to install CTRL+C signal handler");
    info!("=== Shutdown signal received ===");

    // Graceful shutdown
    info!("Stopping connection manager...");
    connection_manager.stop().await;

    info!("Closing P2P connections...");
    adaptor.close().await;

    info!("Shutting down exporter...");
    exporter.shutdown().await;

    info!("Shutting down services...");
    tick_service.shutdown();
    drop(db); // Close database connection

    // Final stats
    if let Ok(stats) = storage.get_statistics() {
        info!("=== Final Statistics ===");
        info!("Total events: {}", stats.total_events);
        info!("Public peers: {}", stats.public_peers);
        info!("Private peers: {}", stats.private_peers);
        info!("Pending exports: {}", stats.pending_exports);
    }

    info!("Sensor shutdown complete");
}

/// Connection initializer for the sensor
struct SensorConnectionInitializer {
    address_manager: Arc<Mutex<AddressManager>>,
    consensus_config: Arc<ConsensusConfig>,
    config: SensorConfig,
    storage: Arc<EventStorage>,
    metrics: Arc<SensorMetrics>,
    prober: Arc<RwLock<Option<ActiveProber>>>,
}

impl SensorConnectionInitializer {
    fn new(
        address_manager: Arc<Mutex<AddressManager>>,
        consensus_config: Arc<ConsensusConfig>,
        config: SensorConfig,
        storage: Arc<EventStorage>,
        metrics: Arc<SensorMetrics>,
    ) -> Self {
        let prober =
            if config.probing.enabled { Some(ActiveProber::with_metrics(config.probing.clone(), metrics.clone())) } else { None };

        Self { address_manager, consensus_config, config, storage, metrics, prober: Arc::new(RwLock::new(prober)) }
    }
}

fn resolve_advertised_socket(peer_version: &VersionMessage) -> Result<Option<SocketAddr>, ClassificationReason> {
    let Some(address) = peer_version.address.clone() else {
        return Ok(None);
    };

    let net_address = NetAddress::try_from(address).map_err(|_| ClassificationReason::InvalidAdvertisedAddress)?;
    Ok(Some(net_address.into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InboundProbePlan {
    Probe { advertised_address: String },
    Unknown { advertised_address: Option<String>, reason: ClassificationReason, error: Option<String> },
}

fn plan_inbound_probe(peer_version: &VersionMessage, skip_private_ips: bool) -> InboundProbePlan {
    match resolve_advertised_socket(peer_version) {
        Ok(Some(advertised_socket)) => {
            let advertised_address = advertised_socket.to_string();
            if skip_private_ips && ActiveProber::is_private_address(&advertised_socket) {
                InboundProbePlan::Unknown {
                    advertised_address: Some(advertised_address),
                    reason: ClassificationReason::AdvertisedPrivateAddress,
                    error: None,
                }
            } else {
                InboundProbePlan::Probe { advertised_address }
            }
        }
        Ok(None) => {
            InboundProbePlan::Unknown { advertised_address: None, reason: ClassificationReason::MissingAdvertisedAddress, error: None }
        }
        Err(reason) => InboundProbePlan::Unknown {
            advertised_address: None,
            reason,
            error: Some("failed to parse advertised address".to_string()),
        },
    }
}

#[async_trait::async_trait]
impl ConnectionInitializer for SensorConnectionInitializer {
    async fn initialize_connection(&self, router: Arc<Router>) -> Result<(), ProtocolError> {
        let source_address = router.net_address().to_string();
        let is_outbound = router.is_outbound();

        // Record connection in metrics
        self.metrics.record_connection(!is_outbound, true);

        // Perform handshake
        let mut handshake = KaspadHandshake::new(&router);
        router.start();

        let network_name = self.consensus_config.network_name();
        let local_address = self.address_manager.lock().best_local_address();

        let version_message = VersionMessage {
            protocol_version: 7,
            services: 0,
            timestamp: kaspa_core::time::unix_now() as i64,
            address: local_address.map(|addr| addr.into()),
            id: Vec::from(uuid::Uuid::new_v4().as_bytes()),
            user_agent: format!("kaspa-sensor:{} ({})", env!("CARGO_PKG_VERSION"), self.config.sensor.sensor_id),
            disable_relay_tx: false,
            subnetwork_id: None,
            network: network_name.to_string(),
        };

        let peer_version = handshake.handshake(version_message).await?;
        handshake.exchange_ready_messages().await?;

        info!(
            "Handshake complete with {} - UserAgent: {} (Protocol: {})",
            source_address, peer_version.user_agent, peer_version.protocol_version
        );

        // Launch address gossip flows immediately after ready exchange
        // CRITICAL: Must be done before any other processing to avoid race condition
        // where peer sends RequestAddresses before we've subscribed
        kaspa_sensor::address_flows::launch_address_flows(self.address_manager.clone(), router.clone());

        // Parse peer address
        let (peer_ip, peer_port) = match source_address.parse::<SocketAddr>() {
            Ok(addr) => (addr.ip(), addr.port()),
            Err(_) => {
                warn!("Failed to parse peer address: {}", source_address);
                self.metrics.record_connection_closed();
                return Ok(());
            }
        };

        // Create connection event
        // Convert peer ID from bytes to hex string
        let peer_id = if !peer_version.id.is_empty() { Some(hex::encode(&peer_version.id)) } else { None };

        let event = PeerConnectionEvent::new(
            self.config.sensor.sensor_id.clone(),
            peer_ip,
            peer_port,
            peer_version.protocol_version as u32,
            peer_version.user_agent.clone(),
            self.consensus_config.network_name().to_string(),
            if is_outbound { ConnectionDirection::Outbound } else { ConnectionDirection::Inbound },
        )
        .with_peer_id(peer_id);

        // Perform active probe if enabled and inbound
        if !is_outbound {
            if let Some(ref prober) = *self.prober.read().await {
                let prober_clone = prober.clone();
                let metrics_clone = self.metrics.clone();
                let storage_clone = self.storage.clone();
                match plan_inbound_probe(&peer_version, self.config.probing.skip_private_ips) {
                    InboundProbePlan::Probe { advertised_address } => {
                        if advertised_address != source_address {
                            info!(
                                "Peer source address differs from advertised address: source={}, advertised={}",
                                source_address, advertised_address
                            );
                        }
                        let event_clone = event
                            .clone()
                            .with_advertised_address(Some(advertised_address.clone()))
                            .with_probe_target_address(Some(advertised_address.clone()));

                        tokio::spawn(async move {
                            match prober_clone.probe_peer(&advertised_address).await {
                                Ok((classification, duration_ms, reason)) => {
                                    info!(
                                        "Peer {} classified as {:?} via {} ({}ms)",
                                        event_clone.source_address(),
                                        classification,
                                        advertised_address,
                                        duration_ms
                                    );
                                    metrics_clone.record_classification_reason(reason.as_metric_label());

                                    let updated_event = event_clone.with_probe_result(classification, duration_ms, reason, None);

                                    if let Err(e) = storage_clone.insert_event(&updated_event) {
                                        error!("Failed to store event: {}", e);
                                    }
                                }
                                Err(e) => {
                                    let reason = match e {
                                        kaspa_sensor::prober::ProbeError::RateLimitExceeded => ClassificationReason::ProbeRateLimited,
                                        kaspa_sensor::prober::ProbeError::InvalidAddress(_) => {
                                            ClassificationReason::InvalidAdvertisedAddress
                                        }
                                        _ => ClassificationReason::AdvertisedProbeIoError,
                                    };
                                    warn!("Failed to probe {}: {}", advertised_address, e);
                                    metrics_clone.record_probe_error("probe_failed");
                                    metrics_clone.record_classification_reason(reason.as_metric_label());
                                    metrics_clone.record_unknown_classification();

                                    let updated_event = event_clone.with_classification(
                                        PeerClassification::Unknown,
                                        reason,
                                        None,
                                        Some(e.to_string()),
                                    );

                                    if let Err(e) = storage_clone.insert_event(&updated_event) {
                                        error!("Failed to store event: {}", e);
                                    }
                                }
                            }
                        });
                    }
                    InboundProbePlan::Unknown { advertised_address, reason, error } => {
                        let updated_event = event.clone().with_advertised_address(advertised_address).with_classification(
                            PeerClassification::Unknown,
                            reason,
                            None,
                            error,
                        );
                        self.metrics.record_classification_reason(reason.as_metric_label());
                        self.metrics.record_unknown_classification();
                        if let Err(e) = self.storage.insert_event(&updated_event) {
                            error!("Failed to store event: {}", e);
                        }
                    }
                }
            } else {
                // Store event without probing
                if let Err(e) = self.storage.insert_event(&event) {
                    error!("Failed to store event: {}", e);
                }
            }
        } else {
            // Outbound connection - no probing needed
            if let Err(e) = self.storage.insert_event(&event) {
                error!("Failed to store event: {}", e);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_utils::networking::IpAddress;
    use std::net::Ipv4Addr;

    fn version_message_with_address(address: Option<kaspa_p2p_lib::pb::NetAddress>) -> VersionMessage {
        VersionMessage {
            protocol_version: 7,
            services: 0,
            timestamp: 0,
            address,
            id: vec![],
            user_agent: "/kaspad:test/".to_string(),
            disable_relay_tx: false,
            subnetwork_id: None,
            network: "mainnet".to_string(),
        }
    }

    #[test]
    fn inbound_probe_plan_uses_public_advertised_address() {
        let address = NetAddress::new(IpAddress::from(Ipv4Addr::new(203, 0, 113, 9)), 16111);
        let peer_version = version_message_with_address(Some(address.into()));

        let plan = plan_inbound_probe(&peer_version, true);

        assert_eq!(plan, InboundProbePlan::Probe { advertised_address: "203.0.113.9:16111".to_string() });
    }

    #[test]
    fn inbound_probe_plan_marks_missing_advertised_as_unknown() {
        let peer_version = version_message_with_address(None);

        let plan = plan_inbound_probe(&peer_version, true);

        assert_eq!(
            plan,
            InboundProbePlan::Unknown {
                advertised_address: None,
                reason: ClassificationReason::MissingAdvertisedAddress,
                error: None,
            }
        );
    }

    #[test]
    fn inbound_probe_plan_marks_private_advertised_as_unknown() {
        let address = NetAddress::new(IpAddress::from(Ipv4Addr::new(10, 0, 0, 5)), 16111);
        let peer_version = version_message_with_address(Some(address.into()));

        let plan = plan_inbound_probe(&peer_version, true);

        assert_eq!(
            plan,
            InboundProbePlan::Unknown {
                advertised_address: Some("10.0.0.5:16111".to_string()),
                reason: ClassificationReason::AdvertisedPrivateAddress,
                error: None,
            }
        );
    }

    #[test]
    fn inbound_probe_plan_marks_invalid_advertised_as_unknown() {
        let peer_version =
            version_message_with_address(Some(kaspa_p2p_lib::pb::NetAddress { timestamp: 0, ip: vec![1, 2, 3], port: 16111 }));

        let plan = plan_inbound_probe(&peer_version, true);

        assert_eq!(
            plan,
            InboundProbePlan::Unknown {
                advertised_address: None,
                reason: ClassificationReason::InvalidAdvertisedAddress,
                error: Some("failed to parse advertised address".to_string()),
            }
        );
    }

    #[test]
    fn inbound_probe_plan_allows_private_advertised_when_private_ips_are_enabled() {
        let address = NetAddress::new(IpAddress::from(Ipv4Addr::new(10, 0, 0, 5)), 16111);
        let peer_version = version_message_with_address(Some(address.into()));

        let plan = plan_inbound_probe(&peer_version, false);

        assert_eq!(plan, InboundProbePlan::Probe { advertised_address: "10.0.0.5:16111".to_string() });
    }
}
