# LoRa Testnet Live Node Lab

Date: 2026-05-14

This report tracks the first attempt to validate the LoRa UDP digest alpha
against real `testnet-10` node state. It is not a mainnet or production
readiness claim.

## Goal

Use a synced `testnet-10` producer kaspad as the `udp-live-digest-producer` RPC
source, transmit signed `DigestV1` datagrams over real SX126X LoRa, ingest them
on a separate receiver kaspad, and verify the received digests reflect real
testnet state as far as the current `DigestV1` schema allows.

## Added Tooling

Two reusable pieces were added for this lab:

- `udp-rpc-node-info`: compact JSON helper for producer/receiver node evidence.
  It captures network id, sync flags, sink, pruning point, virtual DAA score,
  sink blue score, sink blue work, and sink UTXO commitment if the sink block is
  available. It supports gRPC URLs, direct wRPC URLs, and `--rpc-url pnn` for
  the wRPC public-node resolver.
- `udp/tools/lora_testnet_live_node_lab.sh`: real-network LoRa lab script. It
  can start local testnet producer/receiver nodes or attach to an already synced
  producer with `--external-producer-rpc`. Started local nodes use
  `--testnet --netsuffix=10` explicitly.

Fresh local testnet nodes can take hours to sync. The preferred validation path
is therefore:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --external-producer-rpc grpc://<synced-testnet-10-node>:<grpc-port> \
  --duration-seconds 600 \
  --interval-ms 5000 \
  --snapshot-every 10 \
  --report /tmp/lora-testnet-live-node-lab-2026-05-14.md
```

If no synced external producer is available, use a persistent work directory
and a long sync wait so a local producer can sync once and then continue into
the LoRa run:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --workdir /tmp/lora-testnet-live-node-lab-state \
  --sync-wait-seconds 21600 \
  --duration-seconds 600 \
  --interval-ms 5000 \
  --snapshot-every 10 \
  --report /tmp/lora-testnet-live-node-lab-2026-05-14.md
```

If the command is interrupted before sync completes, rerun with the same
`--workdir` so the producer appdir is reused instead of starting from an empty
temporary appdir. The script waits for producer sync before starting the
receiver or LoRa stages, so a long local sync does not hold the receiver UDP/RPC
ports or serial devices.

For quick endpoint checks, reduce the RPC startup wait:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --external-producer-rpc grpc://<candidate-testnet-10-node>:<grpc-port> \
  --rpc-wait-seconds 5 \
  --sync-wait-seconds 10 \
  --duration-seconds 1 \
  --report /tmp/lora-testnet-probe.md
```

The script refuses to continue unless the producer reports synced `testnet-10`
state, unless `--no-require-synced` is explicitly supplied. It always verifies
that producer and receiver RPCs report `testnet-10`, so an accidental mainnet or
wrong-testnet endpoint cannot satisfy the lab. That protects the lab from
accidentally turning a fresh genesis node or wrong network into false "real
network" evidence.
Runs completed with `--no-require-synced` can be useful for debugging, but they
are labeled `complete-unsynced` and exit nonzero so they cannot satisfy this
real-testnet acceptance check.
After transmission, the script also treats failed `getUdpIngestInfo` or
producer-log-vs-receiver-snapshot comparison as a failed lab run.

## Local Fresh-Node Attempt

Command attempted:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --duration-seconds 600 \
  --interval-ms 5000 \
  --snapshot-every 10 \
  --report /tmp/lora-testnet-live-node-lab-2026-05-14.md
```

Result: blocked before LoRa transmission because the fresh producer node did not
reach synced state quickly enough.

Observed producer sync evidence:

```json
{
  "blockCount": 0,
  "dagNetwork": "testnet-10",
  "hasUtxoIndex": false,
  "headerCount": 0,
  "networkId": "testnet-10",
  "pruningPoint": "f896a3034873be1739fc4359236899fd3d65d2bc94f9780df0d0da3eb1cc4370",
  "serverInfoSynced": false,
  "serverVirtualDaaScore": 0,
  "sink": "f896a3034873be1739fc4359236899fd3d65d2bc94f9780df0d0da3eb1cc4370",
  "sinkBlueScore": 0,
  "sinkBlueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
  "syncStatusSynced": false,
  "virtualDaaScore": 0
}
```

