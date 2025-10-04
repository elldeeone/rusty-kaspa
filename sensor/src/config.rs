use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("Invalid configuration: {0}")]
    ValidationError(String),
}

/// Complete configuration for the sensor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorConfig {
    /// Sensor identification
    pub sensor: SensorIdentity,

    /// Network settings
    pub network: NetworkConfig,

    /// Probing configuration
    pub probing: ProbingConfig,

    /// Data export configuration
    pub export: ExportConfig,

    /// Local database configuration
    pub database: DatabaseConfig,

    /// Metrics configuration
    pub metrics: MetricsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorIdentity {
    /// Unique identifier for this sensor instance
    pub sensor_id: String,

    /// Optional description/location
    #[serde(default)]
    pub description: Option<String>,

    /// Deployment environment (e.g., "aws-us-east-1", "do-sgp1")
    #[serde(default)]
    pub environment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Address to listen on
    #[serde(default = "default_listen_address")]
    pub listen_address: String,

    /// Network type (mainnet, testnet, devnet)
    #[serde(default = "default_network_type")]
    pub network_type: String,

    /// Maximum concurrent inbound connections
    #[serde(default = "default_max_inbound_connections")]
    pub max_inbound_connections: usize,

    /// Maximum concurrent outbound connections
    #[serde(default = "default_max_outbound_connections")]
    pub max_outbound_connections: usize,

    /// DNS seeders to query for initial peers
    #[serde(default = "default_dns_seeders")]
    pub dns_seeders: Vec<String>,

    /// Number of peers to fetch from each seeder
    #[serde(default = "default_peers_per_seeder")]
    pub peers_per_seeder: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbingConfig {
    /// Enable active probing
    #[serde(default = "default_enable_probing")]
    pub enabled: bool,

    /// Timeout for probe attempts (milliseconds)
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u64,

    /// Delay before probing after handshake (milliseconds)
    #[serde(default = "default_probe_delay_ms")]
    pub delay_ms: u64,

    /// Maximum concurrent probes
    #[serde(default = "default_max_concurrent_probes")]
    pub max_concurrent_probes: usize,

    /// Skip probing private IP addresses
    #[serde(default = "default_skip_private_ips")]
    pub skip_private_ips: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    /// Enable data export
    #[serde(default = "default_enable_export")]
    pub enabled: bool,

    /// Export backend type (firestore, http, webhook)
    #[serde(default = "default_export_backend")]
    pub backend: String,

    /// Export endpoint URL
    #[serde(default)]
    pub endpoint: Option<String>,

    /// API key or authentication token
    #[serde(default)]
    pub api_key: Option<String>,

    /// Batch size for exports
    #[serde(default = "default_export_batch_size")]
    pub batch_size: usize,

    /// Batch interval (seconds)
    #[serde(default = "default_export_interval_secs")]
    pub interval_secs: u64,

    /// Maximum retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,

    /// Retry backoff multiplier
    #[serde(default = "default_retry_backoff_secs")]
    pub retry_backoff_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to SQLite database file
    #[serde(default = "default_db_path")]
    pub path: PathBuf,

    /// Connection pool size
    #[serde(default = "default_db_pool_size")]
    pub pool_size: u32,

    /// Retention period for events (days)
    #[serde(default = "default_retention_days")]
    pub retention_days: usize,

    /// Enable WAL mode for better concurrency
    #[serde(default = "default_enable_wal")]
    pub enable_wal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Enable Prometheus metrics
    #[serde(default = "default_enable_metrics")]
    pub enabled: bool,

    /// Metrics HTTP endpoint
    #[serde(default = "default_metrics_address")]
    pub address: String,
}

// Default value functions
fn default_listen_address() -> String {
    "0.0.0.0:16111".to_string()
}

fn default_network_type() -> String {
    "mainnet".to_string()
}

fn default_max_inbound_connections() -> usize {
    500
}

fn default_max_outbound_connections() -> usize {
    50
}

fn default_dns_seeders() -> Vec<String> {
    vec![
        "seeder1.kaspad.net".to_string(),
        "seeder2.kaspad.net".to_string(),
        "seeder3.kaspad.net".to_string(),
    ]
}

fn default_peers_per_seeder() -> usize {
    10
}

fn default_enable_probing() -> bool {
    true
}

fn default_probe_timeout_ms() -> u64 {
    5000
}

fn default_probe_delay_ms() -> u64 {
    100
}

fn default_max_concurrent_probes() -> usize {
    100
}

fn default_skip_private_ips() -> bool {
    true
}

fn default_enable_export() -> bool {
    true
}

fn default_export_backend() -> String {
    "http".to_string()
}

fn default_export_batch_size() -> usize {
    100
}

fn default_export_interval_secs() -> u64 {
    60
}

fn default_max_retries() -> usize {
    3
}

fn default_retry_backoff_secs() -> u64 {
    5
}

fn default_db_path() -> PathBuf {
    PathBuf::from("sensor-data.db")
}

fn default_db_pool_size() -> u32 {
    10
}

fn default_retention_days() -> usize {
    30
}

fn default_enable_wal() -> bool {
    true
}

fn default_enable_metrics() -> bool {
    true
}

fn default_metrics_address() -> String {
    "0.0.0.0:9090".to_string()
}

impl Default for SensorConfig {
    fn default() -> Self {
        Self {
            sensor: SensorIdentity {
                sensor_id: "default-sensor".to_string(),
                description: None,
                environment: None,
            },
            network: NetworkConfig {
                listen_address: default_listen_address(),
                network_type: default_network_type(),
                max_inbound_connections: default_max_inbound_connections(),
                max_outbound_connections: default_max_outbound_connections(),
                dns_seeders: default_dns_seeders(),
                peers_per_seeder: default_peers_per_seeder(),
            },
            probing: ProbingConfig {
                enabled: default_enable_probing(),
                timeout_ms: default_probe_timeout_ms(),
                delay_ms: default_probe_delay_ms(),
                max_concurrent_probes: default_max_concurrent_probes(),
                skip_private_ips: default_skip_private_ips(),
            },
            export: ExportConfig {
                enabled: default_enable_export(),
                backend: default_export_backend(),
                endpoint: None,
                api_key: None,
                batch_size: default_export_batch_size(),
                interval_secs: default_export_interval_secs(),
                max_retries: default_max_retries(),
                retry_backoff_secs: default_retry_backoff_secs(),
            },
            database: DatabaseConfig {
                path: default_db_path(),
                pool_size: default_db_pool_size(),
                retention_days: default_retention_days(),
                enable_wal: default_enable_wal(),
            },
            metrics: MetricsConfig {
                enabled: default_enable_metrics(),
                address: default_metrics_address(),
            },
        }
    }
}

impl SensorConfig {
    /// Load configuration from a TOML file
    pub fn from_file(path: &PathBuf) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: SensorConfig = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.sensor.sensor_id.is_empty() {
            return Err(ConfigError::ValidationError("sensor_id cannot be empty".to_string()));
        }

        if self.network.max_inbound_connections == 0 {
            return Err(ConfigError::ValidationError("max_inbound_connections must be > 0".to_string()));
        }

        if self.probing.timeout_ms == 0 {
            return Err(ConfigError::ValidationError("probe timeout must be > 0".to_string()));
        }

        if self.export.enabled && self.export.endpoint.is_none() {
            return Err(ConfigError::ValidationError("export endpoint required when export is enabled".to_string()));
        }

        Ok(())
    }

    /// Save configuration to a TOML file
    pub fn save_to_file(&self, path: &PathBuf) -> Result<(), ConfigError> {
        let contents = toml::to_string_pretty(self)
            .map_err(|e| ConfigError::ValidationError(format!("Failed to serialize config: {}", e)))?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Create a default configuration file
    pub fn create_default_config_file(path: &PathBuf) -> Result<(), ConfigError> {
        let default = Self::default();
        default.save_to_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SensorConfig::default();
        assert_eq!(config.sensor.sensor_id, "default-sensor");
        assert_eq!(config.network.listen_address, "0.0.0.0:16111");
        assert!(config.probing.enabled);
    }

    #[test]
    fn test_config_validation() {
        let mut config = SensorConfig::default();
        assert!(config.validate().is_ok());

        config.sensor.sensor_id = "".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_serialization() {
        let config = SensorConfig::default();
        let toml_str = toml::to_string(&config).unwrap();
        let deserialized: SensorConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config.sensor.sensor_id, deserialized.sensor.sensor_id);
    }
}
