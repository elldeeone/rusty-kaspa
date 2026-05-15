# LoRa UDP Side-Channel Prototype

For review readiness, merge staging, and the current branch inventory, start
with [`review-readiness.md`](review-readiness.md). This file remains the
operator/prototype runbook and evidence index for the LoRa digest alpha.

This is the Phase 0/Phase 1 package for carrying existing rusty-kaspa UDP
side-channel `KUDP` digest datagrams over real Waveshare SX126X UART LoRa
hardware.

The prototype has three layers:

1. `lora-bridge`: byte-preserving serial LoRa transport for `KUDP` datagrams.
2. kaspad UDP ingest lab: recovered LoRa datagrams forwarded into the existing
   UDP side-channel ingest socket and surfaced by `getUdpIngestInfo` /
   `getUdpDigests`.
3. `udp-live-digest-producer`: lab-only live `DigestV1` producer that queries a
   local devnet/simnet kaspad RPC endpoint and emits signed digest datagrams.

## What This Proves

- Real Waveshare SX126X UART LoRa modules can carry existing `KUDP` DigestV1
  datagrams byte-for-byte.
- A 200-byte signed DigestV1 delta fits in one LoRa send.
- A 329-byte signed DigestV1 snapshot can be fragmented by the bridge and
  reassembled byte-for-byte.
- Recovered LoRa datagrams can be forwarded into kaspad's existing UDP ingest
  socket on devnet.
- Existing UDP ingest parses, verifies, stores, and exposes the received digest
  records through existing RPCs.
- A lab live producer can sign DigestV1 heartbeat records from local RPC state
  and feed the same LoRa bridge path.

## What This Does Not Prove

- No mainnet readiness.
- No synced-node requirement.
- No consensus mutation or consensus bypass.
- No `BlockV1` relay.
- No transaction or mempool relay.
- No LoRaWAN support.
- No FEC, encryption, or production RF reliability layer beyond the alpha
  bridge-local ACK/retry and redundant-frame experiments.
- No production signer/key-management story.
- No claim that the live producer is a complete consensus-authentic digest
  oracle yet.

## Hardware Requirements

- 2x Waveshare SX126X LoRa HATs over USB CP2102 UART.
- Stable device aliases:
  - `/dev/lora-left`
  - `/dev/lora-right`
- UART baud: `9600`.
- Tested fixed-send prefix: `00 00 41 00 00 41`.

The CP2102 boards report the same USB serial number, so stable `/dev/serial/by-id`
paths are not enough. Use path-based udev aliases. Example:

```udev
SUBSYSTEM=="tty", ENV{ID_VENDOR_ID}=="10c4", ENV{ID_MODEL_ID}=="ea60", ENV{ID_PATH}=="pci-0000:07:1b.0-usb-0:3:1.0", SYMLINK+="lora-left"
SUBSYSTEM=="tty", ENV{ID_VENDOR_ID}=="10c4", ENV{ID_MODEL_ID}=="ea60", ENV{ID_PATH}=="pci-0000:07:1b.0-usb-0:4:1.0", SYMLINK+="lora-right"
```

## Jumper Modes

Use the USB UART selector:

```text
UART selector: A
```

Normal TX/RX mode:

```text
M0: shorted
M1: shorted
```

Configuration mode:

```text
M0: shorted
M1: open
```

Avoid leaving both M0 and M1 open; that is deep sleep on these modules.

Known module config response:

```text
c1 00 09 00 00 00 62 20 41 c3 00 00
```

Read it in config mode:

```bash
cargo run -p lora-bridge -- config-read --serial /dev/lora-left
cargo run -p lora-bridge -- config-read --serial /dev/lora-right
```

## Measured LoRa Constraints

- Usable application payload per send: `234` bytes.
- Waveshare fixed-send prefix: 6 bytes:
  `dest_hi dest_lo freq src_hi src_lo src_freq`
- Receiver emits 3 source bytes before payload and 1 trailing status/RSSI byte
  after payload.
- `lora-bridge` strips the 3-byte RX prefix and trailing byte before forwarding
  recovered `KUDP` bytes.
