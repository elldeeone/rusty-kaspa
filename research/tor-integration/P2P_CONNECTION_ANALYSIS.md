# Rusty-Kaspa P2P Outbound Connection Analysis

## Overview
This document identifies exactly where Rusty-Kaspa establishes outbound P2P connections and where a SOCKS proxy connector would need to be injected.

## Connection Flow Architecture

```
kaspad (main.rs)
  └── create_core() in daemon.rs
      └── P2pService::new() 
          └── Adaptor::bidirectional() or Adaptor::client_only()
              └── ConnectionHandler (manages actual connections)
                  └── ConnectionHandler::connect() [PRIMARY INJECTION POINT]
                      └── Tonic gRPC channel creation
```

## Key Files and Line Numbers

### 1. **Primary Entry Point: ConnectionHandler::connect()**
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/protocol/p2p/src/core/connection_handler.rs`
**Lines:** 96-142

This is the PRIMARY LOCATION where outbound TCP/gRPC channels are created:

```rust
/// Connect to a new peer
pub(crate) async fn connect(&self, peer_address: String) -> Result<Arc<Router>, ConnectionError> {
    let Some(socket_address) = peer_address.to_socket_addrs()?.next() else {
        return Err(ConnectionError::NoAddress);
    };
    let peer_address = format!("http://{}", peer_address); // Add scheme prefix as required by Tonic

    // CRITICAL LINE: Tonic channel is created here
    let channel = tonic::transport::Endpoint::new(peer_address)?
        .timeout(Duration::from_millis(Self::communication_timeout()))
        .connect_timeout(Duration::from_millis(Self::connect_timeout()))
        .tcp_keepalive(Some(Duration::from_millis(Self::keep_alive())))
        .connect()  // <-- TCP CONNECTION ESTABLISHED HERE
        .await?;

    // Lines 110-113: ServiceBuilder wraps the channel with middleware
    let channel = ServiceBuilder::new()
        .layer(MapResponseBodyLayer::new(move |body| CountBytesBody::new(body, self.counters.bytes_rx.clone())))
        .layer(MapRequestBodyLayer::new(move |body| CountBytesBody::new(body, self.counters.bytes_tx.clone()).boxed_unsync()))
        .service(channel);

    // Lines 115-118: gRPC client is created
    let mut client = ProtoP2pClient::new(channel)
        .send_compressed(tonic::codec::CompressionEncoding::Gzip)
        .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
        .max_decoding_message_size(P2P_MAX_MESSAGE_SIZE);

    // Lines 120-121: Bidirectional streaming established
    let (outgoing_route, outgoing_receiver) = mpsc_channel(Self::outgoing_network_channel_size());
    let incoming_stream = client.message_stream(ReceiverStream::new(outgoing_receiver)).await?.into_inner();

    // Lines 123+: Router created with the established connection
    let router = Router::new(socket_address, true, self.hub_sender.clone(), incoming_stream, outgoing_route).await;
    ...
}
```

**Key Constants (lines 184-194):**
```rust
fn communication_timeout() -> u64 {
    10_000  // 10 seconds
}

fn keep_alive() -> u64 {
    10_000  // 10 seconds
}

fn connect_timeout() -> u64 {
    1_000   // 1 second
}
```

### 2. **Connection Entry Point: Adaptor**
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/protocol/p2p/src/core/adaptor.rs`
**Lines:** 72-85

```rust
/// Connect to a new peer (no retries)
pub async fn connect_peer(&self, peer_address: String) -> Result<PeerKey, ConnectionError> {
    self.connection_handler.connect_with_retry(peer_address, 1, Default::default()).await.map(|r| r.key())
}

/// Connect to a new peer (with params controlling retry behavior)
pub async fn connect_peer_with_retries(
    &self,
    peer_address: String,
    retry_attempts: u8,
    retry_interval: Duration,
) -> Result<PeerKey, ConnectionError> {
    self.connection_handler.connect_with_retry(peer_address, retry_attempts, retry_interval).await.map(|r| r.key())
}
```

**Adaptor Factory Methods (lines 48-70):**
- `Adaptor::client_only()` - Client-only mode (for Tor nodes with --inbound-limit 0)
- `Adaptor::bidirectional()` - Server + client mode (normal nodes)

### 3. **Connection Manager (Orchestrator)**
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/components/connectionmanager/src/lib.rs`
**Lines:** 116-189

```rust
async fn handle_connection_requests(self: &Arc<Self>, peer_by_address: &HashMap<SocketAddr, Peer>) {
    // ...
    if !is_connected && request.next_attempt <= SystemTime::now() {
        debug!("Connecting to peer request {}", address);
        match self.p2p_adaptor.connect_peer(address.to_string()).await {  // Line 130
            Err(err) => { /* retry logic */ }
            Ok(_) if request.is_permanent => { /* keep request */ }
            Ok(_) => {}
        }
    }
}

async fn handle_outbound_connections(self: &Arc<Self>, peer_by_address: &HashMap<SocketAddr, Peer>) {
    // Lines 185-188: Initiates connections to peer addresses
    for _ in 0..missing_connections {
        let Some(net_addr) = addr_iter.next() else { /* ... */ };
        let socket_addr = SocketAddr::new(net_addr.ip.into(), net_addr.port).to_string();
        debug!("Connecting to {}", &socket_addr);
        addrs_to_connect.push(net_addr);
        jobs.push(self.p2p_adaptor.connect_peer(socket_addr.clone()));  // Line 188
    }
}
```

### 4. **P2P Service (Initialization)**
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/protocol/flows/src/service.rs`
**Lines:** 62-99

