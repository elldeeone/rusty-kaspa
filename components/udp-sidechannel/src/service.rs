use crate::{
    block::{BlockChannel, BlockChannelError, BlockParser, BlockQueue, BlockQueueError},
    config::{BindTarget, UdpConfig, UdpMode},
    frame::{
        assembler::{FrameAssembler, FrameAssemblerConfig, ReassembledFrame},
        header::{HeaderParseContext, SatFrameHeader, HEADER_LEN},
        DropEvent, DropReason, FrameKind,
    },
    injector::router_peer::spawn_block_injector,
    metrics::UdpMetrics,
    runtime::{DropClass, FrameRuntime, RuntimeConfig, RuntimeDecision},
    task::spawn_detached,
};
use bytes::Bytes;
use kaspa_connectionmanager::PeerMessageInjector;
use kaspa_core::task::service::{AsyncService, AsyncServiceError, AsyncServiceFuture, AsyncServiceResult};
use kaspa_core::{debug, error, info, trace, warn};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    net::UdpSocket,
    sync::{mpsc, mpsc::error::TrySendError, watch},
};

#[cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt};
#[cfg(unix)]
use tokio::net::UnixDatagram;

const CRC_WARN_INTERVAL: Duration = Duration::from_secs(60);
const FUTURE_VERSION_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_FRAGMENT_TTL: Duration = Duration::from_secs(5);
const DEDUP_WINDOW: u64 = 4096;
const DEDUP_ENTRIES: usize = 4096;
const DEDUP_RETENTION: Duration = Duration::from_secs(30);

pub struct UdpIngestService {
    config: UdpConfig,
    metrics: Arc<UdpMetrics>,
    shutdown: watch::Sender<bool>,
    enabled_flag: AtomicBool,
    enabled_tx: watch::Sender<bool>,
    queue_depth: Arc<QueueDepth>,
    tx: mpsc::Sender<QueuedFrame>,
    rx: Mutex<Option<mpsc::Receiver<QueuedFrame>>>,
    drop_logger: Arc<DropLogger>,
    #[cfg(unix)]
    unix_guard: Mutex<Option<UnixSocketGuard>>,
    block_channel: Option<BlockChannel>,
}

impl UdpIngestService {
    pub const IDENT: &'static str = "udp-ingest";

    pub fn new(config: UdpConfig, metrics: Arc<UdpMetrics>, block_injector: Option<Arc<dyn PeerMessageInjector>>) -> Self {
        let initially_enabled = config.initially_enabled();
        let (digest_cap, block_cap) = queue_caps(&config);
        let total_cap = digest_cap.saturating_add(block_cap).max(1);
        let (tx, rx) = mpsc::channel(total_cap);
        let queue_depth = Arc::new(QueueDepth::new(digest_cap, block_cap));
        let (shutdown, _) = watch::channel(false);
        let (enabled_tx, _) = watch::channel(initially_enabled);
        let drop_logger = Arc::new(DropLogger::new(metrics.clone()));
        let block_channel = if config.blocks_allowed() {
            if let Some(peer) = block_injector.clone() {
                let parser = BlockParser::new(config.block_max_bytes as usize);
                let queue = Arc::new(BlockQueue::new(config.block_queue.max(1), metrics.clone()));
                let channel = BlockChannel::new(parser, queue);
                if let Some(rx) = channel.take_rx() {
                    spawn_block_injector(rx, peer, metrics.clone());
                }
                Some(channel)
            } else {
                warn!("udp.event=block_channel_disabled reason=no_virtual_peer");
                None
            }
        } else {
            None
        };

        Self {
            config,
            metrics,
            shutdown,
            enabled_flag: AtomicBool::new(initially_enabled),
            enabled_tx,
            queue_depth,
            tx,
            rx: Mutex::new(Some(rx)),
            drop_logger,
            #[cfg(unix)]
            unix_guard: Mutex::new(None),
            block_channel,
        }
    }

    pub fn block_queue(&self) -> Option<Arc<BlockQueue>> {
        self.block_channel.as_ref().map(|channel| channel.queue())
    }

    pub fn take_reassembled_rx(&self) -> Option<mpsc::Receiver<QueuedFrame>> {
        self.rx.lock().ok().and_then(|mut guard| guard.take())
    }

