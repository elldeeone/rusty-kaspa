# UDP Block Mode (dev/test only)

Phase 5 introduces an *optional* BlockV1 ingestion path that reuses the same
validation pipeline as TCP peers. The feature is gated aggressively and is
intended for development and testnets only.

## Enabling block ingestion

Block mode stays disabled unless **all** of the following are true:

1. `--udp.enable=true` and the UDP listener is configured (`--udp.listen` or
   `--udp.listen_unix`).
2. `--udp.mode` includes `blocks` (either `blocks` or `both`).
3. `--udp.danger_accept_blocks=true`.
4. On mainnet, an explicit `--udp.block_mainnet_override=true` is also required.

Additional tuning flags:

- `--udp.block_queue=<N>`: bounded queue depth (default 32).
- `--udp.block_max_bytes=<N>`: hard payload cap for fully reassembled blocks
  (default 1 MiB).

When disabled, the node behaves exactly like the Phase‑4 digest build.

## Virtual peer injection

Accepted BlockV1 payloads feed a hidden “satellite” virtual peer inside the
existing Router/Flow plumbing. The peer:

- never increments user-visible peer counts,
- uses low-priority channels so external TCP peers retain throughput, and
- tears down automatically when the UDP service is disabled.

No consensus shortcuts were added—every block still traverses the normal flow
set (IBD, block relay, validation).

## Observability

New metrics (bounded labels only):

- `udp_block_injected_total`
- `udp_queue_occupancy{queue="block"}`
- Block-specific drop reasons (`block_oversize`, `block_queue_full`,
  `block_unsupported_format`, `block_malformed`, `panic`).

`getUdpIngestInfo` now reports the block queue snapshot and injection counter so
operators can confirm whether the path is in use.

Structured logs follow the existing `udp.event=*` convention. Successful
injections are logged at debug level (`udp.event=block_inject seq=...`).

## Quick simnet demo

```
kaspad --simnet --enable-unsynced-mining \
  --udp.enable=true \
  --udp.listen=127.0.0.1:28515 \
  --udp.mode=blocks \
  --udp.danger_accept_blocks=true

# In another shell, craft or capture a full BlockV1 frame and send it over UDP.
# (See `testing/integration/src/udp_block_tests.rs` for a reference encoder.)

# Inspect runtime state
kasparpc-cli getUdpIngestInfo | jq '.blockInjectedTotal'
```

For automated regression coverage, run `cargo test -p kaspa-testing-integration
udp_block_tests::udp_block_equivalence` (block parity) and
`udp_block_tests::udp_block_fairness` (low-priority guard).

## Safety notes

- Never leave `--udp.danger_accept_blocks=true` enabled on production nodes.
- Use the admin RPC `udp.disable` to stop ingestion immediately if divergence
  or queue saturation is observed.
- Mainnet requires the explicit `--udp.block_mainnet_override=true` knob to
  guard against accidental rollout.
