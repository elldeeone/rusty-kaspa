# Rusty-Kaspa SOCKS Proxy Injection Implementation Guide

## Executive Summary

To enable Tor integration via SOCKS proxy in Rusty-Kaspa, inject a custom connector at **ONE** critical point:

**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/protocol/p2p/src/core/connection_handler.rs`  
**Method:** `ConnectionHandler::connect()`  
**Lines:** 103-108

All outbound P2P connections flow through this single point.

---

## Current Code (Lines 96-142)

```rust
/// Connect to a new peer
pub(crate) async fn connect(&self, peer_address: String) -> Result<Arc<Router>, ConnectionError> {
    let Some(socket_address) = peer_address.to_socket_addrs()?.next() else {
        return Err(ConnectionError::NoAddress);
    };
    let peer_address = format!("http://{}", peer_address); // Add scheme prefix as required by Tonic

    // ==================== INJECTION POINT STARTS HERE ====================
    let channel = tonic::transport::Endpoint::new(peer_address)?
        .timeout(Duration::from_millis(Self::communication_timeout()))
        .connect_timeout(Duration::from_millis(Self::connect_timeout()))
        .tcp_keepalive(Some(Duration::from_millis(Self::keep_alive())))
        .connect()  // <-- THIS LINE MAKES THE ACTUAL TCP CONNECTION
        .await?;
    // ==================== INJECTION POINT ENDS HERE ====================

    let channel = ServiceBuilder::new()
        .layer(MapResponseBodyLayer::new(move |body| CountBytesBody::new(body, self.counters.bytes_rx.clone())))
        .layer(MapRequestBodyLayer::new(move |body| CountBytesBody::new(body, self.counters.bytes_tx.clone()).boxed_unsync()))
        .service(channel);

    let mut client = ProtoP2pClient::new(channel)
        .send_compressed(tonic::codec::CompressionEncoding::Gzip)
        .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
        .max_decoding_message_size(P2P_MAX_MESSAGE_SIZE);

    let (outgoing_route, outgoing_receiver) = mpsc_channel(Self::outgoing_network_channel_size());
    let incoming_stream = client.message_stream(ReceiverStream::new(outgoing_receiver)).await?.into_inner();

    let router = Router::new(socket_address, true, self.hub_sender.clone(), incoming_stream, outgoing_route).await;

    // For outbound peers, we perform the initialization as part of the connect logic
    match self.initializer.initialize_connection(router.clone()).await {
        Ok(()) => {
            // Notify the central Hub about the new peer
            self.hub_sender.send(HubEvent::NewPeer(router.clone())).await.expect("hub receiver should never drop before senders");
        }

        Err(err) => {
            router.try_sending_reject_message(&err).await;
            // Ignoring the new router
            router.close().await;
            debug!("P2P, handshake failed for outbound peer {}: {}", router, err);
            return Err(ConnectionError::ProtocolError(err));
        }
    }

    Ok(router)
}
```

---

## Implementation Strategy: Custom Tower Connector

### Step 1: Create SOCKS Connector Module

Create file: `protocol/p2p/src/core/socks_connector.rs`

```rust
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::service::Service;
use tonic::transport::Uri;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub struct SocksConnector {
    socks_proxy: SocketAddr,
}

impl SocksConnector {
    pub fn new(socks_proxy: SocketAddr) -> Self {
        Self { socks_proxy }
    }
}

// Implement tower::service::Service trait
// This allows Tonic to use our custom connector for making TCP connections
impl Service<Uri> for SocksConnector {
    type Response = TcpStream;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let socks_proxy = self.socks_proxy;
        
