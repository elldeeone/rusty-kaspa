use anyhow::{bail, Context, Result};
use clap::Parser;
use faster_hex::hex_decode;
use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_core::time::unix_now;
use kaspa_udp_sidechannel::fixtures::{self, build_delta_vector, build_snapshot_vector, delta_fields, snapshot_fields};
use secp256k1::{Keypair, Secp256k1, SecretKey};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::net::UnixDatagram;
use tokio::time::{sleep, Instant};

#[derive(Parser, Debug)]
#[command(name = "udp-generator")]
#[command(about = "Emit DigestV1 frames at a fixed bitrate for soak/fuzz testing.")]
struct Args {
    /// UDP target (host:port) to send frames to.
    #[arg(long)]
    target: Option<String>,

    /// Unix datagram socket path to send frames to.
    #[arg(long, conflicts_with = "target")]
    target_unix: Option<PathBuf>,

    /// Desired bitrate in kilobits per second.
    #[arg(long, default_value_t = 10.0)]
    rate_kbps: f64,

    /// Snapshot interval in seconds.
    #[arg(long, default_value_t = 30)]
    snapshot_interval: u64,

    /// Source identifier encoded in the frame header.
    #[arg(long, default_value_t = fixtures::DEFAULT_SOURCE_ID)]
    source_id: u16,

    /// Network identifier (e.g. mainnet, testnet-10).
    #[arg(long, default_value = "mainnet")]
    network: String,

    /// Optional 32-byte signer secret (hex). Defaults to the golden test vector key.
    #[arg(long)]
    signer_secret_hex: Option<String>,
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
    let keypair = if let Some(hex) = args.signer_secret_hex.as_deref() { keypair_from_hex(hex)? } else { fixtures::default_keypair() };

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

    let bytes_per_sec = (args.rate_kbps.max(0.1) * 1000.0) / 8.0;
    if bytes_per_sec.is_nan() || bytes_per_sec <= 0.0 {
        bail!("invalid --rate-kbps: {}", args.rate_kbps);
    }

    let snapshot_interval = Duration::from_secs(args.snapshot_interval.max(1));

    println!(
        "udp-generator target={} rate_kbps={:.2} snapshot_interval={}s network={} source_id={}",
        target_desc,
        args.rate_kbps,
        snapshot_interval.as_secs(),
        network_id,
        args.source_id
    );

    let mut pump = GeneratorState::new(args.source_id, network_tag, keypair, bytes_per_sec, snapshot_interval);
    pump.run(socket).await
}

struct GeneratorState {
    source_id: u16,
    network_tag: u8,
    keypair: Keypair,
    bytes_per_sec: f64,
    snapshot_interval: Duration,
    next_snapshot_deadline: Instant,
    seq: u64,
    epoch: u64,
}

impl GeneratorState {
    fn new(source_id: u16, network_tag: u8, keypair: Keypair, bytes_per_sec: f64, snapshot_interval: Duration) -> Self {
        Self {
            source_id,
            network_tag,
            keypair,
            bytes_per_sec,
            snapshot_interval,
            next_snapshot_deadline: Instant::now(),
            seq: 1,
            epoch: 1,
        }
    }

    async fn run(&mut self, socket: OutSocket) -> Result<()> {
        loop {
            let now = Instant::now();
            let send_snapshot = now >= self.next_snapshot_deadline;
            let datagram = if send_snapshot {
                self.next_snapshot_deadline = now + self.snapshot_interval;
                self.snapshot_datagram()
            } else {
                self.delta_datagram()
            };
            socket.send(&datagram).await?;

            // Advance sequence/epoch bookkeeping.
            self.seq = self.seq.wrapping_add(1);
            if !send_snapshot {
                self.epoch = self.epoch.wrapping_add(1);
            }

            let delay_secs = datagram.len() as f64 / self.bytes_per_sec;
            if delay_secs.is_normal() && delay_secs > 0.0 {
                sleep(Duration::from_secs_f64(delay_secs)).await;
            } else {
                tokio::task::yield_now().await;
            }
        }
    }

    fn snapshot_datagram(&self) -> Vec<u8> {
        let ts = unix_now();
        let fields = snapshot_fields(self.epoch, ts, true);
        let vector = build_snapshot_vector(self.seq, self.source_id, &fields, &self.keypair);
        vector.into_datagram(self.network_tag)
    }

    fn delta_datagram(&self) -> Vec<u8> {
        let ts = unix_now();
        let fields = delta_fields(self.epoch, ts);
        let vector = build_delta_vector(self.seq, self.source_id, &fields, &self.keypair);
        vector.into_datagram(self.network_tag)
    }
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

fn keypair_from_hex(hex: &str) -> Result<Keypair> {
    if hex.len() != 64 {
        bail!("signer secret must be 64 hex characters");
    }
    let mut bytes = [0u8; 32];
    hex_decode(hex.as_bytes(), &mut bytes).context("decode signer secret")?;
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&bytes).context("invalid secret key")?;
    Ok(Keypair::from_secret_key(&secp, &sk))
}

fn unix_bind_path() -> PathBuf {
    let pid = std::process::id();
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    std::env::temp_dir().join(format!("udp-generator-{pid}-{ts}.sock"))
}
