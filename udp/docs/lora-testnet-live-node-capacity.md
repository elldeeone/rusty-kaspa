# LoRa Testnet Live Node Capacity Boundary

Date: 2026-05-15

This report diagnoses the real `testnet-10` LoRa capacity boundary seen after
the successful 50-datagram live-node run. It uses the same synced producer
kaspad at `grpc://127.0.0.1:16221` and the same two SX126X HAT path:
`/dev/lora-left` TX to `/dev/lora-right` RX.

This is a lab capacity report, not a production-readiness or mainnet claim.

## Hardening Added

- `udp/tools/lora_testnet_live_node_lab.sh` now exposes the pacing and reliable
  bridge knobs needed for controlled capacity runs:
  `--inter-frame-delay-ms`, `--ack-timeout-ms`, `--retry-count`,
  `--expected-datagram-ms`, and `--session-id`.
- The testnet lab report now includes the expected datagram drain time, session
  id, failure markers, and recent RX recoveries.
- `lora-bridge` TX/RX logs now include datagram id, session id, fragment index,
  and elapsed time on send/recovery/retry lines. This keeps the normal log
  bounded while making retry-exhaustion reports attributable to a concrete
  bridge-local datagram and fragment.

## Run Matrix

All runs used live signed `DigestV1` testnet traffic with a snapshot first and
snapshots every 10 datagrams. Receiver kaspad was a fresh advisory-ingest node,
so divergence from producer state is expected; ingest, signature validation, and
producer-log comparison are the relevant gates.

| Run | Settings | Result | RX recovered | RX fragments | Receiver frames | Failure point |
| --- | --- | --- | ---: | ---: | ---: | --- |
| 50 default final | 2500 ms inter-frame, 6000 ms ACK, 8 retries, 6500 ms expected datagram | pass | 50/50 | 55 | 50 | none |
| 60 default | 2500 ms inter-frame, 6000 ms ACK, 8 retries, 6500 ms expected datagram | pass | 60/60 | 66 | 60 | none |
| 75 default | 2500 ms inter-frame, 6000 ms ACK, 8 retries, 6500 ms expected datagram | fail | 69/75 | 76 | 69 | TX retry exhausted for datagram 70 fragment 0; RX timed out |
| 75 conservative | 4000 ms inter-frame, 10000 ms ACK, 12 retries, 9000 ms expected datagram | fail | 63/75 | 70 | 63 | TX retry exhausted for datagram 64 fragment 0; RX timed out |
| 90 default | 2500 ms inter-frame, 6000 ms ACK, 8 retries, 6500 ms expected datagram | fail | 56/90 | 62 | 56 | TX retry exhausted for datagram 57 fragment 0; RX timed out |

Report artifacts were written under `/home/luke/lora-capacity-boundary/`:

- `default-count50-final.md`
- `default-count60.md`
- `default-count75.md`
- `conservative-count75.md`
- `default-count90.md`

## Evidence

The passing 50 and 60 runs had zero retries, duplicate fragments, corrupt
frames, receive timeouts, and reassembly failures. The final 50 run also proved
the added instrumentation path:

```text
bridge_summary role=rx datagrams_recovered=50 fragments_received=55 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=55
bridge_summary role=tx datagrams_sent=50 fragments_sent=55 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0
```

The 75 and 90 failures had the same shape:

- receiver kaspad continued accepting verified digests until the bridge stopped;
- receiver ingest had zero signature failures;
- lora-bridge RX had no corrupt frames and no reassembly failures;
- lora-bridge TX exhausted ACK retries on a single reliable fragment;
- lora-bridge RX timed out waiting for the next LoRa packet, with no pending
  partial datagram reported.

The 75 default failure:

```text
bridge_summary role=rx datagrams_recovered=69 fragments_received=76 corrupt_frames=0 receive_timeouts=1 reassembly_failures=0 acks_sent=76
Error: retry exhausted for datagram_id=70 frag_ix=0
```

The 90 default failure:

```text
bridge_summary role=rx datagrams_recovered=56 fragments_received=62 corrupt_frames=0 receive_timeouts=1 reassembly_failures=0 acks_sent=62
Error: retry exhausted for datagram_id=57 frag_ix=0
```

The conservative 75 run did not improve the boundary. It failed earlier, at
datagram 64, despite longer inter-frame delay, longer ACK timeout, and a higher
retry count.

## Root Cause Classification

Classification: RF loss or ACK loss, with ACK-loss symptoms at the bridge.

Evidence:

- Producer burst behavior is not the primary cause: lora-bridge TX first
  buffers the requested UDP datagrams, then transmits them over LoRa at the
  configured inter-frame delay.
- Receiver UDP ingest/backpressure is not the primary cause: kaspad accepted all
  recovered frames, reported zero signature failures, and had no UDP malformed,
  queue-full, or rejection evidence at the failure.
