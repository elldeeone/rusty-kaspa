# Live DigestV1 Producer LoRa Lab

This lab replaces static fixture datagrams with a live lab producer that queries
a local devnet/simnet kaspad RPC endpoint, builds signed `DigestV1` `KUDP`
datagrams, feeds them into `lora-bridge tx`, and verifies that a separate
receiver kaspad ingests them through the existing UDP side-channel path.

This is lab tooling only. It does not use mainnet, does not require synced
nodes, does not change consensus behavior, and is not a production digest
oracle.

## Trust And Field Semantics

The current RPC surface is enough to build a signed `DigestV1` datagram that
the existing UDP ingest parser and signature verifier accept. It is not enough
to claim a fully consensus-authentic digest for every snapshot field.

Fields populated from producer-side kaspad RPC:

- `pruning_point`: `getBlockDagInfo.pruning_point_hash`
- `virtual_selected_parent`: first `getBlockDagInfo.virtual_parent_hashes`, or
  `getBlockDagInfo.sink` if the virtual parent list is empty
- `virtual_blue_score`: `getSinkBlueScore.blue_score`, optionally plus the lab
  progress counter
- `daa_score`: `getBlockDagInfo.virtual_daa_score`, optionally plus the lab
  progress counter
- `blue_work`: `getBlock(sink).header.blue_work`, padded to the 32-byte
  DigestV1 field

Lab-placeholder fields:

- `pruning_proof_commitment`: deterministic SHA-256 placeholder derived from
  the pruning point and DAA score
- `utxo_muhash`: deterministic SHA-256 placeholder derived from the sink and
  DAA score
- `kept_headers_mmr_root`: omitted (`None`)

Lab-derived progression:

- In an idle local devnet/simnet, the RPC DAA score and blue score may not
  advance. The lab command below uses `--lab-progress-counter`, which adds the
  output index to `epoch`, `daa_score`, and `virtual_blue_score` so the receiver
  can demonstrate ordered live ingest without mining blocks. This is explicit
  lab metadata, not a consensus claim.

## Build

```bash
cargo build -p udp-generator \
  --bin udp-live-digest-producer \
  --bin udp-rpc-digests \
  --bin udp-digest-fixtures
cargo build -p lora-bridge
cargo test -p lora-bridge
```

Observed result:

```text
cargo build ... Finished dev profile
cargo test -p lora-bridge
5 passed; 0 failed
```

## File Output Smoke Test

Start a producer-side devnet kaspad:

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

Generate inspectable datagrams:

```bash
rm -rf /tmp/live-digests
target/debug/udp-live-digest-producer \
  --rpc-url grpc://127.0.0.1:16111 \
  --network devnet \
  --output file \
  --out-dir /tmp/live-digests \
  --count 3 \
  --interval-ms 10 \
  --lab-progress-counter
wc -c /tmp/live-digests/*.bin
```

Observed result:

```text
field provenance: real_rpc=pruning_point_hash,virtual_parent_hashes/sink,sink_blue_score,virtual_daa_score,sink_header.blue_work; placeholder_lab=pruning_proof_commitment,utxo_muhash,kept_headers_mmr_root_absent; optional_lab_progress_counter=epoch/daa_score/virtual_blue_score increments
produced index=0 kind=Snapshot seq=1000 epoch=0 virtual_blue_score=0 daa_score=0 bytes=297
produced index=1 kind=Delta seq=1001 epoch=1 virtual_blue_score=1 daa_score=1 bytes=200
produced index=2 kind=Delta seq=1002 epoch=2 virtual_blue_score=2 daa_score=2 bytes=200
297 /tmp/live-digests/0000-snapshot.bin
200 /tmp/live-digests/0001-delta.bin
200 /tmp/live-digests/0002-delta.bin
```

## End-To-End LoRa Flow

The signer used by default is the existing lab fixture key:

```text
4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa
```

Start the receiver-side kaspad with UDP ingest enabled:

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

Confirm receiver starts empty:

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

Start LoRa RX, forwarding recovered datagrams into receiver kaspad:

```bash
target/debug/lora-bridge rx \
  --serial /dev/lora-right \
  --output udp \
  --udp-target 127.0.0.1:28515 \
  --count 4 \
  --timeout-ms 150000 \
  --packet-idle-ms 600
```

Start LoRa TX, reading four live datagrams from a local UDP socket:

```bash
target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input udp \
  --udp-bind 127.0.0.1:39000 \
  --count 4 \
  --inter-frame-delay-ms 2500
```

Run the live producer into the LoRa TX socket:

```bash
target/debug/udp-live-digest-producer \
  --rpc-url grpc://127.0.0.1:16111 \
  --network devnet \
  --output udp \
  --udp-target 127.0.0.1:39000 \
  --count 4 \
  --interval-ms 500 \
  --lab-progress-counter
```

Observed producer output:

```text
produced index=0 kind=Snapshot seq=1000 epoch=0 virtual_blue_score=0 daa_score=0 bytes=297
produced index=1 kind=Delta seq=1001 epoch=1 virtual_blue_score=1 daa_score=1 bytes=200
produced index=2 kind=Delta seq=1002 epoch=2 virtual_blue_score=2 daa_score=2 bytes=200
produced index=3 kind=Delta seq=1003 epoch=3 virtual_blue_score=3 daa_score=3 bytes=200
```

Observed LoRa TX output:

```text
received UDP datagram 1/4 from 127.0.0.1:56562: 297 bytes
received UDP datagram 2/4 from 127.0.0.1:56562: 200 bytes
received UDP datagram 3/4 from 127.0.0.1:56562: 200 bytes
received UDP datagram 4/4 from 127.0.0.1:56562: 200 bytes
sent datagram 1/4 frame 1/2: app_payload=234 serial_bytes=240
sent datagram 1/4 frame 2/2: app_payload=91 serial_bytes=97
sent datagram 2/4 frame 1/1: app_payload=200 serial_bytes=206
sent datagram 3/4 frame 1/1: app_payload=200 serial_bytes=206
sent datagram 4/4 frame 1/1: app_payload=200 serial_bytes=206
```

Observed LoRa RX output:

```text
received fragment 1/2 for datagram 1
recovered KUDP datagram 1/4: 297 bytes
recovered KUDP datagram 2/4: 200 bytes
recovered KUDP datagram 3/4: 200 bytes
recovered KUDP datagram 4/4: 200 bytes
```

Observed receiver kaspad logs:

```text
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=snapshot
udp.event=digest_ingested epoch=1 signer=0 source=7 kind=delta
udp.event=digest_ingested epoch=2 signer=0 source=7 kind=delta
udp.event=digest_ingested epoch=3 signer=0 source=7 kind=delta
```

Observed `getUdpIngestInfo`:

```text
frames.digest=4
framesReceived=4
bytesTotal=897
lastDigest.epoch=3
lastDigest.virtualBlueScore=3
lastDigest.daaScore=3
lastDigest.signatureValid=true
signatureFailures=0
```

Observed `getUdpDigests --limit 10`:

```text
epoch=3 kind=delta verified=true
epoch=2 kind=delta verified=true
epoch=1 kind=delta verified=true
epoch=0 kind=snapshot verified=true
```

The observed progression above is the lab progress counter over an idle devnet
node. Without `--lab-progress-counter`, the produced values reflect the RPC
state directly and may remain unchanged unless the producer-side node advances.