The producer log showed active peer discovery and pruning-proof validation, so
the node was progressing through sync work, but it was still at genesis-level
state when the run was stopped. That is not valid acceptance evidence for this
goal.

## Field Semantics For The Real Testnet Run

When run against a synced producer and without `--lab-progress-counter`, the
producer provenance report should classify:

| Field | Source | Status |
| --- | --- | --- |
| `epoch` | `getBlockDagInfo.virtual_daa_score` | real testnet RPC state |
| `pruning_point` | `getBlockDagInfo.pruning_point_hash` | real testnet RPC state |
| `virtual_selected_parent` | `getBlockDagInfo.sink` | real testnet RPC state |
| `virtual_blue_score` | `getSinkBlueScore.blue_score` | real testnet RPC state |
| `daa_score` | `getBlockDagInfo.virtual_daa_score` | real testnet RPC state |
| `blue_work` | `getBlock(sink).header.blue_work` | real sink-header state if available |
| `utxo_muhash` | `getBlock(sink).header.utxo_commitment` | real sink-header commitment, not a virtual-state MuHash proof |
| `pruning_proof_commitment` | deterministic lab placeholder | placeholder |
| `frame_timestamp_ms` | producer wall clock | lab-derived |
| `signer_id`, `source_id`, `signature` | configured lab signer/source | lab/operator-derived |
| `kept_headers_mmr_root` | not encoded | omitted |

## Acceptance Status

Complete for a bounded real-hardware alpha run.

- Synced producer: local `testnet-10` kaspad at
  `grpc://127.0.0.1:16221`, with `serverInfoSynced=true` and
  `syncStatusSynced=true`.
- Transport: two SX126X HATs through `/dev/lora-left` TX and
  `/dev/lora-right` RX.
- Successful run: 50 mixed live DigestV1 datagrams, snapshots every 10
  datagrams, no lab progress counter.
- Receiver ingest: 50 frames, 10,485 bytes, one source, zero signature
  failures.
- Bridge result: TX and RX both exited zero; RX recovered 50/50 datagrams and
  55/55 fragments with zero retries, duplicate fragments, corrupt frames,
  receive timeouts, or reassembly failures.
- Digest comparison: `udp-rpc-digests --check-monotonic --producer-log
  --compare-local` exited zero.

The committed result artifact is
[`lora-testnet-live-node-lab-result-2026-05-14.md`](lora-testnet-live-node-lab-result-2026-05-14.md).

## Measured Operating Envelope

The first synced run attempted 92 datagrams over a 600 second producer window.
It proved real ingest but exceeded the current RF envelope: receiver kaspad
ingested 54 verified digests, then TX exhausted retries on datagram 55 and RX
timed out. The successful bounded run used 50 datagrams over the same target
duration.

Current measured safe command:

```bash
./udp/tools/lora_testnet_live_node_lab.sh \
  --external-producer-rpc grpc://127.0.0.1:16221 \
  --workdir /home/luke/lora-testnet-live-node-lab-state/run-count50 \
  --duration-seconds 600 \
  --count 50 \
  --interval-ms 5000 \
  --snapshot-every 10 \
  --report /home/luke/lora-testnet-live-node-lab-2026-05-14-count50.md
```

For longer unattended runs, keep the datagram target aligned with the measured
RX throughput, or increase RX timeout/headroom. At the observed bridge summary
rates, 90+ mixed datagrams need substantially more than the 600 second target
window once ACK/retry and snapshot fragmentation are included.

## Remaining Gap

The receiver kaspad in this lab is intentionally fresh and unsynced, so it can
prove UDP ingest/storage/signature validation but still reports divergence
against live producer digest state. A production relevance test should run the
receiver against a synced node if the goal is agreement, not only relay/ingest.
