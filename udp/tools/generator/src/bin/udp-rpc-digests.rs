use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kaspa_grpc_client::GrpcClient;
use kaspa_rpc_core::api::rpc::RpcApi;

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
        Command::Digests { from_epoch, limit, check_monotonic } => {
            let response = client.get_udp_digests(from_epoch, limit, None).await.context("get_udp_digests")?;
            println!("{}", serde_json::to_string_pretty(&response).context("serialize digests response")?);
            if check_monotonic {
                print_monotonic_summary(&response);
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
}
