use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use faster_hex::hex_decode;
use kaspa_consensus_core::{
    network::{NetworkId, NetworkType},
    BlueWorkType,
};
use kaspa_core::time::unix_now;
use kaspa_grpc_client::GrpcClient;
use kaspa_hashes::Hash;
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_udp_sidechannel::fixtures::{
    self, build_delta_vector, build_snapshot_vector, DeltaFields, SnapshotFields, DEFAULT_SIGNER_SECRET, DEFAULT_SOURCE_ID,
};
use secp256k1::{Keypair, Secp256k1, SecretKey};
use sha2::{Digest as ShaDigest, Sha256};
use std::{
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    time::Duration,
};

#[derive(Parser, Debug)]
#[command(name = "udp-live-digest-producer")]
#[command(about = "Produce signed lab DigestV1 KUDP datagrams from a local devnet/simnet kaspad RPC endpoint.")]
struct Args {
    /// Producer-side kaspad gRPC URL.
    #[arg(long, default_value = "grpc://127.0.0.1:16111")]
    rpc_url: String,

    /// Network identifier encoded in the KUDP frame.
    #[arg(long, default_value = "devnet")]
    network: String,

    /// Output sink for produced KUDP datagrams.
    #[arg(long, value_enum, default_value_t = OutputKind::Udp)]
    output: OutputKind,

    /// UDP target used by lora-bridge tx --input udp --udp-bind.
    #[arg(long, required_if_eq("output", "udp"))]
    udp_target: Option<SocketAddr>,

    /// Output directory used by --output file.
    #[arg(long, required_if_eq("output", "file"))]
    out_dir: Option<PathBuf>,

    /// Number of datagrams to produce.
    #[arg(long, default_value_t = 4)]
    count: u32,

    /// Sleep between produced datagrams.
    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,

    /// Emit a snapshot as the first datagram.
    #[arg(long, default_value_t = true)]
    snapshot_first: bool,

    /// Emit a snapshot every N datagrams after the first. 0 disables periodic snapshots.
    #[arg(long, default_value_t = 0)]
    snapshot_every: u32,

    /// Apply a lab counter to DAA score, blue score, and epoch so an idle devnet/simnet still shows progression.
    #[arg(long)]
    lab_progress_counter: bool,

    /// Emit a JSON provenance report for each produced digest to stderr.
    #[arg(long)]
    provenance_report: bool,

    /// Initial KUDP sequence number.
    #[arg(long, default_value_t = 1000)]
    initial_seq: u64,

    /// Source identifier encoded in the frame header.
    #[arg(long, default_value_t = DEFAULT_SOURCE_ID)]
    source_id: u16,

    /// Optional 32-byte signer secret (hex). Defaults to the same lab key used by fixture tooling.
    #[arg(long)]
    signer_secret_hex: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputKind {
    File,
    Udp,
}

#[derive(Clone, Copy, Debug)]
enum DigestKind {
    Snapshot,
    Delta,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.count == 0 {
        bail!("--count must be greater than zero");
    }

    let network_id: NetworkId = args.network.parse().context("invalid --network")?;
    if matches!(network_id.network_type(), NetworkType::Mainnet) {
        bail!("mainnet is intentionally unsupported by this lab producer");
    }
    let network_tag = encode_network_tag(&network_id);
    let signer = signer_from_args(args.signer_secret_hex.as_deref())?;
    let signer_hex = fixtures::signer_hex(&signer);
    let client = GrpcClient::connect(args.rpc_url.clone()).await.context("connect grpc")?;
    let mut output = OutputSink::new(&args)?;

    eprintln!(
        "udp-live-digest-producer rpc_url={} network={} network_tag=0x{network_tag:02x} signer_hex={} lab_progress_counter={}",
        args.rpc_url, network_id, signer_hex, args.lab_progress_counter
    );
    eprintln!(
        "field provenance: real_rpc=pruning_point_hash,sink,sink_blue_score,virtual_daa_score,sink_header.blue_work,sink_header.utxo_commitment; placeholder_lab=pruning_proof_commitment; omitted=kept_headers_mmr_root; optional_lab_progress_counter=epoch/daa_score/virtual_blue_score increments"
    );

