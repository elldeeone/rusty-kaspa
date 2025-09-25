use serde::Serialize;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant as StdInstant};
use tokio::net::TcpListener;
use tokio::time::interval;
use tokio_stream::wrappers::TcpListenerStream;
use warp::Filter;

pub struct MetricsCollector {
    transactions_enqueued: AtomicU64,
    transactions_accepted: AtomicU64,
    transactions_failed: AtomicU64,
    utxo_count: AtomicUsize,
    last_tps: AtomicU64,
    peak_tps: AtomicU64,
    start_time: Mutex<Option<StdInstant>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            transactions_enqueued: AtomicU64::new(0),
            transactions_accepted: AtomicU64::new(0),
            transactions_failed: AtomicU64::new(0),
            utxo_count: AtomicUsize::new(0),
            last_tps: AtomicU64::new(0),
            peak_tps: AtomicU64::new(0),
            start_time: Mutex::new(None),
        }
    }

    pub fn record_enqueued(&self, count: usize) {
        if count > 0 {
            self.transactions_enqueued.fetch_add(count as u64, Ordering::Relaxed);
        }
    }

    pub fn record_accepted(&self) {
        self.transactions_accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_failed(&self) {
        self.transactions_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn update_utxo_count(&self, count: usize) {
        self.utxo_count.store(count, Ordering::Relaxed);
    }

    pub fn mark_start(&self) {
        let mut guard = self.start_time.lock().unwrap();
        if guard.is_none() {
            *guard = Some(StdInstant::now());
        }
    }

    pub async fn start_tps_calculator(&self) {
        let mut ticker = interval(Duration::from_secs(1));
        let mut last_count = 0u64;

        loop {
            ticker.tick().await;
            let current_count = self.transactions_accepted.load(Ordering::Relaxed);
            let tps = current_count.saturating_sub(last_count);
            last_count = current_count;

            self.last_tps.store(tps, Ordering::Relaxed);

            let peak = self.peak_tps.load(Ordering::Relaxed);
            if tps > peak {
                self.peak_tps.store(tps, Ordering::Relaxed);
            }

            println!(
                "TPS: {} | Peak: {} | Accepted: {} | Failed: {} | Enqueued: {} | UTXOs: {}",
                tps,
                self.peak_tps.load(Ordering::Relaxed),
                current_count,
                self.transactions_failed.load(Ordering::Relaxed),
                self.transactions_enqueued.load(Ordering::Relaxed),
                self.utxo_count.load(Ordering::Relaxed)
            );
        }
    }

    fn runtime_seconds(&self) -> u64 {
        let guard = self.start_time.lock().unwrap();
        guard.map(|instant| instant.elapsed().as_secs()).unwrap_or(0)
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            current_tps: self.last_tps.load(Ordering::Relaxed),
            peak_tps: self.peak_tps.load(Ordering::Relaxed),
            transactions_sent: self.transactions_enqueued.load(Ordering::Relaxed),
            transactions_confirmed: self.transactions_accepted.load(Ordering::Relaxed),
            transactions_failed: self.transactions_failed.load(Ordering::Relaxed),
            runtime_seconds: self.runtime_seconds(),
            utxo_count: self.utxo_count.load(Ordering::Relaxed),
        }
    }
}

#[derive(Serialize)]
pub struct MetricsSnapshot {
    pub current_tps: u64,
    pub peak_tps: u64,
    pub transactions_sent: u64,
    pub transactions_confirmed: u64,
    pub transactions_failed: u64,
    pub runtime_seconds: u64,
    pub utxo_count: usize,
}

pub async fn start_metrics_server(port: u16, metrics: Arc<MetricsCollector>) -> Result<(), Box<dyn std::error::Error>> {
    let dashboard_html = Arc::new(include_str!("../dashboard/index.html").to_string());

    let metrics_route = {
        let metrics = metrics.clone();
        warp::path("stats").and(warp::get()).map(move || warp::reply::json(&metrics.snapshot()))
    };

    let dashboard_route = {
        let dashboard_html = dashboard_html.clone();
        warp::path::end().map(move || warp::reply::html(dashboard_html.as_str().to_string()))
    };

    let routes = metrics_route.or(dashboard_route);

    // Try to bind starting from the requested port, incrementing until available (up to 20 tries)
    let mut bind_port = port;
    let max_tries: u16 = 20;
    let listener = loop {
        match TcpListener::bind(("0.0.0.0", bind_port)).await {
            Ok(l) => break l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && (bind_port - port) < max_tries => {
                bind_port = bind_port.saturating_add(1);
            }
            Err(e) => return Err(e.into()),
        }
    };

    println!("Dashboard available at http://0.0.0.0:{}", bind_port);
    warp::serve(routes).run_incoming(TcpListenerStream::new(listener)).await;
    Ok(())
}
