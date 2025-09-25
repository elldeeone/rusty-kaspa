mod config;
mod engine;
mod metrics;

use clap::{Parser, Subcommand};
use kaspa_core::{info, kaspad_env::version};
use std::sync::Arc;
use tokio::signal;

#[derive(Parser)]
#[command(name = "kasbench")]
#[command(about = "High-performance Kaspa network benchmarking platform", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// RPC server address
    #[arg(short = 's', long, default_value = "grpc://10.0.4.30:16110")]
    rpc_server: String,

    /// Network type (mainnet, testnet, devnet, simnet)
    #[arg(short = 'n', long, default_value = "mainnet")]
    network: String,

    /// Target transactions per second
    #[arg(short = 't', long, default_value_t = 1000)]
    tps: u64,

    /// Private key in hex format (will generate if not provided)
    #[arg(short = 'k', long)]
    private_key: Option<String>,

    /// Number of client connections to use
    #[arg(short = 'c', long, default_value_t = 16)]
    clients: usize,

    /// Enable web dashboard
    #[arg(long, default_value_t = false)]
    dashboard: bool,

    /// Dashboard port
    #[arg(long, default_value_t = 8080)]
    dashboard_port: u16,

    /// Required confirmations before reusing UTXOs
    #[arg(long, default_value_t = 5)]
    confirmations: u64,

    /// Override UTXO refresh interval in milliseconds (0 = auto)
    #[arg(long, default_value_t = 0)]
    utxo_refresh_ms: u64,

    /// Seconds to hold an input reservation before recycling it
    #[arg(long, default_value_t = 30)]
    pending_ttl: u64,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the benchmark
    Run {
        /// Duration in seconds (0 for unlimited)
        #[arg(short = 'd', long, default_value_t = 300)]
        duration: u64,
    },
    /// Prepare UTXOs by splitting
    Prepare {
        /// Number of UTXOs to create
        #[arg(short = 'n', long, default_value_t = 10000)]
        utxo_count: usize,
    },
    /// Sweep all UTXOs to consolidate
    Sweep,
    /// Send funds to an address
    Send {
        /// Destination address
        #[arg(short = 'a', long)]
        address: String,
        /// Amount in KAS
        #[arg(short = 'm', long)]
        amount: f64,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).format_timestamp_millis().init();

    let cli = Cli::parse();

    info!("Kasbench v{} - Kaspa Performance Benchmarking Platform", version());
    info!("===============================================");

    // Create configuration
    let config = config::Config::from_cli(&cli).await?;

    match &cli.command {
        Some(Commands::Run { duration }) => {
            info!("Starting benchmark run for {} seconds", if *duration == 0 { u64::MAX } else { *duration });
            run_benchmark(config, *duration).await?;
        }
        Some(Commands::Prepare { utxo_count }) => {
            info!("Preparing {} UTXOs for benchmarking", utxo_count);
            prepare_utxos(config, *utxo_count).await?;
        }
        Some(Commands::Sweep) => {
            info!("Sweeping all UTXOs to consolidate");
            sweep_utxos(config).await?;
        }
        Some(Commands::Send { address, amount }) => {
            info!("Sending {} KAS to {}", amount, address);
            send_funds(config, address.clone(), *amount).await?;
        }
        None => {
            // Default: run benchmark
            info!("Starting benchmark run (default 300 seconds)");
            run_benchmark(config, 300).await?;
        }
    }

    Ok(())
}

async fn run_benchmark(config: Arc<config::Config>, duration: u64) -> Result<(), Box<dyn std::error::Error>> {
    let engine = engine::BenchmarkEngine::new(config.clone()).await?;

    // Start metrics server if dashboard is enabled
    if config.dashboard_enabled {
        let metrics = engine.metrics_collector();
        tokio::spawn(async move {
            if let Err(e) = metrics::start_metrics_server(config.dashboard_port, metrics).await {
                kaspa_core::warn!("Failed to start metrics server: {}", e);
            }
        });
    }

    // Start the engine
    let engine_handle = tokio::spawn(async move {
        if let Err(e) = engine.run(duration).await {
            kaspa_core::error!("Engine error: {}", e);
        }
    });

    // Wait for Ctrl+C or engine completion
    tokio::select! {
        result = engine_handle => {
            match result {
                Ok(_) => info!("Benchmark completed successfully"),
                Err(e) => kaspa_core::error!("Task error: {}", e),
            }
        }
        _ = signal::ctrl_c() => {
            info!("Shutting down...");
        }
    }

    Ok(())
}

async fn prepare_utxos(config: Arc<config::Config>, utxo_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    let engine = engine::BenchmarkEngine::new(config).await?;
    engine.prepare_utxos(utxo_count, None).await?;
    Ok(())
}

async fn sweep_utxos(config: Arc<config::Config>) -> Result<(), Box<dyn std::error::Error>> {
    let engine = engine::BenchmarkEngine::new(config).await?;
    engine.sweep_all_utxos().await?;
    Ok(())
}

async fn send_funds(config: Arc<config::Config>, address: String, amount: f64) -> Result<(), Box<dyn std::error::Error>> {
    let engine = engine::BenchmarkEngine::new(config).await?;
    engine.send_funds(address, amount).await?;
    Ok(())
}
