mod config;
mod engine;
mod metrics;

use clap::{Parser, Subcommand};
use kaspa_core::{info, kaspad_env::version};
use warp::Filter;
use kaspa_addresses::{Address, Version};
use std::str::FromStr;
use secp256k1::{Keypair, SecretKey, SECP256K1};
use std::sync::Arc;
use tokio::signal;

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;
type TaskHandle = Arc<parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>>;

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

    /// Max parallel split transactions during UTXO preparation
    #[arg(long, default_value_t = 64)]
    split_parallel: usize,

    /// Allow orphan for split submissions (spend unconfirmed change)
    #[arg(long, default_value_t = false)]
    split_allow_orphan: bool,

    /// Chain-only splitting (disable wave planning and only run chains)
    #[arg(long, default_value_t = false)]
    chain_only: bool,

    /// Max outputs per chain transaction (excluding change)
    #[arg(long, default_value_t = 2)]
    chain_outputs_max: usize,
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
    /// Run HTTP server (UI + API) for local network use
    Serve {
        /// Bind address (default 0.0.0.0)
        #[arg(long, default_value = "0.0.0.0")]
        bind: String,
        /// Port (default 8080)
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Disable LAN guard (not recommended)
        #[arg(long, default_value_t = false)]
        disable_lan_guard: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).format_timestamp_millis().init();

    // Dev convenience: load .env from common locations (KEY=VALUE per line)
    fn load_env_file(path: &str) {
        if let Ok(dotenv) = std::fs::read_to_string(path) {
            for line in dotenv.lines() {
                let t = line.trim();
                if t.is_empty() || t.starts_with('#') { continue; }
                if let Some((k, v)) = t.split_once('=') { std::env::set_var(k.trim(), v.trim()); }
            }
        }
    }
    if let Ok(custom_env) = std::env::var("KASBENCH_ENV_PATH") { load_env_file(&custom_env); }
    load_env_file(".env");
    load_env_file("kasbench/.env");
    load_env_file("/data/.env");

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
        Some(Commands::Serve { bind, port, disable_lan_guard }) => {
            let ui_enabled = std::env::var("KASBENCH_ENABLE_UI").ok().map(|v| v=="1" || v.to_lowercase()=="true").unwrap_or(false);
            if !ui_enabled {
                kaspa_core::warn!("UI server is disabled. Set KASBENCH_ENABLE_UI=1 to enable.");
                return Ok(());
            }
            info!("Starting server mode on {}:{} (LAN-only: {})", bind, port, !disable_lan_guard);
            serve_api_ui(config, bind.clone(), *port, !disable_lan_guard).await?;
        }
        None => {
            // Default: run benchmark
            info!("Starting benchmark run (default 300 seconds)");
            run_benchmark(config, 300).await?;
        }
    }

    Ok(())
}

