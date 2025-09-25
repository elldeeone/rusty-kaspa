use kaspa_addresses::{Address, Prefix, Version};
use kaspa_consensus_core::config::params::{Params, DEVNET_PARAMS, MAINNET_PARAMS, SIMNET_PARAMS, TESTNET_PARAMS};
use kaspa_core::info;
use secp256k1::Keypair;
use std::sync::Arc;
use std::time::Duration;

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub struct Config {
    pub rpc_server: String,
    pub target_tps: u64,
    pub keypair: Keypair,
    pub address: Address,
    pub client_pool_size: usize,
    pub channel_capacity: usize,
    pub dashboard_enabled: bool,
    pub dashboard_port: u16,
    pub tick_ms: u64,
    pub confirmation_depth: u64,
    pub utxo_refresh_interval: Duration,
    pub pending_ttl: Duration,
    pub block_interval_ms: u64,
    pub split_parallel: usize,
    pub split_allow_orphan: bool,
    pub chain_only: bool,
    pub chain_outputs_max: usize,
}

impl Config {
    pub async fn from_cli(cli: &crate::Cli) -> Result<Arc<Self>, AnyError> {
        // Generate or parse keypair
        let keypair = if let Some(private_key_hex) = &cli.private_key {
            let mut private_key_bytes = [0u8; 32];
            faster_hex::hex_decode(private_key_hex.as_bytes(), &mut private_key_bytes)?;
            Keypair::from_seckey_slice(secp256k1::SECP256K1, &private_key_bytes)?
        } else {
            // Generate new keypair
            use secp256k1::rand::thread_rng;
            let (sk, _pk) = secp256k1::generate_keypair(&mut thread_rng());
            let kp = Keypair::from_secret_key(secp256k1::SECP256K1, &sk);
            info!("Generated new private key: {}", sk.display_secret());
            info!("SAVE THIS KEY to reuse the same wallet!");
            kp
        };

        // Determine network prefix and consensus params
        let (prefix, params): (Prefix, &'static Params) = match cli.network.to_lowercase().as_str() {
            "mainnet" => (Prefix::Mainnet, &MAINNET_PARAMS),
            "testnet" => (Prefix::Testnet, &TESTNET_PARAMS),
            "devnet" => (Prefix::Devnet, &DEVNET_PARAMS),
            "simnet" => (Prefix::Simnet, &SIMNET_PARAMS),
            _ => {
                return Err(format!("Invalid network: {}. Use mainnet, testnet, devnet, or simnet", cli.network).into());
            }
        };

        // Create address with correct network prefix
        let address = Address::new(prefix, Version::PubKey, &keypair.x_only_public_key().0.serialize());

        info!("Network: {}", cli.network);
        info!("Using address: {}", address);

        // Auto-optimize based on system resources
        let cpu_cores = num_cpus::get();
        let client_pool_size = cli.clients.min(cpu_cores * 4);

        let confirmation_depth = cli.confirmations.max(1);
        let block_interval_ms = params.crescendo.target_time_per_block;
        let auto_refresh_ms = confirmation_depth.saturating_mul(block_interval_ms);
        let utxo_refresh_ms =
            if cli.utxo_refresh_ms > 0 { cli.utxo_refresh_ms } else { auto_refresh_ms.clamp(block_interval_ms.max(100), 2_000) };
        let pending_ttl = Duration::from_secs(cli.pending_ttl.max(1));

        // For maximum TPS, we need aggressive settings
        let config = Config {
            rpc_server: cli.rpc_server.clone(),
            target_tps: cli.tps,
            keypair,
            address,
            client_pool_size,
            channel_capacity: 50000, // Large buffer for bursts
            dashboard_enabled: cli.dashboard,
            dashboard_port: cli.dashboard_port,
            tick_ms: 5, // 5ms tick for better throughput
            confirmation_depth,
            utxo_refresh_interval: Duration::from_millis(utxo_refresh_ms),
            pending_ttl,
            block_interval_ms,
            split_parallel: cli.split_parallel.max(1),
            split_allow_orphan: cli.split_allow_orphan,
            chain_only: cli.chain_only,
            chain_outputs_max: cli.chain_outputs_max.max(1).min(10),
        };

        info!("Configuration:");
        info!("  RPC Server: {}", config.rpc_server);
        info!("  Target TPS: {}", config.target_tps);
        info!("  Client Pool Size: {}", config.client_pool_size);
        info!("  CPU Cores: {}", cpu_cores);
        info!("  Confirmations: {}", config.confirmation_depth);
        info!("  Block Interval (ms): {}", config.block_interval_ms);
        info!("  UTXO Refresh (ms): {}", utxo_refresh_ms);
        info!("  Pending TTL (s): {}", config.pending_ttl.as_secs());
        info!(
            "  Dashboard: {}",
            if config.dashboard_enabled { format!("http://localhost:{}", config.dashboard_port) } else { "disabled".to_string() }
        );

        Ok(Arc::new(config))
    }
}