- Reassembly timeout is not the primary cause: RX reported no pending fragment
  summary and no reassembly failures; the failure happened on a single-fragment
  datagram waiting for ACK.
- Retry policy alone is not sufficient: increasing ACK timeout to 10000 ms and
  retry count to 12 did not rescue the 75-datagram run.
- Serial write/read exceptions were not observed; the bridge failures were
  normal timeout/retry-exhaustion paths.

The evidence does not distinguish whether the missing event is the outbound data
frame not reaching RX or the RX ACK not reaching TX. Because the receiver log
does not show datagram 64/70/57 recovery before TX exhaustion, the observed
symptom is more consistent with forward RF frame loss or RX-side radio receive
starvation than pure ACK-only loss. The class remains RF/ACK loss rather than a
kaspad ingest or producer pacing defect.

## Recommendation

For ACK mode, keep 50 mixed live datagrams as the reproducible passing operating
point for this hardware/configuration. A 60-datagram run passed once, but the
75- and 90-datagram runs fail inconsistently between datagrams 57 and 70, and a
more conservative pacing/retry run did not improve the boundary.

Do not claim a 90+ datagram ACK-mode live LoRa operating point. The next useful
hardware-focused step for ACK mode is radio-level diagnosis: RSSI/SNR if
available from the module, antenna placement/power/channel checks, and a loop
that can distinguish data-frame loss from ACK loss. FEC or deeper protocol work
should wait until the RF/ACK direction is isolated.

## Robust Reliability Follow-Up

A bridge-local redundant reliability mode was added after the ACK-mode boundary
above. It keeps the same `KLR2` reliable frame envelope and byte-exact receiver
reassembly, but TX sends each frame a bounded number of times and does not wait
for per-frame ACKs. RX suppresses duplicate fragments and late duplicate
datagrams before UDP forwarding.

The protocol rationale is documented in
[`lora-bridge-reliability-protocol.md`](lora-bridge-reliability-protocol.md).

Follow-up runs used the same synced `testnet-10` producer node, snapshot first
and every 10 datagrams, `2500 ms` inter-frame delay, `redundant-copies=2`, and
`13000 ms` expected datagram drain.

| Run | Mode | Result | RX recovered | RX fragments | Duplicate fragments | Receiver frames | Failure point |
| --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| 50 redundant x2 | redundant | pass | 50/50 | 109 | 54 | 50 | none |
| 75 redundant x2 | redundant | pass | 75/75 | 165 | 82 | 75 | none |
| 90 redundant x2 | redundant | pass | 90/90 | 197 | 98 | 90 | none |
| 100 redundant x2 | redundant | pass | 100/100 | 219 | 109 | 100 | none |

The 100-run bridge summaries:

```text
bridge_summary role=rx datagrams_recovered=100 fragments_received=219 retries=0 duplicate_fragments=109 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=219
bridge_summary role=tx datagrams_sent=100 fragments_sent=220 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0
```

This shifts the current measured live mixed-digest operating point beyond the
old ACK-mode 50/60 safe range. The evidence supports ACK-dependent or
single-copy RF loss as the limiting class for ACK mode: one redundant copy was
lost in the 100 run (`220` TX frame copies, `219` RX frames received), but all
100 datagrams recovered and all 100 were visible through receiver UDP ingest
with zero signature failures.

Recommended current setting for longer live testnet lab runs:

```bash
--reliability-mode redundant \
--redundant-copies 2 \
--inter-frame-delay-ms 2500 \
--expected-datagram-ms 13000
```

This remains lab-grade. It costs duplicate airtime, does not prove production
RF reliability, and still needs longer soak runs plus RF diagnostics before
claiming a stable production envelope.

## Reproduction Commands

Default 60:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --external-producer-rpc grpc://127.0.0.1:16221 \
  --workdir /home/luke/lora-capacity-boundary/default-count60 \
  --duration-seconds 600 \
  --count 60 \
  --interval-ms 5000 \
  --snapshot-every 10 \
  --inter-frame-delay-ms 2500 \
  --ack-timeout-ms 6000 \
  --retry-count 8 \
  --expected-datagram-ms 6500 \
  --session-id 601 \
  --report /home/luke/lora-capacity-boundary/default-count60.md
```

Default 90:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --external-producer-rpc grpc://127.0.0.1:16221 \
  --workdir /home/luke/lora-capacity-boundary/default-count90 \
  --duration-seconds 600 \
  --count 90 \
  --interval-ms 5000 \
  --snapshot-every 10 \
  --inter-frame-delay-ms 2500 \
  --ack-timeout-ms 6000 \
  --retry-count 8 \
  --expected-datagram-ms 6500 \
  --session-id 901 \
  --report /home/luke/lora-capacity-boundary/default-count90.md
```
