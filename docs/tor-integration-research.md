# Tor Integration Research Notes (Oct 2025)

## Repository Context
- Working branch: `feature/tor-integration` (branched from upstream `master` on 2025-01-XX).
- Objective: allow rusty-kaspa nodes to run fully over Tor, including outbound peer discovery via SOCKS5 and optional Tor-hidden-service listening.

## Tor Connectivity Building Blocks

### Progress Update — 19 Oct 2025
- ✅ Outbound P2P dials now respect `--tor-proxy` by tunneling gRPC over the configured SOCKS5 endpoint (tested against system Tor 0.4.8.19).
- ✅ Hidden-service publication via the Tor control port is wired into `kaspad` startup; onion keys persist under `~/.rusty-kaspa/kaspa-mainnet/tor/`.
- ✅ Address manager filters `.onion` peers when Tor is disabled, restoring clearnet IBD performance while allowing Tor-only operation when requested.
- ✅ Stream isolation implemented: every outbound peer uses a unique SOCKS5 username/password pair, forcing Tor to create per-peer circuits.
- ✅ Added CLI options `--proxy` and `--tor-only` to mirror Bitcoin Core’s proxy controls; the node can now operate in Tor-only mode without clearnet peers.

### Progress Update — 22 Oct 2025
- ✅ Version-handshake now advertises Kaspa-specific `ADDRv2` capability (service bit) and tracks peer support in `PeerProperties`.
- ✅ Address gossip respects Tor activation: onion addresses are only accepted/advertised when Tor is configured locally *and* the remote peer signalled `ADDRv2`; clearnet remains unchanged.
- ✅ Added flow-level unit tests around the new onion-gossip helpers to guard regressions; integration tests will follow once the Tor harness is ready.
- ✅ Hardened operator UX: Tor bootstrap failures now abort `--tor-only/--listen-onion` startups, logs highlight the persistent onion key for backups, shutdown issues `DEL_ONION` to clean up hidden services, and a background `tor-service` continuously surfaces Tor control-port events.

**Next implementation phases (parity with Bitcoin Core’s Tor stack):**
1. CLI UX parity – document new flags and expand parity further (per-network proxy selection, config-file toggles).
2. Address gossip – advertise onion peers only to BIP155-capable/onion peers; add tests to prevent regression.
3. Operator ergonomics – document restart behaviour, key backup, graceful teardown, optional `--disable-upnp`/loopback binding for Tor-only deployments.
5. Long-term – evaluate Arti once onion-service support is production ready; consider bundled Tor for desktop builds.

### SOCKS5 Support for Outbound Traffic
- Current outbound P2P dials use `tonic::transport::Endpoint::connect()`. We can switch to `connect_with_connector` and supply a Hyper connector that speaks SOCKS5.

**SOCKS5 Library Options** (evaluated Oct 2025):
| Crate | Latest | Maintainer Activity | Hyper 1.x ready? | Notes |
| --- | --- | --- | --- | --- |
| `hyper-socks2` | 0.9.1 (Mar 2024) | Active GH issues, used by tonic clients | ✅ | Built as a Hyper connector; drop-in for `Endpoint::connect_with_connector`. |
| `tokio-socks` | 0.5.2 (Oct 2025) | Widely adopted | ➖ (needs wrapper) | Great low-level client; requires us to write a Hyper `Service`. |
| `fast-socks5` | 0.10.0 (Sep 2025) | Performance-focused | ➖ | Includes UDP/BIND; heavier than we need. |
| `tor-socksproto` | 0.35.0 (Oct 2025) | Tor Project | ➖ | Tor-specific framing + stream isolation helpers; still lower-level. |

**Recommended pattern** (validated with tonic 0.12 / Hyper 1):
1. Build a `HttpConnector` and call `enforce_http(false)` to allow non-HTTP URIs.
2. Wrap it with `hyper_socks2::SocksConnector { proxy_addr, auth, connector }`.
3. Layer TLS (`hyper-rustls` or `hyper-openssl`) and ensure ALPN includes `h2`.
4. Create a Hyper client with `hyper_util::client::legacy::Client::builder`.
5. Pass a `tower::service_fn` that forwards requests to the Hyper client into `Endpoint::connect_with_connector`.

This stack gives us gRPC over SOCKS5 while keeping tonic unchanged. We still generate a unique SOCKS username/password per peer (see “Tor protocol practices”) to enforce Tor stream isolation.

### Tor Daemon Control & Hidden Services
- `tor-interface` crate (v0.6.0, Oct 2025) exposes a `TorProvider` trait with implementations for:
  - **LegacyTorClient** — manages a bundled or system `tor` daemon, including control-port auth, bootstrap monitoring, SOCKS listener discovery, and automatic `.onion` creation.
  - **ArtiClientTorClient / ArtiTorClient** — experimental wrappers around the in-process Rust Tor stack (Arti) for client-only use.
  - **MockTorClient** — local testing without network I/O.
