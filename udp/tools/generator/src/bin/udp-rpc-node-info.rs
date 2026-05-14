use anyhow::{Context, Result};
use clap::Parser;
use kaspa_consensus_core::network::NetworkId;
use kaspa_grpc_client::GrpcClient;
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_wrpc_client::{
    client::{ConnectOptions, ConnectStrategy},
    KaspaRpcClient, Resolver, WrpcEncoding,
};
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "udp-rpc-node-info")]
#[command(about = "Print compact node state used by UDP digest real-network labs.")]
struct Args {
    /// RPC URL, e.g. grpc://127.0.0.1:16111, ws://host:17210, wss://host, or pnn.
    #[arg(long, default_value = "grpc://127.0.0.1:16111")]
    rpc_url: String,

    /// Network used for PNN resolver connections.
    #[arg(long, default_value = "testnet-10")]
    network: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let network_id: NetworkId = args.network.parse().context("invalid --network")?;
    if is_wrpc_url(&args.rpc_url) {
        let client = connect_wrpc(&args.rpc_url, network_id).await?;
        print_node_info(&args.rpc_url, &client).await?;
        client.disconnect().await.context("disconnect wrpc")?;
        return Ok(());
    }

    let client = GrpcClient::connect(args.rpc_url.clone()).await.context("connect grpc")?;
    print_node_info(&args.rpc_url, &client).await?;
    client.disconnect().await.context("disconnect grpc")?;
    Ok(())
}

async fn connect_wrpc(rpc_url: &str, network_id: NetworkId) -> Result<KaspaRpcClient> {
    let use_resolver = matches!(rpc_url, "pnn" | "wrpc://pnn");
    let url = if use_resolver { None } else { Some(rpc_url) };
    let resolver = if use_resolver { Some(Resolver::default()) } else { None };
    let client = KaspaRpcClient::new(WrpcEncoding::Borsh, url, resolver, Some(network_id), None).context("create wrpc client")?;
    tokio::time::timeout(
        Duration::from_millis(10_000),
        client.connect(Some(ConnectOptions {
            block_async_connect: true,
            connect_timeout: Some(Duration::from_millis(5_000)),
            strategy: ConnectStrategy::Fallback,
            ..Default::default()
        })),
    )
    .await
    .context("wrpc connect timed out")?
    .context("connect wrpc")?;
    Ok(client)
}

fn is_wrpc_url(rpc_url: &str) -> bool {
    matches!(rpc_url, "pnn" | "wrpc://pnn") || rpc_url.starts_with("ws://") || rpc_url.starts_with("wss://")
}

async fn print_node_info(client_label: &str, client: &impl RpcApi) -> Result<()> {
    let server = client.get_server_info().await.context("get_server_info")?;
    let sync = client.get_sync_status().await.context("get_sync_status")?;
    let dag = client.get_block_dag_info().await.context("get_block_dag_info")?;
    let sink_blue_score = client.get_sink_blue_score().await.context("get_sink_blue_score")?;
    let sink_block = client.get_block(dag.sink, false).await.ok();
    let sink_blue_work =
        sink_block.as_ref().map(|block| faster_hex::hex_string(&blue_work_bytes(block.header.blue_work.to_be_bytes())));
    let sink_utxo_commitment = sink_block.as_ref().map(|block| block.header.utxo_commitment.to_string());

    let output = serde_json::json!({
        "rpcUrl": client_label,
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
    Ok(())
}

fn blue_work_bytes<const N: usize>(work: [u8; N]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[32 - work.len()..].copy_from_slice(&work);
    out
}