    pub fn spawn_frame_consumer<F, Fut>(self: &Arc<Self>, mut handler: F) -> Result<(), FrameConsumerError>
    where
        F: FnMut(SatFrameHeader, Bytes) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let rx = self.take_reassembled_rx().ok_or(FrameConsumerError::AlreadyTaken)?;
        spawn_detached("frame-consumer", async move {
            let mut rx = rx;
            while let Some(frame) = rx.recv().await {
                let (header, payload) = frame.into_parts();
                handler(header, payload).await;
            }
            trace!("udp.event=frame_consumer_stopped");
        });
        Ok(())
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled_flag.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, enabled: bool) -> bool {
        let previous = self.enabled_flag.swap(enabled, Ordering::SeqCst);
        if previous != enabled {
            let _ = self.enabled_tx.send(enabled);
        }
        previous
    }

    pub fn snapshot(&self) -> UdpIngestSnapshot {
        let (digest_depth, block_depth) = self.queue_depth.snapshot();
        let block_snapshot = if let Some(channel) = &self.block_channel {
            let (capacity, depth) = channel.snapshot();
            QueueSnapshot { capacity, depth }
        } else {
            block_depth
        };
        UdpIngestSnapshot {
            enabled: self.is_enabled(),
            listen: self.config.listen,
            listen_unix: self.config.listen_unix.clone(),
            allow_non_local_bind: self.config.allow_non_local_bind,
            mode: self.config.mode,
            max_kbps: self.config.max_kbps,
            digest_queue: digest_depth,
            block_queue: block_snapshot,
            frames: self.metrics.frames_snapshot(),
            drops: self.metrics.drops_snapshot(),
            bytes_total: self.metrics.bytes_total(),
            rx_kbps: self.metrics.rx_kbps(),
            last_frame_ts_ms: self.metrics.last_frame_ts_ms(),
            signature_failures: self.metrics.signature_failures(),
            skew_seconds: self.metrics.skew_seconds(),
            divergence_detected: self.metrics.divergence_detected(),
            block_injected_total: self.metrics.block_injected_total(),
        }
    }

    async fn run(self: &Arc<Self>) -> AsyncServiceResult<()> {
        let mut shutdown = self.shutdown.subscribe();
        let mut enabled_rx = self.enabled_tx.subscribe();
        let mut logged_disabled = false;

        loop {
            if !self.is_enabled() {
                if !logged_disabled {
                    info!("udp.event=disabled");
                    logged_disabled = true;
                }
                tokio::select! {
                    _ = shutdown.changed() => break,
                    changed = enabled_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        continue;
                    }
                }
            }

            logged_disabled = false;

            match self.bind_listener().await {
                Ok(BoundListener::Udp(socket)) => {
                    let bind_desc = socket.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".to_string());
                    info!("udp.event=bind_ok kind=udp addr={bind_desc}");
                    let mut pump_enabled = self.enabled_tx.subscribe();
                    self.pump_udp(socket, &mut shutdown, &mut pump_enabled).await?;
                }
                #[cfg(unix)]
                Ok(BoundListener::Unix(socket)) => {
                    let bind_desc = socket
                        .local_addr()
                        .ok()
                        .and_then(|addr| addr.as_pathname().map(|p| p.display().to_string()))
                        .unwrap_or_else(|| "unknown".to_string());
                    info!("udp.event=bind_ok kind=unix path={bind_desc}");
                    let mut pump_enabled = self.enabled_tx.subscribe();
                    self.pump_unix(socket, &mut shutdown, &mut pump_enabled).await?;
                }
                Err(UdpIngestError::NotConfigured) => {
                    warn!("udp.event=bind_fail reason=not_configured");
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        changed = enabled_rx.changed() => {
                            if changed.is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    error!("udp.event=bind_fail reason={err}");
                    return Err(AsyncServiceError::Service(err.to_string()));
                }
            }
        }

        trace!("udp.event=ingest_run_stopped");
        Ok(())
    }

    async fn bind_listener(&self) -> Result<BoundListener, UdpIngestError> {
        match self.config.bind_target() {
            BindTarget::Disabled => Err(UdpIngestError::NotConfigured),
            BindTarget::Udp(addr) => {
                self.ensure_loopback(addr)?;
                let socket = UdpSocket::bind(addr).await.map_err(UdpIngestError::Io)?;
                Ok(BoundListener::Udp(socket))
            }
            BindTarget::Unix(path) => {
                #[cfg(not(unix))]
                {
                    Err(UdpIngestError::UnixSocketsUnsupported(path.display().to_string()))
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

    async fn pump_udp(
        &self,
        socket: UdpSocket,
        shutdown: &mut watch::Receiver<bool>,
        enabled_rx: &mut watch::Receiver<bool>,
    ) -> AsyncServiceResult<()> {
        let mut buf = vec![0u8; self.max_datagram_len()];
        let mut processor = FrameProcessor::new(
            &self.config,
            &self.metrics,
            &self.drop_logger,
            &self.queue_depth,
            &self.tx,
            self.block_channel.clone(),
        );
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                changed = enabled_rx.changed() => {
                    if changed.is_err() || !self.is_enabled() {
                        break;
                    }
                }
                result = socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, peer)) => {
                            trace!("udp.event=frame kind=udp bytes={} peer={}", len, peer);
                            processor.process(&buf[..len], RemoteLabel::Loopback);
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
    async fn pump_unix(
        &self,
        socket: UnixDatagram,
        shutdown: &mut watch::Receiver<bool>,
        enabled_rx: &mut watch::Receiver<bool>,
    ) -> AsyncServiceResult<()> {
        let mut buf = vec![0u8; self.max_datagram_len()];
        let mut processor = FrameProcessor::new(
            &self.config,
            &self.metrics,
            &self.drop_logger,
            &self.queue_depth,
            &self.tx,
            self.block_channel.clone(),
        );
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                changed = enabled_rx.changed() => {
                    if changed.is_err() || !self.is_enabled() {
                        break;
                    }
                }
                result = socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, peer_addr)) => {
                            let peer_desc = peer_addr.as_pathname().map(|p| p.display().to_string()).unwrap_or_else(|| "anonymous".into());
                            trace!("udp.event=frame kind=unix bytes={} peer={}", len, peer_desc);
                            processor.process(&buf[..len], RemoteLabel::Unix);
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

    fn max_datagram_len(&self) -> usize {
        (self.config.max_block_payload_bytes as usize).saturating_add(HEADER_LEN + 64).min(1 << 20)
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
        Box::pin(async move {
            let result = self.run().await;
            trace!("udp.event=ingest_service_start_returned");
            result
        })
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
    #[error("udp ingest not configured (no UDP or Unix bind target set)")]
    NotConfigured,
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

#[derive(Debug)]
pub enum BoundListener {
    Udp(UdpSocket),
    #[cfg(unix)]
    Unix(UnixDatagram),
}

pub struct QueuedFrame {
    pub header: SatFrameHeader,
    pub payload: Bytes,
    _slot: QueueSlot,
}

impl QueuedFrame {
    fn new(frame: ReassembledFrame, slot: QueueSlot) -> Self {
        Self { header: frame.header, payload: frame.payload, _slot: slot }
    }

    pub fn into_parts(self) -> (SatFrameHeader, Bytes) {
        (self.header, self.payload)
    }
}

#[derive(Debug, Clone)]
pub struct QueueSnapshot {
    pub capacity: usize,
    pub depth: usize,
}

#[derive(Debug, Clone)]
pub struct UdpIngestSnapshot {
    pub enabled: bool,
    pub listen: Option<SocketAddr>,
    pub listen_unix: Option<PathBuf>,
    pub allow_non_local_bind: bool,
    pub mode: UdpMode,
    pub max_kbps: u32,
    pub digest_queue: QueueSnapshot,
    pub block_queue: QueueSnapshot,
    pub frames: Vec<(&'static str, u64)>,
    pub drops: Vec<(&'static str, u64)>,
    pub bytes_total: u64,
    pub rx_kbps: f64,
    pub last_frame_ts_ms: Option<u64>,
    pub signature_failures: u64,
    pub skew_seconds: u64,
    pub divergence_detected: bool,
    pub block_injected_total: u64,
}

struct QueueDepth {
    digest_cap: usize,
    block_cap: usize,
    digest: AtomicCounter,
    block: AtomicCounter,
}

impl QueueDepth {
    fn new(digest_cap: usize, block_cap: usize) -> Self {
        Self { digest_cap, block_cap, digest: AtomicCounter::new(), block: AtomicCounter::new() }
    }

    fn snapshot(&self) -> (QueueSnapshot, QueueSnapshot) {
        (
            QueueSnapshot { capacity: self.digest_cap, depth: self.digest.current() },
            QueueSnapshot { capacity: self.block_cap, depth: self.block.current() },
        )
    }

    fn try_reserve(self: &Arc<Self>, kind: FrameKind) -> Option<QueueSlot> {
        let (counter, cap) = match kind {
            FrameKind::Digest => (&self.digest, self.digest_cap),
            FrameKind::Block => (&self.block, self.block_cap),
        };
        if cap == 0 {
            return None;
        }
        if counter.reserve(cap) {
            Some(QueueSlot { depth: Arc::clone(self), kind })
        } else {
            None
        }
    }

    fn release(&self, kind: FrameKind) {
        match kind {
            FrameKind::Digest => self.digest.release(),
            FrameKind::Block => self.block.release(),
        }
    }
}

struct QueueSlot {
    depth: Arc<QueueDepth>,
    kind: FrameKind,
}

impl Drop for QueueSlot {
    fn drop(&mut self) {
        self.depth.release(self.kind);
    }
}

struct AtomicCounter {
    value: AtomicUsize,
}

impl AtomicCounter {
    fn new() -> Self {
        Self { value: AtomicUsize::new(0) }
    }

    fn current(&self) -> usize {
        self.value.load(Ordering::Relaxed)
    }

    fn reserve(&self, cap: usize) -> bool {
        loop {
            let current = self.value.load(Ordering::Relaxed);
            if current >= cap {
                return false;
            }
            if self.value.compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                return true;
            }
        }
    }

    fn release(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }
}

struct DropLogger {
    metrics: Arc<UdpMetrics>,
    future_gate: Mutex<RateGate>,
    crc_gate: Mutex<CrcStormGate>,
}

impl DropLogger {
    fn new(metrics: Arc<UdpMetrics>) -> Self {
        Self { metrics, future_gate: Mutex::new(RateGate::new()), crc_gate: Mutex::new(CrcStormGate::new()) }
    }

    fn record(
        &self,
        mut event: DropEvent,
        remote: RemoteLabel,
        drop_class: Option<DropClass>,
        now: Instant,
        datagram_bytes: usize,
        future_version: bool,
    ) {
        if event.bytes == 0 {
            event.bytes = datagram_bytes;
        }
        self.metrics.record_drop(event.reason);

        if future_version && self.future_gate.lock().unwrap().should_log(now, FUTURE_VERSION_INTERVAL) {
            warn!("udp.event=future_version note=frame_dropped");
        }

        if event.reason == DropReason::Crc {
            if let Some(count) = self.crc_gate.lock().unwrap().record(now) {
                warn!("udp.event=crc_storm drops={} window_secs={}", count, CRC_WARN_INTERVAL.as_secs());
            }
        }

        let kind = event.context.kind.map(|k| k.as_str()).unwrap_or("unknown");
        let seq_repr = event.context.seq.map(|s| s.to_string()).unwrap_or_else(|| "na".to_string());
        let mut message = format!(
            "udp.event=frame_drop reason={} kind={} seq={} bytes={} remote={}",
            event.reason.as_str(),
            kind,
            seq_repr,
            event.bytes,
            remote.as_str()
        );
        if let Some(class) = drop_class {
            message.push_str(&format!(" drop_class={}", class.as_str()));
        }
        debug!("{message}");
    }
}

struct RateGate {
    last: Option<Instant>,
}

impl RateGate {
    fn new() -> Self {
        Self { last: None }
    }

    fn should_log(&mut self, now: Instant, interval: Duration) -> bool {
        match self.last {
            Some(prev) if now.duration_since(prev) < interval => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

struct CrcStormGate {
    last: Instant,
    suppressed: u64,
}

impl CrcStormGate {
    fn new() -> Self {
        let now = Instant::now();
        Self { last: now - CRC_WARN_INTERVAL, suppressed: 0 }
    }

    fn record(&mut self, now: Instant) -> Option<u64> {
        self.suppressed += 1;
        if now.duration_since(self.last) >= CRC_WARN_INTERVAL {
            let count = std::mem::take(&mut self.suppressed);
            self.last = now;
            Some(count)
        } else {
            None
        }
    }
}

struct FrameProcessor {
    assembler: FrameAssembler,
    runtime: FrameRuntime,
    header_ctx: HeaderParseContext,
    metrics: Arc<UdpMetrics>,
    drop_logger: Arc<DropLogger>,
    queue_depth: Arc<QueueDepth>,
    tx: mpsc::Sender<QueuedFrame>,
    drop_buffer: Vec<DropEvent>,
    block_channel: Option<BlockChannel>,
}

impl FrameProcessor {
    fn new(
        config: &UdpConfig,
        metrics: &Arc<UdpMetrics>,
        drop_logger: &Arc<DropLogger>,
        queue_depth: &Arc<QueueDepth>,
        tx: &mpsc::Sender<QueuedFrame>,
        block_channel: Option<BlockChannel>,
    ) -> Self {
        let header_ctx = HeaderParseContext { network_tag: config.network_tag(), payload_caps: config.payload_caps() };
        let assembler = FrameAssembler::new(FrameAssemblerConfig {
            max_groups: 256,
            max_buffer_bytes: (config.max_block_payload_bytes as usize).saturating_mul(8).max(256 * 1024),
            fragment_ttl: DEFAULT_FRAGMENT_TTL,
        });
        let runtime = FrameRuntime::new(RuntimeConfig {
            max_kbps: config.max_kbps,
            dedup_window: DEDUP_WINDOW,
            dedup_max_entries: DEDUP_ENTRIES,
            dedup_retention: DEDUP_RETENTION,
            snapshot_overdraft_factor: 2.0,
        });
        Self {
            assembler,
            runtime,
            header_ctx,
            metrics: Arc::clone(metrics),
            drop_logger: Arc::clone(drop_logger),
            queue_depth: Arc::clone(queue_depth),
            tx: tx.clone(),
            drop_buffer: Vec::with_capacity(4),
            block_channel,
        }
    }

    fn process(&mut self, data: &[u8], remote: RemoteLabel) {
        let now = Instant::now();
        self.assembler.collect_expired(now, &mut self.drop_buffer);
        self.flush_drops(remote, now, data.len());

        let parsed = match SatFrameHeader::parse(data, &self.header_ctx) {
            Ok(parsed) => parsed,
            Err(err) => {
                let mut event = err.drop_event();
                event.bytes = data.len();
                self.drop_logger.record(event, remote, None, now, data.len(), err.future_version);
                return;
            }
        };

        let payload = Bytes::copy_from_slice(parsed.payload);
        match self.assembler.ingest(parsed.header, payload, now, &mut self.drop_buffer) {
            Some(frame) => {
                self.flush_drops(remote, now, data.len());
                self.handle_frame(frame, remote, now, data.len());
            }
            None => self.flush_drops(remote, now, data.len()),
        }
    }

    fn handle_frame(&mut self, frame: ReassembledFrame, remote: RemoteLabel, now: Instant, datagram_len: usize) {
        match self.runtime.evaluate(&frame.header, &frame.payload, now) {
            RuntimeDecision::Accept => {
                let bytes = frame.payload.len() + HEADER_LEN;
                match frame.header.kind {
                    FrameKind::Block => self.handle_block_frame(frame, remote, now, datagram_len, bytes),
                    _ => self.enqueue_digest_frame(frame, remote, now, datagram_len, bytes),
                }
            }
            RuntimeDecision::Drop { reason, drop_class } => {
                let event = frame.header.as_drop_event(reason, frame.payload.len());
                self.drop_logger.record(event, remote, drop_class, now, datagram_len, false);
            }
        }
    }

    fn enqueue_digest_frame(&mut self, frame: ReassembledFrame, remote: RemoteLabel, now: Instant, datagram_len: usize, bytes: usize) {
        let header = frame.header;
        let seq = header.seq;
        let kind = header.kind;
        let payload_len = frame.payload.len();
        if let Some(slot) = self.queue_depth.try_reserve(kind) {
            let queued = QueuedFrame::new(frame, slot);
            match self.tx.try_send(queued) {
                Ok(()) => {
                    self.metrics.record_frame(kind, bytes);
                    trace!("udp.event=frame_accept kind={} seq={} bytes={} remote={}", kind.as_str(), seq, bytes, remote.as_str());
                }
                Err(err) => self.handle_queue_error(err, remote, now, datagram_len),
            }
        } else {
            let event = header.as_drop_event(DropReason::QueueFull, payload_len);
            self.drop_logger.record(event, remote, None, now, datagram_len, false);
        }
    }

    fn handle_block_frame(&mut self, frame: ReassembledFrame, remote: RemoteLabel, now: Instant, datagram_len: usize, bytes: usize) {
        let header = frame.header;
        let payload = frame.payload;
        let Some(channel) = &self.block_channel else {
            trace!("udp.event=frame_ignore kind=block seq={} remote={}", header.seq, remote.as_str());
            return;
        };

        match channel.enqueue(header, payload) {
            Ok(()) => {
                self.metrics.record_frame(FrameKind::Block, bytes);
                trace!("udp.event=frame_accept kind=block seq={} bytes={} remote={}", header.seq, bytes, remote.as_str());
            }
            Err(BlockChannelError::Parse(err)) => {
                let event = header.as_drop_event(err.reason(), bytes.saturating_sub(HEADER_LEN));
                self.drop_logger.record(event, remote, None, now, datagram_len, false);
            }
            Err(BlockChannelError::Queue(BlockQueueError::Full)) => {
                let event = header.as_drop_event(DropReason::BlockQueueFull, bytes.saturating_sub(HEADER_LEN));
                self.drop_logger.record(event, remote, None, now, datagram_len, false);
            }
            Err(BlockChannelError::Queue(BlockQueueError::Closed)) => {
                let event = header.as_drop_event(DropReason::Panic, bytes.saturating_sub(HEADER_LEN));
                self.drop_logger.record(event, remote, None, now, datagram_len, false);
                warn!("udp.event=block_queue_closed seq={} remote={}", header.seq, remote.as_str());
            }
        }
    }

    fn handle_queue_error(&self, err: TrySendError<QueuedFrame>, remote: RemoteLabel, now: Instant, datagram_len: usize) {
        match err {
            TrySendError::Closed(frame) | TrySendError::Full(frame) => {
                let (header, payload) = frame.into_parts();
                let event = header.as_drop_event(DropReason::QueueFull, payload.len());
                self.drop_logger.record(event, remote, None, now, datagram_len, false);
            }
        }
    }

    fn flush_drops(&mut self, remote: RemoteLabel, now: Instant, datagram_len: usize) {
        for event in self.drop_buffer.drain(..) {
            self.drop_logger.record(event, remote, None, now, datagram_len, false);
        }
    }
}

#[derive(Clone, Copy)]
enum RemoteLabel {
    Loopback,
    Unix,
}

impl RemoteLabel {
    fn as_str(self) -> &'static str {
        match self {
            RemoteLabel::Loopback => "loopback",
            RemoteLabel::Unix => "unix",
        }
    }
}

fn queue_caps(config: &UdpConfig) -> (usize, usize) {
    let digest_cap = if config.mode.allows_digest() { config.digest_queue } else { 0 };
    let block_cap = if config.blocks_allowed() { config.block_queue } else { 0 };
    (digest_cap, block_cap)
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
            network_id: NetworkId::new(NetworkType::Mainnet),
        }
    }

    #[tokio::test]
    async fn rejects_non_loopback_without_override() {
        let mut cfg = base_config();
        cfg.listen = Some("0.0.0.0:0".parse().unwrap());
        let service = Arc::new(UdpIngestService::new(cfg, Arc::new(UdpMetrics::new()), None));
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
        let service = Arc::new(UdpIngestService::new(cfg, Arc::new(UdpMetrics::new()), None));
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
        let service = Arc::new(UdpIngestService::new(cfg, Arc::new(UdpMetrics::new()), None));
        let err = service.bind_listener().await.expect_err("expected unix support error");
        matches!(err, UdpIngestError::UnixSocketsUnsupported(_));
    }
}
#[derive(Debug, Error)]
pub enum FrameConsumerError {
    #[error("frame consumer already configured")]
    AlreadyTaken,
}