async fn run_benchmark(config: Arc<config::Config>, duration: u64) -> Result<(), AnyError> {
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

async fn prepare_utxos(config: Arc<config::Config>, utxo_count: usize) -> Result<(), AnyError> {
    let engine = engine::BenchmarkEngine::new(config).await?;
    engine.prepare_utxos(utxo_count, None).await?;
    Ok(())
}

async fn sweep_utxos(config: Arc<config::Config>) -> Result<(), AnyError> {
    let engine = engine::BenchmarkEngine::new(config).await?;
    engine.sweep_all_utxos().await?;
    Ok(())
}

async fn send_funds(config: Arc<config::Config>, address: String, amount: f64) -> Result<(), AnyError> {
    let engine = engine::BenchmarkEngine::new(config).await?;
    engine.send_funds(address, amount).await?;
    Ok(())
}

// ====== Server Mode (UI + API) ======
async fn serve_api_ui(config: Arc<config::Config>, bind: String, port: u16, _lan_only: bool) -> Result<(), AnyError> {
    // Security banner HTML (prominent, always visible)
    const BANNER: &str = r#"<div style='background:#ffe9e9;color:#8b0000;padding:12px;font-weight:bold;'>
This wallet and UI are for benchmarking only. Treat the private key as visible/untrusted. Use temporarily, and sweep funds out when done. Do NOT expose outside your local network.
</div>"#;

    // Minimal UI
    // Prep default settings from env (dev convenience)
    let prep_conf_default: u64 = std::env::var("PREP_CONFIRMATIONS").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let prep_refresh_default: u64 = std::env::var("PREP_UTXO_REFRESH_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(1200);
    let prep_clients_default: usize = std::env::var("PREP_CLIENTS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let prep_parallel_default: usize = std::env::var("PREP_SPLIT_PARALLEL").ok().and_then(|s| s.parse().ok()).unwrap_or(12);

    let ui_html = Arc::new(format!(
        r#"<!doctype html><html><head><meta charset='utf-8'><title>Kasbench</title></head>
        <body style='font-family:system-ui,Segoe UI,Arial,sans-serif;margin:0;'>
        {banner}
        <div style='padding:16px'>
          <h2>Kasbench</h2>
          <div id='wallet'></div>
          <div id='walletButtons'>
            <button id='btnCreate' onclick='createWallet()'>Create Wallet</button>
            <button id='btnLoad' onclick='loadWallet()'>Refresh Wallet</button>
          </div>
          <h3>Wallet (set manually)</h3>
          <label>Private Key <input id='walletPriv' placeholder='hex'></label>
          <label>Address <input id='walletAddr' placeholder='kaspa:... (optional)'></label>
          <button onclick='setWallet()'>Use Wallet</button>
          <div><label><input type='checkbox' id='reveal' onchange='toggleReveal()'> Reveal private key</label></div>
          <hr>
          <h3>Prepare UTXOs</h3>
          <label>Count <input id='utxoCount' type='number' value='2400'></label>
          <details><summary>Settings</summary>
            <div style='padding:8px;border:1px solid #ccc;margin:6px 0;'>
              <label>Confirmations <input id='prepConf' type='number' value='{prep_conf}'></label>
              <label>UTXO Refresh (ms) <input id='prepRefresh' type='number' value='{prep_refresh}'></label>
              <label>Clients <input id='prepClients' type='number' value='{prep_clients}'></label>
              <label>Split Parallel <input id='prepParallel' type='number' value='{prep_parallel}'></label>
            </div>
          </details>
          <button onclick='prepare()'>Prepare</button>
          <button onclick='stopPrepare()'>Stop</button>
          <div id='prepStatus'></div>
          <hr>
          <h3>Benchmark</h3>
          <label>TPS <input id='tps' type='number' value='900'></label>
          <label>Duration (s) <input id='dur' type='number' value='180'></label>
          <button onclick='startBench()'>Start</button>
          <button onclick='stopBench()'>Stop</button>
          <pre id='status'></pre>
          <h3>Logs</h3>
          <pre id='logs' style='max-height:240px;overflow:auto;background:#111;color:#0f0;padding:8px;'></pre>
          <hr>
          <h3>Send</h3>
          <label>Address <input id='addr'></label>
          <label>Amount (KAS) <input id='amt' type='number' value='10'></label>
          <button onclick='send()'>Send</button>
          <hr>
          <h3>Sweep</h3>
          <button onclick='sweep()'>Sweep All</button>
        </div>
        <script>
        async function j(url,opts){{const r=await fetch(url,{{headers:{{'Content-Type':'application/json'}},...opts}}); if(!r.ok) throw new Error(await r.text()); return r.json();}}
        let manual=false; let lastPriv='';
        async function loadWallet(){{
          const w=document.getElementById('wallet');
          w.innerText='Refreshing wallet...';
          try{{
            const r=await j('/api/wallet');
            var bal=''; if (r.balance_sompi!==undefined) {{ bal = ' | Balance: ' + (r.balance_sompi/1e8).toFixed(4) + ' KAS'; }}
            var utx=''; if (r.utxo_count!==undefined) {{ utx = ' | UTXOs: ' + r.utxo_count; }}
            w.innerText='Address: '+r.address+bal+utx;
            manual=!!r.manual; lastPriv=r.private_key||lastPriv;
            const wb=document.getElementById('walletButtons');
            wb.style.display='block';
            const bc=document.getElementById('btnCreate');
            if(bc) bc.style.display = manual? 'none':'inline-block';
            const bl=document.getElementById('btnLoad');
            if(bl) bl.disabled=false;
          }}catch(e){{
            w.innerText='Wallet refresh failed: ' + e.message;
          }}
        }}
        async function createWallet(){{const r=await j('/api/wallet/new',{{method:'POST'}}); lastPriv=r.private_key; document.getElementById('wallet').innerText = 'Address: '+r.address+(document.getElementById('reveal').checked? ('\nPRIVATE KEY (benchmark-only): '+r.private_key):''); manual=false; document.getElementById('walletButtons').style.display='block';}}
        async function toggleReveal(){{const chk=document.getElementById('reveal').checked; const r=await j('/api/wallet'+(chk?'?reveal=1':'')); lastPriv=r.private_key||''; document.getElementById('wallet').innerText='Address: '+r.address+(chk&&lastPriv? ('\nPRIVATE KEY (benchmark-only): '+lastPriv):'');}}
        async function setWallet(){{const pk=document.getElementById('walletPriv').value; const ad=document.getElementById('walletAddr').value; const r=await j('/api/wallet/set',{{method:'POST',body:JSON.stringify({{private_key:pk,address:ad}})}}); lastPriv=pk; manual=true; document.getElementById('walletButtons').style.display='none'; document.getElementById('wallet').innerText='Address: '+r.address+(document.getElementById('reveal').checked? ('\nPRIVATE KEY (benchmark-only): '+pk):'');}}
        async function prepare(){{
          const n=Number(document.getElementById('utxoCount').value);
          const conf=Number(document.getElementById('prepConf').value);
          const refMs=Number(document.getElementById('prepRefresh').value);
          const clients=Number(document.getElementById('prepClients').value);
          const par=Number(document.getElementById('prepParallel').value);
          const ps=document.getElementById('prepStatus');
          ps.innerText='Preparing...';
          try{{
            await j('/api/prepare',{{method:'POST',body:JSON.stringify({{utxo_count:n, confirmations:conf, utxo_refresh_ms:refMs, clients:clients, split_parallel:par}})}});
            ps.innerText='Submitted. Check Status.';
          }}catch(e){{
            let txt='';
            try{{ const obj=JSON.parse(e.message); if(obj && obj.reason==='insufficient_funds'){{ txt='Insufficient funds: requested ' + obj.requested + ', achievable ' + obj.achievable + '. Adjust UTXOs or fund wallet.'; }} else {{ txt=e.message; }} }}catch{{ txt=e.message; }}
            ps.innerText=txt;
          }}
        }}
        async function stopPrepare(){{await j('/api/prepare/stop',{{method:'POST'}}); document.getElementById('prepStatus').innerText='Stopping...';}}
        async function startBench(){{const t=Number(document.getElementById('tps').value), d=Number(document.getElementById('dur').value); await j('/api/benchmark/start',{{method:'POST',body:JSON.stringify({{tps:t,duration_s:d}})}}); poll();}}
        async function stopBench(){{await j('/api/benchmark/stop',{{method:'POST'}});}}
        async function poll(){{try{{const s=await j('/api/benchmark/status'); document.getElementById('status').innerText=JSON.stringify(s,null,2);}}catch(e){{}} try{{const l=await j('/api/logs?last=200'); document.getElementById('logs').innerText=l.lines.join('\n');}}catch(e){{}} setTimeout(poll,1000);}}
        loadWallet(); poll();
        async function send(){{const a=document.getElementById('addr').value, m=Number(document.getElementById('amt').value); await j('/api/send',{{method:'POST',body:JSON.stringify({{address:a,amount:m}})}});}}
        async function sweep(){{await j('/api/sweep',{{method:'POST'}});}} 
        </script>
        </body></html>"#,
        banner=BANNER,
        prep_conf=prep_conf_default,
        prep_refresh=prep_refresh_default,
        prep_clients=prep_clients_default,
        prep_parallel=prep_parallel_default));

    // Shared engine base config and wallet state (persisted under /data/wallet.json)
    let config_base = config.clone();
    let shared_metrics: Arc<metrics::MetricsCollector> = Arc::new(metrics::MetricsCollector::new());
    let metrics_filter = {
        let shared_metrics = shared_metrics.clone();
        warp::any().map(move || shared_metrics.clone())
    };
    let wallet_state = Arc::new(parking_lot::Mutex::<Option<(Keypair, Address)>>::new(None));
    let prepare_task: TaskHandle = Arc::new(parking_lot::Mutex::new(None));
    let bench_task: TaskHandle = Arc::new(parking_lot::Mutex::new(None));
    // Try to load existing wallet file
    if let Ok(text) = std::fs::read_to_string("/data/wallet.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(pk_hex) = v.get("private_key").and_then(|s| s.as_str()) {
                if let Ok(sk) = SecretKey::from_str(pk_hex) {
                    let kp = Keypair::from_secret_key(SECP256K1, &sk);
                    let addr = Address::new(config_base.address.prefix, Version::PubKey, &kp.x_only_public_key().0.serialize());
                    *wallet_state.lock() = Some((kp, addr));
                }
            }
        }
    }
    // If no wallet file, persist the server's active (config) wallet so UI and backend match
    if wallet_state.lock().is_none() {
        let kp = config_base.keypair;
        let addr = Address::new(config_base.address.prefix, Version::PubKey, &kp.x_only_public_key().0.serialize());
        let data = serde_json::json!({"private_key": format!("{}", secp256k1::SecretKey::from_keypair(&kp).display_secret()), "address": addr.to_string()});
        let _ = std::fs::create_dir_all("/data");
        let _ = std::fs::write("/data/wallet.json", serde_json::to_vec_pretty(&data).unwrap());
        *wallet_state.lock() = Some((kp, addr));
    }

    // Filters
    let ui_route = {
        let ui_html = ui_html.clone();
        warp::path::end().map(move || warp::reply::html(ui_html.as_str().to_string()))
    };

    // Health
    let health = warp::path!("api" / "health").and(warp::get()).map(|| warp::reply::json(&serde_json::json!({"ok":true})));

    // Wallet current (with optional reveal and balance)
    let wallet_get = {
        let wallet_state = wallet_state.clone();
        let config_base = config_base.clone();
        let metrics_filter = metrics_filter.clone();
        warp::path!("api" / "wallet").and(warp::get()).and(warp::query::query()).and(metrics_filter).and_then(move |q: std::collections::HashMap<String,String>, shared_metrics: Arc<metrics::MetricsCollector>|{
            let wallet_state = wallet_state.clone();
            let config_base = config_base.clone();
            async move {
                let current_pair = wallet_state.lock().clone();
                let (current_addr, manual) = if let Some((_kp, a)) = current_pair.clone() { (a, true) } else { (config_base.address.clone(), false) };
                let reveal = q.get("reveal").map(|v| v=="1" || v.to_lowercase()=="true").unwrap_or(false);
                let mut obj = serde_json::json!({
                    "address": current_addr.to_string(),
                    "manual": manual,
                    "balance_sompi": 0u64,
                    "utxo_count": 0u64,
                    "balance_ok": false
                });
                if reveal {
                    if let Some((kp, _)) = wallet_state.lock().clone() { obj["private_key"] = serde_json::json!(format!("{}", secp256k1::SecretKey::from_keypair(&kp).display_secret())); }
                }
                // Try to include balance and utxo count (best-effort)
                let overrides = if let Some((kp, addr)) = current_pair { config::ConfigOverrides{ keypair: Some(kp), address: Some(addr), ..Default::default() } } else { Default::default() };
                if let Ok(engine) = engine::BenchmarkEngine::new_with_metrics(config_base.clone_with_overrides(overrides), shared_metrics.clone()).await {
                    if let Ok((count, total)) = engine.spendable_summary_with_min_conf(0).await {
                        obj["balance_sompi"] = serde_json::json!(total);
                        obj["utxo_count"] = serde_json::json!(count);
                        obj["balance_ok"] = serde_json::json!(true);
                    }
                }
                Ok::<_, warp::Rejection>(warp::reply::json(&obj))
            }
        })
    };
    // Wallet set from UI (dev convenience)
    let wallet_set = {
        let wallet_state = wallet_state.clone();
        let config_base = config_base.clone();
        warp::path!("api" / "wallet" / "set").and(warp::post()).and(warp::body::json()).and_then(move |body: serde_json::Value|{
            let wallet_state = wallet_state.clone();
            let config_base = config_base.clone();
            async move {
                if let Some(pk_hex) = body.get("private_key").and_then(|v| v.as_str()) {
                    if let Ok(sk) = SecretKey::from_str(pk_hex) {
                        let kp = Keypair::from_secret_key(SECP256K1, &sk);
                        let addr = if let Some(a) = body.get("address").and_then(|v| v.as_str()) { Address::try_from(a).unwrap_or_else(|_| Address::new(config_base.address.prefix, Version::PubKey, &kp.x_only_public_key().0.serialize())) } else { Address::new(config_base.address.prefix, Version::PubKey, &kp.x_only_public_key().0.serialize()) };
                        let data = serde_json::json!({"private_key": pk_hex, "address": addr.to_string()});
                        let _ = std::fs::create_dir_all("/data");
                        let _ = std::fs::write("/data/wallet.json", serde_json::to_vec_pretty(&data).unwrap());
                        *wallet_state.lock() = Some((kp, addr.clone()));
                        return Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({"ok": true, "address": addr.to_string(), "manual": true})));
                    }
                }
                Err(warp::reject())
            }
        })
    };

    // Wallet create (generate new key)
    let wallet_new = {
        let config_base = config_base.clone();
        let wallet_state = wallet_state.clone();
        warp::path!("api" / "wallet" / "new").and(warp::post()).and_then(move || {
            let config_base = config_base.clone();
            let wallet_state = wallet_state.clone();
            async move {
                use secp256k1::rand::thread_rng;
                let (sk, _pk) = secp256k1::generate_keypair(&mut thread_rng());
                let keypair = secp256k1::Keypair::from_secret_key(secp256k1::SECP256K1, &sk);
                let address = kaspa_addresses::Address::new(config_base.address.prefix, kaspa_addresses::Version::PubKey, &keypair.x_only_public_key().0.serialize());
                // Persist
                let data = serde_json::json!({"private_key": sk.display_secret().to_string(), "address": address.to_string()});
                let _ = std::fs::create_dir_all("/data");
                let _ = std::fs::write("/data/wallet.json", serde_json::to_vec_pretty(&data).unwrap());
                // Update active wallet state used by subsequent ops
                *wallet_state.lock() = Some((keypair, address.clone()));
                Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({
                    "address": address.to_string(),
                    "private_key": sk.display_secret().to_string(),
                    "warning": "Benchmark-only wallet. Treat as visible. Sweep funds out when done."
                })))
            }
        })
    };

    // Prepare
    let prepare = {
        let config_base = config_base.clone();
        let wallet_state = wallet_state.clone();
        let metrics_filter = metrics_filter.clone();
        let prepare_task_filter = {
            let prepare_task = prepare_task.clone();
            warp::any().map(move || prepare_task.clone())
        };
        warp::path!("api" / "prepare").and(warp::post()).and(warp::body::json()).and(metrics_filter).and(prepare_task_filter).and_then(move |body: serde_json::Value, shared_metrics: Arc<metrics::MetricsCollector>, prepare_task: TaskHandle|{
            let config_base = config_base.clone();
            let wallet_state = wallet_state.clone();
            async move {
                let n = body.get("utxo_count").and_then(|v| v.as_u64()).unwrap_or(2400) as usize;
                let conf = body.get("confirmations").and_then(|v| v.as_u64());
                let ref_ms = body.get("utxo_refresh_ms").and_then(|v| v.as_u64());
                let clients = body.get("clients").and_then(|v| v.as_u64()).map(|v| v as usize);
                let split_parallel = body.get("split_parallel").and_then(|v| v.as_u64()).map(|v| v as usize);
                // Override config with persisted/active wallet if present
                let mut overrides = if let Some((kp, addr)) = wallet_state.lock().clone() {
                    config::ConfigOverrides{ keypair: Some(kp), address: Some(addr), ..Default::default() }
                } else { Default::default() };
                if let Some(v)=conf { overrides.confirmation_depth=Some(v); }
                if let Some(v)=ref_ms { overrides.utxo_refresh_ms=Some(v); }
                if let Some(v)=clients { overrides.client_pool_size=Some(v); }
                if let Some(v)=split_parallel { overrides.split_parallel=Some(v); }

                // Preflight achievable calculation using a temporary engine instance
                let engine_pf = engine::BenchmarkEngine::new_with_metrics(config_base.clone_with_overrides(overrides), shared_metrics.clone()).await.map_err(|_| warp::reject())?;
                let (current_count_pf, total_balance_pf) = engine_pf.spendable_summary().await.map_err(|_| warp::reject())?;
                let min_utxo_size_pf: u64 = 20_000_000;
                let ideal_utxo_size_pf: u64 = 145_000_000;
                let max_utxo_size_pf: u64 = ideal_utxo_size_pf * 2;
                let estimated_fees_pf: u64 = (n as u64).saturating_mul(100_000);
                let available_pf = total_balance_pf.saturating_sub(estimated_fees_pf);
                let mut utxo_size_pf = if n > 500 && available_pf / (n as u64) > 50_000_000 { 50_000_000 } else { ideal_utxo_size_pf };
                utxo_size_pf = utxo_size_pf.clamp(min_utxo_size_pf, max_utxo_size_pf);
                let achievable_pf = ((available_pf / utxo_size_pf) as usize).max(current_count_pf);
                if n > achievable_pf {
                    let msg = serde_json::json!({
                        "ok": false,
                        "reason": "insufficient_funds",
                        "requested": n,
                        "achievable": achievable_pf,
                        "estimated_utxo_size": utxo_size_pf,
                        "available_balance": available_pf
                    });
                    let reply = warp::reply::json(&msg);
                    return Ok::<_, warp::Rejection>(warp::reply::with_status(reply, warp::http::StatusCode::BAD_REQUEST));
                }

                // Rebuild overrides for the actual run
                let mut overrides2 = if let Some((kp, addr)) = wallet_state.lock().clone() {
                    config::ConfigOverrides{ keypair: Some(kp), address: Some(addr), ..Default::default() }
                } else { Default::default() };
                if let Some(v)=conf { overrides2.confirmation_depth=Some(v); }
                if let Some(v)=ref_ms { overrides2.utxo_refresh_ms=Some(v); }
                if let Some(v)=clients { overrides2.client_pool_size=Some(v); }
                if let Some(v)=split_parallel { overrides2.split_parallel=Some(v); }
                let engine = engine::BenchmarkEngine::new_with_metrics(config_base.clone_with_overrides(overrides2), shared_metrics.clone()).await.map_err(|_| warp::reject())?;
                let metrics_for_task = shared_metrics.clone();
                let handle = tokio::spawn(async move { let _ = engine.prepare_utxos(n, None).await; metrics_for_task.append_line("Prepare task ended"); });
                {
                    let mut guard = prepare_task.lock();
                    if let Some(h) = guard.take() { h.abort(); }
                    *guard = Some(handle);
                }
                let ok = warp::reply::json(&serde_json::json!({"submitted": true, "target": n}));
                Ok::<_, warp::Rejection>(warp::reply::with_status(ok, warp::http::StatusCode::OK))
            }
        })
    };
    // Prepare stop (best-effort: noop placeholder)
    let prepare_stop = {
        let prepare_task = prepare_task.clone();
        let metrics_filter = metrics_filter.clone();
        warp::path!("api" / "prepare" / "stop").and(warp::post()).and(metrics_filter).map(move |shared_metrics: Arc<metrics::MetricsCollector>|{
            let aborted = {
                let mut guard = prepare_task.lock();
                if let Some(h) = guard.take() { h.abort(); true } else { false }
            };
            if aborted { shared_metrics.append_line("Prepare task aborted"); }
            warp::reply::json(&serde_json::json!({"stopping": true, "aborted": aborted}))
        })
    };

    // Benchmark start/stop/status
    let bench_start = {
        let config_base = config_base.clone();
        let wallet_state = wallet_state.clone();
        let metrics_filter = metrics_filter.clone();
        let bench_task_filter = {
            let bench_task = bench_task.clone();
            warp::any().map(move || bench_task.clone())
        };
        warp::path!("api" / "benchmark" / "start").and(warp::post()).and(warp::body::json()).and(metrics_filter).and(bench_task_filter).and_then(move |body: serde_json::Value, shared_metrics: Arc<metrics::MetricsCollector>, bench_task: TaskHandle|{
            let config_base = config_base.clone();
            let wallet_state = wallet_state.clone();
            async move {
                let tps = body.get("tps").and_then(|v| v.as_u64()).unwrap_or(900);
                let duration = body.get("duration_s").and_then(|v| v.as_u64()).unwrap_or(180);
                let mut overrides = config::ConfigOverrides{ target_tps: Some(tps), ..Default::default() };
                if let Some((kp, addr)) = wallet_state.lock().clone() { overrides.keypair = Some(kp); overrides.address = Some(addr); }
                let engine = engine::BenchmarkEngine::new_with_metrics(config_base.clone_with_overrides(overrides), shared_metrics.clone()).await.map_err(|_| warp::reject())?;
                let metrics_for_task = shared_metrics.clone();
                let metrics_for_reply = shared_metrics.clone();
                let handle = tokio::spawn(async move { let _ = engine.run(duration).await; metrics_for_task.append_line("Benchmark task ended"); });
                {
                    let mut guard = bench_task.lock();
                    if let Some(h) = guard.take() { h.abort(); }
                    *guard = Some(handle);
                }
                Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({"started": true, "tps": tps, "duration_s": duration, "peak_tps_so_far": metrics_for_reply.snapshot().peak_tps})))
            }
        })
    };
    let bench_status = {
        let c = config_base.clone();
        let wallet_state = wallet_state.clone();
        let metrics_filter = metrics_filter.clone();
        warp::path!("api" / "benchmark" / "status").and(warp::get()).and(metrics_filter).and_then(move |shared_metrics: Arc<metrics::MetricsCollector>|{
            let _c = c.clone();
            let _wallet_state = wallet_state.clone();
            async move {
                Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!(shared_metrics.snapshot())))
            }
        })
    };
    let bench_stop = {
        let bench_task = bench_task.clone();
        let metrics_filter = metrics_filter.clone();
        warp::path!("api" / "benchmark" / "stop").and(warp::post()).and(metrics_filter).map(move |shared_metrics: Arc<metrics::MetricsCollector>|{
            let aborted = {
                let mut guard = bench_task.lock();
                if let Some(h) = guard.take() { h.abort(); true } else { false }
            };
            if aborted { shared_metrics.append_line("Benchmark task aborted"); }
            warp::reply::json(&serde_json::json!({"stopping": true, "aborted": aborted}))
        })
    };

    // Logs endpoint (last N lines)
    let logs_ep = {
        let c = config_base.clone();
        let wallet_state = wallet_state.clone();
        let metrics_filter = metrics_filter.clone();
        warp::path!("api" / "logs").and(warp::get()).and(warp::query::query()).and(metrics_filter).and_then(move |q: std::collections::HashMap<String,String>, shared_metrics: Arc<metrics::MetricsCollector>|{
            let _c = c.clone();
            let _wallet_state = wallet_state.clone();
            async move {
                let last = q.get("last").and_then(|s| s.parse::<usize>().ok()).unwrap_or(200);
                let lines = shared_metrics.recent_log_lines(last);
                Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({"lines": lines})))
            }
        })
    };

    // Send & sweep
    let send_ep = {
        let config_base = config_base.clone();
        let wallet_state = wallet_state.clone();
        warp::path!("api" / "send").and(warp::post()).and(warp::body::json()).and_then(move |body: serde_json::Value|{
            let config_base = config_base.clone();
            let wallet_state = wallet_state.clone();
            async move {
                let addr = body.get("address").and_then(|v| v.as_str()).ok_or_else(warp::reject)?;
                let amt = body.get("amount").and_then(|v| v.as_f64()).ok_or_else(warp::reject)?;
                let overrides = if let Some((kp, a)) = wallet_state.lock().clone() { config::ConfigOverrides{ keypair: Some(kp), address: Some(a), ..Default::default() } } else { Default::default() };
                let engine = engine::BenchmarkEngine::new(config_base.clone_with_overrides(overrides)).await.map_err(|_| warp::reject())?;
                engine.send_funds(addr.to_string(), amt).await.map_err(|_| warp::reject())?;
                Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({"ok": true})))
            }
        })
    };
    let sweep_ep = {
        let config_base = config_base.clone();
        let wallet_state = wallet_state.clone();
        let metrics_filter = metrics_filter.clone();
        warp::path!("api" / "sweep").and(warp::post()).and(metrics_filter).and_then(move |shared_metrics: Arc<metrics::MetricsCollector>|{
            let config_base = config_base.clone();
            let wallet_state = wallet_state.clone();
            async move {
                let overrides = if let Some((kp, addr)) = wallet_state.lock().clone() { config::ConfigOverrides{ keypair: Some(kp), address: Some(addr), ..Default::default() } } else { Default::default() };
                let engine = engine::BenchmarkEngine::new_with_metrics(config_base.clone_with_overrides(overrides), shared_metrics.clone()).await.map_err(|_| warp::reject())?;
                tokio::spawn(async move { let _ = engine.sweep_all_utxos().await; });
                Ok::<_, warp::Rejection>(warp::reply::json(&serde_json::json!({"submitted": true})))
            }
        })
    };

    // Compose routes
    let routes = ui_route
        .or(health)
        .or(wallet_get)
        .or(wallet_new)
        .or(wallet_set)
        .or(prepare)
        .or(prepare_stop)
        .or(bench_start)
        .or(bench_status)
        .or(bench_stop)
        .or(logs_ep)
        .or(send_ep)
        .or(sweep_ep)
        .with(warp::cors().allow_any_origin().allow_methods(["GET","POST"]).allow_headers(["content-type"]))
        .with(warp::filters::trace::request());

    // Bind and run with LAN guard (basic check via remote IP if desired)
    let addr: std::net::SocketAddr = format!("{}:{}", bind, port).parse()?;
    info!("UI/API listening on http://{}", addr);
    warp::serve(routes).run(addr).await;
    Ok(())
}
