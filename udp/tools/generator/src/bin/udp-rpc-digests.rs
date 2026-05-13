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
        Command::Digests { from_epoch, limit } => {
            let response = client.get_udp_digests(from_epoch, limit, None).await.context("get_udp_digests")?;
            println!("{}", serde_json::to_string_pretty(&response).context("serialize digests response")?);
        }
    }

    client.disconnect().await.context("disconnect grpc")?;
    Ok(())
}
