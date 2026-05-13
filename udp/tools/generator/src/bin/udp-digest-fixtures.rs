use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_core::time::unix_now;
use kaspa_udp_sidechannel::fixtures::{
    self, build_delta_vector, build_snapshot_vector, delta_fields, snapshot_fields, DEFAULT_DELTA_SEQ, DEFAULT_SNAPSHOT_SEQ,
    DEFAULT_SOURCE_ID,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "udp-digest-fixtures")]
#[command(about = "Write DigestV1 KUDP fixture datagrams for UDP/LoRa lab testing.")]
struct Args {
    /// Output directory for fixture datagrams.
    #[arg(long)]
    out_dir: PathBuf,

    /// Network identifier used for the KUDP frame network tag.
    #[arg(long, default_value = "devnet")]
    network: String,

    /// Source identifier encoded in the frame header.
    #[arg(long, default_value_t = DEFAULT_SOURCE_ID)]
    source_id: u16,

    /// Timestamp in milliseconds. Defaults to current wall-clock time.
    #[arg(long)]
    frame_ts_ms: Option<u64>,

    /// Negative fixture set to generate next to delta.bin and snapshot.bin.
    #[arg(long, value_enum, default_value_t = NegativeSet::All)]
    negatives: NegativeSet,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum NegativeSet {
    All,
    None,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let network_id: NetworkId = args.network.parse().context("invalid --network")?;
    let network_tag = encode_network_tag(&network_id);
    let ts = args.frame_ts_ms.unwrap_or_else(unix_now);
    let signer = fixtures::default_keypair();
    let signer_hex = fixtures::signer_hex(&signer);

    std::fs::create_dir_all(&args.out_dir).with_context(|| format!("create {}", args.out_dir.display()))?;

    let snapshot = build_snapshot_vector(DEFAULT_SNAPSHOT_SEQ, args.source_id, &snapshot_fields(42, ts, true), &signer)
        .into_datagram(network_tag);
    let delta = build_delta_vector(DEFAULT_DELTA_SEQ, args.source_id, &delta_fields(43, ts), &signer).into_datagram(network_tag);

    write_fixture(&args.out_dir, "snapshot.bin", &snapshot)?;
    write_fixture(&args.out_dir, "delta.bin", &delta)?;

    if matches!(args.negatives, NegativeSet::All) {
        let mut corrupt = delta.clone();
        let Some(byte) = corrupt.last_mut() else {
            bail!("delta fixture unexpectedly empty");
        };
        *byte ^= 0x01;
        write_fixture(&args.out_dir, "delta-corrupt.bin", &corrupt)?;

        let wrong_tag = wrong_network_tag(network_tag);
        let mut wrong_network = delta;
        wrong_network[6] = wrong_tag;
        refresh_header_crc(&mut wrong_network)?;
        write_fixture(&args.out_dir, "delta-wrong-network.bin", &wrong_network)?;
    }

    println!(
        "wrote DigestV1 fixtures to {} network={} network_tag=0x{network_tag:02x} signer_hex={signer_hex}",
        args.out_dir.display(),
        network_id
    );
    Ok(())
}

fn write_fixture(out_dir: &std::path::Path, name: &str, bytes: &[u8]) -> Result<()> {
    let path = out_dir.join(name);
    std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    println!("{} {} bytes", path.display(), bytes.len());
    Ok(())
}

fn encode_network_tag(id: &NetworkId) -> u8 {
    let base = match id.network_type() {
        NetworkType::Mainnet => 0x01,
        NetworkType::Testnet => 0x02,
        NetworkType::Devnet => 0x03,
        NetworkType::Simnet => 0x04,
    };
    let suffix = id.suffix().unwrap_or(0).min(0x0F) as u8;
    base | (suffix << 4)
}

fn wrong_network_tag(correct: u8) -> u8 {
    if correct == 0x01 {
        0x03
    } else {
        0x01
    }
}

fn refresh_header_crc(datagram: &mut [u8]) -> Result<()> {
    const HEADER_LEN: usize = 38;
    const HEADER_CRC_OFFSET: usize = HEADER_LEN - 4;
    if datagram.len() < HEADER_LEN {
        bail!("datagram shorter than KUDP header");
    }
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&datagram[..HEADER_CRC_OFFSET]);
    datagram[HEADER_CRC_OFFSET..HEADER_LEN].copy_from_slice(&hasher.finalize().to_le_bytes());
    Ok(())
}
