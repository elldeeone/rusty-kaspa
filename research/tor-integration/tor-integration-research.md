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
- ✅ Version-handshake now advertises Kaspa-specific `ADDRv2` capability (service bit) and tracks peer support in `PeerProperties`.
- ✅ Address gossip respects Tor activation: onion addresses are only accepted/advertised when Tor is configured locally *and* the remote peer signalled `ADDRv2`; clearnet remains unchanged.
- ✅ Added flow-level unit tests around the new onion-gossip helpers to guard regressions; integration tests will follow once the Tor harness is ready.
- ✅ Hardened operator UX: Tor bootstrap failures now abort `--tor-only/--listen-onion` startups, logs highlight the persistent onion key for backups, shutdown issues `DEL_ONION` to clean up hidden services, and a background `tor-service` continuously surfaces Tor control-port events.
- ✅ CLI parity in progress: new `--proxy-net=<network=addr>` flag mirrors Bitcoin Core's per-network proxy matrix (ipv4/ipv6/onion), and the runtime routes outbound dials through the most specific SOCKS entry.
- ✅ Added `--onlynet=<network>` to let operators restrict connectivity (e.g., `--onlynet=onion` for Tor-only) with enforcement across address manager, connection manager, seeding, and outbound proxy selection.

### Progress Update — 20 Oct 2025
- 📝 Documentation updates: consolidated Tor research notes, moved them under `research/tor-integration/`, and refreshed the write-up to match the final implementation.

**Remaining follow-ups**
1. Polish CLI/docs parity (surface the new Tor flags in the user guide and config templates).
2. Extend test coverage around onion gossip + `onlynet` enforcement.
3. Document operational guidance (Tor key backup, restart expectations, Tor-only + `--disable-upnp` recommendations).
4. Track long-term Arti readiness for bundled Tor builds.

### SOCKS5 Support for Outbound Traffic
- Current outbound P2P dials use `tonic::transport::Endpoint::connect()`. We can switch to `connect_with_connector` and supply a Hyper connector that speaks SOCKS5.

**SOCKS5 Library Options** (evaluated Oct 2025):
| Crate | Latest | Maintainer Activity | Hyper 1.x ready? | Notes |
| --- | --- | --- | --- | --- |
| `hyper-socks2` | 0.9.1 (Mar 2024) | Active GH issues, used by tonic clients | ✅ | Drop-in Hyper connector, but less flexible if we need custom auth per dial. |
| `tokio-socks` | 0.5.2 (Oct 2025) | Widely adopted | ✅ | Low-level client; we wrap it with `tower::service_fn` + `hyper_util::TokioIo` for tonic. |
| `fast-socks5` | 0.10.0 (Sep 2025) | Performance-focused | ➖ | Includes UDP/BIND; heavier than we need. |
| `tor-socksproto` | 0.35.0 (Oct 2025) | Tor Project | ➖ | Tor-specific framing + stream isolation helpers; still lower-level. |

**Recommended pattern** (validated with tonic 0.12 / Hyper 1):
1. Use `tower::service_fn` to expose an async closure that dials `tokio_socks::Socks5Stream::connect_with_password`.
2. Generate random username/password pairs per connection to force Tor stream isolation (no special prefixes required).
3. Wrap the resulting stream with `hyper_util::rt::TokioIo` so tonic/Hyper recognise it as an I/O object.
4. Call `Endpoint::connect_with_connector` with that service.

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
- **Stream isolation**: Generate a fresh random username/password per outbound peer (we use the `kaspa-<random>` convention). Tor treats each credential pair as an isolated stream, giving us separate circuits automatically.
- **Remote DNS**: Always pass hostnames (SOCKS5 `ATYP=0x03`) so Tor performs DNS resolution, eliminating local DNS leaks.
- **Onion address type**: Represent v3 addresses with a Rust newtype that validates the 56-char base32 payload + `.onion` suffix on construction.
- **Address storage**: Extend `NetAddress`/`ContextualNetAddress` to support an enum variant for onion addresses (see design section) to avoid treating them as plain strings.
- **Daemom CLI plumbing**: `kaspad` now accepts `--tor-proxy`, `--tor-control`, `--tor-password/--tor-cookie`, `--tor-bootstrap-timeout-sec`, `--listen-onion`, `--tor-onion-port`, and `--tor-onion-key`. The daemon instantiates a `TorManager` when these are present, feeds the SOCKS endpoint into the P2P stack, and keeps enforcement guards for invalid combinations.
- **Proxy matrix**: `--proxy` sets a default SOCKS5 proxy for clearnet networks while repeatable `--proxy-net=<network=addr>` overrides IPv4/IPv6/Onion individually (mirroring Bitcoin Core’s proxy map). Onion dials prefer the Tor manager’s autodiscovered SOCKS listener but fall back to the configured `--proxy-net=onion` or default entry.
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
5. P2P networking now waits on the Tor bootstrap signal before dialing peers, so the node never leaks clearnet traffic while Tor is still coming online.