- The trait supports establishing outbound streams, creating listener onion services, and handling client-auth keys, giving us a single abstraction that can back both SOCKS proxying and hidden-service publishing.
- Control-port essentials we need regardless of library choice:
  - **Authentication**: prefer cookie auth (read Tor’s `control.authcookie`); fall back to `HashedControlPassword`.
  - **Service lifecycle**: send `ADD_ONION ED25519-V3:<key> Flags=Detach Port=<virt>,<target>` to create/restore a v3 service, and persist the returned private key.
  - **Discovery**: `GETINFO net/listeners/socks` tells us which SOCKS listeners Tor exposed (helpful for auto-config).
  - **Cleanup**: `DEL_ONION` only applies to non-detached services; persistent services should rely on saved keys.
- `kaspad` now wraps these primitives in a `TorManager` helper used during daemon bootstrap. For the first cut we connect to system Tor and record the SOCKS address for the networking layer; onion publication will hook into the same manager next.

### Status of Arti (Rust Tor Implementation)
- **Client status**: Arti 1.6.0 (Oct 2025) is production-ready for *client* use cases; MSRV 1.85.1.
- **Onion-service status**: Tor Project documentation (Aug 2025) and the `tor-hsservice` crate guide still label onion-service support as **experimental** and disabled by default. Missing items include DoS defenses, vanguard relays, descriptor encryption/client auth, and bounded on-disk state.
- **Verdict**: Arti remains unsuitable for hosting security-sensitive onion services. Continue to depend on the legacy C Tor daemon (via control port) for inbound hidden services. Re‑evaluate once Tor upstream announces full production readiness.
- **Hybrid approach**: We can still reuse Arti components (e.g., `tor-socksproto`, `arti-client`) for outbound-only functionality if we want an all-Rust stack later.

### Tor Protocol Practices & Data Model
- **Stream isolation**: Tor interprets SOCKS usernames starting with `<torS0X>`; format `"<torS0X>0"` + unique password guarantees separate circuits. We should generate a random password per outbound peer.
- **Remote DNS**: Always pass hostnames (SOCKS5 `ATYP=0x03`) so Tor performs DNS resolution, eliminating local DNS leaks.
- **Onion address type**: Represent v3 addresses with a Rust newtype that validates the 56-char base32 payload + `.onion` suffix on construction.
- **Address storage**: Extend `NetAddress`/`ContextualNetAddress` to support an enum variant for onion addresses (see design section) to avoid treating them as plain strings.
- **Daemom CLI plumbing**: `kaspad` now accepts `--tor-proxy`, `--tor-control`, `--tor-password/--tor-cookie`, `--tor-bootstrap-timeout-sec`, `--listen-onion`, `--tor-onion-port`, and `--tor-onion-key`. The daemon instantiates a `TorManager` when these are present, feeds the SOCKS endpoint into the P2P stack, and keeps enforcement guards for invalid combinations.
- **Hidden service publishing**: When `--listen-onion` is supplied, `kaspad` loads (or generates) an Ed25519 key at `<appdir>/<net>/tor/p2p_onion.key`, authenticates to the control port, and issues `ADD_ONION ED25519-V3:<key>` pointing at the existing P2P port. The resulting `V3OnionServiceId` is retained on the `FlowContext` for future advertising.

### Tor-Only Workflow & Key Backup
1. Start a Tor daemon exposing SOCKS (default `127.0.0.1:9050`) and control (`127.0.0.1:9051`) endpoints; ensure cookie permissions allow kaspad to read the auth file.
2. Launch kaspad with Tor flags, for example:
   ```bash
   cargo run -p kaspad -- \
     --tor-proxy=127.0.0.1:9050 \
     --tor-control=127.0.0.1:9051 \
     --tor-cookie="$HOME/Library/Application Support/Tor/control_auth_cookie" \
     --listen-onion \
     --tor-only
   ```
   The daemon blocks until Tor reports 100 % bootstrap. If bootstrap fails (timeout, auth error, etc.) kaspad now aborts instead of enabling a partially configured P2P stack.
3. After the hidden service is published, the log prints `Onion service key stored at …/p2p_onion.key`. Back up this file to persist your `.onion` name across redeployments. Restoring the file before startup re-publishes the same address.
4. On shutdown (Ctrl‑C or RPC stop), the new async `tor-service` calls `DEL_ONION <service id>`, preventing detached services from lingering when operators disable `--listen-onion`. The saved key still lets you recreate the address later by re-running with the same file.

### Bitcoin Core Tor Integration Architecture (Reference)
Bitcoin Core provides battle-tested Tor support that we can learn from. Key implementation details:

