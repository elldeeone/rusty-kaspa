use anyhow::{Context, Result};
use clap::Parser;
use kaspa_grpc_client::GrpcClient;
use kaspa_rpc_core::api::rpc::RpcApi;

#[derive(Parser, Debug)]
#[command(name = "udp-rpc-node-info")]
#[command(about = "Print compact node state used by UDP digest real-network labs.")]
struct Args {
    /// gRPC URL, e.g. grpc://127.0.0.1:16111.
    #[arg(long, default_value = "grpc://127.0.0.1:16111")]
    rpc_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = GrpcClient::connect(args.rpc_url.clone()).await.context("connect grpc")?;
    let server = client.get_server_info().await.context("get_server_info")?;
    let sync = client.get_sync_status().await.context("get_sync_status")?;
    let dag = client.get_block_dag_info().await.context("get_block_dag_info")?;
    let sink_blue_score = client.get_sink_blue_score().await.context("get_sink_blue_score")?;
    let sink_block = client.get_block(dag.sink, false).await.ok();
    let sink_blue_work =
        sink_block.as_ref().map(|block| faster_hex::hex_string(&blue_work_bytes(block.header.blue_work.to_be_bytes())));
    let sink_utxo_commitment = sink_block.as_ref().map(|block| block.header.utxo_commitment.to_string());

    let output = serde_json::json!({
        "rpcUrl": args.rpc_url,
        "networkId": server.network_id.to_string(),
        "serverVersion": server.server_version,
        "serverInfoSynced": server.is_synced,
        "syncStatusSynced": sync,
        "hasUtxoIndex": server.has_utxo_index,
        "serverVirtualDaaScore": server.virtual_daa_score,
        "dagNetwork": dag.network.to_string(),
        "blockCount": dag.block_count,
        "headerCount": dag.header_count,
        "sink": dag.sink.to_string(),
        "pruningPoint": dag.pruning_point_hash.to_string(),
        "virtualDaaScore": dag.virtual_daa_score,
        "sinkBlueScore": sink_blue_score,
        "sinkBlueWorkHex": sink_blue_work,
        "sinkUtxoCommitment": sink_utxo_commitment,
    });
    println!("{}", serde_json::to_string_pretty(&output).context("serialize node info")?);
    client.disconnect().await.context("disconnect grpc")?;
    Ok(())
}

fn blue_work_bytes<const N: usize>(work: [u8; N]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[32 - work.len()..].copy_from_slice(&work);
    out
}
