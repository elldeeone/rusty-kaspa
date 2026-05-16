# LoRa Bridge MVP

`lora-bridge` carries existing UDP side-channel `KUDP` datagrams over
Waveshare SX126X UART LoRa modules. It does not change consensus behavior and
does not change the `KUDP` wire format.

Start with the consolidated prototype guide at
[`udp/docs/lora-prototype.md`](../../docs/lora-prototype.md) for the complete
Phase 0/Phase 1 lab checklist and current limits. The repeatable reliability
sweep is in
[`udp/docs/lora-reliability-report.md`](../../docs/lora-reliability-report.md),
and the harness lives at [`udp/tools/lora_reliability_harness.sh`](../lora_reliability_harness.sh).

## Hardware Setup

Lab assumptions:

- Two Waveshare SX126X LoRa HATs over USB CP2102 UART.
- Stable device aliases: `/dev/lora-left` and `/dev/lora-right`.
- UART baud: `9600`.
- Usable LoRa application payload per send: `234` bytes.
- Waveshare fixed-send prefix: `00 00 41 00 00 41`.
- Receiver output: 3 source bytes, then payload, then one status/RSSI byte.

Normal transmit/receive jumper mode:

```text
UART selector: A
M0: shorted
M1: shorted
```

Configuration jumper mode:

```text
UART selector: A
M0: shorted
M1: open
```

Known module config response:

```text
c1 00 09 00 00 00 62 20 41 c3 00 00
```

Read it in config mode with:

```bash
cargo run -p lora-bridge -- config-read --serial /dev/lora-left
cargo run -p lora-bridge -- config-read --serial /dev/lora-right
```

## Generate KUDP Vectors

```bash
mkdir -p /tmp/kudp-vectors
cargo run -p kaspa-udp-sidechannel --example dump_vectors -- /tmp/kudp-vectors
wc -c /tmp/kudp-vectors/delta.bin /tmp/kudp-vectors/snapshot.bin
```

Expected sizes for the MVP vectors are:

```text
200 /tmp/kudp-vectors/delta.bin
329 /tmp/kudp-vectors/snapshot.bin
```

## MVP 1: Single-Packet Delta

Receiver:

```bash
cargo run -p lora-bridge -- rx \
  --serial /dev/lora-right \
  --output file \
  --file /tmp/kudp-vectors/delta.rx.bin
```

Transmitter:

```bash
cargo run -p lora-bridge -- tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/kudp-vectors/delta.bin
```

Verify:

```bash
cmp /tmp/kudp-vectors/delta.bin /tmp/kudp-vectors/delta.rx.bin
```

## MVP 2: Fragmented Snapshot

The bridge fragments oversized datagrams locally. Each LoRa application payload
stays at or below 234 bytes, and the reassembled output is the original `KUDP`
datagram byte-for-byte.

The default `tx` inter-frame delay is 1500 ms. In lab testing, a shorter
250 ms delay allowed the first snapshot fragment through but the radio dropped
the second fragment.

For fragmented datagrams, alpha mode can use bridge-local ACKs:

```bash
cargo run -p lora-bridge -- rx \
  --serial /dev/lora-right \
  --output file \
  --file /tmp/kudp-vectors/snapshot.rx.bin \
  --session-id 1

cargo run -p lora-bridge -- tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/kudp-vectors/snapshot.bin \
  --reliable-fragments \
  --retry-count 4 \
  --ack-timeout-ms 3000 \
  --session-id 1 \
  --inter-frame-delay-ms 250
```

The reliable envelope is bridge-local (`KLR2` fragments plus `KLA1` ACKs). It
does not alter the recovered `KUDP` datagram bytes and does not change
consensus or core UDP side-channel semantics. `--group-id` is accepted as an
alias-style override for `--session-id` when several bridge groups share the
same RF channel.

For live testnet runs where ACK loss appears to limit sustained capacity, the
same reliable envelope can be run in redundant mode:

```bash
cargo run -p lora-bridge -- tx \
  --serial /dev/lora-left \
  --input udp \
  --udp-bind 127.0.0.1:39000 \
  --stream-udp \
  --reliable-fragments \
  --reliability-mode redundant \
  --redundant-copies 2 \
  --inter-frame-delay-ms 2500
```

Redundant mode sends each `KLR2` frame a bounded number of times and does not
wait for ACKs. RX still suppresses duplicate fragments and late duplicate
datagrams before forwarding. This trades airtime for avoiding ACK-dependent
retry exhaustion; it is bridge-local alpha behavior, not a consensus or `KUDP`
wire-format change. The protocol details are documented in
[`udp/docs/lora-bridge-reliability-protocol.md`](../../docs/lora-bridge-reliability-protocol.md).
For long live UDP producer runs, use `--stream-udp` so signed digests do not age
in a pre-transmit bridge batch before reaching receiver ingest.

Receiver:

```bash
cargo run -p lora-bridge -- rx \
  --serial /dev/lora-right \
  --output file \
  --file /tmp/kudp-vectors/snapshot.rx.bin
```

Transmitter:

```bash
cargo run -p lora-bridge -- tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/kudp-vectors/snapshot.bin
```

Verify:

```bash
cmp /tmp/kudp-vectors/snapshot.bin /tmp/kudp-vectors/snapshot.rx.bin
```

## UDP Socket Mode

TX can read one local UDP datagram and send it over LoRa:

```bash
cargo run -p lora-bridge -- tx \
  --serial /dev/lora-left \
  --input udp \
  --udp-bind 127.0.0.1:39000
```

RX can forward each recovered datagram to an existing UDP ingest socket:

```bash
cargo run -p lora-bridge -- rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515
```

The standalone bridge acceptance test is byte equality between the generated
input files and recovered output files. The follow-up labs then forward
recovered datagrams into kaspad UDP ingest.

For repeated byte-equality runs over the hardware pair:

```bash
./udp/tools/lora_reliability_harness.sh \
  --modes best-effort,reliable \
  --delays 250,500,750,1000,1250,1500 \
  --delta-count 50 \
  --snapshot-count 50 \
  --report /tmp/lora-reliability-report.md
```

For the unattended live ingest soak harness:

```bash
./udp/tools/lora_live_soak_lab.sh \
  --duration-seconds 1800 \
  --inter-frame-delay-ms 2500 \
  --provenance-report \
  --report /tmp/lora-live-soak-report.md
```

The follow-up devnet lab that forwards recovered LoRa datagrams into kaspad's
existing UDP ingest path is documented in
[`udp/docs/lora-end-to-end-lab.md`](../../docs/lora-end-to-end-lab.md).

The live producer lab that feeds current devnet/simnet RPC state into the same
LoRa bridge path is documented in
[`udp/docs/lora-live-digest-producer-lab.md`](../../docs/lora-live-digest-producer-lab.md).
The completed live soak result is summarized in
[`udp/docs/lora-live-soak-report.md`](../../docs/lora-live-soak-report.md).
The real testnet robust/redundant streaming soak is summarized in
[`udp/docs/lora-testnet-robust-soak-report.md`](../../docs/lora-testnet-robust-soak-report.md).
Digest field authenticity is tracked in
[`udp/docs/lora-digest-authenticity.md`](../../docs/lora-digest-authenticity.md).
Two-node agreement/divergence behavior is documented in
[`udp/docs/lora-two-node-divergence-lab.md`](../../docs/lora-two-node-divergence-lab.md).