**Control Socket API Integration**:
- Bitcoin Core uses Tor's control socket API to programmatically create/destroy ephemeral onion services
- Supports Tor v3 onion services only (as of Bitcoin Core 22.0)
- Automatic onion service creation if Tor is running and properly authenticated

**Authentication Methods**:
- Cookie authentication (default): Tor writes a cookie file that Bitcoin Core reads
- Password authentication: via `-torpassword` configuration option
- Bitcoin Core detects and uses whichever method is available

**Network Configuration Flags**:
- `-proxy=ip:port` - Sets proxy server for all outbound connections (including DNS)
- `-onion=ip:port` - Sets proxy specifically for Tor onion services (overrides -proxy)
- `-listenonion=1/0` - Enable/disable listening on Tor (default: enabled if listening)
- `-torcontrol=ip:port` - Tor control port for ephemeral onion service creation

**Stream Isolation**:
- Bitcoin Core uses Tor stream isolation by default
- Each connection gets isolated circuit to prevent correlation attacks
- Implemented via SOCKS5 username/password fields (Tor-specific extension using `<torS0X>` prefix)

**Operational Flow**:
1. On startup, Bitcoin Core connects to Tor SOCKS proxy (default: 127.0.0.1:9050)
2. Connects to Tor control socket for service management
3. If `-listenonion=1`, creates ephemeral v3 onion service via control protocol
4. Routes all P2P connections through SOCKS proxy
5. Advertises .onion address to peers instead of clearnet IP

**Privacy Considerations**:
- With `-onlynet=onion`, Bitcoin Core only connects to .onion peers (maximum privacy)
- Avoids leaking clearnet IP in version messages and addr announcements
- DNS requests only use proxy if `-proxy` is set (not `-onion`)

**Other blockchain precedents**:
- **Monero (`monerod`)** – leans on user-managed torrc entries (`HiddenServiceDir`, `HiddenServicePort`) and uses `--tx-proxy/--anonymous-inbound` to point at SOCKS listeners.
- **Lightning (`lnd`)** – programmatic Tor control with flags like `tor.active`, `tor.control`, `tor.streamisolation=true`; demonstrates best practices in a non-C++ stack.
- **Zcash (`zcashd`)** – mirrors Bitcoin Core (external Tor, `-torcontrol`, `-listenonion`) and publishes v3 services by default.

## Design Implications for Rusty-Kaspa
1. **CLI / Config Surface**
   - Extend `kaspad` args to accept Tor settings (enable flag, path to tor binary/control port, SOCKS endpoint, client-auth keys).
   - Thread new options into `Config`, `FlowContext`, and P2P services.
2. **Outbound Peer Connections**
   - Create a Tor-aware connector that wraps Tonic’s `Endpoint` with `hyper-socks2`, parameterized by the Tor SOCKS listener from the `TorProvider`.
3. **Inbound Hidden Service**
   - When Tor mode is enabled, bind the P2P listener locally (e.g., `127.0.0.1`) and publish the `.onion` via `TorProvider::listener`. Update address advertisement logic to include onion addresses and avoid leaking clearnet IPs.
   - Extend `NetAddress`/`ContextualNetAddress` and address-manager storage to support `.onion` peers (possibly with an enum discriminant for onion vs IP).
4. **Runtime Lifecycle**
   - Initialize `TorProvider` during daemon startup, bootstrap Tor before networking, monitor events (e.g., `OnionServicePublished`), and integrate graceful shutdown.


## Platform-Specific Tor Management Considerations

### Tor Daemon Availability
**Linux**:
- Usually available via package managers: `apt install tor`, `yum install tor`, etc.
- Systemd service management on modern distributions
- Default control port: 9051, SOCKS port: 9050
- Cookie auth file typically at `/var/run/tor/control.authcookie`

**macOS**:
- Available via Homebrew: `brew install tor`
- Can run as LaunchAgent for user-level daemon
- Similar port defaults to Linux

**Windows**:
- Tor Expert Bundle provides standalone tor.exe
- No system service by default; needs process management
- May require bundling tor.exe with kaspad distribution
- Cookie file location varies by installation

### Deployment Strategies

**1. System Tor Daemon (Recommended for servers)**:
- Pros: User manages Tor separately, standard system service, shared among applications
- Cons: Requires pre-installation, configuration coordination
- Implementation: Use `tor-interface` LegacyTorClient pointing to localhost:9050/9051

**2. Bundled Tor Daemon**:
- Pros: No external dependencies, guaranteed compatibility, easier for desktop users
- Cons: Larger distribution size, process lifecycle management complexity
- Implementation: Ship `tor` binary, spawn as child process, manage lifecycle

