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

#[derive(Default)]
pub struct ConfigOverrides {
    pub rpc_server: Option<String>,
    pub target_tps: Option<u64>,
    pub keypair: Option<Keypair>,
    pub address: Option<Address>,
    pub client_pool_size: Option<usize>,
    pub dashboard_enabled: Option<bool>,
    pub dashboard_port: Option<u16>,
    pub confirmation_depth: Option<u64>,
    pub utxo_refresh_ms: Option<u64>,
    pub pending_ttl_secs: Option<u64>,
    pub split_parallel: Option<usize>,
    pub split_allow_orphan: Option<bool>,
    pub chain_only: Option<bool>,
    pub chain_outputs_max: Option<usize>,
}

impl Config {
    pub async fn from_cli(cli: &crate::Cli) -> Result<Arc<Self>, AnyError> {
        // Env overrides for server deployments / .env files
        let rpc_server_env = std::env::var("RPC_SERVER").ok();
        let network_env = std::env::var("NETWORK").ok();
        // Generate or parse keypair
        // Allow PRIVATE_KEY env override (dev convenience)
        let env_priv = std::env::var("PRIVATE_KEY").ok();
        let keypair = if let Some(private_key_hex) = env_priv.as_ref().or(cli.private_key.as_ref()) {
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

        // Determine network prefix and consensus params (allow NETWORK env override)
        // Allow ADDRESS env override (dev convenience)
        let network_str = network_env.as_deref().unwrap_or(&cli.network);
        let (prefix, params): (Prefix, &'static Params) = match network_str.to_lowercase().as_str() {
            "mainnet" => (Prefix::Mainnet, &MAINNET_PARAMS),
            "testnet" => (Prefix::Testnet, &TESTNET_PARAMS),
            "devnet" => (Prefix::Devnet, &DEVNET_PARAMS),
            "simnet" => (Prefix::Simnet, &SIMNET_PARAMS),
            _ => {
                return Err(format!("Invalid network: {}. Use mainnet, testnet, devnet, or simnet", network_str).into());
            }
        };

        // Create address with correct network prefix, unless ADDRESS env provided
        let address = if let Ok(addr_str) = std::env::var("ADDRESS") {
            Address::try_from(addr_str.as_str()).unwrap_or_else(|_| Address::new(prefix, Version::PubKey, &keypair.x_only_public_key().0.serialize()))
        } else {
            Address::new(prefix, Version::PubKey, &keypair.x_only_public_key().0.serialize())
        };

        info!("Network: {}", network_str);
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
            rpc_server: rpc_server_env.unwrap_or_else(|| cli.rpc_server.clone()),
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

    pub fn clone_with_overrides(&self, o: ConfigOverrides) -> Arc<Self> {
        let mut cfg = self.clone_inner();
        if let Some(v) = o.rpc_server { cfg.rpc_server = v; }
        if let Some(v) = o.target_tps { cfg.target_tps = v; }
        if let Some(v) = o.keypair { cfg.keypair = v; }
        if let Some(v) = o.address { cfg.address = v; }
        if let Some(v) = o.client_pool_size { cfg.client_pool_size = v; }
        if let Some(v) = o.dashboard_enabled { cfg.dashboard_enabled = v; }
        if let Some(v) = o.dashboard_port { cfg.dashboard_port = v; }
        if let Some(v) = o.confirmation_depth { cfg.confirmation_depth = v.max(1); }
        if let Some(v) = o.utxo_refresh_ms { cfg.utxo_refresh_interval = Duration::from_millis(v); }
        if let Some(v) = o.pending_ttl_secs { cfg.pending_ttl = Duration::from_secs(v.max(1)); }
        if let Some(v) = o.split_parallel { cfg.split_parallel = v.max(1); }
        if let Some(v) = o.split_allow_orphan { cfg.split_allow_orphan = v; }
        if let Some(v) = o.chain_only { cfg.chain_only = v; }
        if let Some(v) = o.chain_outputs_max { cfg.chain_outputs_max = v.max(1).min(10); }
        Arc::new(cfg)
    }

    fn clone_inner(&self) -> Self {
        Self {
            rpc_server: self.rpc_server.clone(),
            target_tps: self.target_tps,
            keypair: self.keypair,
            address: self.address.clone(),
            client_pool_size: self.client_pool_size,
            channel_capacity: self.channel_capacity,
            dashboard_enabled: self.dashboard_enabled,
            dashboard_port: self.dashboard_port,
            tick_ms: self.tick_ms,
            confirmation_depth: self.confirmation_depth,
            utxo_refresh_interval: self.utxo_refresh_interval,
            pending_ttl: self.pending_ttl,
            block_interval_ms: self.block_interval_ms,
            split_parallel: self.split_parallel,
            split_allow_orphan: self.split_allow_orphan,
            chain_only: self.chain_only,
            chain_outputs_max: self.chain_outputs_max,
        }
    }
}
