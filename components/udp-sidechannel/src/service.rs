use crate::{
    config::{BindTarget, UdpConfig},
    metrics::UdpMetrics,
};
use kaspa_core::task::service::{AsyncService, AsyncServiceError, AsyncServiceFuture, AsyncServiceResult};
use kaspa_core::{error, info, trace, warn};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use thiserror::Error;
use tokio::{net::UdpSocket, sync::watch};

#[cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, sync::Mutex};
#[cfg(unix)]
use tokio::net::UnixDatagram;

#[derive(Debug)]
pub enum BoundListener {
    Udp(UdpSocket),
    #[cfg(unix)]
    Unix(UnixDatagram),
}

pub struct UdpIngestService {
    config: UdpConfig,
    metrics: Arc<UdpMetrics>,
    shutdown: watch::Sender<bool>,
    #[cfg(unix)]
    unix_guard: Mutex<Option<UnixSocketGuard>>,
}

impl UdpIngestService {
    pub const IDENT: &'static str = "udp-ingest";

    pub fn new(config: UdpConfig, metrics: Arc<UdpMetrics>) -> Self {
        let (shutdown, _) = watch::channel(false);
        Self {
            config,
            metrics,
            shutdown,
            #[cfg(unix)]
            unix_guard: Mutex::new(None),
        }
    }

    async fn run(self: &Arc<Self>) -> AsyncServiceResult<()> {
        match self.bind_listener().await {
            Ok(BoundListener::Udp(socket)) => {
                let bind_desc = socket.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".to_string());
                info!("udp.event=bind_ok kind=udp addr={bind_desc}");
                self.pump_udp(socket).await
            }
            #[cfg(unix)]
            Ok(BoundListener::Unix(socket)) => {
                let bind_desc = socket
                    .local_addr()
                    .ok()
                    .and_then(|addr| addr.as_pathname().map(|p| p.display().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());
                info!("udp.event=bind_ok kind=unix path={bind_desc}");
                self.pump_unix(socket).await
            }
            Err(UdpIngestError::Disabled) => {
                info!("udp.event=disabled");
                Ok(())
            }
            Err(err) => {
                error!("udp.event=bind_fail reason={err}");
                Err(AsyncServiceError::Service(err.to_string()))
            }
        }
    }

    async fn bind_listener(&self) -> Result<BoundListener, UdpIngestError> {
        match self.config.bind_target() {
            BindTarget::Disabled => Err(UdpIngestError::Disabled),
            BindTarget::Udp(addr) => {
                self.ensure_loopback(addr)?;
                let socket = UdpSocket::bind(addr).await.map_err(UdpIngestError::Io)?;
                Ok(BoundListener::Udp(socket))
            }
            BindTarget::Unix(path) => {
                #[cfg(not(unix))]
                {
                    return Err(UdpIngestError::UnixSocketsUnsupported(path.display().to_string()));
                }
                #[cfg(unix)]
                {
                    let socket = self.bind_unix(path.clone()).await?;
                    Ok(BoundListener::Unix(socket))
                }
            }
        }
    }

    fn ensure_loopback(&self, addr: SocketAddr) -> Result<(), UdpIngestError> {
        if self.config.allow_non_local_bind || addr.ip().is_loopback() {
            Ok(())
        } else {
            Err(UdpIngestError::NonLocalBind(addr.to_string()))
        }
    }

    async fn pump_udp(&self, socket: UdpSocket) -> AsyncServiceResult<()> {
        let mut shutdown = self.shutdown.subscribe();
        let mut buf = vec![0u8; 2048];
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    break;
                }
                result = socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, peer)) => {
                            self.metrics.record_frame(len);
                            trace!("udp.event=frame kind=udp bytes={} peer={}", len, peer);
                        }
                        Err(err) => {
                            warn!("udp.event=recv_error kind=udp reason={err}");
                            break;
                        }
                    }
                }
            }
        }
        info!("udp.event=listener_stopped kind=udp");
        Ok(())
    }

    #[cfg(unix)]
    async fn pump_unix(&self, socket: UnixDatagram) -> AsyncServiceResult<()> {
        let mut shutdown = self.shutdown.subscribe();
        let mut buf = vec![0u8; 2048];
        loop {
            tokio::select! {
                _ = shutdown.changed() => { break; }
                result = socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, peer_addr)) => {
                            self.metrics.record_frame(len);
                            let peer_desc = peer_addr.as_pathname().map(|p| p.display().to_string()).unwrap_or_else(|| "anonymous".into());
                            trace!("udp.event=frame kind=unix bytes={} peer={}", len, peer_desc);
                        }
                        Err(err) => {
                            warn!("udp.event=recv_error kind=unix reason={err}");
                            break;
                        }
                    }
                }
            }
        }
        info!("udp.event=listener_stopped kind=unix");
        Ok(())
    }

    #[cfg(unix)]
    async fn bind_unix(&self, path: PathBuf) -> Result<UnixDatagram, UdpIngestError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(UdpIngestError::Io)?;
        }
        if path.exists() {
            fs::remove_file(&path).map_err(UdpIngestError::Io)?;
        }
        let socket = UnixDatagram::bind(&path).map_err(UdpIngestError::Io)?;
        let perms = fs::Permissions::from_mode(0o640);
        fs::set_permissions(&path, perms).map_err(UdpIngestError::Io)?;
        self.unix_guard.lock().unwrap().replace(UnixSocketGuard::new(path));
        Ok(socket)
    }
}

