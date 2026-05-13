# LoRa KUDP Reliability Report

Run date: 2026-05-13

This report captures a repeatable byte-equality reliability sweep for the
LoRa KUDP bridge over the lab Waveshare SX126X UART pair.

## Harness

The sweep was run with:

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

The ideal run is 50 delta and 50 snapshot samples per delay. The hardware sweep
below used 5 delta and 5 snapshot samples per delay because a full 50/50 run
across six delays would tie up the RF lab for a much longer session. Treat this
as an initial operating-envelope run, not a production reliability claim.

## Metrics

The harness records these per sample:

- sent fixture kind, delay, sample number, input bytes, and recovered bytes
- recovered count and exact byte match
- RX timeout, inferred fragment loss, corrupt recovery, and duplicate marker
- wall-clock latency from RX start through TX/RX completion

The summarized table reports sent, recovered, exact matches, timeouts,
fragment loss, corrupt output, duplicates, min/avg/max latency, datagrams per
minute, and bytes per minute.

## Results

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

- 200-byte delta datagrams were byte-exact at every tested delay.
- 329-byte snapshots require two LoRa sends; they failed from 250 ms through
  1250 ms because the second fragment was not recovered before timeout.
- 1500 ms is the measured minimum reliable inter-frame delay for the current
  329-byte snapshot fixture in this lab run.
- The conservative live-ingest delay remains 2500 ms until a larger sweep
  proves that 1500 ms has enough margin across longer sessions.
- No corrupt byte outputs or duplicate outputs were observed. Failed snapshots
  were clean fragment-loss timeouts.

## Kaspad Ingest Mode

This sweep intentionally exercised standalone byte equality, because it
isolates the RF adapter from kaspad ingest policy. The kaspad-ingest mode uses
the same recovered bytes and is documented in `udp/docs/lora-prototype.md`.
Previous hardware evidence in that guide shows the recovered snapshot and delta
fixtures being accepted by kaspad UDP ingest and exposed through RPC.

## Operating Settings

Recommended current lab settings:

- Delta-only fixture transport: `--inter-frame-delay-ms 250` is sufficient for
  this 200-byte fixture, though using the default is simpler.
- Fragmented snapshots: use at least `--inter-frame-delay-ms 1500`.
- Live multi-datagram ingest: keep using `--inter-frame-delay-ms 2500` until a
  longer reliability sweep is completed.
- RX timeout for fragmented snapshots should be at least `30000 ms`; the report
  sweep used `12000 ms` to keep failed low-delay samples bounded.

## Remaining Risks

- This is not a production RF reliability layer.
- A single lost fragment drops the whole oversized datagram.
- There is no FEC, retry, compression, encryption, or RF-level replay policy.
- More samples are needed before changing the live lab default below 2500 ms.
