# LoRa Live Soak Report

Generated: 2026-05-13T10:12:19Z

This is the committed evidence artifact for the completed 30-minute target live
LoRa UDP digest soak. The run used the live devnet digest producer, reliable
LoRa TX/RX, receiver kaspad UDP ingest, and RPC polling.

Result: complete.

- Produced datagrams: `276`
- Recovered datagrams: `276`
- Receiver kaspad frames: `276`
- Receiver kaspad signature failures: `0`
- Bridge retries: `0`
- Bridge receive timeouts: `0`
- Bridge corrupt frames: `0`
- Bridge duplicate fragments: `0`

## Configuration

- Duration target: `1800` seconds
- Produced datagrams: `276`
- TX serial: `/dev/lora-left`
- RX serial: `/dev/lora-right`
- Inter-frame delay: `2500` ms
- Retry count: `8`
- ACK timeout: `6000` ms
- Snapshot every: `50`
- Expected datagram budget: `6500` ms
- RX timeout: `1980000` ms
- Session id: `17`
- Workdir: `/tmp/lora-live-soak.p25wgP`

## Bridge Summary

```text
/tmp/lora-live-soak.p25wgP/lora-rx.log:bridge_summary role=rx datagrams_sent=0 datagrams_recovered=276 fragments_sent=0 fragments_received=282 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=282 datagrams_per_minute=9.88 bytes_per_minute=1996
/tmp/lora-live-soak.p25wgP/lora-tx.log:bridge_summary role=tx datagrams_sent=276 datagrams_recovered=0 fragments_sent=282 fragments_received=0 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=0 datagrams_per_minute=10.78 bytes_per_minute=2179
```

## Process Status

- live producer exit: `0`
- lora tx exit: `0`
- lora rx exit: `0`
- result: `complete`

## Kaspad Ingest Summary

```text
  "bytesTotal": 55782,
  "framesReceived": 276,
  "lastDigest": {
  "sourceCount": 1,
  "signatureFailures": 0,
```

## Recent Digests

{
  "digests": [
    {
      "epoch": 90,
      "kind": "delta",
      "summary": {
        "epoch": 90,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 90,
        "daaScore": 90,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666101853,
        "sourceId": 7
      },
      "verified": true
    },
    {
      "epoch": 89,
      "kind": "delta",
      "summary": {
        "epoch": 89,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 89,
        "daaScore": 89,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666096381,
        "sourceId": 7
      },
      "verified": true
    },
    {
      "epoch": 88,
      "kind": "delta",
      "summary": {
        "epoch": 88,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 88,
        "daaScore": 88,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666090911,
        "sourceId": 7
      },
      "verified": true
    },
    {
      "epoch": 87,
      "kind": "delta",
      "summary": {
        "epoch": 87,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 87,
        "daaScore": 87,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666085440,
        "sourceId": 7
      },
      "verified": true
    },
    {
      "epoch": 86,
      "kind": "delta",
      "summary": {
        "epoch": 86,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 86,
        "daaScore": 86,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666079969,
        "sourceId": 7
      },
      "verified": true
    },
    {
      "epoch": 85,
      "kind": "delta",
      "summary": {
        "epoch": 85,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 85,
        "daaScore": 85,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666074500,
        "sourceId": 7
      },
      "verified": true
    },
    {
      "epoch": 84,
      "kind": "delta",
      "summary": {
        "epoch": 84,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 84,
        "daaScore": 84,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666069029,
        "sourceId": 7
      },
      "verified": true
    },
    {
      "epoch": 83,
      "kind": "delta",
      "summary": {
        "epoch": 83,
        "pruningPoint": null,
        "pruningProofCommitment": null,
        "utxoMuhash": null,
        "virtualSelectedParent": "4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2",
        "virtualBlueScore": 83,
        "daaScore": 83,
        "blueWorkHex": "0000000000000000000000000000000000000000000000000000000000000000",
        "keptHeadersMmrRoot": null,
        "signerId": 0,
        "signatureValid": true,
        "recvTsMs": 1778666063556,
        "sourceId": 7
      },

## Logs

- Producer kaspad: `/tmp/lora-live-soak.p25wgP/producer-kaspad.log`
- Receiver kaspad: `/tmp/lora-live-soak.p25wgP/receiver-kaspad.log`
- Live producer: `/tmp/lora-live-soak.p25wgP/live-producer.log`
- LoRa TX: `/tmp/lora-live-soak.p25wgP/lora-tx.log`
- LoRa RX: `/tmp/lora-live-soak.p25wgP/lora-rx.log`
- RPC poll: `/tmp/lora-live-soak.p25wgP/rpc-poll.log`