    for index in 0..args.count {
        let kind = choose_kind(index, args.snapshot_first, args.snapshot_every);
        let state = LiveState::query(&client, index, args.lab_progress_counter).await?;
        let seq = args.initial_seq.wrapping_add(index as u64);
        let datagram = match kind {
            DigestKind::Snapshot => {
                let fields = state.snapshot_fields();
                build_snapshot_vector(seq, args.source_id, &fields, &signer).into_datagram(network_tag)
            }
            DigestKind::Delta => {
                let fields = state.delta_fields();
                build_delta_vector(seq, args.source_id, &fields, &signer).into_datagram(network_tag)
            }
        };
        output.write(index, kind, &datagram)?;
        if args.provenance_report {
            print_provenance_report(index, kind, seq, args.source_id, &state, &signer_hex, args.lab_progress_counter)?;
        }
        eprintln!(
            "produced index={} kind={:?} seq={} epoch={} virtual_blue_score={} daa_score={} bytes={} sink={} pruning_point={}",
            index,
            kind,
            seq,
            state.epoch,
            state.virtual_blue_score,
            state.daa_score,
            datagram.len(),
            state.sink,
            state.pruning_point
        );

        if index + 1 < args.count && args.interval_ms > 0 {
            tokio::time::sleep(Duration::from_millis(args.interval_ms)).await;
        }
    }

    client.disconnect().await.context("disconnect grpc")?;
    Ok(())
}

struct LiveState {
    epoch: u64,
    frame_ts_ms: u64,
    pruning_point: Hash,
    pruning_proof_commitment: Hash,
    utxo_muhash: Hash,
    virtual_selected_parent: Hash,
    virtual_blue_score: u64,
    daa_score: u64,
    blue_work: [u8; 32],
    sink: Hash,
}

impl LiveState {
    async fn query(client: &GrpcClient, index: u32, lab_progress_counter: bool) -> Result<Self> {
        let dag = client.get_block_dag_info().await.context("get_block_dag_info")?;
        let sink_blue_score = client.get_sink_blue_score().await.context("get_sink_blue_score")?;
        let progress = if lab_progress_counter { index as u64 } else { 0 };

        let sink = dag.sink;
        let sink_block = client.get_block(sink, false).await.ok();
        let sink_header = sink_block.as_ref().map(|block| &block.header);
        let blue_work = sink_header
            .map(|header| blue_work_to_digest_bytes(header.blue_work))
            .unwrap_or_else(|| placeholder_bytes("blue_work", sink, dag.virtual_daa_score));
        let utxo_muhash = sink_header
            .map(|header| header.utxo_commitment)
            .unwrap_or_else(|| placeholder_hash("utxo_muhash", sink, dag.virtual_daa_score));

        Ok(Self {
            epoch: dag.virtual_daa_score.saturating_add(progress),
            frame_ts_ms: unix_now(),
            pruning_point: dag.pruning_point_hash,
            pruning_proof_commitment: placeholder_hash("pruning_proof_commitment", dag.pruning_point_hash, dag.virtual_daa_score),
            utxo_muhash,
            virtual_selected_parent: sink,
            virtual_blue_score: sink_blue_score.saturating_add(progress),
            daa_score: dag.virtual_daa_score.saturating_add(progress),
            blue_work,
            sink,
        })
    }

    fn snapshot_fields(&self) -> SnapshotFields {
        SnapshotFields {
            epoch: self.epoch,
            frame_ts_ms: self.frame_ts_ms,
            pruning_point: self.pruning_point,
            pruning_proof_commitment: self.pruning_proof_commitment,
            utxo_muhash: self.utxo_muhash,
            virtual_selected_parent: self.virtual_selected_parent,
            virtual_blue_score: self.virtual_blue_score,
            daa_score: self.daa_score,
            blue_work: self.blue_work,
            kept_headers_mmr_root: None,
        }
    }

    fn delta_fields(&self) -> DeltaFields {
        DeltaFields {
            epoch: self.epoch,
            frame_ts_ms: self.frame_ts_ms,
            virtual_selected_parent: self.virtual_selected_parent,
            virtual_blue_score: self.virtual_blue_score,
            daa_score: self.daa_score,
            blue_work: self.blue_work,
        }
    }
}

enum OutputSink {
    File { out_dir: PathBuf },
    Udp { socket: UdpSocket, target: SocketAddr },
}

impl OutputSink {
    fn new(args: &Args) -> Result<Self> {
        match args.output {
            OutputKind::File => {
                let out_dir = args.out_dir.clone().context("--out-dir is required")?;
                std::fs::create_dir_all(&out_dir).with_context(|| format!("create {}", out_dir.display()))?;
                Ok(Self::File { out_dir })
            }
            OutputKind::Udp => {
                let target = args.udp_target.context("--udp-target is required")?;
                let socket = UdpSocket::bind("127.0.0.1:0").context("bind UDP output socket")?;
                Ok(Self::Udp { socket, target })
            }
        }
    }