- Writes larger than the hardware packet size can be silently truncated by the
  radio module. The bridge therefore fragments above 234 bytes.

Observed DigestV1 sizes:

```text
delta.bin    200 bytes
snapshot.bin 329 bytes
```

Without bridge-local ACKs, the default bridge inter-frame delay is `1500 ms`.
In real hardware testing, `250 ms` let the first snapshot fragment through but
dropped the second fragment. With `--reliable-fragments`, 100 snapshots passed
byte-exact at `250 ms`. The live lab still uses `2500 ms` because UDP-input TX
is currently batch-oriented and long live runs need more margin.

The bridge-local reliability protocol and current ACK-vs-redundant tradeoffs are
documented in
[`lora-bridge-reliability-protocol.md`](lora-bridge-reliability-protocol.md).
The repeatable reliability harness and current operating-envelope results are
documented in [`lora-reliability-report.md`](lora-reliability-report.md).
The completed live digest soak is summarized in
[`lora-live-soak-report.md`](lora-live-soak-report.md).
DigestV1 field authenticity and remaining production data gaps are tracked in
[`lora-digest-authenticity.md`](lora-digest-authenticity.md).
The current keep-`DigestV1` schema decision is documented in
[`lora-digest-schema-decision.md`](lora-digest-schema-decision.md).
Two-node agreement/divergence behavior is documented in
[`lora-two-node-divergence-lab.md`](lora-two-node-divergence-lab.md).

## Bridge Protocol

Raw datagrams up to 234 bytes are sent unchanged as the LoRa application
payload. Oversized datagrams are wrapped in a bridge-local fragmentation
envelope:

Best-effort envelope:

```text
bytes 0..3   magic = "KLR1"
bytes 4..7   datagram_id (u32 LE)
bytes 8..9   frag_ix (u16 LE)
bytes 10..11 frag_cnt (u16 LE)
bytes 12..13 original_datagram_len (u16 LE)
bytes 14..   KUDP byte slice
```

Reliable alpha envelope:

```text
bytes 0..3   magic = "KLR2"
bytes 4..7   session_id/group_id (u32 LE)
bytes 8..11  datagram_id (u32 LE)
bytes 12..13 frag_ix (u16 LE)
bytes 14..15 frag_cnt (u16 LE)
bytes 16..17 original_datagram_len (u16 LE)
bytes 18..   KUDP byte slice
```

Reliable ACK envelope:

```text
bytes 0..3   magic = "KLA1"
bytes 4..7   session_id/group_id (u32 LE)
bytes 8..11  datagram_id (u32 LE)
bytes 12..13 frag_ix (u16 LE)
```

These envelopes do not alter `KUDP`; they are RF adapter envelopes only.
Reassembly removes the envelope and recovers the exact original `KUDP`
datagram. In `--reliability-mode ack`, the reliable path ACKs each frame,
retransmits when ACKs time out, and treats duplicate fragments as safe. In
`--reliability-mode redundant`, TX sends each reliable frame
`--redundant-copies` times without waiting for ACKs, while RX suppresses
duplicate fragments and late duplicate datagrams before forwarding. `--group-id`
overrides `--session-id` for operators who prefer to describe the reliability
domain as a bridge group on a shared RF channel.

## Build

```bash
cargo build -p lora-bridge
cargo test -p lora-bridge
cargo build -p udp-generator \
  --bin udp-live-digest-producer \
  --bin udp-rpc-digests \
  --bin udp-digest-fixtures
```

## Checklist: Fixture Transport

Generate devnet fixtures:

```bash
rm -rf /tmp/lora-e2e
mkdir -p /tmp/lora-e2e
target/debug/udp-digest-fixtures --out-dir /tmp/lora-e2e --network devnet
wc -c /tmp/lora-e2e/*.bin
```

Expected:

```text
delta.bin 200 bytes
snapshot.bin 329 bytes
```

Delta receiver:

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output file \
  --file /tmp/lora-e2e/delta.rx.bin \
  --count 1 \
  --timeout-ms 30000 \
  --packet-idle-ms 600
