use crate::config::{PostgresConfig, SensorIdentity};
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_postgres::NoTls;

#[derive(Debug, Clone)]
pub struct PeerEvent {
    pub sensor_id: String,
    pub peer_address: String,
    pub event_type: String,
    pub classification: Option<String>,
    pub timestamp: i64,
    pub metadata: Option<String>,
}

pub struct PostgresWriter {
    pool: Pool,
    sensor_id: String,
    batch_size: usize,
    flush_interval: Duration,
    retry_enabled: bool,
    max_retries: usize,
    event_tx: mpsc::UnboundedSender<PeerEvent>,
}

impl PostgresWriter {
    /// Create a new PostgreSQL writer with connection pooling
    pub async fn new(
        config: &PostgresConfig,
        sensor_identity: &SensorIdentity,
    ) -> Result<Arc<Self>, Box<dyn std::error::Error + Send + Sync>> {
        let sensor_id = sensor_identity.sensor_id.clone();
        info!("Initializing PostgreSQL writer for sensor: {}", sensor_id);

        // Get password from config or environment
        let password = config
            .password
            .clone()
            .or_else(|| std::env::var("SENSOR_POSTGRES_PASSWORD").ok())
            .ok_or("PostgreSQL password not configured")?;

        // Create connection pool config
        let mut pg_config = Config::new();
        pg_config.host = Some(config.host.clone());
        pg_config.port = Some(config.port);
        pg_config.dbname = Some(config.database.clone());
        pg_config.user = Some(config.user.clone());
        pg_config.password = Some(password);
        pg_config.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });

        // Create connection pool
        let pool = pg_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("Failed to create PostgreSQL pool: {}", e))?;

        // Test connection
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("Failed to connect to PostgreSQL: {}", e))?;

        info!("Successfully connected to PostgreSQL at {}:{}", config.host, config.port);

        // Create peer_events table if it doesn't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS peer_events (
                id BIGSERIAL PRIMARY KEY,
                sensor_id TEXT NOT NULL,
                peer_address TEXT NOT NULL,
                event_type TEXT NOT NULL,
                classification TEXT,
                timestamp BIGINT NOT NULL,
                metadata JSONB,
                created_at TIMESTAMP DEFAULT NOW()
            )",
            &[],
        )
        .await
        .map_err(|e| format!("Failed to create table: {}", e))?;

        // Create indexes for fast queries
        let _ = conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_peer_events_sensor_id ON peer_events(sensor_id)",
                &[],
            )
            .await;

        let _ = conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_peer_events_timestamp ON peer_events(timestamp DESC)",
                &[],
            )
            .await;

        let _ = conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_peer_events_classification ON peer_events(classification) WHERE classification IS NOT NULL",
                &[],
            )
            .await;

        info!("PostgreSQL schema initialized");

        // Register this sensor in sensor_metadata table
        let _ = conn
            .execute(
                "INSERT INTO sensor_metadata (sensor_id, description, location, environment, last_seen)
                 VALUES ($1, $2, $3, $4, NOW())
                 ON CONFLICT (sensor_id) DO UPDATE SET
                     description = COALESCE(EXCLUDED.description, sensor_metadata.description),
                     location = COALESCE(EXCLUDED.location, sensor_metadata.location),
                     environment = COALESCE(EXCLUDED.environment, sensor_metadata.environment),
                     last_seen = NOW()",
                &[
                    &sensor_identity.sensor_id,
                    &sensor_identity.description,
                    &sensor_identity.environment, // Using environment as location for now
                    &sensor_identity.environment,
                ],
            )
            .await
            .map_err(|e| warn!("Failed to register sensor metadata: {}", e));

        info!("Sensor registered in metadata table");

        // Create channel for batching events
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let writer = Arc::new(Self {
            pool,
            sensor_id: sensor_id.clone(),
            batch_size: config.batch_size,
            flush_interval: Duration::from_secs(config.flush_interval_secs),
            retry_enabled: config.retry_enabled,
            max_retries: config.max_retries,
            event_tx,
        });

        // Spawn background batch writer
        let writer_clone = writer.clone();
        tokio::spawn(async move {
            writer_clone.batch_writer_loop(event_rx).await;
        });

        Ok(writer)
    }

    /// Queue an event to be written (non-blocking)
    pub fn queue_event(&self, event: PeerEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.event_tx.send(event).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(std::io::Error::new(std::io::ErrorKind::Other, format!("Failed to queue event: {}", e))) })
    }

    /// Background loop that batches and writes events
    async fn batch_writer_loop(&self, mut event_rx: mpsc::UnboundedReceiver<PeerEvent>) {
        let mut batch = Vec::with_capacity(self.batch_size);
        let mut flush_timer = tokio::time::interval(self.flush_interval);
        flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                // Receive events
                Some(event) = event_rx.recv() => {
                    batch.push(event);

                    // Flush if batch is full
                    if batch.len() >= self.batch_size {
                        debug!("Batch full ({}), flushing to PostgreSQL", batch.len());
                        self.flush_batch(&mut batch).await;
                    }
                }

                // Periodic flush
                _ = flush_timer.tick() => {
                    if !batch.is_empty() {
                        debug!("Flush interval elapsed, flushing {} events", batch.len());
                        self.flush_batch(&mut batch).await;
                    }
                }
            }
        }
    }

    /// Flush a batch of events to PostgreSQL
    async fn flush_batch(&self, batch: &mut Vec<PeerEvent>) {
        if batch.is_empty() {
            return;
        }

        let count = batch.len();
        debug!("Flushing {} events to PostgreSQL", count);

        let mut retries = 0;
        loop {
            match self.write_batch_internal(batch).await {
                Ok(_) => {
                    info!("Successfully wrote {} events to PostgreSQL", count);
                    batch.clear();
                    return;
                }
                Err(e) => {
                    error!("Failed to write batch to PostgreSQL: {}", e);

                    if !self.retry_enabled || retries >= self.max_retries {
                        error!("Dropping {} events after {} retries", count, retries);
                        batch.clear();
                        return;
                    }

                    retries += 1;
                    let backoff = Duration::from_secs(2u64.pow(retries.min(5) as u32));
                    warn!("Retrying in {:?} (attempt {}/{})", backoff, retries, self.max_retries);
                    sleep(backoff).await;
                }
            }
        }
    }

    /// Internal method to write a batch to PostgreSQL
    async fn write_batch_internal(&self, batch: &[PeerEvent]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if batch.is_empty() {
            return Ok(());
        }

        let conn = self.pool.get().await?;

        // Build bulk insert query
        let mut query = String::from("INSERT INTO peer_events (sensor_id, peer_address, peer_id, event_type, classification, timestamp, metadata) VALUES ");

        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
        let mut param_groups = Vec::new();

        for (i, _event) in batch.iter().enumerate() {
            let offset = i * 7;
            param_groups.push(format!(
                "(${}, ${}, ${}, ${}, ${}, ${}, ${}::jsonb)",
                offset + 1,
                offset + 2,
                offset + 3,
                offset + 4,
                offset + 5,
                offset + 6,
                offset + 7
            ));
        }

        query.push_str(&param_groups.join(", "));

        // Collect parameters
        // We need to build this carefully to maintain lifetimes
        let mut sensor_ids = Vec::new();
        let mut peer_addresses = Vec::new();
        let mut peer_ids: Vec<Option<String>> = Vec::new();
        let mut event_types = Vec::new();
        let mut classifications = Vec::new();
        let mut timestamps = Vec::new();
        let mut metadatas = Vec::new();

        for event in batch.iter() {
            sensor_ids.push(&event.sensor_id);
            peer_addresses.push(&event.peer_address);
            event_types.push(&event.event_type);
            classifications.push(&event.classification);
            timestamps.push(&event.timestamp);

            // Parse metadata String into JSON Value for PostgreSQL JSONB and extract peer_id
            let (metadata_json, peer_id_value) = match &event.metadata {
                Some(s) => match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(v) => {
                        // Extract peer_id from metadata
                        let peer_id = v.get("peer_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        (v, peer_id)
                    }
                    Err(e) => {
                        warn!("Failed to parse metadata JSON for {}: {}", event.peer_address, e);
                        (serde_json::Value::Null, None)
                    }
                },
                None => (serde_json::Value::Null, None),
            };
            metadatas.push(metadata_json);
            peer_ids.push(peer_id_value);
        }

        // Build params vector
        for i in 0..batch.len() {
            params.push(&sensor_ids[i] as &(dyn tokio_postgres::types::ToSql + Sync));
            params.push(&peer_addresses[i] as &(dyn tokio_postgres::types::ToSql + Sync));
            params.push(&peer_ids[i] as &(dyn tokio_postgres::types::ToSql + Sync));
            params.push(&event_types[i] as &(dyn tokio_postgres::types::ToSql + Sync));
            params.push(&classifications[i] as &(dyn tokio_postgres::types::ToSql + Sync));
            params.push(&timestamps[i] as &(dyn tokio_postgres::types::ToSql + Sync));
            params.push(&metadatas[i] as &(dyn tokio_postgres::types::ToSql + Sync));
        }

        conn.execute(&query, &params).await?;

        Ok(())
    }

    /// Query events from PostgreSQL (for debugging/testing)
    pub async fn get_event_count(&self) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.pool.get().await?;
        let row = conn.query_one("SELECT COUNT(*) FROM peer_events WHERE sensor_id = $1", &[&self.sensor_id]).await?;
        Ok(row.get(0))
    }

    /// Get classification statistics for this sensor
    pub async fn get_classification_stats(&self) -> Result<Vec<(String, i64)>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.pool.get().await?;
        let rows = conn
            .query(
                "SELECT classification, COUNT(*) as count FROM peer_events WHERE sensor_id = $1 AND classification IS NOT NULL GROUP BY classification",
                &[&self.sensor_id],
            )
            .await?;

        let mut stats = Vec::new();
        for row in rows {
            let classification: String = row.get(0);
            let count: i64 = row.get(1);
            stats.push((classification, count));
        }

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_event_creation() {
        let event = PeerEvent {
            sensor_id: "test-sensor".to_string(),
            peer_address: "192.168.1.1:16111".to_string(),
            event_type: "discovered".to_string(),
            classification: Some("public".to_string()),
            timestamp: 1234567890,
            metadata: Some(r#"{"test": "data"}"#.to_string()),
        };

        assert_eq!(event.sensor_id, "test-sensor");
        assert_eq!(event.event_type, "discovered");
    }
}
