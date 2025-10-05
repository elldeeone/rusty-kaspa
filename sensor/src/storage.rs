use crate::config::DatabaseConfig;
use crate::models::{PeerConnectionEvent, PeerClassification};
use crate::postgres::{PostgresWriter, PeerEvent};
use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OptionalExtension, Result as SqliteResult};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] rusqlite::Error),

    #[error("Connection pool error: {0}")]
    PoolError(#[from] r2d2::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Dual storage for peer connection events (local SQLite + optional remote PostgreSQL)
pub struct EventStorage {
    pool: Arc<Pool<SqliteConnectionManager>>,
    postgres: Option<Arc<PostgresWriter>>,
}

impl EventStorage {
    /// Create a new event storage instance
    pub fn new(config: &DatabaseConfig) -> StorageResult<Self> {
        let enable_wal = config.enable_wal;
        let manager = SqliteConnectionManager::file(&config.path)
            .with_init(move |conn| {
                // Enable WAL mode for better concurrency if configured
                if enable_wal {
                    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
                }
                // Performance optimizations
                conn.execute_batch(
                    "
                    PRAGMA synchronous = NORMAL;
                    PRAGMA cache_size = -64000;
                    PRAGMA temp_store = MEMORY;
                    ",
                )?;
                Ok(())
            });

        let pool = Pool::builder()
            .max_size(config.pool_size)
            .build(manager)?;

        let storage = Self {
            pool: Arc::new(pool),
            postgres: None,
        };

        // Initialize database schema
        storage.init_schema()?;

        info!("Event storage initialized at {:?}", config.path);
        Ok(storage)
    }

    /// Create a new event storage with PostgreSQL integration
    pub async fn new_with_postgres(
        config: &DatabaseConfig,
        sensor_id: String,
    ) -> StorageResult<Self> {
        // Initialize local SQLite storage
        let mut storage = Self::new(config)?;

        // Initialize PostgreSQL if configured
        if let Some(pg_config) = &config.postgres {
            match PostgresWriter::new(pg_config, sensor_id).await {
                Ok(pg_writer) => {
                    info!("PostgreSQL writer initialized successfully");
                    storage.postgres = Some(pg_writer);
                }
                Err(e) => {
                    error!("Failed to initialize PostgreSQL writer: {}", e);
                    warn!("Continuing with local storage only");
                }
            }
        }

        Ok(storage)
    }

    /// Initialize database schema
    fn init_schema(&self) -> StorageResult<()> {
        let conn = self.pool.get()?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS peer_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT UNIQUE NOT NULL,
                timestamp TEXT NOT NULL,
                sensor_id TEXT NOT NULL,
                peer_ip TEXT NOT NULL,
                peer_port INTEGER NOT NULL,
                protocol_version INTEGER NOT NULL,
                user_agent TEXT NOT NULL,
                network TEXT NOT NULL,
                direction TEXT NOT NULL,
                classification TEXT NOT NULL,
                probe_duration_ms INTEGER,
                probe_error TEXT,
                exported INTEGER DEFAULT 0,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_timestamp ON peer_events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_sensor_id ON peer_events(sensor_id);
            CREATE INDEX IF NOT EXISTS idx_exported ON peer_events(exported);
            CREATE INDEX IF NOT EXISTS idx_classification ON peer_events(classification);
            CREATE INDEX IF NOT EXISTS idx_peer_ip ON peer_events(peer_ip);

            CREATE TABLE IF NOT EXISTS export_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                retry_count INTEGER DEFAULT 0,
                last_error TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(event_id) REFERENCES peer_events(event_id)
            );

            CREATE INDEX IF NOT EXISTS idx_export_queue_event ON export_queue(event_id);
            ",
        )?;

        debug!("Database schema initialized");
        Ok(())
    }

    /// Insert a new peer connection event (writes to both SQLite and PostgreSQL)
    pub fn insert_event(&self, event: &PeerConnectionEvent) -> StorageResult<i64> {
        // Write to local SQLite (synchronous)
        let conn = self.pool.get()?;

        let classification_str = serde_json::to_string(&event.classification)?;
        let direction_str = serde_json::to_string(&event.direction)?;

        conn.execute(
            "INSERT INTO peer_events (
                event_id, timestamp, sensor_id, peer_ip, peer_port,
                protocol_version, user_agent, network, direction, classification,
                probe_duration_ms, probe_error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                &event.event_id,
                event.timestamp.to_rfc3339(),
                &event.sensor_id,
                event.peer_ip.to_string(),
                event.peer_port as i64,
                event.protocol_version as i64,
                &event.user_agent,
                &event.network,
                direction_str,
                classification_str,
                event.probe_duration_ms.map(|d| d as i64),
                &event.probe_error,
            ],
        )?;

        let id = conn.last_insert_rowid();
        debug!("Inserted event {} with id {} to local SQLite", event.event_id, id);

        // Write to PostgreSQL (asynchronous, non-blocking)
        if let Some(pg_writer) = &self.postgres {
            let peer_address = format!("{}:{}", event.peer_ip, event.peer_port);
            let classification = match &event.classification {
                PeerClassification::Public => Some("public".to_string()),
                PeerClassification::Private => Some("private".to_string()),
                PeerClassification::Unknown => None,
            };

            // Build metadata JSON
            let metadata = serde_json::json!({
                "protocol_version": event.protocol_version,
                "user_agent": event.user_agent,
                "network": event.network,
                "direction": event.direction,
                "probe_duration_ms": event.probe_duration_ms,
                "probe_error": event.probe_error,
            });

            let pg_event = PeerEvent {
                sensor_id: event.sensor_id.clone(),
                peer_address,
                event_type: "discovered".to_string(),
                classification,
                timestamp: event.timestamp.timestamp(),
                metadata: Some(metadata.to_string()),
            };

            if let Err(e) = pg_writer.queue_event(pg_event) {
                warn!("Failed to queue event for PostgreSQL: {}", e);
                // Don't fail the entire operation if PostgreSQL queueing fails
            } else {
                debug!("Queued event {} for PostgreSQL", event.event_id);
            }
        }

        Ok(id)
    }

    /// Get events that haven't been exported yet
    pub fn get_unexported_events(&self, limit: usize) -> StorageResult<Vec<PeerConnectionEvent>> {
        let conn = self.pool.get()?;

        let mut stmt = conn.prepare(
            "SELECT event_id, timestamp, sensor_id, peer_ip, peer_port,
                    protocol_version, user_agent, network, direction, classification,
                    probe_duration_ms, probe_error
             FROM peer_events
             WHERE exported = 0
             ORDER BY timestamp ASC
             LIMIT ?1",
        )?;

        let events = stmt
            .query_map([limit], |row| {
                let timestamp_str: String = row.get(1)?;
                let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
                    .map_err(|_e| rusqlite::Error::InvalidQuery)
                    .map(|dt| dt.with_timezone(&Utc))?;

                let peer_ip_str: String = row.get(3)?;
                let peer_ip = peer_ip_str.parse()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;

                let direction_str: String = row.get(8)?;
                let direction = serde_json::from_str(&direction_str)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;

                let classification_str: String = row.get(9)?;
                let classification = serde_json::from_str(&classification_str)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;

                Ok(PeerConnectionEvent {
                    event_id: row.get(0)?,
                    timestamp,
                    sensor_id: row.get(2)?,
                    peer_ip,
                    peer_port: row.get::<_, i64>(4)? as u16,
                    protocol_version: row.get::<_, i64>(5)? as u32,
                    user_agent: row.get(6)?,
                    network: row.get(7)?,
                    direction,
                    classification,
                    probe_duration_ms: row.get::<_, Option<i64>>(10)?.map(|d| d as u64),
                    probe_error: row.get(11)?,
                })
            })?
            .collect::<SqliteResult<Vec<_>>>()?;

        debug!("Retrieved {} unexported events", events.len());
        Ok(events)
    }

    /// Mark events as exported
    pub fn mark_events_exported(&self, event_ids: &[String]) -> StorageResult<usize> {
        let conn = self.pool.get()?;

        let placeholders = event_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!("UPDATE peer_events SET exported = 1 WHERE event_id IN ({})", placeholders);

        let params: Vec<&dyn rusqlite::ToSql> = event_ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        let updated = conn.execute(&query, params.as_slice())?;

        debug!("Marked {} events as exported", updated);
        Ok(updated)
    }

    /// Get total event count
    pub fn get_event_count(&self) -> StorageResult<i64> {
        let conn = self.pool.get()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM peer_events", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Get statistics
    pub fn get_statistics(&self) -> StorageResult<StorageStatistics> {
        let conn = self.pool.get()?;

        let total_events: i64 = conn.query_row("SELECT COUNT(*) FROM peer_events", [], |row| row.get(0))?;

        let exported_events: i64 = conn.query_row("SELECT COUNT(*) FROM peer_events WHERE exported = 1", [], |row| row.get(0))?;

        let public_peers: i64 = conn.query_row(
            "SELECT COUNT(*) FROM peer_events WHERE classification = ?1",
            [serde_json::to_string(&PeerClassification::Public)?],
            |row| row.get(0),
        )?;

        let private_peers: i64 = conn.query_row(
            "SELECT COUNT(*) FROM peer_events WHERE classification = ?1",
            [serde_json::to_string(&PeerClassification::Private)?],
            |row| row.get(0),
        )?;

        let oldest_event: Option<String> = conn
            .query_row("SELECT timestamp FROM peer_events ORDER BY timestamp ASC LIMIT 1", [], |row| row.get(0))
            .optional()?;

        let newest_event: Option<String> = conn
            .query_row("SELECT timestamp FROM peer_events ORDER BY timestamp DESC LIMIT 1", [], |row| row.get(0))
            .optional()?;

        Ok(StorageStatistics {
            total_events: total_events as usize,
            exported_events: exported_events as usize,
            pending_exports: (total_events - exported_events) as usize,
            public_peers: public_peers as usize,
            private_peers: private_peers as usize,
            oldest_event: oldest_event.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
            newest_event: newest_event.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
        })
    }

    /// Clean up old events beyond retention period
    pub fn cleanup_old_events(&self, retention_days: usize) -> StorageResult<usize> {
        let conn = self.pool.get()?;

        let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();

        let deleted = conn.execute(
            "DELETE FROM peer_events WHERE timestamp < ?1 AND exported = 1",
            params![cutoff_str],
        )?;

        if deleted > 0 {
            info!("Cleaned up {} old events beyond {} days retention", deleted, retention_days);
        }

        Ok(deleted)
    }

    /// Get database file size
    pub fn get_database_size(&self) -> StorageResult<u64> {
        let conn = self.pool.get()?;
        let path: String = conn.query_row("PRAGMA database_list", [], |row| row.get(2))?;
        let metadata = std::fs::metadata(&path)?;
        Ok(metadata.len())
    }

    /// Vacuum database to reclaim space
    pub fn vacuum(&self) -> StorageResult<()> {
        let conn = self.pool.get()?;
        conn.execute_batch("VACUUM;")?;
        info!("Database vacuumed");
        Ok(())
    }
}

