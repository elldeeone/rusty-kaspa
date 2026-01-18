use anyhow::{bail, Context, Result};
use clap::Parser;
use kaspa_addresses::Address;
use kaspa_grpc_client::GrpcClient;
use kaspa_rpc_core::{api::rpc::RpcApi, GetMempoolEntriesByAddressesRequest, GetMempoolEntriesRequest};

#[derive(Parser, Debug)]
#[command(name = "udp-rpc-mempool")]
#[command(about = "Query mempool entries via gRPC for lab verification.")]
struct Args {
    /// gRPC URL, e.g. grpc://127.0.0.1:16110
    #[arg(long, default_value = "grpc://127.0.0.1:16110")]
    rpc_url: String,

    /// Address to filter mempool entries (optional).
    #[arg(long)]
    address: Option<String>,

    /// Expected transaction id to verify (hex, optional).
    #[arg(long)]
    expect_txid: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = GrpcClient::connect(args.rpc_url.clone()).await.context("connect grpc")?;

    let mut txids: Vec<String> = Vec::new();
    let count = if let Some(addr) = args.address.as_deref() {
        let address = Address::try_from(addr).context("parse --address")?;
        let response = client
            .get_mempool_entries_by_addresses_call(None, GetMempoolEntriesByAddressesRequest::new(vec![address], true, false))
            .await
            .context("get_mempool_entries_by_addresses")?;
        for entry in &response.entries {
            for sending in &entry.sending {
                if let Some(verbose) = &sending.transaction.verbose_data {
                    txids.push(verbose.transaction_id.to_string());
                }
            }
            for receiving in &entry.receiving {
                if let Some(verbose) = &receiving.transaction.verbose_data {
                    txids.push(verbose.transaction_id.to_string());
                }
            }
        }
        response.entries.iter().map(|entry| entry.sending.len() + entry.receiving.len()).sum::<usize>()
    } else {
        let response = client
            .get_mempool_entries_call(None, GetMempoolEntriesRequest { include_orphan_pool: true, filter_transaction_pool: false })
            .await
            .context("get_mempool_entries")?;
        for entry in &response.mempool_entries {
            if let Some(verbose) = &entry.transaction.verbose_data {
                txids.push(verbose.transaction_id.to_string());
            }
        }
        response.mempool_entries.len()
    };

    println!("mempool_entries={count}");
    if count == 0 {
        bail!("no mempool entries found");
    }

    if let Some(expect) = args.expect_txid.as_deref() {
        let expect = expect.trim().to_lowercase();
        let found = txids.iter().any(|txid| txid.to_lowercase() == expect);
        if !found {
            bail!("expected txid not found in mempool");
        }
        println!("mempool_contains_txid={expect}");
    }
    Ok(())
}
