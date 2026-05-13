# LoRa End-to-End UDP Ingest Lab

This lab proves the standalone `lora-bridge` can deliver existing signed
`DigestV1` `KUDP` datagrams over real Waveshare SX126X UART LoRa modules and
forward the recovered bytes into kaspad's existing UDP side-channel ingest.

The lab does not use mainnet, does not require a synced node, does not change
consensus behavior, and does not exercise `BlockV1` or transaction relay.

## Hardware

- Left radio: `/dev/lora-left`
- Right radio: `/dev/lora-right`
- UART baud: `9600`
- TX/RX mode: UART selector `A`, `M0` shorted, `M1` shorted
- Config mode: UART selector `A`, `M0` shorted, `M1` open
- Waveshare fixed-send prefix: `00 00 41 00 00 41`
- Receiver framing: 3 source bytes before payload, 1 status/RSSI byte after
  payload
- Snapshot fragment delay used for reliable lab delivery: `2500 ms`

Known config response:

```text
c1 00 09 00 00 00 62 20 41 c3 00 00
```

## Build

```bash
cargo build -p udp-generator --bin udp-digest-fixtures --bin udp-rpc-digests --bin udp-generator
cargo build -p lora-bridge
cargo test -p lora-bridge
```

Observed result:

```text
cargo build -p udp-generator --bin udp-digest-fixtures --bin udp-rpc-digests --bin udp-generator
Finished dev profile

cargo test -p lora-bridge
5 passed; 0 failed
```

## Generate Devnet Fixtures

```bash
rm -rf /tmp/lora-e2e
mkdir -p /tmp/lora-e2e
target/debug/udp-digest-fixtures --out-dir /tmp/lora-e2e --network devnet
wc -c /tmp/lora-e2e/*.bin
```

Observed result:

```text
/tmp/lora-e2e/snapshot.bin 329 bytes
/tmp/lora-e2e/delta.bin 200 bytes
/tmp/lora-e2e/delta-corrupt.bin 200 bytes
/tmp/lora-e2e/delta-wrong-network.bin 200 bytes
wrote DigestV1 fixtures to /tmp/lora-e2e network=devnet network_tag=0x03 signer_hex=4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa
```

The generated snapshot uses epoch `42`; the generated delta uses epoch `43`.
Send the snapshot first when testing a clean kaspad instance so both records
are accepted.

## Start Local Devnet kaspad

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
  --appdir=/tmp/lora-e2e-app \
  --reset-db --yes \
  --nologfiles \
  --disable-upnp \
  --connect=127.0.0.1:1 \
  > /tmp/lora-e2e-kaspad.log 2>&1
```

Confirm bind:

```bash
rg 'udp.event=bind_ok' /tmp/lora-e2e-kaspad.log
```

Observed result:

```text
udp.event=bind_ok kind=udp addr=127.0.0.1:28515
```

Initial RPC check:

```bash
target/debug/udp-rpc-digests --rpc-url grpc://127.0.0.1:16110 info
```

Observed initial state:

```text
enabled=true
bindAddress=127.0.0.1:28515
frames.digest=0
framesReceived=0
lastDigest=null
```

## Send Snapshot Over LoRa

Receiver:

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515 \
  --count 1 \
  --timeout-ms 70000 \
  --packet-idle-ms 600
```

Transmitter:

```bash
target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/lora-e2e/snapshot.bin \
  --inter-frame-delay-ms 2500
```

Observed bridge output:

```text
sent frame 1/2: app_payload=234 serial_bytes=240
sent frame 2/2: app_payload=123 serial_bytes=129
received fragment 1/2 for datagram 1
recovered KUDP datagram 1/1: 329 bytes
```

## Send Delta Over LoRa

Receiver:

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515 \
  --count 1 \
  --timeout-ms 30000 \
  --packet-idle-ms 600
```

Transmitter:

```bash
target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/lora-e2e/delta.bin
```

Observed bridge output:

```text
sent frame 1/1: app_payload=200 serial_bytes=206
recovered KUDP datagram 1/1: 200 bytes
```

Observed kaspad logs:

```text
udp.event=digest_ingested epoch=42 signer=0 source=7 kind=snapshot
udp.event=digest_ingested epoch=43 signer=0 source=7 kind=delta
```

Observed `getUdpIngestInfo`:

```text
frames.digest=2
framesReceived=2
bytesTotal=529
lastDigest.epoch=43
lastDigest.signatureValid=true
sourceCount=1
sources[0].lastEpoch=43
```

Observed `getUdpDigests --limit 10`:

```text
digests[0].kind=delta
digests[0].verified=true
digests[1].kind=snapshot
digests[1].verified=true
```

## Negative Cases

### Corrupt Payload

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515 \
  --count 1 \
  --timeout-ms 30000 \
  --packet-idle-ms 600

target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/lora-e2e/delta-corrupt.bin
```

Observed result:

```text
udp.event=digest_drop reason=signature seq=20
drops.signature=1
signatureFailures=1
```

### Duplicate Resend

The dedup retention window is short, so send the duplicate immediately after
the previous accepted copy.

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515 \
  --count 1 \
  --timeout-ms 30000 \
  --packet-idle-ms 600

target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/lora-e2e/delta.bin
```

Observed result:

```text
udp.event=frame_drop reason=duplicate kind=digest seq=20 bytes=162 remote=loopback
drops.duplicate=1
```

If the same frame is resent after the dedup retention expires, the current
runtime accepts it again. In the lab run, a resend about 54 seconds after the
first delta produced another `digest_ingested` log line; the immediate resend
produced the duplicate drop above.

### Wrong Network Tag

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515 \
  --count 1 \
  --timeout-ms 30000 \
  --packet-idle-ms 600

target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input file \
  --file /tmp/lora-e2e/delta-wrong-network.bin
```

Observed result:

```text
udp.event=frame_drop reason=network_mismatch kind=digest seq=20 bytes=200 remote=loopback
drops.network_mismatch=1
```

Final `getUdpIngestInfo` after the negative cases:

```text
frames.digest=4
drops.signature=1
drops.duplicate=1
drops.network_mismatch=1
lastDigest.signatureValid=true
```

Final `getUdpDigests --limit 10` still returned verified records only. The
lab output contained the original verified snapshot, the original verified
delta, and one later verified delta resend that occurred after the dedup
retention window expired:

```text
delta verified=true
delta verified=true
snapshot verified=true
```