```

Delta transmitter:

```bash
target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/lora-e2e/delta.bin
```

Verify:

```bash
cmp /tmp/lora-e2e/delta.bin /tmp/lora-e2e/delta.rx.bin
```

Snapshot receiver:

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output file \
  --file /tmp/lora-e2e/snapshot.rx.bin \
  --count 1 \
  --timeout-ms 70000 \
  --packet-idle-ms 600
```

Snapshot transmitter:

```bash
target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/lora-e2e/snapshot.bin \
  --inter-frame-delay-ms 2500
```

Verify:

```bash
cmp /tmp/lora-e2e/snapshot.bin /tmp/lora-e2e/snapshot.rx.bin
```

## Checklist: kaspad UDP Ingest Over LoRa

Start receiver kaspad:

```bash
rm -rf /tmp/lora-e2e-app /tmp/lora-e2e-kaspad.log
mkdir -p /tmp/lora-e2e-app

RUST_LOG='info,kaspa_udp_sidechannel=debug' \
cargo run -p kaspad --features devnet-prealloc -- \
  --devnet \
  --udp.enable \
  --udp.listen=127.0.0.1:28515 \
  --udp.require_signature=true \
  --udp.allowed_signers=4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa \
  --udp.db_migrate=true \
  --udp.retention_count=20 \
  --udp.retention_days=1 \
  --rpclisten=127.0.0.1:16110 \
  --listen=127.0.0.1:0 \
  --appdir=/tmp/lora-e2e-app \
  --reset-db --yes \
  --nologfiles \
  --disable-upnp \
  --connect=127.0.0.1:1 \
  > /tmp/lora-e2e-kaspad.log 2>&1
```

Confirm UDP bind:

```bash
rg 'udp.event=bind_ok' /tmp/lora-e2e-kaspad.log
```

Forward snapshot first, then delta, because the fixture snapshot epoch is `42`
and the delta epoch is `43`:

```bash
target/debug/lora-bridge rx --serial /dev/lora-right --output udp --udp-target 127.0.0.1:28515 --count 1 --timeout-ms 70000 --packet-idle-ms 600
target/debug/lora-bridge tx --serial /dev/lora-left --input file --file /tmp/lora-e2e/snapshot.bin --inter-frame-delay-ms 2500

target/debug/lora-bridge rx --serial /dev/lora-right --output udp --udp-target 127.0.0.1:28515 --count 1 --timeout-ms 30000 --packet-idle-ms 600
target/debug/lora-bridge tx --serial /dev/lora-left --input file --file /tmp/lora-e2e/delta.bin
```

Expected logs:

```text
udp.event=digest_ingested epoch=42 signer=0 source=7 kind=snapshot
udp.event=digest_ingested epoch=43 signer=0 source=7 kind=delta
```

Query receiver:

```bash
target/debug/udp-rpc-digests --rpc-url grpc://127.0.0.1:16110 info
target/debug/udp-rpc-digests --rpc-url grpc://127.0.0.1:16110 digests --limit 10
```

Expected:

```text
frames.digest=2
framesReceived=2
lastDigest.signatureValid=true
delta verified=true
snapshot verified=true
```

Negative checks:

```bash
# corrupt payload -> signature drop
target/debug/lora-bridge rx --serial /dev/lora-right --output udp --udp-target 127.0.0.1:28515 --count 1
target/debug/lora-bridge tx --serial /dev/lora-left --input file --file /tmp/lora-e2e/delta-corrupt.bin

# immediate duplicate resend -> duplicate drop
target/debug/lora-bridge rx --serial /dev/lora-right --output udp --udp-target 127.0.0.1:28515 --count 1
target/debug/lora-bridge tx --serial /dev/lora-left --input file --file /tmp/lora-e2e/delta.bin

# wrong network tag -> network_mismatch drop
target/debug/lora-bridge rx --serial /dev/lora-right --output udp --udp-target 127.0.0.1:28515 --count 1
target/debug/lora-bridge tx --serial /dev/lora-left --input file --file /tmp/lora-e2e/delta-wrong-network.bin
```

Expected counters:

