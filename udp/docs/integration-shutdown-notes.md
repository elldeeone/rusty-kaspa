# Integration Shutdown Notes

Command used to capture the failing gate (with verbose flow tracing):

```
RUST_LOG=info,kaspa_p2p_flows=trace,kaspa_connectionmanager=trace,kaspa_udp_sidechannel=trace \
cargo test -p kaspa-testing-integration udp_block_tests::udp_block_equivalence -- --ignored --test-threads=1 --nocapture
```

Timeline extracted from `/tmp/udp_integration_trace.log` while reproducing the hang on
2025‑11‑17:

- **11:20:21.386** – UDP block `d39151f6…` accepted by the satellite injector (`satellite block … accepted`).
- **11:20:21.487** – Test harness prints “shutting down daemons”; both nodes emit `sending an exit signal to p2p-service`, gRPC connections close, and the connection manager event loop exits.
- **11:20:21.989–11:20:21.995** – Each daemon logs `p2p-service stopped`, `udp.event=listener_stopped`, and all servers (GRPC/P2P/WRPC) report clean shutdown.
- **11:22:18.223** – The process is still alive and the first `ReceiveAddressesFlow flow error: timeout expired after 120s, disconnecting from peer 127.0.0.1:0.` warning appears, repeating afterwards while the test never returns.

Conclusion: even though both daemons reach their shutdown logs, the virtual peer used for UDP block
injection never observes the cancellation signal, so background flows (ReceiveAddressesFlow et al.)
stay alive until their 120 s timeout expires, keeping the integration test hanging.