**3. Embedded Arti (Future)**:
- Pros: Pure Rust, in-process, no external binary, excellent embedding story
- Cons: Onion service support not yet production-ready (Oct 2025)
- Implementation: Link `arti-client` directly into kaspad
- Timeline: Monitor Arti 1.7+ releases for stable onion service support

### Control Port Authentication

**Cookie Authentication** (preferred for automated operation):
- Tor writes random cookie to filesystem
- Application reads cookie to authenticate
- No user configuration needed
- Challenge: File permissions and discovery
- Tor config: `CookieAuthentication 1`

**Password Authentication**:
- User sets hashed password in Tor config
- Application provides password via control protocol
- Challenge: Secure password distribution
- Tor config: `HashedControlPassword <hash>`

**Recommendation**: Support both methods. Try cookie auth first, fall back to password if specified via `-torpassword` flag.

### Bootstrap and Lifecycle Management

**Startup Sequence**:
1. If using bundled Tor: spawn tor process with custom torrc
2. Wait for control port to become available (poll with timeout)
3. Authenticate via control protocol
4. Wait for Tor bootstrap complete (monitor `BOOTSTRAP` events)
5. Retrieve SOCKS port info via `GETINFO net/listeners/socks`
6. If onion service needed: create via `ADD_ONION` command
7. Start P2P networking with SOCKS proxy configured

**Graceful Shutdown**:
1. Stop accepting new P2P connections
2. Close existing connections
3. If ephemeral onion service: send `DEL_ONION` command
4. Disconnect from control port
5. If bundled Tor: send SIGTERM, wait for clean exit

**Health Monitoring**:
- Monitor control connection for async events
- Watch for circuit failures, bootstrap issues
- Expose Tor status via RPC for user visibility
- Consider metrics: circuits built, bandwidth, connection count


## Rusty-Kaspa Tor Integration Design

### Architecture Overview

Based on the codebase analysis, the Tor integration requires modifications across 4 main layers:

1. **Configuration Layer** (CLI → Config)
2. **P2P Connection Layer** (SOCKS proxy injection)
3. **Network Address Layer** (.onion address support)
4. **Runtime Layer** (Tor lifecycle management)

### Layer 1: Configuration & CLI

**Files to Modify**:
- `kaspad/src/args.rs` (lines 27-94: Args struct, 197-391: CLI definition)
- `consensus/core/src/config/mod.rs` (lines 20-74: Config struct)

**New CLI Flags** (mirroring Bitcoin Core where sensible):
```rust
// kaspad/src/args.rs - Add to Args struct
pub struct Args {
    // ... existing fields ...
    
    // Tor SOCKS Proxy Configuration
    pub tor_proxy: Option<String>,              // --tor-proxy=127.0.0.1:9050
    
    // Tor Control Port (for hidden service management)
    pub tor_control: Option<String>,            // --tor-control=127.0.0.1:9051
    pub tor_password: Option<String>,           // --tor-password=<password>
    
    // Onion Service Configuration
    pub listen_onion: bool,                     // --listen-onion (default: true if tor_control set)
    pub onion_address: Option<String>,          // --onion-address=<addr>.onion (manual override)
    
    // Privacy/Routing Options
    pub onlynet: Option<String>,                // --onlynet=onion (tor-only mode)
    pub tor_stream_isolation: bool,             // --tor-stream-isolation (default: true)
}
```

**Config Struct Updates**:
```rust
// consensus/core/src/config/mod.rs
pub struct Config {
    // ... existing fields ...
    pub tor_proxy: Option<String>,
    pub tor_control: Option<String>,
    pub tor_password: Option<String>,
    pub listen_onion: bool,
    pub onion_address: Option<OnionAddress>,  // New type
    pub onlynet: Option<NetworkType>,         // enum: Clearnet, Onion
    pub tor_stream_isolation: bool,
}
```

**Config Flow**:
```
kaspad/src/main.rs:19-50 (create_core)
    ↓
kaspad/src/daemon.rs:206-243 (create_core_with_runtime)
    ↓ ConfigBuilder::new(network).apply_args(|config| args.apply_to_config(config))
    ↓
kaspad/src/args.rs:150-170 (Args::apply_to_config) ← ADD TOR SETTINGS HERE
    ↓
Config threaded through to P2P services
```

### Layer 2: P2P Connection (SOCKS Injection)

**Critical Discovery**: ALL outbound P2P connections flow through **ONE SINGLE POINT**:

**File**: `protocol/p2p/src/core/connection_handler.rs`
**Line**: 103-108
**Current Code**:
```rust
async fn connect(&self, peer_address: String) -> Result<KaspadMessageStream> {
    let endpoint = tonic::transport::Endpoint::new(peer_address)?;
    let conn = endpoint.connect().await?;  // ← INJECT SOCKS HERE
    // ... rest of connection setup
}
```

**Modified Implementation Strategy**:

