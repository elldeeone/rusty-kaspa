use anyhow::{bail, Context, Result};
use clap::Parser;
use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_core::time::unix_now;
use kaspa_udp_sidechannel::fixtures::DEFAULT_SOURCE_ID;
use kaspa_udp_sidechannel::frame::{FrameFlags, FrameKind, SatFrameHeader, HEADER_LEN, KUDP_MAGIC};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::net::{UdpSocket, UnixDatagram};

#[derive(Parser, Debug)]
#[command(name = "udp-tx-generator")]
#[command(about = "Send a single Tx capsule frame over UDP/Unix datagram.")]
struct Args {
    /// Path to a borsh-encoded Transaction payload.
    #[arg(long)]
    tx_file: PathBuf,

    /// UDP target (host:port) to send frames to.
    #[arg(long)]
    target: Option<String>,

    /// Unix datagram socket path to send frames to.
    #[arg(long, conflicts_with = "target")]
    target_unix: Option<PathBuf>,

    /// Source identifier encoded in the frame header.
    #[arg(long, default_value_t = DEFAULT_SOURCE_ID)]
    source_id: u16,

    /// Network identifier (e.g. mainnet, testnet-10).
    #[arg(long, default_value = "mainnet")]
    network: String,

    /// Sequence number to embed in the header.
    #[arg(long, default_value_t = 1)]
    seq: u64,

    /// Allow orphan flag in the capsule (requires node unsafe rpc for effect).
    #[arg(long)]
    allow_orphan: bool,

    /// Wallet timestamp override (milliseconds since epoch). Defaults to now.
    #[arg(long)]
    wallet_ts_ms: Option<u64>,

    /// Max tx payload bytes (capsule payload limit enforcement).
    #[arg(long, default_value_t = 65_536)]
    max_tx_payload_bytes: usize,
}

enum OutSocket {
    Udp(UdpSocket),
    Unix(UnixDatagram),
}

impl OutSocket {
    async fn send(&self, buf: &[u8]) -> Result<()> {
        match self {
            OutSocket::Udp(socket) => {
                socket.send(buf).await.context("send udp")?;
            }
            OutSocket::Unix(socket) => {
                socket.send(buf).await.context("send unix")?;
            }
        }
        Ok(())
    }
}

struct UnixSocketGuard {
    path: PathBuf,
}

impl Drop for UnixSocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let network_id: NetworkId = args.network.parse().context("invalid --network")?;
    let network_tag = encode_network_tag(&network_id);

    let tx_bytes = read_tx_bytes(&args.tx_file)?;
    if tx_bytes.is_empty() {
        bail!("tx file is empty: {}", args.tx_file.display());
    }
    if tx_bytes.len() > args.max_tx_payload_bytes {
        bail!("tx payload {} bytes exceeds cap {}", tx_bytes.len(), args.max_tx_payload_bytes);
    }

    let (socket, _guard, target_desc) = if let Some(path) = args.target_unix.as_ref() {
        let local_path = unix_bind_path();
        let _ = std::fs::remove_file(&local_path);
        let socket = UnixDatagram::bind(&local_path).context("bind unix")?;
        socket.connect(path).context("connect unix")?;
        (OutSocket::Unix(socket), Some(UnixSocketGuard { path: local_path }), path.to_string_lossy().to_string())
    } else {
        let target = args.target.as_deref().unwrap_or("127.0.0.1:28515");
        let target_addr: SocketAddr = target.parse().context("invalid --target")?;
        let socket = UdpSocket::bind("127.0.0.1:0").await.context("bind")?;
        socket.connect(target_addr).await.context("connect")?;
        (OutSocket::Udp(socket), None, target.to_string())
    };

    let wallet_ts = args.wallet_ts_ms.unwrap_or_else(unix_now);
    let capsule = build_capsule(wallet_ts, args.allow_orphan, &tx_bytes);
    let datagram = build_datagram(args.seq, args.source_id, network_tag, &capsule);

    println!(
        "udp-tx-generator target={} network={} seq={} source_id={} tx_bytes={} capsule_bytes={} allow_orphan={} wallet_ts_ms={}",
        target_desc,
        network_id,
        args.seq,
        args.source_id,
        tx_bytes.len(),
        capsule.len(),
        args.allow_orphan,
        wallet_ts
    );

    socket.send(&datagram).await?;
    Ok(())
}

fn read_tx_bytes(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("read tx file {}", path.display()))
}

fn build_capsule(wallet_ts_ms: u64, allow_orphan: bool, tx_bytes: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(14 + tx_bytes.len());
    payload.push(1u8);
    payload.push(if allow_orphan { 0x01 } else { 0x00 });
    payload.extend_from_slice(&wallet_ts_ms.to_le_bytes());
    payload.extend_from_slice(&(tx_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(tx_bytes);
    payload
}

fn build_datagram(seq: u64, source_id: u16, network_tag: u8, payload: &[u8]) -> Vec<u8> {
    let header = SatFrameHeader {
        kind: FrameKind::Tx,
        flags: FrameFlags::from_bits(0x00),
        seq,
        group_id: 0,
        group_k: 0,
        group_n: 0,
        frag_ix: 0,
        frag_cnt: 1,
        payload_len: payload.len() as u32,
        source_id,
    };
    encode_datagram(&header, payload, network_tag)
}

fn encode_datagram(header: &SatFrameHeader, payload: &[u8], network_tag: u8) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_LEN + payload.len()];
    buf[..4].copy_from_slice(&KUDP_MAGIC);
    buf[4] = 1;
    buf[5] = header.kind as u8;
    buf[6] = network_tag;
    buf[7] = header.flags.bits();
    buf[8..16].copy_from_slice(&header.seq.to_le_bytes());
    buf[16..20].copy_from_slice(&header.group_id.to_le_bytes());
    buf[20..22].copy_from_slice(&header.group_k.to_le_bytes());
    buf[22..24].copy_from_slice(&header.group_n.to_le_bytes());
    buf[24..26].copy_from_slice(&header.frag_ix.to_le_bytes());
    buf[26..28].copy_from_slice(&header.frag_cnt.to_le_bytes());
    buf[28..32].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    buf[32..34].copy_from_slice(&header.source_id.to_le_bytes());
    buf[HEADER_LEN..].copy_from_slice(payload);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&buf[..HEADER_LEN - 4]);
    buf[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&hasher.finalize().to_le_bytes());
    buf
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

fn unix_bind_path() -> PathBuf {
    let pid = std::process::id();
    std::env::temp_dir().join(format!("udp-tx-generator-{}.sock", pid))
}
