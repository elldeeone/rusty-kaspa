mod prober;

use prober::ActiveProber;
use clap::Parser;
use kaspa_addressmanager::AddressManager;
use kaspa_consensus_core::config::Config as ConsensusConfig;
use kaspa_consensus_core::network::NetworkType;
use kaspa_core::task::tick::TickService;
use kaspa_core::{debug, info};
use kaspa_database::create_temp_db;
use kaspa_database::prelude::ConnBuilder;
use kaspa_p2p_lib::common::ProtocolError;
use kaspa_p2p_lib::{Adaptor, ConnectionHandler, ConnectionInitializer, Hub, Router};
use kaspa_protocol_flows::handshake::KaspadHandshake;
use kaspa_utils::networking::ContextualNetAddress;
use kaspa_utils_tower::counters::TowerConnectionCounters;
use parking_lot::Mutex;
use std::sync::Arc;
use std::str::FromStr;
use tokio::signal;

#[derive(Parser, Debug)]
#[command(name = "kaspa-sensor")]
#[command(version, about = "Kaspa Network Sensor - Maps network topology")]
struct Args {
    #[arg(long, default_value = "sensor")]
    sensor_id: String,

    #[arg(long, default_value = "0.0.0.0:16111")]
    listen: String,

    #[arg(long, default_value = "true")]
    enable_probing: bool,
}

#[tokio::main]
async fn main() {
    kaspa_core::log::init_logger(None, "INFO");
    let args = Args::parse();

    info!("Starting Kaspa Network Sensor - ID: {}", args.sensor_id);

    // Create minimal components
    let config = Arc::new(ConsensusConfig::new(NetworkType::Mainnet));
    let tick_service = Arc::new(TickService::default());
    let (db_lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));

    // Create address manager
    let (address_manager, _) = AddressManager::new(config.clone(), db.clone(), tick_service.clone());

    // Create hub
    let hub = Arc::new(Hub::new());

    // Create connection handler with our initializer
    let initializer = Arc::new(SensorConnectionInitializer {
        address_manager: address_manager.clone(),
        config: config.clone(),
        sensor_id: args.sensor_id.clone(),
        prober: ActiveProber::new(5000),
        enable_probing: args.enable_probing,
    });

    // Create connection handler (handles P2P connections)
    let handler = Arc::new(ConnectionHandler::new(
        hub.clone(),
        TowerConnectionCounters::default(),
        initializer,
    ));

    // Start listening for connections
    let listen_addr = ContextualNetAddress::from_str(&args.listen).unwrap();
    handler.start_listening(listen_addr, None).await;
    info!("P2P service started on {}", args.listen);

    // Connect to some initial peers from DNS
    tokio::spawn(async move {
        let dns_seeders = vec![
            "seeder1.kaspad.net",
            "seeder2.kaspad.net",
            "seeder3.kaspad.net",
        ];

        for seeder in dns_seeders {
            info!("Querying DNS seeder {}", seeder);
            match tokio::net::lookup_host((seeder, 16111)).await {
                Ok(addrs) => {
                    for addr in addrs.take(5) {
                        let peer_addr = addr.to_string();
                        info!("Connecting to {}", peer_addr);
                        if let Err(e) = handler.connect_peer(peer_addr).await {
                            debug!("Failed to connect: {}", e);
                        }
                    }
                }
                Err(e) => {
                    debug!("DNS lookup failed for {}: {}", seeder, e);
                }
            }
        }
    });

    // Wait for shutdown
    signal::ctrl_c().await.expect("Failed to install CTRL+C signal handler");
    info!("Shutting down...");

    handler.terminate_all_peers().await;
    tick_service.shutdown();
    drop(db_lifetime);
}

// Minimal connection initializer with probe-back
struct SensorConnectionInitializer {
    address_manager: Arc<Mutex<AddressManager>>,
    config: Arc<ConsensusConfig>,
    sensor_id: String,
    prober: ActiveProber,
    enable_probing: bool,
}

#[async_trait::async_trait]
impl ConnectionInitializer for SensorConnectionInitializer {
    async fn initialize_connection(&self, router: Arc<Router>) -> Result<(), ProtocolError> {
        // Use kaspad's EXACT handshake
        let mut handshake = KaspadHandshake::new(&router);
        router.start();

        // Build version message
        let network_name = self.config.network_name();
        let local_address = self.address_manager.lock().best_local_address();

        let mut version = kaspa_p2p_lib::Version::new(
            local_address,
            uuid::Uuid::new_v4(),
            network_name.clone(),
            None,
            7, // Protocol version
        );
        version.add_user_agent("kaspa-sensor", env!("CARGO_PKG_VERSION"), &[&self.sensor_id]);

        // Perform handshake
        let peer_version = handshake.handshake(version, network_name.clone()).await?;
        handshake.exchange_ready_messages().await?;

        let peer_address = router.identity().to_string();
        info!("Connected to peer: {} - UserAgent: {}", peer_address, peer_version.user_agent);

        // SURGICAL ADDITION: Probe-back classification
        if self.enable_probing {
            let prober = self.prober.clone();
            let peer_address_clone = peer_address.clone();
            tokio::spawn(async move {
                match prober.probe_peer(&peer_address_clone).await {
                    Ok(classification) => {
                        info!("Peer {} classified as {:?}", peer_address_clone, classification);
                    }
                    Err(e) => {
                        debug!("Failed to probe {}: {}", peer_address_clone, e);
                    }
                }
            });
        }

        // Register minimal flows for gossip
        use kaspa_p2p_lib::{KaspadMessagePayloadType, make_message};
        use kaspa_p2p_lib::pb::{kaspad_message::Payload, AddressesMessage};

        let incoming_route = router.subscribe(vec![
            KaspadMessagePayloadType::RequestAddresses,
            KaspadMessagePayloadType::Addresses,
        ]);

        let router_clone = router.clone();
        let address_manager = self.address_manager.clone();
        tokio::spawn(async move {
            let mut incoming = incoming_route;
            while let Some(msg) = incoming.recv().await {
                match msg.payload {
                    Some(Payload::RequestAddresses(_)) => {
                        let addresses = address_manager.lock()
                            .iterate_addresses()
                            .take(1000)
                            .map(|addr| (addr.ip, addr.port).into())
                            .collect();

                        let _ = router_clone.enqueue(make_message!(
                            Payload::Addresses,
                            AddressesMessage { address_list: addresses }
                        )).await;
                    }
                    Some(Payload::Addresses(msg)) => {
                        let mut am = address_manager.lock();
                        for addr in msg.address_list {
                            if let Ok(net_addr) = addr.try_into() {
                                am.add_address(net_addr);
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }
}