use crate::config::ExportConfig;
use crate::models::{EventBatch, PeerConnectionEvent};
use crate::storage::EventStorage;
use log::{debug, error, info, warn};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::time;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("HTTP client error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Export failed after {0} retries")]
    MaxRetriesExceeded(usize),
}

pub type ExportResult<T> = Result<T, ExportError>;

/// Handles exporting peer connection events to external services
pub struct EventExporter {
    config: ExportConfig,
    client: Client,
    storage: Arc<EventStorage>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl EventExporter {
    /// Create a new event exporter
    pub fn new(config: ExportConfig, storage: Arc<EventStorage>) -> ExportResult<Self> {
        if config.enabled && config.endpoint.is_none() {
            return Err(ExportError::ConfigError("Export endpoint required when export is enabled".to_string()));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            config,
            client,
            storage,
            shutdown_tx: None,
        })
    }

    /// Start the export worker loop
    pub async fn start(&mut self) -> ExportResult<()> {
        if !self.config.enabled {
            info!("Event export is disabled");
            return Ok(());
        }

        info!("Starting event exporter (backend: {}, interval: {}s)",
              self.config.backend, self.config.interval_secs);

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let config = self.config.clone();
        let client = self.client.clone();
        let storage = self.storage.clone();

        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(config.interval_secs));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = Self::export_batch(&config, &client, &storage).await {
                            error!("Export batch failed: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Export worker shutting down");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Export a batch of events
    async fn export_batch(config: &ExportConfig, client: &Client, storage: &EventStorage) -> ExportResult<()> {
        // Get unexported events from storage
        let events = storage
            .get_unexported_events(config.batch_size)
            .map_err(|e| ExportError::StorageError(e.to_string()))?;

        if events.is_empty() {
            debug!("No events to export");
            return Ok(());
        }

        info!("Exporting batch of {} events", events.len());

        let sensor_id = events.first().map(|e| e.sensor_id.clone()).unwrap_or_default();
        let batch = EventBatch::new(sensor_id, events.clone());

        // Attempt export with retries
        let mut retry_count = 0;
        let mut last_error = None;

        while retry_count <= config.max_retries {
            match Self::send_batch(config, client, &batch).await {
                Ok(_) => {
                    // Mark events as exported
                    let event_ids: Vec<String> = events.iter().map(|e| e.event_id.clone()).collect();
                    storage
                        .mark_events_exported(&event_ids)
                        .map_err(|e| ExportError::StorageError(e.to_string()))?;

                    info!("Successfully exported {} events", events.len());
                    return Ok(());
                }
                Err(e) => {
                    last_error = Some(e);
                    retry_count += 1;

                    if retry_count <= config.max_retries {
                        let backoff = config.retry_backoff_secs * retry_count as u64;
                        warn!("Export failed (attempt {}/{}), retrying in {}s: {}",
                              retry_count, config.max_retries, backoff, last_error.as_ref().unwrap());
                        time::sleep(Duration::from_secs(backoff)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or(ExportError::MaxRetriesExceeded(config.max_retries)))
    }

    /// Send a batch to the configured endpoint
    async fn send_batch(config: &ExportConfig, client: &Client, batch: &EventBatch) -> ExportResult<()> {
        let endpoint = config.endpoint.as_ref().ok_or_else(|| {
            ExportError::ConfigError("Export endpoint not configured".to_string())
        })?;

        match config.backend.as_str() {
            "http" | "webhook" => Self::send_http(client, endpoint, config.api_key.as_deref(), batch).await,
            "firestore" => Self::send_firestore(client, endpoint, config.api_key.as_deref(), batch).await,
            other => Err(ExportError::ConfigError(format!("Unsupported backend: {}", other))),
        }
    }

    /// Send batch via HTTP POST
    async fn send_http(
        client: &Client,
        endpoint: &str,
        api_key: Option<&str>,
        batch: &EventBatch,
    ) -> ExportResult<()> {
        let mut request = client
            .post(endpoint)
            .json(&batch);

        if let Some(key) = api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await?;

        if response.status().is_success() {
            debug!("HTTP export successful");
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(ExportError::ConfigError(format!("HTTP {} - {}", status, body)))
        }
    }

    /// Send batch to Firestore
    async fn send_firestore(
        client: &Client,
        endpoint: &str,
        api_key: Option<&str>,
        batch: &EventBatch,
    ) -> ExportResult<()> {
        // Firestore REST API format
        // endpoint should be like: https://firestore.googleapis.com/v1/projects/{project}/databases/{database}/documents/{collection}

        let api_key = api_key.ok_or_else(|| {
            ExportError::ConfigError("API key required for Firestore".to_string())
        })?;

        // Send each event as a separate document
        for event in &batch.events {
            let document = Self::convert_to_firestore_document(event)?;

            let url = format!("{}?key={}", endpoint, api_key);
            let response = client
                .post(&url)
                .json(&document)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(ExportError::ConfigError(format!("Firestore HTTP {} - {}", status, body)));
            }
        }

        debug!("Firestore export successful");
        Ok(())
    }

    /// Convert event to Firestore document format
    fn convert_to_firestore_document(event: &PeerConnectionEvent) -> ExportResult<serde_json::Value> {
        // Firestore uses a specific document format
        Ok(serde_json::json!({
            "fields": {
                "event_id": {"stringValue": event.event_id},
                "timestamp": {"timestampValue": event.timestamp.to_rfc3339()},
                "sensor_id": {"stringValue": event.sensor_id},
                "peer_ip": {"stringValue": event.peer_ip.to_string()},
                "peer_port": {"integerValue": event.peer_port.to_string()},
                "protocol_version": {"integerValue": event.protocol_version.to_string()},
                "user_agent": {"stringValue": event.user_agent},
                "network": {"stringValue": event.network},
                "direction": {"stringValue": serde_json::to_string(&event.direction)?},
                "classification": {"stringValue": serde_json::to_string(&event.classification)?},
                "probe_duration_ms": {
                    "integerValue": event.probe_duration_ms.map(|d| d.to_string()).unwrap_or_default()
                },
                "probe_error": {
                    "stringValue": event.probe_error.as_ref().unwrap_or(&String::new())
                }
            }
        }))
    }

    /// Manually trigger an export
    pub async fn export_now(&self) -> ExportResult<()> {
        if !self.config.enabled {
            return Err(ExportError::ConfigError("Export is disabled".to_string()));
        }

        Self::export_batch(&self.config, &self.client, &self.storage).await
    }

    /// Shutdown the exporter
    pub async fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
            info!("Event exporter shutdown signal sent");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::models::{ConnectionDirection, PeerConnectionEvent};
    use std::net::IpAddr;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[test]
    fn test_exporter_creation() {
        let config = ExportConfig {
            enabled: false,
            backend: "http".to_string(),
            endpoint: None,
            api_key: None,
            batch_size: 100,
            interval_secs: 60,
            max_retries: 3,
            retry_backoff_secs: 5,
        };

        let db_config = DatabaseConfig {
            path: PathBuf::from(":memory:"),
            pool_size: 5,
            retention_days: 30,
            enable_wal: false,
            addressdb_path: PathBuf::from(":memory:"),
        };

        let storage = Arc::new(EventStorage::new(&db_config).unwrap());
        let exporter = EventExporter::new(config, storage);
        assert!(exporter.is_ok());
    }

    #[test]
    fn test_firestore_document_conversion() {
        let event = PeerConnectionEvent::new(
            "test-sensor".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        let doc = EventExporter::convert_to_firestore_document(&event).unwrap();
        assert!(doc["fields"]["event_id"]["stringValue"].is_string());
        assert!(doc["fields"]["peer_ip"]["stringValue"].is_string());
    }
}