        Box::pin(async move {
            // 1. Connect to SOCKS proxy
            let mut socks_stream = TcpStream::connect(socks_proxy).await?;

            // 2. Extract target host and port from URI
            let host = uri.host().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "No host in URI")
            })?;
            let port = uri.port_u16().unwrap_or(16210); // Kaspa default P2P port

            // 3. Build SOCKS5 request (simplified)
            // Format: [VER=5] [CMD=CONNECT] [RSV=0] [ATYP] [DST.ADDR] [DST.PORT]
            let mut request = vec![5, 1, 0]; // Version 5, Connect, Reserved
            
            // ATYP: 1=IPv4, 3=Domain, 4=IPv6
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                match ip {
                    std::net::IpAddr::V4(addr) => {
                        request.push(1); // IPv4
                        request.extend_from_slice(&addr.octets());
                    }
                    std::net::IpAddr::V6(addr) => {
                        request.push(4); // IPv6
                        request.extend_from_slice(&addr.octets());
                    }
                }
            } else {
                // Domain name
                request.push(3); // Domain
                request.push(host.len() as u8);
                request.extend_from_slice(host.as_bytes());
            }
            
            // Add port (big-endian)
            request.extend_from_slice(&port.to_be_bytes());

            // 4. Send request through SOCKS proxy
            socks_stream.write_all(&request).await?;

            // 5. Read SOCKS response (simplified)
            let mut response = vec![0; 6]; // Min response size
            socks_stream.read_exact(&mut response).await?;

            // Check response: [VER=5] [STATUS=0] [RSV=0] [ATYP] [BND.ADDR] [BND.PORT]
            if response[0] != 5 || response[1] != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "SOCKS proxy rejected connection"
                ));
            }

            Ok(socks_stream)
        })
    }
}
```

### Step 2: Modify ConnectionHandler

Edit: `/Users/luke/Documents/GitHub/rusty-kaspa/protocol/p2p/src/core/connection_handler.rs`

Add at the top:
```rust
mod socks_connector;
use crate::core::socks_connector::SocksConnector;
```

Modify the `ConnectionHandler` struct to store optional SOCKS proxy address:
```rust
pub struct ConnectionHandler {
    hub_sender: MpscSender<HubEvent>,
    initializer: Arc<dyn ConnectionInitializer>,
    counters: Arc<TowerConnectionCounters>,
    socks_proxy: Option<SocketAddr>,  // ADD THIS
}
```

Update the `connect()` method:
```rust
pub(crate) async fn connect(&self, peer_address: String) -> Result<Arc<Router>, ConnectionError> {
    let Some(socket_address) = peer_address.to_socket_addrs()?.next() else {
        return Err(ConnectionError::NoAddress);
    };
    let peer_address = format!("http://{}", peer_address);

    // UPDATED: Add conditional connector
    let channel = match self.socks_proxy {
        Some(socks_addr) => {
            // Route through SOCKS proxy
            tonic::transport::Endpoint::new(peer_address)?
                .timeout(Duration::from_millis(Self::communication_timeout()))
                .connect_timeout(Duration::from_millis(Self::connect_timeout()))
                .tcp_keepalive(Some(Duration::from_millis(Self::keep_alive())))
                .connector(SocksConnector::new(socks_addr))
                .connect()
                .await?
        }
        None => {
            // Direct connection (existing behavior)
            tonic::transport::Endpoint::new(peer_address)?
                .timeout(Duration::from_millis(Self::communication_timeout()))
                .connect_timeout(Duration::from_millis(Self::connect_timeout()))
                .tcp_keepalive(Some(Duration::from_millis(Self::keep_alive())))
                .connect()
                .await?
        }
    };

    // ... rest of the method unchanged
}
```

### Step 3: Thread SOCKS Proxy Address Through the Stack

#### A. Adaptor (protocol/p2p/src/core/adaptor.rs)

```rust
pub struct Adaptor {
    _server_termination: Option<OneshotSender<()>>,
    connection_handler: ConnectionHandler,
    hub: Hub,
}

pub fn client_only(
    hub: Hub, 
    initializer: Arc<dyn ConnectionInitializer>, 
    counters: Arc<TowerConnectionCounters>,
    socks_proxy: Option<SocketAddr>,  // ADD THIS
) -> Arc<Self> {
    let (hub_sender, hub_receiver) = mpsc_channel(Self::hub_channel_size());
    let connection_handler = ConnectionHandler::new(hub_sender, initializer.clone(), counters, socks_proxy);
    let adaptor = Arc::new(Adaptor::new(None, connection_handler, hub));
    adaptor.hub.clone().start_event_loop(hub_receiver, initializer);
    adaptor
}

pub fn bidirectional(
    serve_address: NetAddress,
    hub: Hub,
    initializer: Arc<dyn ConnectionInitializer>,
    counters: Arc<TowerConnectionCounters>,
    socks_proxy: Option<SocketAddr>,  // ADD THIS
) -> Result<Arc<Self>, ConnectionError> {
    let (hub_sender, hub_receiver) = mpsc_channel(Self::hub_channel_size());
    let connection_handler = ConnectionHandler::new(hub_sender, initializer.clone(), counters, socks_proxy);
    // ... rest unchanged
}
```

#### B. P2pService (protocol/flows/src/service.rs)

```rust
pub struct P2pService {
    flow_context: Arc<FlowContext>,
    connect_peers: Vec<NetAddress>,
    add_peers: Vec<NetAddress>,
    listen: NetAddress,
    outbound_target: usize,
    inbound_limit: usize,
    dns_seeders: &'static [&'static str],
    default_port: u16,
    shutdown: SingleTrigger,
    counters: Arc<TowerConnectionCounters>,
    socks_proxy: Option<SocketAddr>,  // ADD THIS
}

