# Tor/DNS Parity Plan

## Goals
Match Bitcoin Core's Tor behavior so all network traffic (including DNS lookups) can be routed via Tor when desired, and CLI semantics align. This includes:
1. Proxy parsing that preserves hostnames, allowing Tor to resolve them.
2. A "name proxy" concept so DNS lookups avoid the OS resolver when proxying.
3. Default listening behavior and CLI flags identical to Bitcoin's.

## Tasks
1. **Host-Preserving Proxy Targets**
   - Extend `ContextualNetAddress` (or add a new `ProxyTarget`) to store either `SocketAddr` or `(hostname, port)` without immediately resolving.
   - Update proxy parsing (`--proxy`, `--proxy-net`, `--tor-proxy`) to keep the original host string.
   - Adjust `ProxyConfigEntry`/`ProxyEndpoints` to carry the new type.

2. **Name Proxy Plumbing**
   - Track when a general proxy is configured (similar to Bitcoin's `nameProxy`).
   - When `nameProxy` is set, skip `.to_socket_addr()` in all outbound dial paths; pass the hostname to `Socks5Stream::connect` so Tor performs RESOLVE.
   - Ensure DNS seeding and `--addnode`/`--connect` follow the same rule: if name proxy is set, no OS resolver calls occur.

3. **DNS CLI Flags**
   - Introduce `--dns`/`--no-dnsseed`/`--force-dnsseed` equivalents so operators can mirror Bitcoin's behavior (allow or block clearnet DNS entirely).
   - Document that `--proxy` implies DNS lookups go through the proxy unless `--dns` is explicitly disabled.

4. **Listening Defaults**
   - When any proxy (ipv4/ipv6/onion) is set, default `--listen` to false (only onion listener stays active) unless the user opts in, matching Bitcoin.
   - Keep `--tor-only` as a convenience but align its behavior with `--onlynet=onion`.

5. **Testing / Validation**
   - Unit/integration tests covering hostname proxies, DNS seeding behavior with and without proxies, and ensuring no `to_socket_addr()` call happens when proxying.
   - Manual verification (lsof, Tor logs) similar to what we did today.

## Notes
- tokio-socks already handles hostnames by issuing SOCKS RESOLVE, so once we stop pre-resolving the names, Tor will handle DNS.
- Need to audit every place we call `.normalize()`/`.to_socket_addr()` to ensure we only do it when we truly want clearnet resolution (e.g., binding listeners).
- This plan intentionally mirrors Bitcoin's semantics to avoid surprises for operators moving between implementations.