**Option A: Hyper Custom Connector (Recommended)**
```rust
// NEW FILE: protocol/p2p/src/core/socks_connector.rs
use tokio_socks::tcp::Socks5Stream;
use tower::service_fn;
use hyper::Uri;

pub struct SocksConnector {
    proxy_addr: String,
    stream_isolation: bool,
}

impl tower::Service<Uri> for SocksConnector {
    type Response = tokio::net::TcpStream;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    
    fn call(&mut self, uri: Uri) -> Self::Future {
        let proxy = self.proxy_addr.clone();
        let isolation = self.stream_isolation;
        
        Box::pin(async move {
            let target = format!("{}:{}", uri.host().unwrap(), uri.port_u16().unwrap());
            
            // Tor stream isolation via SOCKS username (random per connection)
            let auth = if isolation {
                Some((format!("stream-{}", rand::random::<u64>()), String::new()))
            } else {
                None
            };
            
            let stream = if let Some((user, pass)) = auth {
                Socks5Stream::connect_with_password(proxy, target, &user, &pass).await?
            } else {
                Socks5Stream::connect(proxy, target).await?
            };
            
            Ok(stream.into_inner())
        })
    }
}

// MODIFIED: protocol/p2p/src/core/connection_handler.rs
pub struct ConnectionHandler {
    // ... existing fields ...
    socks_proxy: Option<SocksConnector>,  // NEW
}

impl ConnectionHandler {
    async fn connect(&self, peer_address: String) -> Result<KaspadMessageStream> {
        let endpoint = tonic::transport::Endpoint::new(peer_address)?;
        
        let conn = if let Some(ref socks) = self.socks_proxy {
            // Use SOCKS connector
            endpoint.connect_with_connector(socks.clone()).await?
        } else {
            // Direct connection (existing behavior)
            endpoint.connect().await?
        };
        
        // ... rest unchanged
    }
}
```

**Call Chain for SOCKS Injection**:
```
kaspad/src/daemon.rs:597-607 (P2pService creation) ← Pass tor_proxy config
    ↓
protocol/flows/src/service.rs:17-114 (P2pService::new) ← Accept socks_proxy param
    ↓
protocol/p2p/src/core/adaptor.rs:72-85 (connect_peer) ← Thread through
    ↓
protocol/p2p/src/core/connection_handler.rs:145-176 (retry logic) ← Thread through
    ↓
protocol/p2p/src/core/connection_handler.rs:96-142 (connect method) ← USE SOCKS HERE
```

**Files to Modify** (6 total):
1. `protocol/p2p/src/core/socks_connector.rs` - NEW FILE
2. `protocol/p2p/src/core/connection_handler.rs` - Add socks_proxy field
3. `protocol/p2p/src/core/adaptor.rs` - Thread socks_proxy parameter
4. `protocol/flows/src/service.rs` - Thread socks_proxy parameter
5. `kaspad/src/daemon.rs` - Parse and pass socks_proxy from Config
6. `Cargo.toml` for protocol/p2p - Add `tokio-socks` dependency

### Layer 3: Network Address (.onion Support)

**Current State**:
- `IpAddress` wraps `std::net::IpAddr` (IPv4/IPv6 only) - `utils/src/networking.rs:76`
- `NetAddress` = `IpAddress + u16 port` - `utils/src/networking.rs:223`
- Database key: Fixed 18 bytes (16 IPv6 + 2 port) - `components/addressmanager/src/stores/address_store.rs:35-73`
- Protobuf: `bytes ip + uint32 port` - `protocol/p2p/proto/p2p.proto:15-19`

**Problem**: `.onion` addresses are 56-character strings (v3), can't fit in current types.

**Solution: Enum Discriminant Approach**

**Modified Types**:
```rust
// utils/src/networking.rs - Replace IpAddress
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkAddress {
    IPv4(Ipv4Addr),
    IPv6(Ipv6Addr),
    Onion(OnionAddress),  // NEW
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OnionAddress {
    addr: String,  // e.g., "abcdef1234567890.onion"
}

// NetAddress becomes:
pub struct NetAddress {
    pub network_addr: NetworkAddress,  // Was: IpAddress
    pub port: u16,
}
```

**Protobuf Changes**:
```protobuf
// protocol/p2p/proto/p2p.proto
message NetAddress {
    oneof address {
        bytes ipv4 = 1;         // 4 bytes
        bytes ipv6 = 2;         // 16 bytes
        string onion = 3;       // Variable length .onion
    }
    uint32 port = 4;
}
```

