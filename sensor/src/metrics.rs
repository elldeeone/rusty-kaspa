use crate::config::MetricsConfig;
use crate::models::PeerClassification;
use log::{error, info};
use prometheus::{
    Counter, CounterVec, Gauge, GaugeVec, Histogram, HistogramOpts, IntCounter, IntCounterVec,
    IntGauge, Opts, Registry,
};
use std::sync::Arc;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("Prometheus error: {0}")]
    PrometheusError(#[from] prometheus::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Metrics server error: {0}")]
    ServerError(String),
}

pub type MetricsResult<T> = Result<T, MetricsError>;

/// Prometheus metrics collector for the sensor
pub struct SensorMetrics {
    registry: Arc<Registry>,

    // Connection metrics
    pub connections_total: IntCounterVec,
    pub connections_active: IntGauge,
    pub connections_inbound: IntCounter,
    pub connections_outbound: IntCounter,

    // Probe metrics
    pub probes_total: IntCounterVec,
    pub probe_duration: Histogram,
    pub probe_errors_total: IntCounterVec,

    // Classification metrics
    pub peers_classified_public: IntCounter,
    pub peers_classified_private: IntCounter,

    // Export metrics
    pub exports_total: IntCounterVec,
    pub export_batch_size: Histogram,
    pub export_errors_total: IntCounterVec,

    // Storage metrics
    pub storage_events_total: IntGauge,
    pub storage_pending_exports: IntGauge,
    pub storage_size_bytes: IntGauge,

    // System metrics
    pub uptime_seconds: IntGauge,
}

impl SensorMetrics {
    /// Create a new metrics collector
    pub fn new() -> MetricsResult<Self> {
        let registry = Registry::new();

        // Connection metrics
        let connections_total = IntCounterVec::new(
            Opts::new("sensor_connections_total", "Total number of peer connections"),
            &["direction", "result"],
        )?;
        registry.register(Box::new(connections_total.clone()))?;

        let connections_active = IntGauge::new(
            "sensor_connections_active",
            "Number of currently active connections",
        )?;
        registry.register(Box::new(connections_active.clone()))?;

        let connections_inbound = IntCounter::new(
            "sensor_connections_inbound_total",
            "Total inbound connections",
        )?;
        registry.register(Box::new(connections_inbound.clone()))?;

        let connections_outbound = IntCounter::new(
            "sensor_connections_outbound_total",
            "Total outbound connections",
        )?;
        registry.register(Box::new(connections_outbound.clone()))?;

        // Probe metrics
        let probes_total = IntCounterVec::new(
            Opts::new("sensor_probes_total", "Total number of peer probes"),
            &["result"],
        )?;
        registry.register(Box::new(probes_total.clone()))?;

        let probe_duration = Histogram::with_opts(
            HistogramOpts::new("sensor_probe_duration_seconds", "Duration of probe attempts")
                .buckets(vec![0.001, 0.01, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0]),
        )?;
        registry.register(Box::new(probe_duration.clone()))?;

        let probe_errors_total = IntCounterVec::new(
            Opts::new("sensor_probe_errors_total", "Total probe errors"),
            &["error_type"],
        )?;
        registry.register(Box::new(probe_errors_total.clone()))?;

        // Classification metrics
        let peers_classified_public = IntCounter::new(
            "sensor_peers_classified_public_total",
            "Number of peers classified as public",
        )?;
        registry.register(Box::new(peers_classified_public.clone()))?;

        let peers_classified_private = IntCounter::new(
            "sensor_peers_classified_private_total",
            "Number of peers classified as private",
        )?;
        registry.register(Box::new(peers_classified_private.clone()))?;

        // Export metrics
        let exports_total = IntCounterVec::new(
            Opts::new("sensor_exports_total", "Total export attempts"),
            &["result"],
        )?;
        registry.register(Box::new(exports_total.clone()))?;

        let export_batch_size = Histogram::with_opts(
            HistogramOpts::new("sensor_export_batch_size", "Size of export batches")
                .buckets(vec![1.0, 10.0, 50.0, 100.0, 500.0, 1000.0]),
        )?;
        registry.register(Box::new(export_batch_size.clone()))?;

        let export_errors_total = IntCounterVec::new(
            Opts::new("sensor_export_errors_total", "Total export errors"),
            &["error_type"],
        )?;
        registry.register(Box::new(export_errors_total.clone()))?;

        // Storage metrics
        let storage_events_total = IntGauge::new(
            "sensor_storage_events_total",
            "Total events in storage",
        )?;
        registry.register(Box::new(storage_events_total.clone()))?;

        let storage_pending_exports = IntGauge::new(
            "sensor_storage_pending_exports",
            "Number of events pending export",
        )?;
        registry.register(Box::new(storage_pending_exports.clone()))?;

        let storage_size_bytes = IntGauge::new(
            "sensor_storage_size_bytes",
            "Database size in bytes",
        )?;
        registry.register(Box::new(storage_size_bytes.clone()))?;

        // System metrics
        let uptime_seconds = IntGauge::new(
            "sensor_uptime_seconds",
            "Sensor uptime in seconds",
        )?;
        registry.register(Box::new(uptime_seconds.clone()))?;

        Ok(Self {
            registry: Arc::new(registry),
            connections_total,
            connections_active,
            connections_inbound,
            connections_outbound,
            probes_total,
            probe_duration,
            probe_errors_total,
            peers_classified_public,
            peers_classified_private,
            exports_total,
            export_batch_size,
            export_errors_total,
            storage_events_total,
            storage_pending_exports,
            storage_size_bytes,
            uptime_seconds,
        })
    }

