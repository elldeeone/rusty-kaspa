# LoRa Bridge MVP

`lora-bridge` carries existing UDP side-channel `KUDP` datagrams over
Waveshare SX126X UART LoRa modules. It does not change consensus behavior and
does not change the `KUDP` wire format.

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

Forwarding to `kaspad` is a later integration milestone. The MVP acceptance
test is byte equality between the generated input files and recovered output
files.

The follow-up devnet lab that forwards recovered LoRa datagrams into kaspad's
existing UDP ingest path is documented in
[`udp/docs/lora-end-to-end-lab.md`](../../docs/lora-end-to-end-lab.md).