```text
drops.signature=1
drops.duplicate=1
drops.network_mismatch=1
```

## Checklist: Live Digest Producer Over LoRa

Start producer-side kaspad:

```bash
rm -rf /tmp/live-producer-app /tmp/live-producer-kaspad.log
mkdir -p /tmp/live-producer-app

RUST_LOG='info' \
cargo run -p kaspad --features devnet-prealloc -- \
  --devnet \
  --rpclisten=127.0.0.1:16111 \
  --listen=127.0.0.1:0 \
  --appdir=/tmp/live-producer-app \
  --reset-db --yes \
  --nologfiles \
  --disable-upnp \
  --connect=127.0.0.1:1 \
  > /tmp/live-producer-kaspad.log 2>&1
```

Start receiver-side kaspad:

```bash
rm -rf /tmp/live-receiver-app /tmp/live-receiver-kaspad.log
mkdir -p /tmp/live-receiver-app

RUST_LOG='info,kaspa_udp_sidechannel=debug' \
cargo run -p kaspad --features devnet-prealloc -- \
  --devnet \
  --udp.enable \
  --udp.listen=127.0.0.1:28515 \
  --udp.require_signature=true \
  --udp.allowed_signers=4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa \
  --udp.db_migrate=true \
  --udp.retention_count=20 \
  --udp.retention_days=1 \
  --rpclisten=127.0.0.1:16110 \
  --listen=127.0.0.1:0 \
  --appdir=/tmp/live-receiver-app \
  --reset-db --yes \
  --nologfiles \
  --disable-upnp \
  --connect=127.0.0.1:1 \
  > /tmp/live-receiver-kaspad.log 2>&1
```

Start LoRa RX:

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515 \
  --count 4 \
  --timeout-ms 150000 \
  --packet-idle-ms 600
```

Start LoRa TX:

```bash
target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input udp \
  --udp-bind 127.0.0.1:39000 \
  --count 4 \
  --inter-frame-delay-ms 2500
```

Run live producer:

```bash
target/debug/udp-live-digest-producer \
  --rpc-url grpc://127.0.0.1:16111 \
  --network devnet \
  --output udp \
  --udp-target 127.0.0.1:39000 \
  --count 4 \
  --interval-ms 500 \
  --provenance-report