    /// Record a new connection
    pub fn record_connection(&self, inbound: bool, success: bool) {
        let direction = if inbound { "inbound" } else { "outbound" };
        let result = if success { "success" } else { "failure" };

        self.connections_total.with_label_values(&[direction, result]).inc();

        if inbound {
            self.connections_inbound.inc();
        } else {
            self.connections_outbound.inc();
        }

        if success {
            self.connections_active.inc();
        }
    }

    /// Record connection closed
    pub fn record_connection_closed(&self) {
        self.connections_active.dec();
    }

    /// Record a probe attempt
    pub fn record_probe(&self, classification: PeerClassification, duration_secs: f64) {
        let result = match classification {
            PeerClassification::Public => "public",
            PeerClassification::Private => "private",
            PeerClassification::Unknown => "unknown",
        };

        self.probes_total.with_label_values(&[result]).inc();
        self.probe_duration.observe(duration_secs);

        match classification {
            PeerClassification::Public => self.peers_classified_public.inc(),
            PeerClassification::Private => self.peers_classified_private.inc(),
            PeerClassification::Unknown => {}
        }
    }

    /// Record a probe error
    pub fn record_probe_error(&self, error_type: &str) {
        self.probe_errors_total.with_label_values(&[error_type]).inc();
    }

    /// Record an export attempt
    pub fn record_export(&self, success: bool, batch_size: usize) {
        let result = if success { "success" } else { "failure" };
        self.exports_total.with_label_values(&[result]).inc();
        self.export_batch_size.observe(batch_size as f64);
    }

    /// Record export error
    pub fn record_export_error(&self, error_type: &str) {
        self.export_errors_total.with_label_values(&[error_type]).inc();
    }

    /// Update storage metrics
    pub fn update_storage_metrics(&self, total_events: i64, pending_exports: i64, size_bytes: u64) {
        self.storage_events_total.set(total_events);
        self.storage_pending_exports.set(pending_exports);
        self.storage_size_bytes.set(size_bytes as i64);
    }

    /// Update uptime
    pub fn update_uptime(&self, seconds: i64) {
        self.uptime_seconds.set(seconds);
    }

    /// Get metrics as Prometheus text format
    pub fn gather(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder.encode_to_string(&metric_families).unwrap_or_default()
    }

    /// Start the metrics HTTP server
    pub async fn start_server(self: Arc<Self>, config: MetricsConfig) -> MetricsResult<()> {
        if !config.enabled {
            info!("Metrics server is disabled");
            return Ok(());
        }

        info!("Starting metrics server on {}", config.address);

        let listener = TcpListener::bind(&config.address).await?;

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((mut stream, _addr)) => {
                        let metrics_text = self.gather();

                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                            metrics_text.len(),
                            metrics_text
                        );

                        if let Err(e) = stream.write_all(response.as_bytes()).await {
                            error!("Failed to write metrics response: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to accept metrics connection: {}", e);
                    }
                }
            }
        });

        Ok(())
    }
}

impl Default for SensorMetrics {
    fn default() -> Self {
        Self::new().expect("Failed to create default metrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let metrics = SensorMetrics::new().unwrap();
        assert_eq!(metrics.connections_active.get(), 0);
    }

    #[test]
    fn test_record_connection() {
        let metrics = SensorMetrics::new().unwrap();
        metrics.record_connection(true, true);
        assert_eq!(metrics.connections_active.get(), 1);
        assert_eq!(metrics.connections_inbound.get(), 1);
    }

    #[test]
    fn test_record_probe() {
        let metrics = SensorMetrics::new().unwrap();
        metrics.record_probe(PeerClassification::Public, 0.25);
        assert_eq!(metrics.peers_classified_public.get(), 1);
        assert_eq!(metrics.peers_classified_private.get(), 0);
    }

    #[test]
    fn test_gather_metrics() {
        let metrics = SensorMetrics::new().unwrap();
        metrics.record_connection(true, true);
        let output = metrics.gather();
        assert!(output.contains("sensor_connections_active"));
    }
}
