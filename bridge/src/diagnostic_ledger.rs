use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{OpenOptions, create_dir_all};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: &str = "ks5-local-stratum-diagnostic-ledger/v2";
pub const ENV_JSONL_PATH: &str = "RKSTRATUM_DIAGNOSTIC_JSONL";
pub const ENV_RUN_ID: &str = "RKSTRATUM_DIAGNOSTIC_RUN_ID";
pub const ENV_SESSION_ID: &str = "RKSTRATUM_DIAGNOSTIC_SESSION_ID";
pub const ENV_REQUEST_OWNER: &str = "RKSTRATUM_DIAGNOSTIC_REQUEST_OWNER";
pub const ENV_STRATUM_LISTENER: &str = "RKSTRATUM_DIAGNOSTIC_STRATUM_LISTENER";
pub const ENV_NODE_RPC: &str = "RKSTRATUM_DIAGNOSTIC_NODE_RPC";
pub const PRE_SUBMIT_QUOTE_SCHEMA_VERSION: &str = "ks5-local-stratum-pre-submit-quote/v1";
pub const ENV_PRE_SUBMIT_QUOTE_JSONL_PATH: &str = "RKSTRATUM_DIAGNOSTIC_QUOTE_JSONL";
pub const ENV_BRIDGE_COMMIT: &str = "RKSTRATUM_DIAGNOSTIC_BRIDGE_COMMIT";
pub const ENV_TARGET_OVERRIDE_HEX: &str = "RKSTRATUM_DIAGNOSTIC_TARGET_OVERRIDE_HEX";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLedgerConfig {
    pub path: PathBuf,
    pub run_id: String,
    pub session_id: String,
    pub request_owner: String,
    pub stratum_listener: String,
    pub node_rpc: String,
}

impl DiagnosticLedgerConfig {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os(ENV_JSONL_PATH)?;
        if path.is_empty() {
            return None;
        }

