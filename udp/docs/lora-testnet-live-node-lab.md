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
  available.
- `udp/tools/lora_testnet_live_node_lab.sh`: real-network LoRa lab script. It
  can start local testnet producer/receiver nodes or attach to an already synced
  producer with `--external-producer-rpc`.

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

The script refuses to continue unless the producer reports synced, unless
`--no-require-synced` is explicitly supplied. That protects the lab from
accidentally turning a fresh genesis node into false "real network" evidence.

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

Not yet complete.

- A real synced testnet producer was not available during this run.
- No real testnet DigestV1 traffic was transmitted over LoRa in this attempt.
- No receiver ingest/soak result is claimed here.

The branch now has the tooling and guarded script needed to run the real testnet
lab once a synced `testnet-10` producer RPC endpoint is available.

## Remaining Blocker

Provide or wait for a synced `testnet-10` kaspad gRPC endpoint, then rerun the
script with `--external-producer-rpc` or a pre-synced local appdir.