impl AsyncService for UdpIngestService {
    fn ident(self: Arc<Self>) -> &'static str {
        Self::IDENT
    }

    fn start(self: Arc<Self>) -> AsyncServiceFuture {
        Box::pin(async move { self.run().await })
    }

    fn signal_exit(self: Arc<Self>) {
        let _ = self.shutdown.send(true);
    }

    fn stop(self: Arc<Self>) -> AsyncServiceFuture {
        Box::pin(async move {
            let _ = self.shutdown.send(true);
            #[cfg(unix)]
            {
                self.unix_guard.lock().unwrap().take();
            }
            Ok(())
        })
    }
}

#[derive(Debug, Error)]
pub enum UdpIngestError {
    #[error("udp ingest disabled")]
    Disabled,
    #[error("non-loopback bind attempted for {0} without override")]
    NonLocalBind(String),
    #[error("unix datagram sockets unsupported on this platform (path: {0})")]
    UnixSocketsUnsupported(String),
    #[error("udp ingest io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(unix)]
struct UnixSocketGuard {
    path: PathBuf,
}

#[cfg(unix)]
impl UnixSocketGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[cfg(unix)]
impl Drop for UnixSocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UdpMode;
    use kaspa_consensus_core::network::{NetworkId, NetworkType};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn base_config() -> UdpConfig {
        UdpConfig {
            enable: true,
            listen: Some("127.0.0.1:0".parse().unwrap()),
            listen_unix: None,
            allow_non_local_bind: false,
            mode: UdpMode::Digest,
            max_kbps: 10,
            require_signature: true,
            allowed_signers: vec![],
            digest_queue: 16,
            block_queue: 8,
            discard_unsigned: true,
            db_migrate: false,
            retention_count: 1,
            retention_days: 1,
            log_verbosity: "info".into(),
            admin_remote_allowed: false,
            admin_token_file: None,
            network_id: NetworkId::new(NetworkType::Mainnet),
        }
    }

    #[tokio::test]
    async fn rejects_non_loopback_without_override() {
        let mut cfg = base_config();
        cfg.listen = Some("0.0.0.0:0".parse().unwrap());
        let service = Arc::new(UdpIngestService::new(cfg, Arc::new(UdpMetrics::new())));
        let err = service.bind_listener().await.expect_err("expected bind failure");
        matches!(err, UdpIngestError::NonLocalBind(_));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_socket_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("udp-test.sock");
        let mut cfg = base_config();
        cfg.listen = None;
        cfg.listen_unix = Some(path.clone());
        let service = Arc::new(UdpIngestService::new(cfg, Arc::new(UdpMetrics::new())));
        let listener = service.bind_listener().await.expect("bind unix");
        drop(listener);
        let metadata = fs::metadata(&path).expect("metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o640);
        drop(service);
        assert!(!path.exists());
    }

    #[cfg(not(unix))]
    #[tokio::test]
    async fn unix_socket_unsupported_on_non_unix() {
        let mut cfg = base_config();
        cfg.listen = None;
        cfg.listen_unix = Some(PathBuf::from("/tmp/does-not-matter.sock"));
        let service = Arc::new(UdpIngestService::new(cfg, Arc::new(UdpMetrics::new())));
        let err = service.bind_listener().await.expect_err("expected unix support error");
        matches!(err, UdpIngestError::UnixSocketsUnsupported(_));
    }
}