**Database Storage**:
```rust
// components/addressmanager/src/stores/mod.rs
// Replace fixed AddressKey with variable-length approach
pub enum AddressKey {
    Ip(Ipv6Addr, u16),      // 18 bytes (existing)
    Onion(String, u16),     // Variable (new)
}

impl AddressKey {
    fn to_db_key(&self) -> Vec<u8> {
        match self {
            AddressKey::Ip(ip, port) => {
                let mut key = Vec::with_capacity(19);
                key.push(0x00);  // Discriminant: IP
                key.extend_from_slice(&ip.octets());
                key.extend_from_slice(&port.to_le_bytes());
                key
            }
            AddressKey::Onion(addr, port) => {
                let mut key = Vec::with_capacity(1 + addr.len() + 2);
                key.push(0x01);  // Discriminant: Onion
                key.extend_from_slice(addr.as_bytes());
                key.extend_from_slice(&port.to_le_bytes());
                key
            }
        }
    }
}
```

**Validation Updates**:
```rust
// utils/src/networking.rs - Update is_publicly_routable()
impl NetworkAddress {
    pub fn is_publicly_routable(&self) -> bool {
        match self {
            NetworkAddress::IPv4(ip) => /* existing IPv4 logic */,
            NetworkAddress::IPv6(ip) => /* existing IPv6 logic */,
            NetworkAddress::Onion(_) => true,  // .onion always routable via Tor
        }
    }
}
```

**Files to Modify** (8 total):
1. `utils/src/networking.rs` - Redefine IpAddress → NetworkAddress enum
2. `protocol/p2p/proto/p2p.proto` - Update NetAddress message
3. `protocol/p2p/src/convert/net_address.rs` - Update protobuf conversion (lines 13-66)
4. `components/addressmanager/src/stores/mod.rs` - Update AddressKey
5. `components/addressmanager/src/stores/address_store.rs` - Update serialization
6. `components/addressmanager/src/lib.rs` - Update validation logic
7. `protocol/flows/src/v5/address.rs` - Update address protocol
8. `rpc/core/src/model/peer.rs` - Update RPC peer info

**Migration Strategy**:
- Keep backward compatibility by checking discriminant byte
- Old 18-byte keys (no discriminant) assumed to be IPv4/IPv6
- New keys start with 0x00 (IP) or 0x01 (Onion)

### Layer 4: Tor Runtime Lifecycle

**Integration Point**: `kaspad/src/daemon.rs` (daemon initialization)

**Lifecycle Sequence**:
```
1. main.rs:19-50 → create_core()
2. daemon.rs:206-243 → create_core_with_runtime()
   ↓
   [INSERT TOR INITIALIZATION HERE]
   ↓
3. daemon.rs:484-607 → P2P Service setup
4. Start networking with Tor configured
```

**Implementation Approach**:

```rust
// NEW FILE: kaspad/src/tor_manager.rs
use tor_interface::{TorProvider, LegacyTorClient, TorEvent};

pub struct TorManager {
    client: Box<dyn TorProvider>,
    socks_addr: String,
    onion_address: Option<String>,
}

impl TorManager {
    pub async fn new(config: &Config) -> Result<Option<Self>> {
        if config.tor_control.is_none() && config.tor_proxy.is_none() {
            return Ok(None);  // Tor not enabled
        }
        
        // Option 1: Use system Tor daemon via control port
        let client = if let Some(control_addr) = &config.tor_control {
            let mut tor = LegacyTorClient::new(control_addr)?;
            
            // Authenticate (try cookie first, then password)
            if let Some(password) = &config.tor_password {
                tor.authenticate_with_password(password).await?;
            } else {
                tor.authenticate_with_cookie().await?;
            }
            
            // Wait for bootstrap
            tor.wait_for_bootstrap().await?;
            
            Box::new(tor) as Box<dyn TorProvider>
        }
        // Option 2: Use bundled Tor (future work)
        // Option 3: Use Arti in-process (future work)
        else {
            return Err("Tor control port required")?;
        };
        
        // Get SOCKS proxy address
        let socks_addr = client.socks_addr().await?;
        
        // Create onion service if requested
        let onion_address = if config.listen_onion {
            let local_addr = config.p2p_listen_address.normalize(config.default_p2p_port());
            let onion = client.create_onion_service(
                vec![(config.default_p2p_port(), local_addr.port)],
                None,  // No client auth
            ).await?;
            Some(onion.address)
        } else {
            None
        };
        
        Ok(Some(TorManager {
            client,
            socks_addr,
            onion_address,
        }))
    }
    
    pub fn socks_addr(&self) -> &str {
        &self.socks_addr
    }
    
    pub fn onion_address(&self) -> Option<&str> {
        self.onion_address.as_deref()
    }
    
    pub async fn shutdown(self) -> Result<()> {
        // Cleanup onion service
        if let Some(onion) = self.onion_address {
            self.client.delete_onion_service(&onion).await?;
        }
        Ok(())
    }
}

// MODIFIED: kaspad/src/daemon.rs
pub fn create_core_with_runtime(...) -> (...) {
    // ... existing config creation ...
    
    // NEW: Initialize Tor if configured
    let tor_manager = runtime.block_on(async {
        TorManager::new(&config).await
    })?;
    
    // Extract SOCKS proxy for P2P
    let socks_proxy = tor_manager.as_ref().map(|tm| tm.socks_addr().to_string());
    
    // ... existing core initialization ...
    
    // Pass socks_proxy to P2P service creation (line 597-607)
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
        socks_proxy,  // NEW PARAMETER
    ));
    
    // ... rest of initialization ...
    
    // Store tor_manager for later shutdown
}
```