```rust
fn start(self: Arc<Self>) -> AsyncServiceFuture {
    // Line 68-71: Creates Adaptor (client-only or bidirectional)
    let p2p_adaptor = if self.inbound_limit == 0 {
        Adaptor::client_only(self.flow_context.hub().clone(), self.flow_context.clone(), self.counters.clone())
    } else {
        Adaptor::bidirectional(self.listen, self.flow_context.hub().clone(), self.flow_context.clone(), self.counters.clone())
            .unwrap()
    };
    
    // Line 74-81: Creates ConnectionManager
    let connection_manager = ConnectionManager::new(
        p2p_adaptor.clone(),
        self.outbound_target,
        self.inbound_limit,
        self.dns_seeders,
        self.default_port,
        self.flow_context.address_manager.clone(),
    );
}
```

### 5. **Kaspad Daemon Initialization**
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/kaspad/src/daemon.rs`
**Lines:** 578-607

```rust
let hub = Hub::new();
// ... (consensus, mining setup) ...
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
));
```

## Dependency Stack

### Tonic Version
```
tonic = { version = "0.12.3", features = ["tls-webpki-roots", "gzip", "transport"] }
tokio = { version = "1.33.0", features = ["sync", "rt-multi-thread"] }
tower = "0.5.1"
```

## Where to Inject SOCKS Proxy Support

### Option 1: Intercept at `Endpoint::new()` (Recommended)
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/protocol/p2p/src/core/connection_handler.rs`
**Line:** 103

Currently:
```rust
let channel = tonic::transport::Endpoint::new(peer_address)?
    .timeout(Duration::from_millis(Self::communication_timeout()))
    .connect_timeout(Duration::from_millis(Self::connect_timeout()))
    .tcp_keepalive(Some(Duration::from_millis(Self::keep_alive())))
    .connect()
    .await?;
```

Tonic 0.12.3 supports custom `Connector` implementations. You would:
1. Create a custom `Connector` struct that implements `tower::service::Service`
2. Use `Endpoint::connector(custom_connector)` to inject SOCKS proxy logic
3. Reference: tonic uses Tower's `MakeConnectorService` trait

### Option 2: Intercept at ConnectionManager Level
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/components/connectionmanager/src/lib.rs`
**Lines:** 130, 188

Add SOCKS proxy routing decision logic before calling `self.p2p_adaptor.connect_peer()`.
This would allow selectively routing certain addresses through the SOCKS proxy.

### Option 3: Intercept at Adaptor::connect() Method
**File:** `/Users/luke/Documents/GitHub/rusty-kaspa/protocol/p2p/src/core/adaptor.rs`
**Lines:** 73-85

Add a SOCKS proxy wrapper around `self.connection_handler.connect()`.

## Connection Flow with All Key Points

1. **kaspad/src/main.rs** → calls `create_core(args, fd_total_budget)`
2. **kaspad/src/daemon.rs:207-691** → creates P2pService with configuration
3. **protocol/flows/src/service.rs:62-99** → P2pService::start() creates Adaptor & ConnectionManager
4. **protocol/p2p/src/core/adaptor.rs:73-85** → Adaptor::connect_peer() entry point
5. **protocol/p2p/src/core/connection_handler.rs:96-142** → **ConnectionHandler::connect() ← PRIMARY INJECTION POINT**
   - Line 103: `Endpoint::new()` - creates Tonic endpoint
   - Line 107: `.connect()` - **ACTUAL TCP CONNECTION HAPPENS HERE**
   - Line 121: `client.message_stream()` - establishes bidirectional gRPC stream
6. **components/connectionmanager/src/lib.rs** → manages retry logic and address selection

## Implementation Notes for SOCKS Proxy Integration

### Tonic Connector API
Tonic 0.12.3 uses Tower's `MakeConnectorService`. To add SOCKS support:

```rust
// Pseudocode
use tower::service::Service;
use hyper::client::HttpConnector;

pub struct SocksConnector {
    socks_proxy: SocketAddr,
    fallback: HttpConnector,
}

impl Service<Uri> for SocksConnector {
    type Response = TcpStream;
    type Error = io::Error;
    type Future = /* custom future that routes through SOCKS */;
    
    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        // Implementation
    }
    
    fn call(&mut self, uri: Uri) -> Self::Future {
        // Route through SOCKS proxy instead of direct TCP
    }
}
```

### Configuration Entry Points
1. **Command-line argument:** Add `--socks5-proxy <addr:port>` to `kaspad/src/args.rs`
2. **Pass through daemon.rs** to ConnectionHandler
3. **Inject into Endpoint in connection_handler.rs**

### Testing the Integration
1. Run kaspad with `--socks5-proxy 127.0.0.1:9050` (Tor SOCKS port)
2. Monitor connections via `socat` or Tor logs
3. Verify that all P2P connections route through the SOCKS proxy

## Summary

The critical insertion point for SOCKS proxy support is:

**`/Users/luke/Documents/GitHub/rusty-kaspa/protocol/p2p/src/core/connection_handler.rs:103-108`**

This is where Tonic's `Endpoint::connect()` is called. By providing a custom `Connector` to the `Endpoint` via the `.connector()` method before calling `.connect()`, all outbound P2P connections can be routed through a SOCKS proxy.

The flow is:
1. ConnectionManager decides which peer to connect to
2. Calls Adaptor::connect_peer()
3. Which calls ConnectionHandler::connect()
4. Which creates a Tonic Endpoint and calls `.connect()`  ← **INJECT HERE**
5. Tonic/Tower makes the actual TCP connection using the provided Connector

This single injection point controls all outbound P2P connections in Rusty-Kaspa.
