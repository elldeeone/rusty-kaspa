use anyhow::{bail, Context, Result};
use borsh::to_vec;
use clap::Parser;
use kaspa_addresses::{Address, Prefix, Version};
use kaspa_consensus_core::{
    constants::{MAX_TX_IN_SEQUENCE_NUM, TX_VERSION},
    network::{NetworkId, NetworkType},
    sign::sign_with_multiple_v2,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{SignableTransaction, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry},
};
use kaspa_txscript::standard::pay_to_address_script;
use secp256k1::Keypair;
use std::{fs, path::PathBuf};

#[derive(Parser, Debug)]
#[command(name = "udp-tx-build")]
#[command(about = "Build a signed Transaction and serialize it to borsh for UDP tx capsule tests.")]
struct Args {
    /// Private key hex (32 bytes).
    #[arg(long)]
    privkey_hex: String,

    /// Network identifier (e.g. mainnet, testnet-10, devnet).
    #[arg(long, default_value = "devnet")]
    network: String,

    /// Prealloc UTXO id as u64 (devnet-prealloc uses 1..N).
    #[arg(long, default_value_t = 1)]
    prev_utxo_id: u64,

    /// Prealloc UTXO output index.
    #[arg(long, default_value_t = 0)]
    prev_utxo_index: u32,

    /// Previous output amount (sompi).
    #[arg(long, default_value_t = 10_000_000_000)]
    prev_amount: u64,

    /// Fee to pay (sompi).
    #[arg(long, default_value_t = 1_000_000)]
    fee_sompi: u64,

    /// Destination address (defaults to sender).
    #[arg(long)]
    dest_address: Option<String>,

    /// Output path for the borsh-serialized Transaction.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Print derived sender address and exit if --out is not provided.
    #[arg(long)]
    print_address: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let privkey = decode_hex32(&args.privkey_hex)?;
    let keypair = Keypair::from_seckey_slice(secp256k1::SECP256K1, &privkey).context("invalid privkey")?;

    let network_id: NetworkId = args.network.parse().context("invalid --network")?;
    let prefix = prefix_from_network(&network_id);
    let pubkey = keypair.x_only_public_key().0;
    let sender_address = Address::new(prefix, Version::PubKey, pubkey.serialize().as_slice());

    if args.print_address {
        println!("{sender_address}");
        if args.out.is_none() {
            return Ok(());
        }
    }

    let out_path = args.out.context("--out is required unless --print-address is used alone")?;

    if args.prev_amount <= args.fee_sompi {
        bail!("prev_amount {} must exceed fee_sompi {}", args.prev_amount, args.fee_sompi);
    }

    let dest_address = match args.dest_address {
        Some(addr) => Address::try_from(addr.as_str()).context("invalid --dest-address")?,
        None => sender_address.clone(),
    };

    let sender_spk = pay_to_address_script(&sender_address);
    let dest_spk = pay_to_address_script(&dest_address);

    let outpoint = TransactionOutpoint::new(args.prev_utxo_id.into(), args.prev_utxo_index);
    let input = TransactionInput::new(outpoint, vec![], MAX_TX_IN_SEQUENCE_NUM, 1);
    let output_value = args.prev_amount - args.fee_sompi;
    let output = TransactionOutput::new(output_value, dest_spk);

    let tx = Transaction::new(TX_VERSION, vec![input], vec![output], 0, SUBNETWORK_ID_NATIVE, 0, vec![]);

    let entry = UtxoEntry { amount: args.prev_amount, script_public_key: sender_spk, block_daa_score: 0, is_coinbase: false };
    let signable = SignableTransaction::with_entries(tx, vec![entry]);
    let signed = sign_with_multiple_v2(signable, &[privkey]).fully_signed().context("transaction not fully signed")?;

    let bytes = to_vec(&signed.tx).context("borsh serialize")?;
    fs::write(&out_path, bytes).with_context(|| format!("write {}", out_path.display()))?;

    println!(
        "udp-tx-build out={} sender={} dest={} prev_id={} amount={} fee={}",
        out_path.display(),
        sender_address,
        dest_address,
        args.prev_utxo_id,
        args.prev_amount,
        args.fee_sompi
    );
    Ok(())
}

fn prefix_from_network(id: &NetworkId) -> Prefix {
    match id.network_type() {
        NetworkType::Mainnet => Prefix::Mainnet,
        NetworkType::Testnet => Prefix::Testnet,
        NetworkType::Devnet => Prefix::Devnet,
        NetworkType::Simnet => Prefix::Simnet,
    }
}

fn decode_hex32(value: &str) -> Result<[u8; 32]> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.len() != 64 {
        bail!("expected 64 hex chars, got {}", trimmed.len());
    }
    let mut out = [0u8; 32];
    faster_hex::hex_decode(trimmed.as_bytes(), &mut out).context("decode privkey hex")?;
    Ok(out)
}