        Some(Self {
            path: PathBuf::from(path),
            run_id: env_or_default(ENV_RUN_ID, "unset"),
            session_id: env_or_default(ENV_SESSION_ID, "unset"),
            request_owner: env_or_default(ENV_REQUEST_OWNER, "unknown"),
            stratum_listener: env_or_default(ENV_STRATUM_LISTENER, "unknown"),
            node_rpc: env_or_default(ENV_NODE_RPC, "unknown"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticPreSubmitQuoteConfig {
    pub path: PathBuf,
    pub run_id: String,
    pub session_id: String,
    pub request_owner: String,
    pub stratum_listener: String,
    pub node_rpc: String,
    pub bridge_commit: String,
}

impl DiagnosticPreSubmitQuoteConfig {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os(ENV_PRE_SUBMIT_QUOTE_JSONL_PATH)?;
        if path.is_empty() {
            return None;
        }

        Some(Self {
            path: PathBuf::from(path),
            run_id: env_or_default(ENV_RUN_ID, "unset"),
            session_id: env_or_default(ENV_SESSION_ID, "unset"),
            request_owner: env_or_default(ENV_REQUEST_OWNER, "unknown"),
            stratum_listener: env_or_default(ENV_STRATUM_LISTENER, "unknown"),
            node_rpc: env_or_default(ENV_NODE_RPC, "unknown"),
            bridge_commit: env_or_default(ENV_BRIDGE_COMMIT, "unknown"),
        })
    }
}

fn env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|value| !value.trim().is_empty()).unwrap_or_else(|| default.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticOutcome {
    Accepted,
    Weak,
    Stale,
    Bad,
    Duplicate,
}

impl DiagnosticOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Weak => "weak",
            Self::Stale => "stale",
            Self::Bad => "bad",
            Self::Duplicate => "duplicate",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticCounters {
    pub valid_shares: u64,
    pub weak_shares: u64,
    pub stale_shares: u64,
    pub bad_shares: u64,
    pub duplicate_submits: u64,
}

impl DiagnosticCounters {
    pub fn increment(&mut self, outcome: DiagnosticOutcome) {
        match outcome {
            DiagnosticOutcome::Accepted => self.valid_shares += 1,
            DiagnosticOutcome::Weak => self.weak_shares += 1,
            DiagnosticOutcome::Stale => self.stale_shares += 1,
            DiagnosticOutcome::Bad => self.bad_shares += 1,
            DiagnosticOutcome::Duplicate => self.duplicate_submits += 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SameWorkIdentity {
    pub job_id_matches_notify: bool,
    pub nonce_matches_request: bool,
    pub bridge_job_found: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticJsonRpcError {
    pub code: i32,
    pub category: String,
    pub message_class: String,
}

impl DiagnosticJsonRpcError {
    pub fn stale() -> Self {
        Self { code: 21, category: "stale".to_string(), message_class: "job_not_found".to_string() }
    }

    pub fn weak() -> Self {
        Self { code: 23, category: "weak".to_string(), message_class: "invalid_difficulty".to_string() }
    }

    pub fn bad() -> Self {
        Self { code: 20, category: "bad".to_string(), message_class: "unknown_problem".to_string() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticShareQuality {
    pub configured_share_difficulty: f64,
    pub bridge_target: String,
    pub pow_value: String,
    pub pow_lt_target: bool,
    pub validation_job_id: String,
    pub fallback_job_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticSubmitRecord {
    pub schema_version: String,
    pub timestamp_utc: String,
    pub run_id: String,
    pub session_id: String,
    pub connection_id: String,
    pub request_owner: String,
    pub worker: String,
    pub wallet: String,
    pub remote_addr: String,
    pub stratum_listener: String,
    pub node_rpc: String,
    pub request_id: Value,
    pub response_id: Value,
    pub job_id: String,
    pub submitted_nonce: String,
    pub final_nonce: String,
    pub same_work_identity: SameWorkIdentity,
    pub configured_share_difficulty: Option<f64>,
    pub bridge_target: Option<String>,
    pub pow_value: Option<String>,
    pub pow_lt_target: Option<bool>,
    pub validation_job_id: Option<String>,
    pub fallback_job_id: Option<String>,
    pub bridge_outcome: String,
    pub jsonrpc_result: Option<bool>,
    pub jsonrpc_error: Option<DiagnosticJsonRpcError>,
    pub reject_class: String,
    pub counters_before: DiagnosticCounters,
    pub counters_after: DiagnosticCounters,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticPreSubmitQuoteRecord {
    pub schema_version: String,
    pub timestamp_utc: String,
    pub run_id: String,
    pub session_id: String,
    pub quote_id: String,
    pub bridge_commit: String,
    pub diagnostic_method: String,
    pub connection_id: String,
    pub request_owner: String,
    pub worker_identity: String,
    pub wallet_identity: String,
    pub remote_addr: String,
    pub stratum_listener: String,
    pub node_rpc: String,
    pub request_id: Value,
    pub submitted_job_id: String,
    pub validation_job_id: String,
    pub fallback_job_id: Option<String>,
    pub bridge_job_found: bool,
    pub submitted_nonce: String,
    pub final_nonce: String,
    pub configured_share_difficulty: f64,
    pub bridge_target: String,
    pub pow_value: String,
    pub pow_lt_target: bool,
    pub block_timestamp: u64,
    pub block_bits: u32,
    pub pre_pow_hash: String,
    pub notify_job_match: bool,
    pub quote_does_not_submit: bool,
    pub quote_does_not_touch_node: bool,
}

#[allow(clippy::too_many_arguments)]
impl DiagnosticPreSubmitQuoteRecord {
    pub fn new(
        config: &DiagnosticPreSubmitQuoteConfig,
        quote_id: String,
        diagnostic_method: &str,
        connection_id: String,
        worker_identity: String,
        wallet_identity: String,
        remote_addr: String,
        request_id: Value,
        submitted_job_id: String,
        validation_job_id: String,
        fallback_job_id: Option<String>,
        bridge_job_found: bool,
        submitted_nonce: String,
        final_nonce: String,
        configured_share_difficulty: f64,
        bridge_target: String,
        pow_value: String,
        pow_lt_target: bool,
        block_timestamp: u64,
        block_bits: u32,
        pre_pow_hash: String,
        notify_job_match: bool,
    ) -> Self {
        Self {
            schema_version: PRE_SUBMIT_QUOTE_SCHEMA_VERSION.to_string(),
            timestamp_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            run_id: config.run_id.clone(),
            session_id: config.session_id.clone(),
            quote_id,
            bridge_commit: config.bridge_commit.clone(),
            diagnostic_method: diagnostic_method.to_string(),
            connection_id,
            request_owner: config.request_owner.clone(),
            worker_identity,
            wallet_identity,
            remote_addr,
            stratum_listener: config.stratum_listener.clone(),
            node_rpc: config.node_rpc.clone(),
            request_id,
            submitted_job_id,
            validation_job_id,
            fallback_job_id,
            bridge_job_found,
            submitted_nonce,
            final_nonce,
            configured_share_difficulty,
            bridge_target,
            pow_value,
            pow_lt_target,
            block_timestamp,
            block_bits,
            pre_pow_hash,
            notify_job_match,
            quote_does_not_submit: true,
            quote_does_not_touch_node: true,
        }
    }
}

#[allow(clippy::too_many_arguments)]
impl DiagnosticSubmitRecord {
    pub fn new(
        config: &DiagnosticLedgerConfig,
        connection_id: String,
        worker: String,
        wallet: String,
        remote_addr: String,
        request_id: Value,
        response_id: Value,
        job_id: String,
        submitted_nonce: String,
        final_nonce: String,
        same_work_identity: SameWorkIdentity,
        share_quality: Option<DiagnosticShareQuality>,
        outcome: DiagnosticOutcome,
        jsonrpc_result: Option<bool>,
        jsonrpc_error: Option<DiagnosticJsonRpcError>,
        reject_class: &str,
        counters_before: DiagnosticCounters,
        counters_after: DiagnosticCounters,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            timestamp_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            run_id: config.run_id.clone(),
            session_id: config.session_id.clone(),
            connection_id,
            request_owner: config.request_owner.clone(),
            worker,
            wallet,
            remote_addr,
            stratum_listener: config.stratum_listener.clone(),
            node_rpc: config.node_rpc.clone(),
            request_id,
            response_id,
            job_id,
            submitted_nonce,
            final_nonce,
            same_work_identity,
            configured_share_difficulty: share_quality.as_ref().map(|quality| quality.configured_share_difficulty),
            bridge_target: share_quality.as_ref().map(|quality| quality.bridge_target.clone()),
            pow_value: share_quality.as_ref().map(|quality| quality.pow_value.clone()),
            pow_lt_target: share_quality.as_ref().map(|quality| quality.pow_lt_target),
            validation_job_id: share_quality.as_ref().map(|quality| quality.validation_job_id.clone()),
            fallback_job_id: share_quality.as_ref().and_then(|quality| quality.fallback_job_id.clone()),
            bridge_outcome: outcome.as_str().to_string(),
            jsonrpc_result,
            jsonrpc_error,
            reject_class: reject_class.to_string(),
            counters_before,
            counters_after,
        }
    }
}

pub fn append_submit_record(path: &Path, record: &DiagnosticSubmitRecord) -> io::Result<()> {
    append_jsonl(path, record)
}

pub fn append_pre_submit_quote_record(path: &Path, record: &DiagnosticPreSubmitQuoteRecord) -> io::Result<()> {
    append_jsonl(path, record)
}

fn append_jsonl<T: Serialize>(path: &Path, record: &T) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, record).map_err(io::Error::other)?;
    writeln!(file)?;
    Ok(())
}

pub fn canonical_nonce_hex(value: &str) -> String {
    format!("0x{}", value.trim_start_matches("0x").trim_start_matches("0X").to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config(path: PathBuf) -> DiagnosticLedgerConfig {
        DiagnosticLedgerConfig {
            path,
            run_id: "w2191-fixture".to_string(),
            session_id: "session-local".to_string(),
            request_owner: "open_controller".to_string(),
            stratum_listener: "10.0.4.30:16120".to_string(),
            node_rpc: "10.0.4.30:16110".to_string(),
        }
    }

    fn quote_config(path: PathBuf) -> DiagnosticPreSubmitQuoteConfig {
        DiagnosticPreSubmitQuoteConfig {
            path,
            run_id: "w2247-fixture".to_string(),
            session_id: "session-local".to_string(),
            request_owner: "open_controller".to_string(),
            stratum_listener: "10.0.4.30:16120".to_string(),
            node_rpc: "10.0.4.30:16110".to_string(),
            bridge_commit: "test-commit".to_string(),
        }
    }

    fn temp_jsonl_path() -> PathBuf {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("rkstratum-diagnostic-ledger-{unique}.jsonl"))
    }

    #[test]
    fn counter_delta_matches_each_outcome() {
        for (outcome, expected_field) in [
            (DiagnosticOutcome::Accepted, "valid_shares"),
            (DiagnosticOutcome::Weak, "weak_shares"),
            (DiagnosticOutcome::Stale, "stale_shares"),
            (DiagnosticOutcome::Bad, "bad_shares"),
            (DiagnosticOutcome::Duplicate, "duplicate_submits"),
        ] {
            let before = DiagnosticCounters::default();
            let mut after = before.clone();
            after.increment(outcome);
            let encoded = serde_json::to_value(after).unwrap();
            assert_eq!(encoded[expected_field], json!(1));
        }
    }

    #[test]
    fn appends_w2189_schema_row_without_raw_fields() {
        let path = temp_jsonl_path();
        let cfg = config(path.clone());
        let before = DiagnosticCounters::default();
        let mut after = before.clone();
        after.increment(DiagnosticOutcome::Accepted);
        let record = DiagnosticSubmitRecord::new(
            &cfg,
            "10.0.0.197:41000#7".to_string(),
            "worker_1".to_string(),
            "kaspa:qfixture".to_string(),
            "10.0.0.197:41000".to_string(),
            json!(21),
            json!(21),
            "1001".to_string(),
            canonical_nonce_hex("0XABC"),
            canonical_nonce_hex("0000000000000abc"),
            SameWorkIdentity { job_id_matches_notify: true, nonce_matches_request: true, bridge_job_found: true },
            Some(DiagnosticShareQuality {
                configured_share_difficulty: 2048.0,
                bridge_target: "0x000fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
                pow_value: "0x0007ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
                pow_lt_target: true,
                validation_job_id: "1001".to_string(),
                fallback_job_id: None,
            }),
            DiagnosticOutcome::Accepted,
            Some(true),
            None,
            "none",
            before,
            after,
        );

        append_submit_record(&cfg.path, &record).unwrap();
        let line = fs::read_to_string(&path).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        fs::remove_file(&path).ok();

        assert_eq!(decoded["schema_version"], SCHEMA_VERSION);
        assert_eq!(decoded["bridge_outcome"], "accepted");
        assert_eq!(decoded["request_owner"], "open_controller");
        assert_eq!(decoded["request_id"], decoded["response_id"]);
        assert_eq!(decoded["configured_share_difficulty"], json!(2048.0));
        assert_eq!(decoded["pow_lt_target"], json!(true));
        assert_eq!(decoded["validation_job_id"], json!("1001"));
        assert_eq!(decoded["fallback_job_id"], serde_json::Value::Null);
        assert_eq!(decoded["counters_before"]["valid_shares"], json!(0));
        assert_eq!(decoded["counters_after"]["valid_shares"], json!(1));
        assert!(decoded.get("raw_line").is_none());
        assert!(decoded.get("password").is_none());
        assert!(decoded.get("authorize_params").is_none());
    }

    #[test]
    fn appends_pre_submit_quote_schema_row() {
        let path = temp_jsonl_path();
        let cfg = quote_config(path.clone());
        let record = DiagnosticPreSubmitQuoteRecord::new(
            &cfg,
            "quote-1".to_string(),
            "mining.diagnostic_quote",
            "10.0.0.197:41000#7".to_string(),
            "worker_1".to_string(),
            "kaspa:qfixture".to_string(),
            "10.0.0.197:41000".to_string(),
            json!(21),
            "1001".to_string(),
            "1001".to_string(),
            None,
            true,
            canonical_nonce_hex("0XABC"),
            canonical_nonce_hex("0000000000000abc"),
            2048.0,
            "0x000fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
            "0x0007ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
            true,
            1234567890,
            0x1a2b3c4d,
            "00".repeat(32),
            true,
        );

        append_pre_submit_quote_record(&cfg.path, &record).unwrap();
        let line = fs::read_to_string(&path).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        fs::remove_file(&path).ok();

        assert_eq!(decoded["schema_version"], PRE_SUBMIT_QUOTE_SCHEMA_VERSION);
        assert_eq!(decoded["quote_id"], json!("quote-1"));
        assert_eq!(decoded["diagnostic_method"], json!("mining.diagnostic_quote"));
        assert_eq!(decoded["submitted_job_id"], decoded["validation_job_id"]);
        assert_eq!(decoded["fallback_job_id"], serde_json::Value::Null);
        assert_eq!(decoded["bridge_job_found"], json!(true));
        assert_eq!(decoded["pow_lt_target"], json!(true));
        assert_eq!(decoded["quote_does_not_submit"], json!(true));
        assert_eq!(decoded["quote_does_not_touch_node"], json!(true));
    }

    #[test]
    fn reject_error_classes_match_bridge_responses() {
        assert_eq!(DiagnosticJsonRpcError::stale().code, 21);
        assert_eq!(DiagnosticJsonRpcError::weak().message_class, "invalid_difficulty");
        assert_eq!(DiagnosticJsonRpcError::bad().category, "bad");
        assert_eq!(canonical_nonce_hex("0XDEADBEEF"), "0xdeadbeef");
    }
}
