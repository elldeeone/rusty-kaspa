use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kaspa_grpc_client::GrpcClient;
use kaspa_rpc_core::api::rpc::RpcApi;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Parser, Debug)]
#[command(name = "udp-rpc-digests")]
#[command(about = "Query UDP digest ingest RPCs via gRPC for lab verification.")]
struct Args {
    /// gRPC URL, e.g. grpc://127.0.0.1:16110.
    #[arg(long, default_value = "grpc://127.0.0.1:16110")]
    rpc_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Query getUdpIngestInfo.
    Info,
    /// Query getUdpDigests.
    Digests {
        /// Optional lower epoch bound.
        #[arg(long)]
        from_epoch: Option<u64>,

        /// Optional result limit.
        #[arg(long)]
        limit: Option<u32>,

        /// Print a compact receiver-side verification summary after the JSON response.
        #[arg(long)]
        check_monotonic: bool,

        /// Compare latest produced snapshot provenance from this log against received RPC fields.
        #[arg(long)]
        producer_log: Option<PathBuf>,

        /// Compare latest received snapshot against this receiver node's local consensus RPC state.
        #[arg(long)]
        compare_local: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = GrpcClient::connect(args.rpc_url.clone()).await.context("connect grpc")?;

    match args.command {
        Command::Info => {
            let response = client.get_udp_ingest_info(None).await.context("get_udp_ingest_info")?;
            println!("{}", serde_json::to_string_pretty(&response).context("serialize info response")?);
        }
        Command::Digests { from_epoch, limit, check_monotonic, producer_log, compare_local } => {
            let response = client.get_udp_digests(from_epoch, limit, None).await.context("get_udp_digests")?;
            println!("{}", serde_json::to_string_pretty(&response).context("serialize digests response")?);
            if check_monotonic {
                print_monotonic_summary(&response);
            }
            if let Some(path) = producer_log {
                compare_producer_log(&path, &response)?;
            }
            if compare_local {
                compare_local_state(&client, &response).await?;
            }
        }
    }

    client.disconnect().await.context("disconnect grpc")?;
    Ok(())
}

fn print_monotonic_summary(response: &kaspa_rpc_core::GetUdpDigestsResponse) {
    let mut digests: Vec<_> = response.digests.iter().collect();
    digests.sort_by_key(|record| record.summary.epoch);

    let all_signature_valid = digests.iter().all(|record| record.summary.signature_valid);
    let epoch_monotonic = digests.windows(2).all(|pair| pair[0].summary.epoch <= pair[1].summary.epoch);
    let daa_monotonic = digests.windows(2).all(|pair| pair[0].summary.daa_score <= pair[1].summary.daa_score);
    let blue_score_monotonic = digests.windows(2).all(|pair| pair[0].summary.virtual_blue_score <= pair[1].summary.virtual_blue_score);
    let sources: std::collections::BTreeSet<_> = digests.iter().map(|record| record.summary.source_id).collect();
    let signers: std::collections::BTreeSet<_> = digests.iter().map(|record| record.summary.signer_id).collect();

    eprintln!(
        "udp_digest_check count={} all_signature_valid={} epoch_monotonic={} daa_score_monotonic={} virtual_blue_score_monotonic={} sources={:?} signers={:?}",
        digests.len(),
        all_signature_valid,
        epoch_monotonic,
        daa_monotonic,
        blue_score_monotonic,
        sources,
        signers
    );
}

fn compare_producer_log(path: &PathBuf, response: &kaspa_rpc_core::GetUdpDigestsResponse) -> Result<()> {
    let log = std::fs::read_to_string(path).with_context(|| format!("read producer log {}", path.display()))?;
    let produced = latest_snapshot_provenance(&log).context("producer log did not contain snapshot provenance")?;
    let received = latest_received_snapshot(response).context("RPC response did not contain a received snapshot")?;
    let summary = &received.summary;

    let virtual_blue_score = summary.virtual_blue_score.to_string();
    let daa_score = summary.daa_score.to_string();
    let signer_id = summary.signer_id.to_string();
    let source_id = summary.source_id.to_string();
    let checks = [
        ("pruning_point", produced.get("pruning_point").map(String::as_str), summary.pruning_point.as_deref()),
        ("utxo_muhash", produced.get("utxo_muhash").map(String::as_str), summary.utxo_muhash.as_deref()),
        (
            "virtual_selected_parent",
            produced.get("virtual_selected_parent").map(String::as_str),
            Some(summary.virtual_selected_parent.as_str()),
        ),
        ("virtual_blue_score", produced.get("virtual_blue_score").map(String::as_str), Some(virtual_blue_score.as_str())),
        ("daa_score", produced.get("daa_score").map(String::as_str), Some(daa_score.as_str())),
        ("blue_work", produced.get("blue_work").map(String::as_str), Some(summary.blue_work_hex.as_str())),
        ("signer_id", produced.get("signer_id").map(String::as_str), Some(signer_id.as_str())),
        ("source_id", produced.get("source_id").map(String::as_str), Some(source_id.as_str())),
    ];

    let mismatches: Vec<_> =
        checks
            .iter()
            .filter_map(|(field, produced, received)| {
                if produced == received {
                    None
                } else {
                    Some(format!("{field}: produced={produced:?} received={received:?}"))
                }
            })
            .collect();
    eprintln!(
        "udp_digest_compare snapshot_match={} compared_fields={} mismatches={:?}",
        mismatches.is_empty(),
        checks.len(),
        mismatches
    );
    anyhow::ensure!(mismatches.is_empty(), "producer provenance did not match received snapshot fields");
    Ok(())
}

async fn compare_local_state(client: &GrpcClient, response: &kaspa_rpc_core::GetUdpDigestsResponse) -> Result<()> {
    let received = latest_received_snapshot(response).context("RPC response did not contain a received snapshot")?;
    let summary = &received.summary;
    let dag = client.get_block_dag_info().await.context("get_block_dag_info")?;
    let sink_blue_score = client.get_sink_blue_score().await.context("get_sink_blue_score")?;
    let sink_block = client.get_block(dag.sink, false).await.ok();
    let local_blue_work = sink_block.as_ref().map(|block| {
        let bytes = block.header.blue_work.to_be_bytes();
        let mut out = [0u8; 32];
        out[32 - bytes.len()..].copy_from_slice(&bytes);
        faster_hex::hex_string(&out)
    });

    let local_pruning_point = dag.pruning_point_hash.to_string();
    let local_sink = dag.sink.to_string();
    let local_blue_score = sink_blue_score.to_string();
    let local_daa_score = dag.virtual_daa_score.to_string();
    let received_blue_score = summary.virtual_blue_score.to_string();
    let received_daa_score = summary.daa_score.to_string();
    let checks = [
        ("pruning_point", summary.pruning_point.as_deref(), Some(local_pruning_point.as_str())),
        ("virtual_selected_parent", Some(summary.virtual_selected_parent.as_str()), Some(local_sink.as_str())),
        ("virtual_blue_score", Some(received_blue_score.as_str()), Some(local_blue_score.as_str())),
        ("daa_score", Some(received_daa_score.as_str()), Some(local_daa_score.as_str())),
        ("blue_work", Some(summary.blue_work_hex.as_str()), local_blue_work.as_deref()),
    ];

    let mismatches: Vec<_> = checks
        .iter()
        .filter_map(
            |(field, received, local)| {
                if received == local {
                    None
                } else {
                    Some(format!("{field}: received={received:?} local={local:?}"))
                }
            },
        )
        .collect();
    let info = client.get_udp_ingest_info(None).await.context("get_udp_ingest_info")?;
    let compared_at_ms = SystemTime::now().duration_since(UNIX_EPOCH).context("system clock before UNIX epoch")?.as_millis();
    eprintln!(
        "udp_digest_local_compare agreement={} compared_fields={} mismatches={:?} divergence_detected={} divergence_epoch={:?} compared_at_ms={} source_id={} signer_id={} recv_ts_ms={}",
        mismatches.is_empty(),
        checks.len(),
        mismatches,
        info.divergence.detected,
        info.divergence.last_mismatch_epoch,
        compared_at_ms,
        summary.source_id,
        summary.signer_id,
        summary.recv_ts_ms
    );
    Ok(())
}

fn latest_snapshot_provenance(log: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let mut latest = None;
    for line in log.lines() {
        if !line.contains("\"event\":\"udp_live_digest_provenance\"") || !line.contains("\"kind\":\"snapshot\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let fields = value.get("fields")?.as_array()?;
        let mut map = std::collections::BTreeMap::new();
        for field in fields {
            let name = field.get("field")?.as_str()?.to_string();
            let value = field.get("value")?;
            let value = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Number(value) => value.to_string(),
                serde_json::Value::Bool(value) => value.to_string(),
                serde_json::Value::Null => "null".to_string(),
                _ => continue,
            };
            map.insert(name, value);
        }
        latest = Some(map);
    }
    latest
}

fn latest_received_snapshot(response: &kaspa_rpc_core::GetUdpDigestsResponse) -> Option<&kaspa_rpc_core::RpcUdpDigestRecord> {
    response.digests.iter().filter(|record| record.kind == "snapshot").max_by_key(|record| record.summary.epoch)
}

#[cfg(test)]
mod tests {
    use kaspa_rpc_core::{RpcUdpDigestRecord, RpcUdpDigestSummary};
    use std::path::PathBuf;