```

Expected receiver logs:

```text
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=snapshot
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=delta
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=delta
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=delta
```

Expected RPC:

```text
frames.digest=4
framesReceived=4
lastDigest.epoch=0
lastDigest.signatureValid=true
getUdpDigests --check-monotonic: all_signature_valid=true epoch_monotonic=true
```

## Live Producer Semantic Limits

The live producer signs valid DigestV1 records that are useful as a lab
heartbeat. It does not yet prove complete production digest semantics.

Real node/RPC-derived fields:

- pruning point hash
- sink hash as the selected-parent comparison point
- sink blue score
- virtual DAA score
- sink header blue work
- sink header UTXO commitment

Explicit placeholders/lab-derived values:

- pruning proof commitment
- kept headers MMR root is omitted
- optional `--lab-progress-counter` increments epoch, DAA score, and blue score
  so idle devnet/simnet nodes show ordered ingest

Needed before production relevance:

- RPC or node-side access to the real pruning proof commitment used by the
  digest format.
- RPC or node-side access to a virtual-state UTXO commitment if production
  semantics require virtual state rather than sink-header state.
- RPC or node-side access to a real kept headers MMR root, or a format decision
  that marks it intentionally absent.
- A signer policy that binds keys to network, epoch range, and operator role.
- Replay/window policy tuned for slow RF links and delayed fragments.

## Hardware Evidence

Successful real-hardware runs on `/dev/lora-left -> /dev/lora-right`:

- `delta.bin` 200 bytes sent as one LoRa payload and recovered byte-for-byte.
- `snapshot.bin` 329 bytes split into 234-byte and 123-byte bridge frames,
  reassembled, and recovered byte-for-byte.
- End-to-end static ingest accepted snapshot epoch `42` and delta epoch `43`.
- Negative cases produced signature, duplicate, and network mismatch drops.
- Live run delivered 1 snapshot plus 3 deltas with epochs `0..3`.
- Live `getUdpIngestInfo`: `framesReceived=4`, `signatureValid=true`.
- Live `getUdpDigests`: epochs `0..3`, all `verified=true`.
- Reliability sweep: deltas were byte-exact at every tested delay from 250 ms
  through 1500 ms; snapshots failed from 250 ms through 1250 ms and were
  byte-exact at 1500 ms. Use 2500 ms for live multi-datagram runs until a
  larger sweep proves lower pacing has enough margin.
- Reliable alpha sweep: 100 deltas and 100 snapshots were byte-exact at 250 ms
  with `--reliable-fragments`; no retries, timeouts, corrupt outputs, or
  duplicates were observed.

## Troubleshooting

- `open serial ... Not a typewriter`: the path is not a serial TTY.
- Serial open permission errors: add the operator to the correct group or fix
  udev permissions for `/dev/lora-left` and `/dev/lora-right`.
- RX timeout with no bytes: check TX/RX jumper mode, UART selector `A`, device
  alias direction, and baud `9600`.
- RX timeout with pending fragments: increase `--inter-frame-delay-ms` on TX
  and `--timeout-ms` on RX; inspect the pending fragment summary in the error.
- First fragment arrives but second does not: use `--inter-frame-delay-ms 2500`
  for fragmented snapshots or live runs.
- `network_mismatch`: regenerate fixtures/live digests with the same
  `--network` as the receiver kaspad.
- `digest_drop reason=signature`: receiver `--udp.allowed_signers` does not
  include the producer signer, or the datagram was corrupted.
- Fixture snapshot dropped after delta: send snapshot first; the fixture
  snapshot epoch is lower than the fixture delta epoch.
- `getUdpDigests` empty but `lora-bridge rx` recovered bytes: confirm receiver
  kaspad has `--udp.enable`, `--udp.db_migrate=true`, and the RX `--udp-target`
  matches `--udp.listen`.

## Productionizing Live Digests

The next real milestone is not more transport plumbing; it is making the digest
content authoritative.

Needed real fields:

- pruning proof commitment
- UTXO MuHash
- kept headers MMR root or an explicit absence rule
- exact virtual selected parent semantics for digest signing
- blue work and score at the advertised point

Signer/key management:

- separate lab keys from production keys
- per-network allowed signer sets
- key rotation and revocation plan
- signer identity and epoch-range binding
- offline signing or hardened signer process for any non-lab deployment

Replay/window policy over slow RF:

- duplicate windows long enough for slow retransmits
- stale epoch behavior that tolerates RF delay without accepting replay spam
- sequence handling across bridge restarts
- clear metrics for duplicate, stale, fragment timeout, and signature drops

Compression opportunities:

- reduce repeated snapshot fields for low-rate RF
- dictionary or delta coding for repeated hashes
- compact bridge envelope for RF-only links
- optional batching if delay budget allows

Fragmentation design:

- Current fragmentation is bridge-local and intentionally outside `KUDP`.
- Retry and fragmentation should remain bridge-local for LoRa alpha. Making
  them core `KUDP` semantics would leak one slow RF transport into the digest
  wire format.
- LoRa should remain a sidecar bridge unless multiple RF adapters need the same
  scheduler, retry, and observability layer. A first-class transport adapter
  would need streaming input, flow control, replay windows, and a clearer
  operator lifecycle.

Throughput expectations and risks:

- 234-byte payload ceiling is the current practical packet size.
- 200-byte deltas fit; 297-329 byte snapshots require two fragments.
- Inter-frame delay dominates throughput.
- Missing fragments are expected on aggressive pacing.
- ACK/retry can recover missing fragments when the reverse link works. FEC
  becomes worth adding only when ACK turnaround or bidirectional RF is the
  limiting factor.
- No encryption means RF observers can read the digest payload.
- The minimum production-relevant digest schema still needs real pruning proof
  commitment, UTXO MuHash, kept headers commitment or explicit absence rule,
  signer policy, and replay/window policy.
