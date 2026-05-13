use std::sync::Arc;

use kaspa_consensus_core::BlueWorkType;
use kaspa_consensusmanager::ConsensusManager;
use kaspa_core::{
    task::service::{AsyncService, AsyncServiceFuture},
    trace,
};
use kaspa_udp_sidechannel::UdpDigestManager;
use kaspa_utils::triggers::Listener;
use tokio::{
    sync::watch,
    time::{interval, Duration as TokioDuration, MissedTickBehavior},
};

#[derive(Clone)]
pub struct UdpDivergenceMonitor {
    consensus_manager: Arc<ConsensusManager>,
    digest_manager: Arc<UdpDigestManager>,
    shutdown: watch::Sender<bool>,
    flow_shutdown: Listener,
}

impl UdpDivergenceMonitor {
    pub fn new(consensus_manager: Arc<ConsensusManager>, digest_manager: Arc<UdpDigestManager>, flow_shutdown: Listener) -> Arc<Self> {
        let (shutdown, _) = watch::channel(false);
        Arc::new(Self { consensus_manager, digest_manager, shutdown, flow_shutdown })
    }

    async fn tick(&self) {
        if let Some(snapshot) = self.digest_manager.latest_snapshot() {
            let session = self.consensus_manager.consensus().session().await;
            let pruning_point = session.async_pruning_point().await;
            let sink = session.async_get_sink().await;
            let sink_blue_score = session.async_get_sink_blue_score().await;
            let virtual_daa_score = session.get_virtual_daa_score();
            let mut mismatch_fields = Vec::new();
            if snapshot.pruning_point != pruning_point {
                mismatch_fields.push("pruning_point");
            }
            if snapshot.virtual_selected_parent != sink {
                mismatch_fields.push("virtual_selected_parent");
            }
            if snapshot.virtual_blue_score != sink_blue_score {
                mismatch_fields.push("virtual_blue_score");
            }
            if snapshot.daa_score != virtual_daa_score {
                mismatch_fields.push("daa_score");
            }
            if let Ok(header) = session.async_get_header(sink).await {
                if snapshot.blue_work != blue_work_to_digest_bytes(header.blue_work) {
                    mismatch_fields.push("blue_work");
                }
            }
            let mismatch = !mismatch_fields.is_empty();
            let mismatch_epoch = mismatch.then_some(snapshot.epoch);
            self.digest_manager.update_divergence(mismatch, mismatch_epoch, "snapshot_mismatch", &mismatch_fields);
        } else {
            self.digest_manager.update_divergence(false, None, "resolved", &[]);
        }
    }
}

fn blue_work_to_digest_bytes(blue_work: BlueWorkType) -> [u8; 32] {
    let source = blue_work.to_be_bytes();
    let mut out = [0u8; 32];
    out[32 - source.len()..].copy_from_slice(&source);
    out
}

impl AsyncService for UdpDivergenceMonitor {
    fn ident(self: Arc<Self>) -> &'static str {
        "udp-divergence-monitor"
    }

    fn start(self: Arc<Self>) -> AsyncServiceFuture {
        Box::pin(async move {
            let mut shutdown_rx = self.shutdown.subscribe();
            let flow_shutdown = self.flow_shutdown.clone();
            let mut ticker = interval(TokioDuration::from_secs(5));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => break,
                    _ = flow_shutdown.clone() => break,
                    _ = ticker.tick() => {
                        self.tick().await;
                    }
                }
            }
            trace!("udp-divergence-monitor stopped");
            Ok(())
        })
    }

    fn signal_exit(self: Arc<Self>) {
        trace!("udp-divergence-monitor signal_exit");
        let _ = self.shutdown.send(true);
    }

    fn stop(self: Arc<Self>) -> AsyncServiceFuture {
        Box::pin(async move {
            let _ = self.shutdown.send(true);
            Ok(())
        })
    }
}