/// Storage statistics
#[derive(Debug, Clone)]
pub struct StorageStatistics {
    pub total_events: usize,
    pub exported_events: usize,
    pub pending_exports: usize,
    pub public_peers: usize,
    pub private_peers: usize,
    pub oldest_event: Option<DateTime<Utc>>,
    pub newest_event: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConnectionDirection, PeerConnectionEvent};
    use std::net::IpAddr;
    use std::str::FromStr;

    #[test]
    fn test_storage_creation() {
        let config = DatabaseConfig {
            path: PathBuf::from(":memory:"),
            pool_size: 5,
            retention_days: 30,
            enable_wal: false,
            addressdb_path: PathBuf::from(":memory:"),
        };

        let storage = EventStorage::new(&config).unwrap();
        assert!(storage.get_event_count().unwrap() == 0);
    }

    #[test]
    fn test_insert_and_retrieve() {
        let config = DatabaseConfig {
            path: PathBuf::from(":memory:"),
            pool_size: 5,
            retention_days: 30,
            enable_wal: false,
            addressdb_path: PathBuf::from(":memory:"),
        };

        let storage = EventStorage::new(&config).unwrap();

        let event = PeerConnectionEvent::new(
            "test-sensor".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        storage.insert_event(&event).unwrap();

        let events = storage.get_unexported_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sensor_id, "test-sensor");
    }

    #[test]
    fn test_mark_exported() {
        let config = DatabaseConfig {
            path: PathBuf::from(":memory:"),
            pool_size: 5,
            retention_days: 30,
            enable_wal: false,
            addressdb_path: PathBuf::from(":memory:"),
        };

        let storage = EventStorage::new(&config).unwrap();

        let event = PeerConnectionEvent::new(
            "test-sensor".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        );

        storage.insert_event(&event).unwrap();

        let events = storage.get_unexported_events(10).unwrap();
        assert_eq!(events.len(), 1);

        let event_ids: Vec<String> = events.iter().map(|e| e.event_id.clone()).collect();
        storage.mark_events_exported(&event_ids).unwrap();

        let remaining = storage.get_unexported_events(10).unwrap();
        assert_eq!(remaining.len(), 0);
    }

    #[test]
    fn test_statistics() {
        let config = DatabaseConfig {
            path: PathBuf::from(":memory:"),
            pool_size: 5,
            retention_days: 30,
            enable_wal: false,
            addressdb_path: PathBuf::from(":memory:"),
        };

        let storage = EventStorage::new(&config).unwrap();

        let event = PeerConnectionEvent::new(
            "test-sensor".to_string(),
            IpAddr::from_str("203.0.113.1").unwrap(),
            16111,
            7,
            "kaspad:0.14.0".to_string(),
            "kaspa-mainnet".to_string(),
            ConnectionDirection::Inbound,
        ).with_probe_result(PeerClassification::Public, 250, None);

        storage.insert_event(&event).unwrap();

        let stats = storage.get_statistics().unwrap();
        assert_eq!(stats.total_events, 1);
        assert_eq!(stats.public_peers, 1);
        assert_eq!(stats.private_peers, 0);
    }
}