    fn record(epoch: u64, daa_score: u64, virtual_blue_score: u64, signature_valid: bool) -> RpcUdpDigestRecord {
        RpcUdpDigestRecord {
            epoch,
            kind: "delta".to_string(),
            summary: RpcUdpDigestSummary {
                epoch,
                pruning_point: None,
                pruning_proof_commitment: None,
                utxo_muhash: None,
                virtual_selected_parent: "00".repeat(32),
                virtual_blue_score,
                daa_score,
                blue_work_hex: "00".repeat(32),
                kept_headers_mmr_root: None,
                signer_id: 0,
                signature_valid,
                recv_ts_ms: 0,
                source_id: 7,
            },
            verified: signature_valid,
        }
    }

    #[test]
    fn monotonic_summary_accepts_reverse_order_rpc_results() {
        let response = kaspa_rpc_core::GetUdpDigestsResponse { digests: vec![record(2, 20, 200, true), record(1, 10, 100, true)] };
        super::print_monotonic_summary(&response);
    }

    #[test]
    fn latest_snapshot_provenance_extracts_comparable_fields() {
        let log = r#"{"event":"udp_live_digest_provenance","kind":"snapshot","fields":[{"field":"pruning_point","value":"aa"},{"field":"daa_score","value":"42"},{"field":"source_id","value":"7"}]}"#;
        let fields = super::latest_snapshot_provenance(log).expect("snapshot provenance");
        assert_eq!(fields.get("pruning_point").map(String::as_str), Some("aa"));
        assert_eq!(fields.get("daa_score").map(String::as_str), Some("42"));
        assert_eq!(fields.get("source_id").map(String::as_str), Some("7"));
    }