**Files to Modify** (3 total):
1. `kaspad/src/tor_manager.rs` - NEW FILE
2. `kaspad/src/daemon.rs` - Initialize TorManager, pass to P2P
3. `Cargo.toml` for kaspad - Add `tor-interface` dependency

**Dependencies to Add**:
```toml
[dependencies]
tor-interface = "0.6.0"  # For Tor control protocol
tokio-socks = "0.5"      # For SOCKS5 client in P2P layer
```

### Implementation Phases

**Phase 1: Outbound SOCKS Proxy** (Minimum Viable Tor)
- Add CLI flags for --tor-proxy
- Implement SocksConnector in connection_handler.rs
- Thread socks_proxy through P2P stack
- Test: kaspad --tor-proxy=127.0.0.1:9050 connects to clearnet peers via Tor

**Phase 2: Onion Service Publishing**
- Add --tor-control and --listen-onion flags
- Implement TorManager for lifecycle management
- Test: kaspad creates .onion address, peers can connect to it

**Phase 3: .onion Address Support**
- Refactor NetworkAddress enum to support .onion
- Update database storage, protobuf, serialization
- Update address manager validation
- Test: kaspad can store, advertise, connect to .onion peers

**Phase 4: Privacy Enhancements**
- Implement stream isolation (random SOCKS username per connection)
- Add --onlynet=onion flag for Tor-only mode
- Prevent clearnet IP leakage in Tor-only mode
- Test: kaspad --onlynet=onion never connects to clearnet

**Phase 5: Production Hardening**
- Cross-platform Tor daemon management
- Bundled Tor binary option
- Health monitoring and metrics
- Arti integration (when onion services mature)

## Operational Considerations
- **Tor packaging & distribution**
  - Linux/macOS: document use of system packages (`apt install tor`, `brew install tor`) and group membership needed to read `control.authcookie`.
  - Windows: bundle the Tor Expert Bundle (`tor.exe` plus data directory) and launch/manage it as a child process.
  - Containers: run Tor in a sibling container (e.g., docker-compose `tor` service) and communicate via the internal bridge network.
- **Security posture**
  - Run kaspad and Tor under separate users or containers; bind control port to localhost.
  - Prefer cookie authentication; support password auth as a fallback only.
  - When Tor mode or `--onlynet=onion` is active, disable UPnP/NAT-PMP, mDNS/local address gossip, and any logs that could leak clearnet IPs.
- **Performance expectations**
  - First circuit build adds seconds of latency; steady-state round trips are typically 100–300 ms. Tune dial retries/timeouts accordingly.
  - C Tor’s single-threaded design is acceptable for our expected connection counts; still capture CPU/memory metrics for observability.
- **Testing strategy**
  - Use Tor Project’s Chutney to spin up disposable Tor networks for integration tests and CI.
  - Add smoke tests for control-port failures, onion publication, and stream isolation (unique username/password per connection).
  - Manual QA: stage at least two Tor-backed nodes and verify full sync/transaction propagation with clearnet peers.

## Next Implementation Steps

**Research Complete** ✓ - Architecture analysis finished, comprehensive design documented above.

### Phase 1: Prototype & Validate (Recommended Next Step)

1. **Proof-of-Concept Binary** (standalone validation):
   - Create `examples/tor_poc.rs` in rusty-kaspa repo
   - Demonstrate `tor-interface` LegacyTorClient usage:
     - Connect to local Tor daemon on 127.0.0.1:9051
     - Try cookie auth, fall back to password
     - Wait for Tor bootstrap completion
     - Retrieve SOCKS proxy address
     - Create ephemeral v3 onion service
     - Print `.onion` address
   - Demonstrate `tokio-socks` outbound connection:
     - Connect to a test clearnet endpoint via SOCKS proxy
     - Verify stream isolation (multiple connections get different circuits)
   - **Goal**: Validate all Tor libraries work as expected before touching rusty-kaspa core

2. **SOCKS Connector Implementation**:
   - Implement `protocol/p2p/src/core/socks_connector.rs` as designed
   - Write unit tests with mock SOCKS server
   - Integration test with real Tor daemon