pub fn new(
    flow_context: Arc<FlowContext>,
    connect_peers: Vec<NetAddress>,
    add_peers: Vec<NetAddress>,
    listen: NetAddress,
    outbound_target: usize,
    inbound_limit: usize,
    dns_seeders: &'static [&'static str],
    default_port: u16,
    counters: Arc<TowerConnectionCounters>,
    socks_proxy: Option<SocketAddr>,  // ADD THIS
) -> Self {
    Self {
        flow_context,
        connect_peers,
        add_peers,
        shutdown: SingleTrigger::default(),
        listen,
        outbound_target,
        inbound_limit,
        dns_seeders,
        default_port,
        counters,
        socks_proxy,  // ADD THIS
    }
}

fn start(self: Arc<Self>) -> AsyncServiceFuture {
    // ... existing code ...
    let p2p_adaptor = if self.inbound_limit == 0 {
        Adaptor::client_only(
            self.flow_context.hub().clone(), 
            self.flow_context.clone(), 
            self.counters.clone(),
            self.socks_proxy,  // PASS THROUGH
        )
    } else {
        Adaptor::bidirectional(
            self.listen, 
            self.flow_context.hub().clone(), 
            self.flow_context.clone(), 
            self.counters.clone(),
            self.socks_proxy,  // PASS THROUGH
        ).unwrap()
    };
    // ... rest unchanged
}
```

#### C. Daemon (kaspad/src/daemon.rs)

```rust
// Around line 500: Parse SOCKS proxy from args
let socks_proxy: Option<SocketAddr> = args.socks_proxy
    .as_ref()
    .and_then(|addr| addr.parse().ok());

// Around line 597-607: Pass to P2pService
let p2p_service = Arc::new(P2pService::new(
    flow_context.clone(),
    connect_peers,
    add_peers,
    p2p_server_addr,
    outbound_target,
    inbound_limit,
    dns_seeders,
    config.default_p2p_port(),
    p2p_tower_counters.clone(),
    socks_proxy,  // ADD THIS
));
```

#### D. Arguments (kaspad/src/args.rs)

Add to the `Args` struct:
```rust
#[arg(long, help = "SOCKS5 proxy address for P2P connections (for Tor)")]
pub socks_proxy: Option<String>,
```

---

## Testing the Integration

### Test 1: Verify Proxy is Used
```bash
# Terminal 1: Run socat to monitor SOCKS connections
socat -v TCP4-LISTEN:9050,reuseaddr,fork STDOUT

# Terminal 2: Run kaspad with SOCKS proxy
cargo run --release --bin kaspad -- --socks5-proxy 127.0.0.1:9050

# Watch socat output for SOCKS5 protocol traffic
```

### Test 2: Verify with Tor
```bash
# Start Tor
tor

# Run kaspad pointing to Tor's SOCKS port (usually 9050)
cargo run --release --bin kaspad -- --socks5-proxy 127.0.0.1:9050

# Monitor: cat ~/.tor/log to see Tor activity
```

### Test 3: Connection Verification
```bash
# Check if SOCKS proxy is receiving connections
netstat -an | grep 9050

# Or use ss
ss -an | grep 9050
```

---

## Summary of Changes Required

| File | Change | Lines |
|------|--------|-------|
| protocol/p2p/src/core/connection_handler.rs | Add SOCKS connector field, implement conditional logic | 96-142 |
| protocol/p2p/src/core/connection_handler.rs | Create socks_connector.rs module | New file |
| protocol/p2p/src/core/adaptor.rs | Add socks_proxy parameter to factory methods | 48-85 |
| protocol/flows/src/service.rs | Add socks_proxy field, thread through | 62-99 |
| kaspad/src/daemon.rs | Parse socks_proxy from args, pass to P2pService | ~500, ~607 |
| kaspad/src/args.rs | Add --socks5-proxy command line argument | New field |

---

## Key Benefits of This Approach

1. **Single Injection Point**: All P2P connections controlled from one location
2. **Optional**: Works with or without SOCKS proxy (backward compatible)
3. **Clean**: Uses Tonic's official `.connector()` API
4. **Testable**: Easy to unit test with mock connectors
5. **No Protocol Changes**: Transparent to P2P protocol
6. **Tor Compatible**: Works with any SOCKS5 proxy (Tor, others)

---

## Alternative: Simpler Version (Without Generic Service)

If you need a faster, less generic implementation:

```rust
// In connection_handler.rs::connect()
let channel = if let Some(socks_addr) = self.socks_proxy {
    // Route through SOCKS proxy
    let socket = tokio::net::TcpStream::connect(socks_addr).await?;
    // Perform SOCKS5 handshake manually
    // Return socket wrapped in TcpStream
    tonic::transport::Channel::from_shared(...)  // Convert to Channel
} else {
    // Direct connection
    tonic::transport::Endpoint::new(peer_address)?
        .connect()
        .await?
};
```

However, the custom Service implementation is more maintainable and type-safe.