    fn write(&mut self, index: u32, kind: DigestKind, datagram: &[u8]) -> Result<()> {
        match self {
            OutputSink::File { out_dir } => {
                let name = format!("{index:04}-{}.bin", kind_name(kind));
                let path = out_dir.join(name);
                std::fs::write(&path, datagram).with_context(|| format!("write {}", path.display()))?;
                println!("{} {} bytes", path.display(), datagram.len());
            }
            OutputSink::Udp { socket, target } => {
                socket.send_to(datagram, *target).with_context(|| format!("send UDP to {target}"))?;
            }
        }
        Ok(())
    }
}

fn choose_kind(index: u32, snapshot_first: bool, snapshot_every: u32) -> DigestKind {
    if index == 0 && snapshot_first {
        DigestKind::Snapshot
    } else if snapshot_every > 0 && index > 0 && index % snapshot_every == 0 {
        DigestKind::Snapshot
    } else {
        DigestKind::Delta
    }
}

fn kind_name(kind: DigestKind) -> &'static str {
    match kind {
        DigestKind::Snapshot => "snapshot",
        DigestKind::Delta => "delta",
    }
}

fn signer_from_args(hex: Option<&str>) -> Result<Keypair> {
    match hex {
        Some(hex) => keypair_from_hex(hex),
        None => {
            let secp = Secp256k1::new();
            let sk = SecretKey::from_slice(&DEFAULT_SIGNER_SECRET).expect("default signer secret is valid");
            Ok(Keypair::from_secret_key(&secp, &sk))
        }
    }
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

fn blue_work_to_digest_bytes(work: BlueWorkType) -> [u8; 32] {
    let work = work.to_be_bytes();
    let mut out = [0u8; 32];
    out[32 - work.len()..].copy_from_slice(&work);
    out
}

fn print_provenance_report(
    index: u32,
    kind: DigestKind,
    seq: u64,
    source_id: u16,
    state: &LiveState,
    signer_hex: &str,
    lab_progress_counter: bool,
) -> Result<()> {
    let mut fields = vec![
        provenance_field(
            "epoch",
            state.epoch.to_string(),
            progress_auth(lab_progress_counter),
            progress_source("getBlockDagInfo.virtual_daa_score", lab_progress_counter),
            progress_note(lab_progress_counter),
        ),
        provenance_field(
            "frame_timestamp_ms",
            state.frame_ts_ms.to_string(),
            "lab-derived",
            "producer clock unix_now()",
            "wall-clock observation time, not consensus state",
        ),
        provenance_field(
            "pruning_point",
            state.pruning_point.to_string(),
            "real",
            "getBlockDagInfo.pruning_point_hash",
            "current node pruning point",
        ),
        provenance_field(
            "virtual_selected_parent",
            state.virtual_selected_parent.to_string(),
            "real",
            "getBlockDagInfo.sink",
            "matches receiver divergence monitor comparison against async_get_sink()",
        ),
        provenance_field(
            "virtual_blue_score",
            state.virtual_blue_score.to_string(),
            progress_auth(lab_progress_counter),
            progress_source("getSinkBlueScore.blue_score", lab_progress_counter),
            progress_note(lab_progress_counter),
        ),
        provenance_field(
            "daa_score",
            state.daa_score.to_string(),
            progress_auth(lab_progress_counter),
            progress_source("getBlockDagInfo.virtual_daa_score", lab_progress_counter),
            progress_note(lab_progress_counter),
        ),
        provenance_field(
            "blue_work",
            faster_hex::hex_string(&state.blue_work),
            "real",
            "getBlock(sink).header.blue_work",
            "sink header blue work, left-padded to DigestV1 32-byte field",
        ),
        provenance_field(
            "signer_id",
            "0".to_string(),
            "lab-derived",
            "fixture signer id",
            "DigestV1 fixture builder currently encodes signer id 0",
        ),
        provenance_field(
            "signature",
            signer_hex.to_string(),
            "lab-derived",
            "configured Schnorr signer",
            "signature is cryptographically valid for the configured lab signer",
        ),
        provenance_field("source_id", source_id.to_string(), "lab-derived", "--source-id", "operator-selected source identity"),
    ];

    if matches!(kind, DigestKind::Snapshot) {
        fields.push(provenance_field(
            "pruning_proof_commitment",
            state.pruning_proof_commitment.to_string(),
            "placeholder",
            "deterministic lab SHA-256 placeholder",
            "needs a node/RPC commitment over the actual pruning-point proof",
        ));
        fields.push(provenance_field(
            "utxo_muhash",
            state.utxo_muhash.to_string(),
            "real",
            "getBlock(sink).header.utxo_commitment",
            "real header UTXO commitment for sink; a virtual-state commitment still needs a dedicated node/RPC hook",
        ));
        fields.push(provenance_field(
            "kept_headers_mmr_root",
            serde_json::Value::Null,
            "omitted",
            "not encoded",
            "DigestV1 supports an optional field, but rusty-kaspa does not expose this MMR root today",
        ));
    }

    let report = serde_json::json!({
        "event": "udp_live_digest_provenance",
        "index": index,
        "kind": kind_name(kind),
        "seq": seq,
        "sink": state.sink.to_string(),
        "fields": fields,
    });
    eprintln!("{}", serde_json::to_string(&report).context("serialize provenance report")?);
    Ok(())
}

fn provenance_field(
    name: &'static str,
    value: impl Into<serde_json::Value>,
    authenticity: &'static str,
    source: impl Into<String>,
    notes: &'static str,
) -> serde_json::Value {
    serde_json::json!({
        "field": name,
        "value": value.into(),
        "source": source.into(),
        "authenticity": authenticity,
        "notes": notes,
    })
}

fn progress_auth(lab_progress_counter: bool) -> &'static str {
    if lab_progress_counter {
        "lab-derived"
    } else {
        "real"
    }
}

