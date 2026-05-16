# LoRa Testnet Robust Soak Report

Date: 2026-05-16

Result: complete after a targeted harness/bridge fix.

This is a real `testnet-10` LoRa soak over `/dev/lora-left` TX to
`/dev/lora-right` RX using signed live `DigestV1` traffic, receiver kaspad UDP
ingest, and `lora-bridge` redundant reliability mode. It is lab evidence only:
no mainnet, consensus, block relay, tx relay, FEC, encryption, or production
signer claim is made.

## Soak Plan

- Target: minimum 30 minutes; preferred 2 hours was not practical for this run.
- Producer: synced public testnet-10 wRPC node
  `wss://electron-10.kaspa.stream/kaspa/testnet-10/wrpc/borsh`.
- Receiver: fresh advisory-ingest kaspad on `testnet-10`, UDP ingest at
  `127.0.0.1:28620`.
- Reliability mode: `redundant`, `--redundant-copies 2`.
- Pacing: live producer `--interval-ms 5000`, bridge
  `--inter-frame-delay-ms 2500`.
- Snapshot cadence: snapshot first and every 10 datagrams.
- Target count after fix: 305 datagrams, enough to put the measured soak and
  RF recovery clocks past 30 minutes at the measured streaming throughput.

## Batch-Mode Failure

The first 165-datagram robust run preserved the prior batch-oriented TX path:
`lora-bridge tx --input udp --count 165` buffered all live UDP datagrams before
starting serial transmission.

Artifacts:

- Workdir: `/home/luke/lora-robust-soak/redundant2-count165-20260516`
- RX summary: `datagrams_recovered=165`, `fragments_received=363`,
  `duplicate_fragments=181`, `receive_timeouts=0`, `reassembly_failures=0`
- TX summary: `datagrams_sent=165`, `fragments_sent=364`, `retries=0`
- Receiver log: 165 `udp.event=digest_drop reason=skew` entries

Classification: ingest freshness failure caused by bridge batching, not RF loss
and not signature failure. The first recovered digest arrived about 991 seconds
after the RX process started, and receiver digest skew policy rejects digests
older than 600 seconds. Transport recovered every datagram, but receiver digest
storage correctly rejected stale signed digests.

Fix: add explicit UDP streaming mode to `lora-bridge` and use it from the
testnet lab harness. In streaming mode TX sends each UDP datagram as it arrives
instead of waiting for all `--count` datagrams first.

## Streaming Soak

Command:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --external-producer-rpc wss://electron-10.kaspa.stream/kaspa/testnet-10/wrpc/borsh \
  --workdir /home/luke/lora-robust-soak/redundant2-stream-count305-20260516 \
  --duration-seconds 1830 \
  --count 305 \
  --interval-ms 5000 \
  --snapshot-every 10 \
  --reliability-mode redundant \
  --redundant-copies 2 \
  --inter-frame-delay-ms 2500 \
  --expected-datagram-ms 6500 \
  --retry-count 8 \
  --ack-timeout-ms 6000 \
  --session-id 3004 \
  --report /home/luke/lora-robust-soak/redundant2-stream-count305-20260516.md
```

Artifacts:

- Report: `/home/luke/lora-robust-soak/redundant2-stream-count305-20260516.md`
- Workdir: `/home/luke/lora-robust-soak/redundant2-stream-count305-20260516`
- Poll CSV:
  `/home/luke/lora-robust-soak/redundant2-stream-count305-20260516/rpc-poll.csv`

Timing:

- First poll sample: `2026-05-16T09:46:56Z`
- Generated report: `2026-05-16T10:17:38Z`
- Measured soak runtime: `1831` seconds
- RX process elapsed time to last recovered datagram: `1831.574` seconds
- Datagram 301 crossed the 30-minute RF recovery mark at `1809.162` seconds.
- The measured command/report window and the bridge RF recovery clock both
  exceeded the 30-minute target.

The harness now computes measured runtime in milliseconds with Python. This
replaced an earlier `date +%s%3N` implementation that produced invalid runtime
values on this host.

## Results

Bridge recovery:

```text
bridge_summary role=rx datagrams_recovered=305 fragments_received=671 retries=0 duplicate_fragments=335 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=671 datagrams_per_minute=9.99 bytes_per_minute=2097
bridge_summary role=tx datagrams_sent=305 fragments_sent=672 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 datagrams_per_minute=9.99 bytes_per_minute=2097
```

Receiver ingest:

- `framesReceived=305`
- `bytesTotal=64007`
- `signatureFailures=0`
- all receiver drop counters remained `0`
- `sourceCount=1`
- latest digest `signatureValid=true`
- final `skewSeconds=0`
- poll samples: 61
- poll errors: 0
- max observed latest digest age: 5431 ms

Digest verification:

```text
udp_digest_check count=10 all_signature_valid=true epoch_monotonic=true daa_score_monotonic=true virtual_blue_score_monotonic=true sources={7} signers={0}
udp_digest_compare snapshot_match=true compared_fields=8 mismatches=[]
```

Native divergence/agreement:

- Receiver-native comparison reported `agreement=false` and
  `divergence_detected=true`.
- This is expected for this run because the receiver was a fresh unsynced
  `testnet-10` node at genesis-local state while the producer was synced to the
  public network.
- The divergence result verifies that receiver-native comparison was active; it
  does not invalidate transport recovery, signature verification, or advisory
  ingest storage.

## Recommended Settings

For longer live testnet LoRa digest lab runs on this hardware, use streaming UDP
TX plus redundant x2:

```bash
--reliability-mode redundant \
--redundant-copies 2 \
--stream-udp \
--inter-frame-delay-ms 2500 \
--interval-ms 5000 \
--expected-datagram-ms 6500
```

The key change from the earlier 100/100 redundant x2 run is `--stream-udp`.
Without it, long runs can produce RF-perfect but ingest-invalid stale digests.

## Remaining Risks

- This is still lab-grade hardware evidence, not production RF reliability.
- Redundant x2 spends duplicate airtime and does not identify which direction
  loses the occasional redundant copy.
- The receiver in this run was intentionally unsynced, so agreement was not
  expected. A synced receiver run is still needed for sustained native
  agreement evidence.
- The public wRPC producer endpoint was used because the local persistent
  producer appdir was stale and would have required a long resync.
