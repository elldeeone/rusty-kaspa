use std::{sync::Arc, time::Duration};

use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_core::task::service::AsyncService;
use kaspa_udp_sidechannel::{UdpConfig, UdpIngestService, UdpMetrics, UdpMode};
use tokio::time::{sleep, timeout};

fn test_config() -> UdpConfig {
    UdpConfig {
        enable: true,
        listen: Some("127.0.0.1:0".parse().expect("loopback addr")),
        listen_unix: None,
        allow_non_local_bind: false,
        mode: UdpMode::Digest,
        max_kbps: 16,
        require_signature: false,
        allowed_signers: vec![],
        digest_queue: 4,
        block_queue: 0,
        danger_accept_blocks: false,
        block_mainnet_override: false,
        discard_unsigned: true,
        db_migrate: false,
        retention_count: 1,
        retention_days: 1,
        max_digest_payload_bytes: 2048,
        max_block_payload_bytes: 131_072,
        block_max_bytes: 131_072,
        log_verbosity: "info".into(),
        admin_remote_allowed: false,
        admin_token_file: None,
        network_id: NetworkId::new(NetworkType::Simnet),
    }
}

#[tokio::test]
async fn ingest_service_exits_on_signal() {
    let cfg = test_config();
    let service = Arc::new(UdpIngestService::new(cfg, Arc::new(UdpMetrics::new()), None, None));
    let runner = {
        let svc = service.clone();
        tokio::spawn(async move { svc.start().await })
    };

    sleep(Duration::from_millis(50)).await;
    service.signal_exit();

    timeout(Duration::from_secs(5), runner)
        .await
        .expect("ingest service join timed out")
        .expect("ingest service task panicked")
        .expect("ingest service returned error");
}