fn progress_source(base: &'static str, lab_progress_counter: bool) -> String {
    if lab_progress_counter {
        format!("{base} + --lab-progress-counter index")
    } else {
        base.to_string()
    }
}

fn progress_note(lab_progress_counter: bool) -> &'static str {
    if lab_progress_counter {
        "lab progression counter applied so idle devnet/simnet output remains monotonic"
    } else {
        "direct node/RPC state; may not advance on an idle devnet/simnet node"
    }
}

fn placeholder_hash(label: &'static str, seed: Hash, epoch: u64) -> Hash {
    Hash::from_bytes(placeholder_bytes(label, seed, epoch))
}

fn placeholder_bytes(label: &'static str, seed: Hash, epoch: u64) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"kaspa/udp-live-digest-lab-placeholder/v1");
    hasher.update(label.as_bytes());
    hasher.update(seed.as_bytes());
    hasher.update(epoch.to_le_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_kind_honors_snapshot_first_and_periodic_snapshots() {
        assert!(matches!(choose_kind(0, true, 0), DigestKind::Snapshot));
        assert!(matches!(choose_kind(1, true, 0), DigestKind::Delta));
        assert!(matches!(choose_kind(3, false, 3), DigestKind::Snapshot));
        assert!(matches!(choose_kind(4, false, 3), DigestKind::Delta));
    }

    #[test]
    fn lab_progress_classification_is_explicit() {
        assert_eq!(progress_auth(false), "real");
        assert_eq!(progress_auth(true), "lab-derived");
        assert_eq!(progress_source("rpc.field", false), "rpc.field");
        assert_eq!(progress_source("rpc.field", true), "rpc.field + --lab-progress-counter index");
    }

    #[test]
    fn provenance_field_contains_required_classification_keys() {
        let field = provenance_field("daa_score", "42", "real", "getBlockDagInfo.virtual_daa_score", "direct node/RPC state");
        assert_eq!(field["field"], "daa_score");
        assert_eq!(field["value"], "42");
        assert_eq!(field["authenticity"], "real");
        assert_eq!(field["source"], "getBlockDagInfo.virtual_daa_score");
        assert_eq!(field["notes"], "direct node/RPC state");
    }
}