#### Config-file snippet
```toml
# ~/.rusty-kaspa/kaspad.toml
tor-control = "127.0.0.1:9051"
tor-cookie = "/var/run/tor/control.authcookie"
listen-onion = true
tor-proxy = "127.0.0.1:9050"

# Route specific networks via distinct proxies (overrides the default above).
proxy-net = [
  "ipv4=127.0.0.1:9052",
  "onion=127.0.0.1:9050"
]

# Restrict connectivity to onion peers only (same as passing --onlynet=onion).
onlynet = ["onion"]
```
The TOML keys mirror the CLI flags introduced above. Multiple `proxy-net`/`onlynet` entries are expressed as arrays, matching Bitcoin Core’s configuration syntax.

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
- Implemented via SOCKS5 username/password fields with unique credentials per peer (no prefix required; Tor still isolates the circuits)

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
   - Create a Tor-aware connector that wraps Tonic’s `Endpoint` with a `tokio-socks` dialer, parameterized by the Tor SOCKS listener from the `TorProvider`, and expose it via `tower::service_fn`.
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
- `kaspad/src/args.rs` (Args struct, CLI flag wiring, proxy resolution)
- `kaspad/src/args.rs::apply_to_config` (default listen policies, UPnP toggles)
- `kaspad/src/daemon.rs` (Tor validation + runtime)

**Args Excerpt (actual implementation)**:
```rust
pub struct Args {
    // ...
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub tor_proxy: Option<ContextualNetAddress>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub tor_control: Option<ContextualNetAddress>,
    pub tor_password: Option<String>,
    pub tor_cookie: Option<PathBuf>,
    pub tor_bootstrap_timeout_sec: u64,
    pub listen_onion: bool,
    pub tor_onion_port: Option<u16>,
    pub tor_onion_key: Option<PathBuf>,
    pub tor_only: bool,
    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub proxy_net: Vec<ProxyRule>,
    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub onlynet: Vec<OnlyNet>,
    // ...
}
```

`Args::proxy_settings` merges these flags (default/proxy-net/onion) into a `ProxySettings` structure, while `Args::allowed_networks` honors `--onlynet`/`--tor-only`. `apply_to_config` ensures Tor-only nodes listen on loopback, drop `externalip`, and disable UPnP automatically.

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

**Implemented Connector**

```rust
// protocol/p2p/src/core/connection_handler.rs
let channel = if let Some(proxy_addr) = self.socks_proxy.and_then(|cfg| {
    if peer_net_address.as_onion().is_some() {
        cfg.onion.or(cfg.default)
    } else if let Some(ip) = peer_net_address.as_ip() {
        match IpAddr::from(ip) {
            IpAddr::V4(_) => cfg.ipv4.or(cfg.default),
            IpAddr::V6(_) => cfg.ipv6.or(cfg.default),
        }
    } else {
        cfg.default
    }
}) {
    let connector = service_fn(move |uri: Uri| {
        let proxy_addr = proxy_addr;
        async move {
            let host = uri.host().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing host in URI"))?.to_string();
            let port = uri.port_u16().unwrap_or(80);
            let target = format!("{host}:{port}");
            let (username, password) = generate_socks_credentials();
            let stream = Socks5Stream::connect_with_password(proxy_addr, target, &username, &password)
                .await
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
            Ok::<_, io::Error>(TokioIo::new(stream.into_inner()))
        }
    });
    endpoint.connect_with_connector(connector).await?
} else {
    endpoint.connect().await?
};
```

The connector lives entirely inside `connection_handler.rs`, reusing the existing `SocksProxyConfig` plumbing and delivering a Hyper-compatible stream via `hyper_util::rt::TokioIo`.

**Call Chain for SOCKS Injection**:
```
kaspad/src/daemon.rs → resolve proxies and pass them into P2pService
protocol/flows/src/service.rs → build SocksProxyConfig and create adaptor
protocol/p2p/src/core/adaptor.rs → forward config to ConnectionHandler
protocol/p2p/src/core/connection_handler.rs → dial peers through tokio-socks when configured
```

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

**TorManager Highlights**

- `TorManager::connect_system` (kaspad/src/tor_manager.rs) authenticates against the configured control port (cookie or password), waits for bootstrap completion, and records the Tor-provided SOCKS listener.
- `TorManager::publish_hidden_service` wraps the `ADD_ONION` control command, persisting Ed25519 keys on disk (creating them if absent) and returning the v3 onion identifier.
- `TorRuntimeService` polls Tor events asynchronously, forwarding bootstrap status to the P2P layer and issuing `DEL_ONION` on shutdown.
- `create_core_with_runtime` wires these pieces together: validate CLI flags, connect to Tor if configured, derive the effective proxy map, publish the onion service when `--listen-onion` is set, and delay network bring-up until Tor reports readiness.

## Implementation Summary

