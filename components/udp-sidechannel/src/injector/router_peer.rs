use crate::{
    block::{BlockPayload, QueuedBlock},
    frame::DropReason,
    metrics::UdpMetrics,
};
use kaspa_connectionmanager::{InjectError, PeerMessageInjector};
use kaspa_core::{debug, warn};
use kaspa_p2p_lib::pb::{self, kaspad_message::Payload as KaspadPayload, KaspadMessage};
use prost::Message;
use std::sync::Arc;
use tokio::sync::mpsc;

pub fn spawn_block_injector(mut rx: mpsc::Receiver<QueuedBlock>, injector: Arc<dyn PeerMessageInjector>, metrics: Arc<UdpMetrics>) {
    tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            handle_block(item, injector.as_ref(), &metrics);
        }
    });
}

fn handle_block(block: QueuedBlock, injector: &dyn PeerMessageInjector, metrics: &UdpMetrics) {
    let (header, payload_variant) = block.into_parts();
    let seq = header.seq;
    let BlockPayload::Full(payload) = payload_variant;

    let block_message = match pb::BlockMessage::decode(payload.as_slice()) {
        Ok(msg) => msg,
        Err(err) => {
            warn!("udp.event=block_decode_fail seq={} err={}", seq, err);
            metrics.record_drop(DropReason::BlockMalformed);
            return;
        }
    };

    let msg = KaspadMessage { payload: Some(KaspadPayload::Block(block_message)), ..Default::default() };
    match injector.inject(msg) {
        Ok(()) => {
            metrics.record_block_injected();
            debug!("udp.event=block_inject seq={}", seq);
        }
        Err(InjectError::QueueFull) => {
            metrics.record_drop(DropReason::BlockQueueFull);
            warn!("udp.event=block_drop reason=queue_full seq={}", seq);
        }
        Err(InjectError::Disconnected) => {
            metrics.record_drop(DropReason::Panic);
            warn!("udp.event=block_drop reason=injector_disconnected seq={}", seq);
        }
        Err(InjectError::Protocol(err)) => {
            metrics.record_drop(DropReason::Panic);
            warn!("udp.event=block_drop reason=injector_protocol seq={} err={}", seq, err);
        }
    }
}
