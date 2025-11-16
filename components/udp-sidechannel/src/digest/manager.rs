use super::{DigestError, DigestParser, DigestSnapshot, DigestStore, DigestStoreError, DigestVariant, SignerRegistry, TimestampSkew};
use crate::{
    config::UdpConfig,
    frame::{DropReason, FrameKind, SatFrameHeader},
    metrics::UdpMetrics,
    service::{FrameConsumerError, UdpIngestService},
};
use bytes::Bytes;
use kaspa_core::{debug, info, trace, warn};
use kaspa_database::prelude::DB;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::sleep;

pub struct UdpDigestManager {
    parser: DigestParser,
    metrics: Arc<UdpMetrics>,
    state: Mutex<DigestState>,
    store: Option<DigestStore>,
    retention_count: usize,
    retention_days: u32,
}

#[derive(Default)]
struct DigestState {
    last: Option<DigestVariant>,
    sources: HashMap<u16, SourceState>,
    divergence: DivergenceState,
}

#[derive(Debug, Clone)]
struct SourceState {
    last_epoch: u64,
    last_timestamp_ms: u64,
    signer_id: u16,
    signature_valid: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DivergenceState {
    pub detected: bool,
    pub last_mismatch_epoch: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DigestSourceReport {
    pub source_id: u16,
    pub last_epoch: u64,
    pub last_timestamp_ms: u64,
    pub signer_id: u16,
    pub signature_valid: bool,
}

#[derive(Debug, Clone)]
pub struct DigestReport {
    pub last_digest: Option<DigestVariant>,
    pub sources: Vec<DigestSourceReport>,
    pub divergence: DivergenceState,
    pub signature_failures: u64,
    pub skew_seconds: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum DigestInitError {
    #[error("signer registry error: {0:?}")]
    Signers(super::SignerError),
    #[error("failed to start frame consumer: {0}")]
    Frame(#[from] FrameConsumerError),
}

impl UdpDigestManager {
    pub fn start(
        config: &UdpConfig,
        service: Arc<UdpIngestService>,
        metrics: Arc<UdpMetrics>,
        db: Option<Arc<DB>>,
    ) -> Result<Arc<Self>, DigestInitError> {
        let registry = SignerRegistry::from_hex(&config.allowed_signers).map_err(DigestInitError::Signers)?;
        let parser = DigestParser::new(config.require_signature, registry, TimestampSkew::default());
        let store = db.map(DigestStore::new);
        let manager = Arc::new(Self {
            parser,
            metrics,
            state: Mutex::new(DigestState::default()),
            store,
            retention_count: config.retention_count as usize,
            retention_days: config.retention_days,
        });
        manager.spawn_consumer(service)?;
        manager.spawn_pruner();
        Ok(manager)
    }

    fn spawn_pruner(self: &Arc<Self>) {
        if self.store.is_none() {
            return;
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(60)).await;
                if let Some(store) = &this.store {
                    if let Err(err) = store.prune(this.retention_count, this.retention_days) {
                        warn!("udp.event=digest_prune_error err={err}");
                    }
                }
            }
        });
    }

    fn spawn_consumer(self: &Arc<Self>, service: Arc<UdpIngestService>) -> Result<(), DigestInitError> {
        let this = Arc::clone(self);
        service
            .spawn_frame_consumer(move |header, payload| {
                let this = Arc::clone(&this);
                async move {
                    this.handle_frame(header, payload);
                }
            })
            .map_err(DigestInitError::Frame)?;
        Ok(())
    }

    fn handle_frame(&self, header: SatFrameHeader, payload: Bytes) {
        match header.kind {
            FrameKind::Digest => match self.parser.parse(&header, payload.as_ref()) {
                Ok(variant) => self.record_variant(variant),
                Err(err) => self.handle_error(&header, err),
            },
            FrameKind::Block => {
                trace!("udp.event=frame_ignored kind=block seq={}", header.seq);
            }
        }
    }

    fn record_variant(&self, variant: DigestVariant) {
        {
            let mut state = self.state.lock().expect("digest state poisoned");
            let source_id = variant.source_id();
            state.last = Some(variant.clone());
            let entry = state.sources.entry(source_id).or_insert_with(|| SourceState {
                last_epoch: 0,
                last_timestamp_ms: 0,
                signer_id: 0,
                signature_valid: false,
            });
            if entry.last_epoch > 0 && variant.epoch() < entry.last_epoch {
                warn!("udp.event=digest_drop reason=epoch source={} epoch={}", source_id, variant.epoch());
                return;
            }
            entry.last_epoch = variant.epoch();
            entry.last_timestamp_ms = variant.recv_timestamp_ms();
            entry.signer_id = variant.signer_id();
            entry.signature_valid = variant.signature_valid();
        }
        if let Some(store) = &self.store {
            if let Err(err) = store.insert(&variant) {
                warn!("udp.event=digest_store_error err={err}");
            }
        }
        debug!(
            "udp.event=digest_ingested epoch={} signer={} source={} kind={}",
            variant.epoch(),
            variant.signer_id(),
            variant.source_id(),
            match variant {
                DigestVariant::Snapshot(_) => "snapshot",
                DigestVariant::Delta(_) => "delta",
            }
        );
    }

    fn handle_error(&self, header: &SatFrameHeader, err: DigestError) {
        match err {
            DigestError::InvalidSignature | DigestError::SignatureVerificationFailed => {
                self.metrics.record_drop(DropReason::Signature);
                self.metrics.record_signature_failure();
                warn!("udp.event=digest_drop reason=signature seq={}", header.seq);
            }
            DigestError::UnknownSigner(id) => {
                self.metrics.record_drop(DropReason::Signature);
                self.metrics.record_signature_failure();
                warn!("udp.event=digest_drop reason=unknown_signer signer_id={} seq={}", id, header.seq);
            }
            DigestError::TimestampFuture(delta) | DigestError::TimestampStale(delta) => {
                self.metrics.record_skew_seconds(delta / 1000);
                warn!("udp.event=digest_drop reason=skew delta_ms={} seq={}", delta, header.seq);
            }
            _ => {
                warn!("udp.event=digest_drop reason=parser seq={} err={}", header.seq, err);
            }
        }
    }

    pub fn report(&self) -> DigestReport {
        let state = self.state.lock().expect("digest state poisoned");
        let sources = state
            .sources
            .iter()
            .map(|(id, s)| DigestSourceReport {
                source_id: *id,
                last_epoch: s.last_epoch,
                last_timestamp_ms: s.last_timestamp_ms,
                signer_id: s.signer_id,
                signature_valid: s.signature_valid,
            })
            .collect();
        DigestReport {
            last_digest: state.last.clone(),
            sources,
            divergence: state.divergence,
            signature_failures: self.metrics.signature_failures(),
            skew_seconds: self.metrics.skew_seconds(),
        }
    }

    pub fn fetch_digests(&self, from_epoch: Option<u64>, limit: usize) -> Result<Vec<DigestVariant>, DigestStoreError> {
        match &self.store {
            Some(store) => store.fetch_recent(from_epoch, limit),
            None => Ok(Vec::new()),
        }
    }

    pub fn latest_snapshot(&self) -> Option<DigestSnapshot> {
        let state = self.state.lock().expect("digest state poisoned");
        match &state.last {
            Some(DigestVariant::Snapshot(snapshot)) if snapshot.signature_valid => Some(snapshot.clone()),
            _ => None,
        }
    }

    pub fn update_divergence(&self, detected: bool, last_epoch: Option<u64>, reason: &'static str) {
        let mut state = self.state.lock().expect("digest state poisoned");
        if state.divergence.detected != detected {
            let state_str = if detected { "entered" } else { "cleared" };
            let reason_str = if detected { reason } else { "resolved" };
            info!("udp.event=divergence state={} reason={} epoch={:?}", state_str, reason_str, last_epoch);
        }
        state.divergence = DivergenceState { detected, last_mismatch_epoch: last_epoch };
        self.metrics.set_divergence_detected(detected);
    }

    pub fn divergence_state(&self) -> DivergenceState {
        self.state.lock().expect("digest state poisoned").divergence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::types::DIGEST_SIGNATURE_LEN;
    use crate::DigestDelta;
    use kaspa_database::{create_temp_db, prelude::ConnBuilder};
    use kaspa_hashes::Hash;

    #[test]
    fn divergence_simulation() {
        let manager = UdpDigestManager::test_instance();
        manager.update_divergence(true, Some(10), "snapshot_mismatch");
        let state = manager.divergence_state();
        assert!(state.detected);
        assert_eq!(state.last_mismatch_epoch, Some(10));
        assert!(manager.metrics.divergence_detected());

        manager.update_divergence(false, Some(11), "resolved");
        let state = manager.divergence_state();
        assert!(!state.detected);
        assert_eq!(state.last_mismatch_epoch, Some(11));
        assert!(!manager.metrics.divergence_detected());
    }

    #[test]
    fn delta_epoch_guard() {
        let manager = UdpDigestManager::test_instance();
        let first = sample_delta(1);
        manager.record_variant(first);
        let state = manager.report();
        assert_eq!(state.sources.len(), 1);
        assert_eq!(state.sources[0].last_epoch, 1);

        let older = sample_delta(0);
        manager.record_variant(older);
        let state = manager.report();
        assert_eq!(state.sources[0].last_epoch, 1, "older epoch ignored");
    }

    #[test]
    fn e2e_digest_path() {
        let (_guard, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let store = DigestStore::new(db);
        let manager = UdpDigestManager {
            parser: DigestParser::new(false, SignerRegistry::empty(), TimestampSkew::default()),
            metrics: Arc::new(UdpMetrics::new()),
            state: Mutex::new(DigestState::default()),
            store: Some(store),
            retention_count: 10,
            retention_days: 0,
        };
        manager.record_variant(sample_snapshot(1));
        manager.record_variant(sample_delta(2));
        let digests = manager.fetch_digests(None, 10).unwrap();
        assert_eq!(digests.len(), 2);
        assert_eq!(digests[0].epoch(), 2);
        assert_eq!(digests[1].epoch(), 1);
        let filtered = manager.fetch_digests(Some(2), 10).unwrap();
        assert_eq!(filtered.len(), 1, "from_epoch filter keeps newest entries only");
        let report = manager.report();
        assert!(report.last_digest.is_some(), "last digest tracked");
        assert!(matches!(report.last_digest.unwrap(), DigestVariant::Delta(_)), "latest digest is the delta");
        assert_eq!(report.sources.len(), 1);
        assert!(report.sources[0].signature_valid);
    }

    fn sample_snapshot(epoch: u64) -> DigestVariant {
        DigestVariant::Snapshot(DigestSnapshot {
            epoch,
            pruning_point: Hash::from_bytes([epoch as u8; 32]),
            pruning_proof_commitment: Hash::from_bytes([1; 32]),
            utxo_muhash: Hash::from_bytes([2; 32]),
            virtual_selected_parent: Hash::from_bytes([3; 32]),
            virtual_blue_score: epoch * 5,
            daa_score: epoch * 7,
            blue_work: [4; 32],
            kept_headers_mmr_root: None,
            signer_id: 0,
            signature: [0u8; DIGEST_SIGNATURE_LEN],
            signature_valid: true,
            frame_timestamp_ms: epoch,
            recv_timestamp_ms: epoch,
            source_id: 1,
        })
    }

    fn sample_delta(epoch: u64) -> DigestVariant {
        let base_hash = Hash::from_bytes([epoch as u8; 32]);
        DigestVariant::Delta(DigestDelta {
            epoch,
            virtual_selected_parent: base_hash,
            virtual_blue_score: epoch * 10,
            daa_score: epoch * 5,
            blue_work: [epoch as u8; 32],
            signer_id: 0,
            signature: [1u8; DIGEST_SIGNATURE_LEN],
            signature_valid: true,
            frame_timestamp_ms: 0,
            recv_timestamp_ms: epoch,
            source_id: 1,
        })
    }
}

#[cfg(test)]
impl UdpDigestManager {
    fn test_instance() -> Self {
        Self {
            parser: DigestParser::new(false, SignerRegistry::empty(), TimestampSkew::default()),
            metrics: Arc::new(UdpMetrics::new()),
            state: Mutex::new(DigestState::default()),
            store: None,
            retention_count: 0,
            retention_days: 0,
        }
    }
}