- **CLI & Config** – `kaspad/src/args.rs` gained Tor-focused flags (`--tor-proxy`, `--tor-control`, `--tor-only`, `--listen-onion`, onion key paths, bootstrap timeout, per-network proxy matrix) and propagates them through proxy resolution and allowed-network logic.
- **Daemon Startup** – `kaspad/src/daemon.rs` validates Tor args, instantiates `TorManager`, waits for bootstrap, wires the effective SOCKS map into the P2P service, publishes/removes onion services, and persists onion keys under the network data dir.
- **Tor Control Wrapper** – `kaspad/src/tor_manager.rs` wraps `tor-interface` (connect, bootstrap, publish/delete hidden services, load/save keys) so the daemon has a single integration point.
- **Outbound P2P** – `protocol/p2p/src/core/connection_handler.rs` now detects when Tor is active and routes outbound gRPC dials through `tokio-socks`, generating fresh `kaspa-<random>` credentials per peer and wrapping the stream with `hyper_util::rt::TokioIo`.
- **Address Handling** – `utils/src/networking.rs`, `components/addressmanager`, and associated stores understand the new `OnionAddress` type, store base32 payloads, and enforce Tor/onion policies across gossip and connection attempts.
- **Flow Context & Gossip** – `protocol/flows/src/flow_context.rs` threads Tor/onion state into handshake logic, advertises ADDRv2 support, and only gossips onion endpoints when both sides are capable. Address flows (`protocol/flows/src/v5/address.rs`) respect the same gating.
- **RPC & Proto Updates** – `protocol/p2p/proto/p2p.proto` plus converters in `protocol/p2p/src/convert` accept onion addresses, ensuring RPC layers surface the new transport family.
- **Runtime Services** – `protocol/flows/src/service.rs` passes a `SocksProxyConfig` into the adaptor so both inbound and outbound contexts agree on proxy routing, and waits for Tor bootstrap before opening the network.
- **Docs & Research** – this document plus the `research/tor-integration/` folder capture the architecture decisions, connector analysis, and operator guidance that informed the implementation.

**Dependencies Added**
- `tor-interface` (Tor control + hidden services)
- `tokio-socks` (async SOCKS5 client)
- `hyper-util` (Tokio I/O wrapper for Hyper)
- `tower` (connector glue via `service_fn`)
- `data-encoding` (base32 parsing/encoding for `.onion`)

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

## Follow-up Work

- Document the new Tor CLI flags and config knobs in the public operator guides, including examples for Tor-only deployments.
- Extend automated coverage around onion gossip, `--onlynet` enforcement, and Tor bootstrap failure handling.
- Capture operational runbooks (Tor cookie vs. password auth, onion-key backup, Tor-only + `--disable-upnp` recommendations).
- Monitor Arti’s onion-service roadmap to reassess bundled/in-process Tor once it can replace legacy c-tor.

## Open Questions

- Packaging: do we continue to rely on system Tor, or bundle a managed daemon for desktop builds?
- Observability: which control-port/metrics hooks should surface in diagnostics or dashboards?
- Long-term Arti integration: timeline for swapping the control/hidden-service path to Arti once feature-complete.


## Reference Links
1. `tor-interface` crate 0.6.0 (2025-10-03): <https://crates.io/crates/tor-interface>
2. `tokio-socks` crate 0.5.2 (2025-10-01): <https://crates.io/crates/tokio-socks>
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

Detailed technical analysis documents are available under `research/tor-integration/`:

1. [`research/tor-integration/README_P2P_ANALYSIS.md`](research/tor-integration/README_P2P_ANALYSIS.md) — index and navigation guide.
2. [`research/tor-integration/P2P_CONNECTION_ANALYSIS.md`](research/tor-integration/P2P_CONNECTION_ANALYSIS.md) — detailed P2P connection flow analysis.
3. [`research/tor-integration/P2P_QUICK_REFERENCE.txt`](research/tor-integration/P2P_QUICK_REFERENCE.txt) — quick lookup table with file paths and line numbers.
4. [`research/tor-integration/P2P_FINDINGS_SUMMARY.txt`](research/tor-integration/P2P_FINDINGS_SUMMARY.txt) — executive summary of the networking architecture.
5. [`research/tor-integration/P2P_INJECTION_VISUAL.txt`](research/tor-integration/P2P_INJECTION_VISUAL.txt) — ASCII diagrams of connection flow and injection points.
6. [`research/tor-integration/SOCKS_PROXY_IMPLEMENTATION.md`](research/tor-integration/SOCKS_PROXY_IMPLEMENTATION.md) — step-by-step SOCKS implementation guide (superseded by the final connector code but kept for historical context).

Additional third-party research dumps:
- [`research/tor-integration/CHATGPT_RESEARCH.MD`](research/tor-integration/CHATGPT_RESEARCH.MD)
- [`research/tor-integration/GEMINI_RESEARCH.MD`](research/tor-integration/GEMINI_RESEARCH.MD)
