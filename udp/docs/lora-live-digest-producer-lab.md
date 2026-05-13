# Live DigestV1 Producer LoRa Lab

This lab replaces static fixture datagrams with a live lab producer that queries
a local devnet/simnet kaspad RPC endpoint, builds signed `DigestV1` `KUDP`
datagrams, feeds them into `lora-bridge tx`, and verifies that a separate
receiver kaspad ingests them through the existing UDP side-channel path.

This is lab tooling only. It does not use mainnet, does not require synced
nodes, does not change consensus behavior, and is not a production digest
oracle.

For the field-by-field authenticity audit and remaining production gaps, see
[`lora-digest-authenticity.md`](lora-digest-authenticity.md).

## Trust And Field Semantics

The current RPC surface is enough to build a signed `DigestV1` datagram that
the existing UDP ingest parser and signature verifier accept. It is not enough
to claim a fully consensus-authentic digest for every snapshot field.

Fields populated from producer-side kaspad RPC:

- `pruning_point`: `getBlockDagInfo.pruning_point_hash`
- `virtual_selected_parent`: `getBlockDagInfo.sink`, matching the receiver
  divergence monitor comparison against `async_get_sink()`
- `virtual_blue_score`: `getSinkBlueScore.blue_score`, optionally plus the lab
  progress counter
- `daa_score`: `getBlockDagInfo.virtual_daa_score`, optionally plus the lab
  progress counter
- `blue_work`: `getBlock(sink).header.blue_work`, padded to the 32-byte
  DigestV1 field
- `utxo_muhash`: `getBlock(sink).header.utxo_commitment`

Lab-placeholder fields:

- `pruning_proof_commitment`: deterministic SHA-256 placeholder derived from
  the pruning point and DAA score
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
  --provenance-report
wc -c /tmp/live-digests/*.bin
```

Observed result:

```text
field provenance: real_rpc=pruning_point_hash,sink,sink_blue_score,virtual_daa_score,sink_header.blue_work,sink_header.utxo_commitment; placeholder_lab=pruning_proof_commitment; omitted=kept_headers_mmr_root; optional_lab_progress_counter=epoch/daa_score/virtual_blue_score increments
{"event":"udp_live_digest_provenance","index":0,"kind":"snapshot","seq":1000,"sink":"...","fields":[...]}
produced index=0 kind=Snapshot seq=1000 epoch=0 virtual_blue_score=0 daa_score=0 bytes=297
produced index=1 kind=Delta seq=1001 epoch=0 virtual_blue_score=0 daa_score=0 bytes=200
produced index=2 kind=Delta seq=1002 epoch=0 virtual_blue_score=0 daa_score=0 bytes=200
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
  --packet-idle-ms 600 \
  --session-id 8
```

Start LoRa TX, reading four live datagrams from a local UDP socket:

```bash
target/debug/lora-bridge tx \
  --serial /dev/lora-left \
  --input udp \
  --udp-bind 127.0.0.1:39000 \
  --count 4 \
  --reliable-fragments \
  --retry-count 4 \
  --ack-timeout-ms 3000 \
  --session-id 8 \
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
  --provenance-report
```

Observed producer output:

```text
produced index=0 kind=Snapshot seq=1000 epoch=0 virtual_blue_score=0 daa_score=0 bytes=297
produced index=1 kind=Delta seq=1001 epoch=0 virtual_blue_score=0 daa_score=0 bytes=200
produced index=2 kind=Delta seq=1002 epoch=0 virtual_blue_score=0 daa_score=0 bytes=200
produced index=3 kind=Delta seq=1003 epoch=0 virtual_blue_score=0 daa_score=0 bytes=200
```

Observed LoRa TX output:

```text
received UDP datagram 1/4 from 127.0.0.1:...: 297 bytes
received UDP datagram 2/4 from 127.0.0.1:...: 200 bytes
received UDP datagram 3/4 from 127.0.0.1:...: 200 bytes
received UDP datagram 4/4 from 127.0.0.1:...: 200 bytes
sent datagram 1/4 frame 1/2: app_payload=234 serial_bytes=240
sent datagram 1/4 frame 2/2: app_payload=99 serial_bytes=105
sent datagram 2/4 frame 1/1: app_payload=200 serial_bytes=206
sent datagram 3/4 frame 1/1: app_payload=200 serial_bytes=206
sent datagram 4/4 frame 1/1: app_payload=200 serial_bytes=206
bridge_summary role=tx datagrams_sent=4 fragments_sent=5 retries=0
```

Observed LoRa RX output:

```text
received fragment 1/2 for datagram 1
recovered KUDP datagram 1/4: 297 bytes
recovered KUDP datagram 2/4: 200 bytes
recovered KUDP datagram 3/4: 200 bytes
recovered KUDP datagram 4/4: 200 bytes
bridge_summary role=rx datagrams_recovered=4 fragments_received=2 acks_sent=2
```

Observed receiver kaspad logs:

```text
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=snapshot
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=delta
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=delta
udp.event=digest_ingested epoch=0 signer=0 source=7 kind=delta
```

Observed `getUdpIngestInfo`:

```text
frames.digest=4
framesReceived=4
bytesTotal=897
lastDigest.epoch=0
lastDigest.virtualBlueScore=0
lastDigest.daaScore=0
lastDigest.signatureValid=true
signatureFailures=0
```

Observed `getUdpDigests --limit 10 --check-monotonic`:

```text
epoch=0 kind=delta verified=true
epoch=0 kind=delta verified=true
epoch=0 kind=delta verified=true
epoch=0 kind=snapshot verified=true
udp_digest_check count=4 all_signature_valid=true epoch_monotonic=true daa_score_monotonic=true virtual_blue_score_monotonic=true sources={7} signers={0}
```

The observed repeated zero values above are direct RPC state from an idle
devnet node. Add `--lab-progress-counter` only when you explicitly need
monotonic demo output without mining blocks; those values are lab-derived, not
consensus-authentic.

## Unattended Soak Harness

Use the soak harness when you want the full live path without manually keeping
five terminals coordinated:

```bash
./udp/tools/lora_live_soak_lab.sh \
  --duration-seconds 1800 \
  --inter-frame-delay-ms 2500 \
  --interval-ms 500 \
  --ack-timeout-ms 6000 \
  --retry-count 8 \
  --snapshot-every 50 \
  --provenance-report \
  --expected-datagram-ms 6500 \
  --report /tmp/lora-live-soak-report.md
```

The script starts producer and receiver devnet kaspad instances, runs
`lora-bridge rx` into receiver UDP ingest, runs reliable `lora-bridge tx`, runs
the live producer, polls `getUdpIngestInfo`, and writes a markdown report with
bridge summaries plus final `getUdpIngestInfo` and `getUdpDigests` samples.

Current alpha limitation: `lora-bridge tx --input udp --count N` reads all
`N` UDP datagrams before starting serial transmission. For long soaks this
means RF transmission begins after the live producer has already emitted the
configured batch. A production sidecar should stream UDP input directly into
the RF scheduler.

The completed target soak is summarized in
[`lora-live-soak-report.md`](lora-live-soak-report.md): 276 produced datagrams,
276 recovered datagrams, 276 receiver kaspad frames, zero signature failures,
zero retries, and zero receive timeouts.
