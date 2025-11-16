use bytes::Bytes;
use kaspa_addressmanager::AddressManager;
use kaspa_consensus::{
    config::{params::SIMNET_PARAMS, ConfigBuilder},
    consensus::test_consensus::{TestConsensus, TestConsensusFactory},
};
use kaspa_consensus_core::{
    block::Block,
    blockstatus::BlockStatus,
    coinbase::MinerData,
    tx::{ScriptPublicKey, Transaction},
};
use kaspa_consensusmanager::{ConsensusManager, ConsensusProxy};
use kaspa_core::task::tick::TickService;
use kaspa_database::{
    prelude::{ConnBuilder, DB},
    utils::get_kaspa_tempdir,
};
use kaspa_hashes::Hash;
use kaspa_mining::{manager::MiningManager, manager::MiningManagerProxy, MiningCounters};
use kaspa_p2p_flows::flow_context::FlowContext;
use kaspa_p2p_lib::{
    common::ProtocolError,
    pb::{self, kaspad_message::Payload as KaspadPayload, KaspadMessage},
    Hub,
};
use kaspa_p2p_mining::rule_engine::MiningRuleEngine;
use kaspa_udp_sidechannel::{
    block::{BlockChannel, BlockChannelError, BlockParser, BlockQueue},
    frame::{FrameFlags, FrameKind, SatFrameHeader},
    injector::router_peer::spawn_block_injector,
    metrics::UdpMetrics,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use prost::Message;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};
