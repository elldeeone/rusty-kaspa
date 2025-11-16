use std::sync::Arc;

use kaspa_consensusmanager::ConsensusManager;
use kaspa_core::task::service::{AsyncService, AsyncServiceFuture};
use kaspa_udp_sidechannel::UdpDigestManager;
use tokio::{
    sync::watch,
    time::{interval, Duration as TokioDuration, MissedTickBehavior},
};

#[derive(Clone)]
pub struct UdpDivergenceMonitor {
    consensus_manager: Arc<ConsensusManager>,
    digest_manager: Arc<UdpDigestManager>,
    shutdown: watch::Sender<bool>,
}

impl UdpDivergenceMonitor {
    pub fn new(consensus_manager: Arc<ConsensusManager>, digest_manager: Arc<UdpDigestManager>) -> Arc<Self> {
        let (shutdown, _) = watch::channel(false);
        Arc::new(Self { consensus_manager, digest_manager, shutdown })
    }

    async fn tick(&self) {
        if let Some(snapshot) = self.digest_manager.latest_snapshot() {
            let session = self.consensus_manager.consensus().session().await;
            let pruning_point = session.async_pruning_point().await;
            let sink = session.async_get_sink().await;
            let sink_blue_score = session.async_get_sink_blue_score().await;
            let virtual_daa_score = session.get_virtual_daa_score();
            let mismatch = snapshot.pruning_point != pruning_point
                || snapshot.virtual_selected_parent != sink
                || snapshot.virtual_blue_score != sink_blue_score
                || snapshot.daa_score != virtual_daa_score;
            self.digest_manager.update_divergence(mismatch, Some(snapshot.epoch), "snapshot_mismatch");
        } else {
            self.digest_manager.update_divergence(false, None, "resolved");
        }
    }
}

impl AsyncService for UdpDivergenceMonitor {
    fn ident(self: Arc<Self>) -> &'static str {
        "udp-divergence-monitor"
    }

    fn start(self: Arc<Self>) -> AsyncServiceFuture {
        Box::pin(async move {
            let mut shutdown_rx = self.shutdown.subscribe();
            let mut ticker = interval(TokioDuration::from_secs(5));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => break,
                    _ = ticker.tick() => {
                        self.tick().await;
                    }
                }
            }
            Ok(())
        })
    }

    fn signal_exit(self: Arc<Self>) {
        let _ = self.shutdown.send(true);
    }

    fn stop(self: Arc<Self>) -> AsyncServiceFuture {
        Box::pin(async move {
            let _ = self.shutdown.send(true);
            Ok(())
        })
    }
}