3. **Minimal Integration** (Phase 1 from design):
   - Add `--tor-proxy` CLI flag only
   - Thread through to ConnectionHandler
   - Test: Connect to existing Kaspa testnet peers via Tor

### Phase 2-5: Full Integration (See design section above)

Follow the phased approach in the "Rusty-Kaspa Tor Integration Design" section.

- **Outstanding spikes / validation**
  - Audit Bitcoin Core sources (`src/torcontrol.cpp`, `src/net.cpp`, `src/netbase.cpp`) for nuanced error handling and stream-isolation behavior.
  - Prototype `tor-interface::LegacyTorClient` vs. a minimal bespoke control-port client to decide which path we adopt.
  - Benchmark Tor-backed sync using a Chutney testnet (connection latency, throughput, Tor CPU/memory) to validate assumptions.

## Research Handover Checklist (For Next Engineer)
- **Validate TorProvider options in practice**
  - Confirm `LegacyTorClient` works cross-platform (Linux/macOS/Windows) with our deployment assumptions and note required Tor versions.
  - Evaluate the experimental `arti` modes to confirm onion-service limitations and track upstream milestones for when we could switch.
- **Prototype & Document**
  - Build the proposed Tokio proof-of-concept (outbound SOCKS stream + onion listener) and commit the sample under `examples/` or a `research/` folder with usage instructions.
  - Capture command-line steps for provisioning Tor control auth (cookie vs. hashed password) and any pitfalls (e.g., permissions).
- **Networking Model Updates**
  - Draft a proposal (diagram + notes) for extending `NetAddress` and related RPC/serialization flows to support onion URIs without breaking existing peers.
  - Identify where address advertisement occurs (version message, address flow, RPC) and flag code touchpoints requiring onion-aware logic.
- **Config & UX Alignment**
  - Compare Bitcoin Core Tor CLI flags to our `Args` structure; produce a recommendation mapping (what we mirror verbatim vs. rename).
  - Suggest telemetry/logging hooks to ensure Tor bootstrap and onion publication status surface clearly to users.
- **Deliverables**
- Update this document with findings, links, and TODOs.
- Open GitHub issues or Notion tasks for any sizable follow-on work identified during research.


## Reference Links
1. `tor-interface` crate 0.6.0 (2025-10-03): <https://crates.io/crates/tor-interface>
2. `hyper-socks2` crate 0.9.1 (2024-03-08): <https://crates.io/crates/hyper-socks2>
3. Arti Compatibility Notes (2025-10): <https://gitlab.torproject.org/tpo/core/arti/-/raw/main/doc/Compatibility.md>
4. `tokio-socks` crate: <https://crates.io/crates/tokio-socks>
5. `fast-socks5` crate: <https://crates.io/crates/fast-socks5>
6. `tor-socksproto` crate (Arti): <https://crates.io/crates/tor-socksproto>
7. `tor-hsservice` crate (Arti onion services): <https://lib.rs/crates/tor-hsservice>
8. Arti 1.6.0 release notes: <https://forum.torproject.org/t/arti-1-6-0-released/20649>
9. Arti project repository: <https://gitlab.torproject.org/tpo/core/arti>
10. Bitcoin Core Tor documentation: <https://github.com/bitcoin/bitcoin/blob/master/doc/tor.md>
11. Tor Control Protocol Specification: <https://spec.torproject.org/control-spec/>
12. Tor SOCKS Extensions: <https://spec.torproject.org/socks-extensions.html>

## Auxiliary Research Materials

Detailed technical analysis documents are available in `/research/tor-integration/`:

1. **README_P2P_ANALYSIS.md** - Index and navigation guide
2. **P2P_CONNECTION_ANALYSIS.md** - Detailed P2P connection flow analysis (289 lines)
3. **P2P_QUICK_REFERENCE.txt** - Quick lookup table with file paths and line numbers
4. **P2P_FINDINGS_SUMMARY.txt** - Executive summary of networking architecture
5. **P2P_INJECTION_VISUAL.txt** - ASCII diagrams of connection flow and injection points
6. **SOCKS_PROXY_IMPLEMENTATION.md** - Step-by-step SOCKS implementation guide

These documents provide granular implementation details and can serve as reference during development.

Additional third-party research dumps (original prompts):  
- `research/tor-integration/CHATGPT_RESEARCH.MD`  
- `research/tor-integration/GEMINI_RESEARCH.MD`
- Implementation progress: `FlowContext` now records an optional SOCKS endpoint and `ConnectionHandler::connect` uses a `tokio-socks`/`tower::Service` bridge (wrapped in `hyper_util::rt::TokioIo`) whenever Tor is enabled. `Adaptor::client_only`/`bidirectional` accept a `SocksProxyConfig` so the runtime toggles proxy behavior without knowledge of Tor internals.
