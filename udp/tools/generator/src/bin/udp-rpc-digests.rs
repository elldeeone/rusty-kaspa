use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kaspa_grpc_client::GrpcClient;
use kaspa_rpc_core::api::rpc::RpcApi;
use std::path::PathBuf;

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
        Command::Digests { from_epoch, limit, check_monotonic, producer_log } => {
            let response = client.get_udp_digests(from_epoch, limit, None).await.context("get_udp_digests")?;
            println!("{}", serde_json::to_string_pretty(&response).context("serialize digests response")?);
            if check_monotonic {
                print_monotonic_summary(&response);
            }
            if let Some(path) = producer_log {
                compare_producer_log(&path, &response)?;
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
    let received = response
        .digests
        .iter()
        .find(|record| record.kind == "snapshot")
        .context("RPC response did not contain a received snapshot")?;
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

#[cfg(test)]
mod tests {
    use kaspa_rpc_core::{RpcUdpDigestRecord, RpcUdpDigestSummary};

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
}