    #[test]
    fn producer_log_compare_fails_on_snapshot_mismatch() {
        let log_path = unique_temp_path("udp-rpc-digests-mismatch.log");
        std::fs::write(
            &log_path,
            r#"{"event":"udp_live_digest_provenance","kind":"snapshot","fields":[{"field":"pruning_point","value":"aa"},{"field":"utxo_muhash","value":"bb"},{"field":"virtual_selected_parent","value":"cc"},{"field":"virtual_blue_score","value":1},{"field":"daa_score","value":2},{"field":"blue_work","value":"dd"},{"field":"signer_id","value":0},{"field":"source_id","value":7}]}"#,
        )
        .expect("write producer log");
        let response = kaspa_rpc_core::GetUdpDigestsResponse {
            digests: vec![RpcUdpDigestRecord {
                epoch: 2,
                kind: "snapshot".to_string(),
                summary: RpcUdpDigestSummary {
                    epoch: 2,
                    pruning_point: Some("different".to_string()),
                    pruning_proof_commitment: None,
                    utxo_muhash: Some("bb".to_string()),
                    virtual_selected_parent: "cc".to_string(),
                    virtual_blue_score: 1,
                    daa_score: 2,
                    blue_work_hex: "dd".to_string(),
                    kept_headers_mmr_root: None,
                    signer_id: 0,
                    signature_valid: true,
                    recv_ts_ms: 0,
                    source_id: 7,
                },
                verified: true,
            }],
        };

        let result = super::compare_producer_log(&log_path, &response);
        let _ = std::fs::remove_file(&log_path);
        assert!(result.is_err());
    }

    #[test]
    fn latest_received_snapshot_selects_highest_epoch_snapshot() {
        let mut older = record(10, 10, 10, true);
        older.kind = "snapshot".to_string();
        let mut newer = record(20, 20, 20, true);
        newer.kind = "snapshot".to_string();
        let delta = record(30, 30, 30, true);
        let response = kaspa_rpc_core::GetUdpDigestsResponse { digests: vec![delta, older, newer] };

        let latest = super::latest_received_snapshot(&response).expect("latest snapshot");
        assert_eq!(latest.summary.epoch, 20);
    }

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}-{}", std::process::id(), name))
    }
}
