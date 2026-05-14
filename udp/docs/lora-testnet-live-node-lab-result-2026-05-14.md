# LoRa Testnet Live Node Lab Result

Date: 2026-05-14

Result: complete for a bounded real-hardware alpha run.

This is committed evidence for a synced `testnet-10` producer feeding live
signed `DigestV1` traffic through two SX126X LoRa HATs into receiver kaspad UDP
ingest. It is not a mainnet or production-readiness claim.

## Setup

- Producer RPC: `grpc://127.0.0.1:16221`
- Producer network: `testnet-10`
- Producer sync: `serverInfoSynced=true`, `syncStatusSynced=true`
- TX serial: `/dev/lora-left`
- RX serial: `/dev/lora-right`
- Receiver RPC: `grpc://127.0.0.1:16220`
- Receiver UDP ingest: `127.0.0.1:28620`
- Workdir: `/home/luke/lora-testnet-live-node-lab-state/run-count50`
- Report: `/home/luke/lora-testnet-live-node-lab-2026-05-14-count50.md`

## Successful Command

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

## Successful Run Summary

- Result: `complete`
- Produced datagrams: 50
- Snapshot cadence: first datagram and every 10 datagrams
- Live producer exit: 0
- LoRa TX exit: 0
- LoRa RX exit: 0
- Ingest RPC check exit: 0
- Digest comparison exit: 0
- Receiver frames: 50
- Receiver bytes: 10,485
- Receiver source count: 1
- Receiver signature failures: 0

Bridge summaries:

```text
bridge_summary role=rx datagrams_sent=0 datagrams_recovered=50 fragments_sent=0 fragments_received=55 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=55 datagrams_per_minute=5.54 bytes_per_minute=1161
bridge_summary role=tx datagrams_sent=50 datagrams_recovered=0 fragments_sent=55 fragments_received=0 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=0 datagrams_per_minute=10.17 bytes_per_minute=2132
```

## Capacity Boundary Run

A prior synced run attempted the default derived count of 92 datagrams over the
same 600 second target. It confirmed real ingest, but exceeded the reliable RF
window for the current timeout:

- Result: `failed`
- RX recovered: 54/92 datagrams
- RX fragments: 60
- Receiver frames: 54
- Receiver bytes: 11,382
- TX failure: retry exhausted for `datagram_id=55 frag_ix=0`
- RX failure: timed out waiting for LoRa packet
- RX observed rate: 4.15 datagrams/minute, 875 bytes/minute

That run demonstrates that the producer can generate faster than the current
LoRa bridge can drain mixed snapshot/delta traffic. The bounded 50-datagram
run is the reproducible operating point for this hardware/configuration.

## Semantic Notes

The live producer used real testnet RPC state for pruning point, sink/virtual
selected parent, sink blue score, virtual DAA score, sink-header blue work, and
sink-header UTXO commitment when available.

Known lab-grade fields remain unchanged: `pruning_proof_commitment` is still a
deterministic placeholder, `kept_headers_mmr_root` is omitted, and signer/source
identity are operator-configured lab policy. The receiver was a fresh unsynced
node, so divergence logs are expected and do not invalidate the transport/ingest
result.
