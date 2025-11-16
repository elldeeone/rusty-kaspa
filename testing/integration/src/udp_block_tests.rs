use crate::common::{daemon::Daemon, utils::wait_for};
use kaspa_addresses::{Address, Version};
use kaspa_alloc::init_allocator_with_default_settings;
use kaspa_consensus_core::{
    block::Block,
    header::Header,
    network::{NetworkId, NetworkType},
};
use kaspa_core::log;
use kaspa_grpc_client::GrpcClient;
use kaspa_p2p_lib::pb;
use kaspa_rpc_core::{api::rpc::RpcApi, RpcRawBlock};
use kaspa_udp_sidechannel::frame::{FrameFlags, FrameKind, SatFrameHeader, HEADER_LEN, KUDP_MAGIC};
use kaspad_lib::args::{Args, UdpModeArg};
use prost::Message;
use std::{
    convert::TryInto,
    net::{SocketAddr, UdpSocket as StdUdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

const BASELINE_BLOCKS: u64 = 2;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Spawns full kaspad instances (~2 min); run manually when validating UDP block mode"]
async fn udp_block_equivalence() {
    init_allocator_with_default_settings();
    log::try_init_logger("INFO");

    let mut args_control = base_args();
    args_control.enable_unsynced_mining = true;
    args_control.udp.db_migrate = true;
    let mut args_udp = base_args();
    args_udp.enable_unsynced_mining = true;
    let udp_addr = reserve_udp_addr();
    args_udp.udp.enable = true;
    args_udp.udp.listen = Some(udp_addr);
    args_udp.udp.mode = UdpModeArg::Blocks;
    args_udp.udp.danger_accept_blocks = true;
    args_udp.udp.block_max_bytes = 1_048_576;
    args_udp.udp.db_migrate = true;

    let mut miner = Daemon::new_random_with_args(args_control, 64);
    let mut udp_node = Daemon::new_random_with_args(args_udp, 64);

    let miner_client = miner.start().await;
    let udp_client = udp_node.start().await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let network = miner.client_manager().network;
    let pay_address = fixed_address(network);

    let template = miner_client.get_block_template(pay_address.clone(), vec![]).await.unwrap();
    let rpc_block = template.block.clone();
    let block_hash = Header::from(&rpc_block.header).hash;
    miner_client.submit_block(template.block, false).await.unwrap();

    wait_for(
        100,
        150,
        || {
            let client = miner_client.clone();
            let hash = block_hash;
            async move { client.get_block_dag_info().await.unwrap().sink == hash }
        },
        "miner failed to process block",
    )
    .await;

    let block_message = rpc_block_to_pb(&rpc_block);
    let udp_network = udp_node.client_manager().network;
    send_block_over_udp(udp_addr, &udp_network, 42, &block_message).await;

    wait_for(
        100,
        150,
        || {
            let client = udp_client.clone();
            let hash = block_hash;
            async move {
                let info = client.get_block_dag_info().await.unwrap();
                println!("udp sink {} (target {})", info.sink, hash);
                info.sink == hash
            }
        },
        "udp node did not ingest block",
    )
    .await;

    let control_info = miner_client.get_block_dag_info().await.unwrap();
    let udp_info = udp_client.get_block_dag_info().await.unwrap();
    assert_eq!(control_info.block_count, udp_info.block_count, "block counts differ");
    assert_eq!(udp_info.sink, block_hash, "udp node tip differs");

    let ingest_info = udp_client.get_udp_ingest_info(None).await.unwrap();
    assert_eq!(ingest_info.block_injected_total, 1, "block injector metric did not increment");

    miner_client.disconnect().await.unwrap();
    udp_client.disconnect().await.unwrap();
    miner.shutdown();
    udp_node.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Spawns full kaspad instances (~2 min); run manually when validating UDP block mode"]
async fn udp_block_fairness() {
    init_allocator_with_default_settings();
    log::try_init_logger("INFO");

    let mut args_source = base_args();
    args_source.enable_unsynced_mining = true;
    let mut args_target = base_args();
    args_target.enable_unsynced_mining = true;
    args_target.udp.enable = true;
    args_target.udp.mode = UdpModeArg::Blocks;
    args_target.udp.danger_accept_blocks = true;
    args_target.udp.listen = Some(reserve_udp_addr());
    args_target.udp.db_migrate = true;

    let mut source = Daemon::new_random_with_args(args_source, 64);
    let mut target = Daemon::new_random_with_args(args_target, 64);

    let miner_client = source.start().await;
    let target_client = target.start().await;

    target_client.add_peer(format!("127.0.0.1:{}", source.p2p_port).try_into().unwrap(), true).await.unwrap();
    wait_for(
        50,
        40,
        || {
            let client = target_client.clone();
            async move { client.get_connected_peer_info().await.unwrap().peer_info.len() == 1 }
        },
        "nodes failed to connect",
    )
    .await;

    let network = source.client_manager().network;
    let pay_address = fixed_address(network);

    let baseline = measure_sync_duration(BASELINE_BLOCKS, &miner_client, &target_client, &pay_address).await;

    let sample_block = miner_client.get_block_template(pay_address.clone(), vec![]).await.unwrap().block;
    let sample_payload = pb_block_bytes(&rpc_block_to_pb(&sample_block));
    let stop = Arc::new(AtomicBool::new(false));
    let udp_target_addr = target.client_manager().args.read().udp.listen.unwrap();
    let target_network = target.client_manager().network;
    let flood_handle = spawn_udp_flood(udp_target_addr, target_network, sample_payload, stop.clone());

    let stressed = measure_sync_duration(BASELINE_BLOCKS, &miner_client, &target_client, &pay_address).await;
    stop.store(true, Ordering::SeqCst);
    flood_handle.await.unwrap();

    let allowed = baseline.mul_f64(1.2) + Duration::from_millis(200);
    assert!(stressed <= allowed, "UDP flood degraded throughput beyond tolerance");

    miner_client.disconnect().await.unwrap();
    target_client.disconnect().await.unwrap();
    source.shutdown();
    target.shutdown();
}

fn base_args() -> Args {
    Args { simnet: true, unsafe_rpc: true, disable_upnp: true, ..Default::default() }
}

fn reserve_udp_addr() -> SocketAddr {
    let socket = StdUdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
    addr
}

fn fixed_address(network: NetworkId) -> Address {
    let payload = [0u8; 32];
    Address::new(network.into(), Version::PubKey, &payload)
}

async fn measure_sync_duration(blocks: u64, miner: &GrpcClient, target: &GrpcClient, pay_address: &Address) -> Duration {
    let mut expected = target.get_block_dag_info().await.unwrap().block_count;
    let start = Instant::now();
    for _ in 0..blocks {
        let template = miner.get_block_template(pay_address.clone(), vec![]).await.unwrap();
        miner.submit_block(template.block, false).await.unwrap();
        expected += 1;
        wait_for(
            50,
            80,
            || {
                let client = target.clone();
                let expected = expected;
                async move { client.get_block_dag_info().await.unwrap().block_count >= expected }
            },
            "target did not process block",
        )
        .await;
    }
    start.elapsed()
}

fn rpc_block_to_pb(rpc_block: &RpcRawBlock) -> pb::BlockMessage {
    let block: Block = rpc_block.clone().try_into().expect("raw block conversion");
    (&block).into()
}

fn pb_block_bytes(block: &pb::BlockMessage) -> Vec<u8> {
    let mut buf = Vec::new();
    block.encode(&mut buf).unwrap();
    buf
}

async fn send_block_over_udp(addr: SocketAddr, network: &NetworkId, seq: u64, block: &pb::BlockMessage) {
    let payload = pb_block_bytes(block);
    let datagram = encode_datagram(&build_header(seq), &payload, encode_network_tag(network));
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(&datagram, addr).await.unwrap();
}

fn build_header(seq: u64) -> SatFrameHeader {
    SatFrameHeader {
        kind: FrameKind::Block,
        flags: FrameFlags::from_bits(0),
        seq,
        group_id: 0,
        group_k: 0,
        group_n: 0,
        frag_ix: 0,
        frag_cnt: 1,
        payload_len: 0,
        source_id: 1,
    }
}

fn encode_datagram(header: &SatFrameHeader, payload: &[u8], network_tag: u8) -> Vec<u8> {
    use crc32fast::Hasher;

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
    let mut hasher = Hasher::new();
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
    let suffix = id.suffix().unwrap_or(0) as u8;
    base | (suffix << 4)
}

fn spawn_udp_flood(addr: SocketAddr, network: NetworkId, payload: Vec<u8>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut seq = 10_000u64;
        while !stop.load(Ordering::Relaxed) {
            let header = build_header(seq);
            let datagram = encode_datagram(&header, &payload, encode_network_tag(&network));
            let _ = socket.send_to(&datagram, addr).await;
            seq = seq.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
}