use thiserror::Error;
use tokio::time::{sleep, timeout};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_block_equivalence_fast() -> Result<(), HarnessError> {
    let harness_peer = FlowHarnessBuilder::new().build().await?;
    harness_peer.advance_chain(2).await?;
    let block = harness_peer.build_block_on_tip().await?;
    harness_peer.deliver_peer_block(&block).await?;
    harness_peer.wait_for_block(block.hash()).await?;
    let peer_state = harness_peer.snapshot_state().await?;

    let harness_udp = FlowHarnessBuilder::new().build().await?;
    harness_udp.advance_chain(2).await?;
    harness_udp.deliver_udp_block(&block).await?;
    harness_udp.wait_for_block(block.hash()).await?;
    let udp_state = harness_udp.snapshot_state().await?;

    assert_eq!(peer_state, udp_state, "peer and UDP paths diverged");
    assert_eq!(harness_udp.metrics().block_injected_total(), 1, "injection metric mismatch");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_block_fairness_fast() -> Result<(), HarnessError> {
    let harness = FlowHarnessBuilder::new().block_queue_capacity(8).build().await?;
    harness.advance_chain(3).await?;

    let base_parent = harness.sink().await?;
    let udp_blocks: Vec<HarnessBlock> = (0..12).map(|_| harness.build_block_with_parents(vec![base_parent])).collect();
    let udp_hashes: Vec<Hash> = udp_blocks.iter().map(|blk| blk.hash()).collect();
    let udp_blocks_for_task = udp_blocks.clone();
    let flood_harness = harness.clone();
    let flood = tokio::spawn(async move {
        let mut max_depth = 0u64;
        for blk in udp_blocks_for_task {
            flood_harness.deliver_udp_block(&blk).await.expect("udp inject");
            let depth = flood_harness.metrics().block_queue_depth();
            max_depth = max_depth.max(depth);
            sleep(Duration::from_millis(5)).await;
        }
        max_depth
    });

    let mut peer_chain = Vec::new();
    for _ in 0..3 {
        let block = harness.build_block_on_tip().await?;
        harness.deliver_peer_block(&block).await?;
        harness.wait_for_block(block.hash()).await?;
        peer_chain.push(block.hash());
        sleep(Duration::from_millis(10)).await;
    }

    let max_depth = flood.await.expect("udp flood task panicked");
    for hash in udp_hashes {
        harness.wait_for_block(hash).await?;
    }

    let sink = harness.sink().await?;
    let peer_tail = *peer_chain.last().expect("peer blocks present");
    assert_eq!(sink, peer_tail, "peer chain failed to advance under UDP flood");
    assert_eq!(harness.metrics().block_injected_total(), 12, "unexpected injection count");
    let capacity = harness.queue_capacity() as u64;
    assert!(max_depth < capacity, "block queue saturated ({max_depth} vs {capacity})");
    assert_eq!(harness.metrics().block_queue_depth(), 0, "block queue failed to drain");

    Ok(())
}

#[derive(Clone)]
struct FlowHarness {
    inner: Arc<FlowHarnessInner>,
}

struct FlowHarnessInner {
    test_consensus: Arc<TestConsensus>,
    consensus_handles: Mutex<Option<Vec<JoinHandle<()>>>>,
    flow_context: Arc<FlowContext>,
    udp_metrics: Arc<UdpMetrics>,
    block_channel: BlockChannel,
    block_id: AtomicU64,
    frame_seq: AtomicU64,
    queue_capacity: usize,
    tick_service: Arc<TickService>,
}

impl Drop for FlowHarnessInner {
    fn drop(&mut self) {
        if let Some(handles) = self.consensus_handles.lock().take() {
            self.test_consensus.shutdown(handles);
        }
        self.tick_service.shutdown();
        retain_test_consensus(self.test_consensus.clone());
    }
}

impl FlowHarness {
    async fn advance_chain(&self, count: usize) -> Result<(), HarnessError> {
        for _ in 0..count {
            let block = self.build_block_on_tip().await?;
            self.deliver_peer_block(&block).await?;
            self.wait_for_block(block.hash()).await?;
        }
        Ok(())
    }

    fn metrics(&self) -> Arc<UdpMetrics> {
        self.inner.udp_metrics.clone()
    }

    fn queue_capacity(&self) -> usize {
        self.inner.queue_capacity
    }

    fn build_block_with_parents(&self, parents: Vec<Hash>) -> HarnessBlock {
        let hash = Hash::from_u64_word(self.inner.block_id.fetch_add(1, Ordering::Relaxed) + 1);
        let miner_data = MinerData::new(ScriptPublicKey::from_vec(0, vec![]), vec![]);
        let block = self
            .inner
            .test_consensus
            .build_utxo_valid_block_with_parents(hash, parents, miner_data, Vec::<Transaction>::new())
            .to_immutable();
        let pb_block = (&block).into();
        HarnessBlock::new(block, pb_block)
    }

    async fn build_block_on_tip(&self) -> Result<HarnessBlock, HarnessError> {
        let parent = self.sink().await?;
        Ok(self.build_block_with_parents(vec![parent]))
    }

    async fn deliver_peer_block(&self, block: &HarnessBlock) -> Result<(), HarnessError> {
        let session = self.consensus_session().await;
        self.inner.flow_context.submit_rpc_block(&session, block.consensus_block()).await.map_err(HarnessError::Protocol)
    }

    async fn deliver_udp_block(&self, block: &HarnessBlock) -> Result<(), HarnessError> {
        let payload = Bytes::from(block.encode());
        let header = SatFrameHeader {
            kind: FrameKind::Block,
            flags: FrameFlags::from_bits(0),
            seq: self.inner.frame_seq.fetch_add(1, Ordering::Relaxed) + 1,
            group_id: 0,
            group_k: 0,
            group_n: 0,
            frag_ix: 0,
            frag_cnt: 1,
            payload_len: payload.len() as u32,
            source_id: 0,
        };
        self.inner.block_channel.enqueue(header, payload).map_err(HarnessError::Queue)
    }

    async fn wait_for_block(&self, hash: Hash) -> Result<(), HarnessError> {
        timeout(Duration::from_secs(10), async {
            loop {
                if let Some(status) = self.block_status(hash).await? {
                    if matches!(status, BlockStatus::StatusUTXOValid) {
                        break Ok(());
                    }
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| HarnessError::Timeout(hash))?
    }

    async fn block_status(&self, hash: Hash) -> Result<Option<BlockStatus>, HarnessError> {
        Ok(self.consensus_session().await.async_get_block_status(hash).await)
    }

    async fn snapshot_state(&self) -> Result<HarnessState, HarnessError> {
        let session = self.consensus_session().await;
        let block_counts = session.async_estimate_block_count().await;
        Ok(HarnessState {
            sink: session.async_get_sink().await,
            blue_score: session.async_get_sink_blue_score().await,
            block_count: block_counts.block_count,
        })
    }

    async fn sink(&self) -> Result<Hash, HarnessError> {
        Ok(self.consensus_session().await.async_get_sink().await)
    }

    async fn consensus_session(&self) -> ConsensusProxy {
        self.inner.flow_context.consensus().session().await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HarnessState {
    sink: Hash,
    blue_score: u64,
    block_count: u64,
}

#[derive(Clone)]
struct HarnessBlock {
    block: Block,
    message: Arc<pb::BlockMessage>,
}

impl HarnessBlock {
    fn new(block: Block, message: pb::BlockMessage) -> Self {
        Self { block, message: Arc::new(message) }
    }

    fn hash(&self) -> Hash {
        self.block.hash()
    }

    fn kaspad_message(&self) -> KaspadMessage {
        KaspadMessage { payload: Some(KaspadPayload::Block((&*self.message).clone())), ..Default::default() }
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1024);
        self.message.encode(&mut buf).expect("encode block");
        buf
    }

    fn consensus_block(&self) -> Block {
        self.block.clone()
    }
}

struct FlowHarnessBuilder {
    block_queue: usize,
    block_max_bytes: usize,
}

impl FlowHarnessBuilder {
    fn new() -> Self {
        Self { block_queue: 32, block_max_bytes: 1 << 20 }
    }

    fn block_queue_capacity(mut self, capacity: usize) -> Self {
        self.block_queue = capacity;
        self
    }

    async fn build(self) -> Result<FlowHarness, HarnessError> {
        let mut config = ConfigBuilder::new(SIMNET_PARAMS).skip_proof_of_work().build();
        config.disable_upnp = true;
        let config = Arc::new(config);
        let test_consensus = Arc::new(TestConsensus::new(config.as_ref()));
        let handles = test_consensus.init();
        let factory = Arc::new(TestConsensusFactory::new(test_consensus.clone()));
        let consensus_manager = Arc::new(ConsensusManager::new(factory));

        let tick_service = Arc::new(TickService::new());
        let meta_db = test_meta_db();
        let (address_manager, _) = AddressManager::new(config.clone(), meta_db, tick_service.clone());

        let hub = Hub::new();
        let mining_rules = Arc::new(kaspa_consensus_core::mining_rules::MiningRules::default());
        let mining_counters = Arc::new(MiningCounters::default());
        let mining_manager = MiningManagerProxy::new(Arc::new(MiningManager::new_with_extended_config(
            config.params.target_time_per_block(),
            false,
            config.params.max_block_mass,
            config.ram_scale,
            config.block_template_cache_lifetime,
            mining_counters,
        )));
        let mining_rule_engine = Arc::new(MiningRuleEngine::new(
            consensus_manager.clone(),
            config.clone(),
            test_consensus.processing_counters().clone(),
            tick_service.clone(),
            hub.clone(),
            mining_rules,
        ));

        let flow_context = Arc::new(FlowContext::new(
            consensus_manager.clone(),
            address_manager,
            config.clone(),
            mining_manager,
            tick_service.clone(),
            test_consensus.notification_root(),
            hub,
            mining_rule_engine.clone(),
        ));
        flow_context.start_async_services();

        let sat_injector = flow_context.create_sat_virtual_peer().map_err(HarnessError::Protocol)?;

        let udp_metrics = Arc::new(UdpMetrics::new());
        let parser = BlockParser::new(self.block_max_bytes);
        let queue = Arc::new(BlockQueue::new(self.block_queue, udp_metrics.clone()));
        let block_channel = BlockChannel::new(parser, queue);
        let rx = block_channel.take_rx().expect("block queue receiver available");
        spawn_block_injector(rx, sat_injector.clone(), udp_metrics.clone());

        Ok(FlowHarness {
            inner: Arc::new(FlowHarnessInner {
                test_consensus,
                consensus_handles: Mutex::new(Some(handles)),
                flow_context,
                udp_metrics,
                block_channel,
                block_id: AtomicU64::new(0),
                frame_seq: AtomicU64::new(0),
                queue_capacity: self.block_queue,
                tick_service,
            }),
        })
    }
}

fn test_meta_db() -> Arc<DB> {
    static META_DB: Lazy<Arc<DB>> = Lazy::new(|| {
        let dir = get_kaspa_tempdir();
        let path = dir.into_path();
        ConnBuilder::default().with_files_limit(2).with_db_path(path).build().unwrap()
    });
    META_DB.clone()
}

fn retain_test_consensus(tc: Arc<TestConsensus>) {
    static LEAKED: Lazy<parking_lot::Mutex<Vec<Arc<TestConsensus>>>> = Lazy::new(|| parking_lot::Mutex::new(Vec::new()));
    LEAKED.lock().push(tc);
}

#[derive(Debug, Error)]
enum HarnessError {
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("block queue error: {0:?}")]
    Queue(BlockChannelError),
    #[error("timed out waiting for block {0}")]
    Timeout(Hash),
}
