# Integration Shutdown Notes

Command used to reproduce and verify the slow gate with verbose flow tracing:

```
RUST_LOG=info,kaspa_core=trace,kaspa_p2p_flows=trace,kaspa_connectionmanager=trace,kaspa_udp_sidechannel=trace,kaspad::udp=trace \
cargo test -p kaspa-testing-integration udp_block_tests::udp_block_equivalence -- --ignored --test-threads=1 --nocapture
```

## Pre-fix (2025‑11‑17 13:29 AEST)

Captured symptoms before the ingest/divergence wiring fix:

- **13:29:23.404** – `sat_virtual_peer injector loop started (127.0.0.1:0)` and `udp.event=block_injector_start`.
- **13:29:24–13:29:29** – Block equivalence test accepts the injected block and both daemons announce shutdown, but no `udp.event=ingest_signal_exit`/`udp.event=ingest_run_stopped` logs ever appear.
- **13:31+:** The test remains stuck; after ~120 s the harness starts printing `ReceiveAddressesFlow flow error: timeout expired...` while `daemon.join()` for the UDP node never completes. Command aborted after 300 s with no additional UDP lifecycle logs.

Conclusion: even though both daemons reach their regular shutdown logs, the UDP ingest/monitor tasks never observe the shared trigger, so background flows stay alive until their 120 s timeout expires and the integration gate hangs.

## Post-fix (2025‑11‑17 14:23 AEST)

Re-run of the same command (output saved to `/tmp/udp_slow_gate_phase5_after.log`) shows deterministic teardown driven by the shared flow context trigger:

- **14:23:35.019** – `sat_virtual_peer injector loop started (127.0.0.1:0)` and `udp.event=block_injector_start` confirm the UDP ingest stack is wired into the flow shutdown listener prior to block injection.
- **14:23:38.275** – The daemon drains services, emitting `udp.event=ingest_signal_exit`, `udp.event=ingest_shutdown`, `udp.event=ingest_run_stopped`, and `udp.event=ingest_service_start_returned` back-to-back before the fast fairness timers fire.
- **14:23:38.786** – `SendAddressesFlow shutdown for peer 127.0.0.1:0` appears alongside `sat_virtual_peer injector loop stopped (127.0.0.1:0)` and `udp.event=block_injector_stop reason=shutdown`, proving every long-lived UDP/P2P loop observed the same listener.
- **14:23:39.28x** – Both `p2p-service` instances log `stopped`, the WRPC/GRPC listeners close, and the ignored slow gate completes in ~4.3 s with no lingering ReceiveAddressesFlow timeouts.
- (Digest mode remains disabled in this scenario, so `udp-divergence-monitor stopped` will only appear when digest ingest is turned on.)

## Digest-mode sanity run (2025‑11‑17 14:36 AEST)

Re-running the slow gate with `UdpModeArg::Both` forces the UDP daemon to instantiate the digest pipeline. The captured log (`/tmp/udp_slow_gate_phase5_after_digest_info.log`) shows both divergence monitors observing the shared shutdown signal along with the expected ingest markers:

```
2025-11-17 14:36:26.661+11:00 [TRACE] udp.event=ingest_signal_exit
2025-11-17 14:36:26.661+11:00 [INFO ] udp-divergence-monitor signal_exit
2025-11-17 14:36:26.661+11:00 [TRACE] udp.event=ingest_shutdown
2025-11-17 14:36:26.661+11:00 [TRACE] udp.event=ingest_run_stopped
2025-11-17 14:36:26.661+11:00 [INFO ] udp-divergence-monitor stopped
…
2025-11-17 14:36:26.661+11:00 [TRACE] SendAddressesFlow shutdown for peer 127.0.0.1:0
2025-11-17 14:36:26.661+11:00 [TRACE] udp.event=block_injector_stop reason=shutdown
2025-11-17 14:36:27.050+11:00 [INFO ] Accepted block acda137b8bd20e5e72724b4f8d3f13dfa9520b074eace40fe825d62fe91490ac via submit block
test result: ok. 1 passed; 0 failed; 0 ignored; …
```

This confirms the digest-specific tasks exit deterministically without waiting for timeouts.
