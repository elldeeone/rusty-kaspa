# LoRa KUDP Reliability Report

Run date: 2026-05-13

This report captures a repeatable byte-equality reliability sweep for the
LoRa KUDP bridge over the lab Waveshare SX126X UART pair.

## Harness

The original fixed-delay sweep was run with:

```bash
./udp/tools/lora_reliability_harness.sh \
  --delays 250,500,750,1000,1250,1500 \
  --delta-count 5 \
  --snapshot-count 5 \
  --rx-timeout-ms 12000 \
  --report /tmp/lora-reliability-2026-05-13.md
```

Defaults match the lab setup:

- TX serial: `/dev/lora-left`
- RX serial: `/dev/lora-right`
- UART baud: `9600`
- Packet idle boundary: `600 ms`
- Network tag source: `devnet` fixtures

The harness builds on the standalone byte-equality mode: it generates signed
DigestV1 fixtures, transmits each fixture over LoRa, compares recovered bytes
with `cmp`, and writes both Markdown and CSV reports.

After adding bridge-local ACKs for fragmented datagrams, the alpha reliability
run was:

```bash
./udp/tools/lora_reliability_harness.sh \
  --modes reliable \
  --delays 250 \
  --delta-count 100 \
  --snapshot-count 100 \
  --rx-timeout-ms 30000 \
  --ack-timeout-ms 3000 \
  --retry-count 4 \
  --report /tmp/lora-reliable-100-2026-05-13.md
```

The harness also supports `--modes best-effort,reliable` to compare the old
fixed-delay path against the `KLR2` reliable-fragment envelope.

## Metrics

The harness records these per sample:

- mode, fixture kind, delay, sample number, input bytes, and recovered bytes
- recovered count and exact byte match
- RX timeout, inferred fragment loss, corrupt recovery, and duplicate marker
- retry count inferred from bridge TX logs
- wall-clock latency from RX start through TX/RX completion

The summarized table reports sent, recovered, exact matches, timeouts,
fragment loss, corrupt output, duplicates, retries, min/avg/max latency,
datagrams per minute, and bytes per minute.

## Alpha Reliable Results

| Mode | Kind | Delay ms | Sent | Recovered | Exact | Timeouts | Fragment loss | Corrupt | Duplicate | Retries | Latency min/avg/max ms | Datagrams/min | Bytes/min |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| reliable | delta | 250 | 100 | 100 | 100 | 0 | 0 | 0 | 0 | 0 | 2518/2519/2522 | 23.81 | 4763 |
| reliable | snapshot | 250 | 100 | 100 | 100 | 0 | 0 | 0 | 0 | 0 | 5961/5962/5965 | 10.06 | 3311 |

The CSV audit for `/tmp/lora-reliable-100-2026-05-13.csv` had 200 rows,
zero failed rows, zero retries, zero timeouts, zero corrupt outputs, and zero
duplicate markers.

## Best-Effort Baseline

| Kind | Delay ms | Sent | Recovered | Exact | Timeouts | Fragment loss | Corrupt | Duplicate | Latency min/avg/max ms | Datagrams/min | Bytes/min |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| delta | 250 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 2519/2519/2519 | 23.82 | 4764 |
| snapshot | 250 | 5 | 0 | 0 | 5 | 5 | 0 | 0 | 12154/12154/12155 | 4.94 | 1624 |
| delta | 500 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 2519/2519/2520 | 23.82 | 4763 |
| snapshot | 500 | 5 | 0 | 0 | 5 | 5 | 0 | 0 | 12153/12154/12155 | 4.94 | 1624 |
| delta | 750 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 2518/2519/2521 | 23.82 | 4763 |
| snapshot | 750 | 5 | 0 | 0 | 5 | 5 | 0 | 0 | 12153/12155/12159 | 4.94 | 1624 |
| delta | 1000 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 2519/2519/2521 | 23.81 | 4762 |
| snapshot | 1000 | 5 | 0 | 0 | 5 | 5 | 0 | 0 | 12153/12154/12156 | 4.94 | 1624 |
| delta | 1250 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 2519/2519/2520 | 23.82 | 4763 |
| snapshot | 1250 | 5 | 0 | 0 | 5 | 5 | 0 | 0 | 12271/12272/12273 | 4.89 | 1608 |
| delta | 1500 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 2519/2519/2520 | 23.82 | 4763 |
| snapshot | 1500 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 3761/3763/3766 | 15.94 | 5245 |

## Interpretation

- 200-byte delta datagrams were byte-exact at every tested delay in the
  best-effort baseline.
- Best-effort 329-byte snapshots require two LoRa sends; they failed from
  250 ms through 1250 ms because the second fragment was not recovered before
  timeout. Best-effort snapshots were byte-exact at 1500 ms in the small sweep.
- Reliable `KLR2` snapshots were 100/100 byte-exact at 250 ms. This is a lower
  operating point than the fixed-delay best-effort bridge.
- The 100-snapshot reliable run did not need retries, because per-fragment ACKs
  paced the second fragment and confirmed delivery. Retry behavior is still
  implemented and covered by unit tests for retry exhaustion/ACK parsing paths,
  but this hardware run did not encounter an ACK timeout.
- No corrupt byte outputs or duplicate outputs were observed. Failed snapshots
  were clean fragment-loss timeouts.

## Kaspad Ingest Mode

The reliability sweep intentionally exercises standalone byte equality, because
it isolates the RF adapter from kaspad ingest policy.

The live mixed ingest check used reliable bridge mode with one live snapshot
and three live deltas:

```text
lora-bridge tx: datagrams_sent=4 fragments_sent=5 retries=0
lora-bridge rx: datagrams_recovered=4 fragments_received=2 acks_sent=2
getUdpIngestInfo: framesReceived=4 bytesTotal=897 signatureFailures=0
getUdpDigests: epochs 0..3 verified=true
```

An earlier 10-datagram attempt at 250 ms recovered only the first three live
datagrams before RX stalled, so live multi-datagram runs still use 2500 ms until
more streaming TX work is done.

## Operating Settings

Recommended current lab settings:

- Delta-only fixture transport: `--inter-frame-delay-ms 250` is sufficient for
  this 200-byte fixture, though using the default is simpler.
- Fragmented snapshots with `--reliable-fragments`: 250 ms passed the 100-sample
  byte-equality run.
- Fragmented snapshots without `--reliable-fragments`: use at least 1500 ms.
- Live multi-datagram ingest: keep using `--inter-frame-delay-ms 2500` because
  the bridge TX currently buffers all requested UDP input before serial send,
  and the 250 ms live mixed attempt did not complete cleanly.
- RX timeout for fragmented snapshots should be at least `30000 ms`; the report
  sweep used `12000 ms` to keep failed low-delay samples bounded.

## Remaining Risks

- This is not a production RF reliability layer.
- A single lost fragment drops the whole oversized datagram.
- Retry exists for bridge-local reliable fragments, but there is no FEC,
  compression, encryption, or RF-level replay policy.
- UDP-input TX is batch-oriented today: it reads `--count` datagrams before
  serial transmission. A production sidecar should stream UDP input into the RF
  scheduler instead.
- More live mixed samples are needed before changing the live lab default below
  2500 ms.
