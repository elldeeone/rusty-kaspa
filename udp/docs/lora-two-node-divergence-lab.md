# LoRa Two-Node Agreement And Divergence Lab

Run date: 2026-05-13

This lab proves the semantic value of the LoRa digest side-channel on two local
devnet kaspad instances:

- a producer kaspad supplies live RPC state to `udp-live-digest-producer`
- a separate receiver kaspad ingests recovered LoRa `DigestV1` datagrams
- helper tooling compares producer provenance, receiver stored digests, and
  receiver-local consensus state

No mainnet, consensus changes, block relay, tx relay, FEC, or encryption are
involved.

## Why It Matters

Byte-exact LoRa transport only proves delivery. This lab checks whether the
receiver can use a signed digest from another node to decide if that producer's
view agrees with its own local consensus view. The receiver can currently prove:

- the digest was signed by an allowed signer id
- the digest was received and stored by UDP ingest
- the stored fields match the producer provenance report
- selected snapshot fields match or mismatch receiver-local RPC state

The helper comparison is the current semantic signal. `getUdpIngestInfo`
also exposes a `divergence` object, but in these runs it remained
`detected=false`; that flag is visible in output but is not yet the primary
lab assertion.

## Harness

The shared harness is `udp/tools/lora_live_soak_lab.sh`. It starts both kaspad
instances, LoRa RX/TX, the live producer, periodic receiver RPC polling, and a
final `udp-rpc-digests` check with:

```bash
digests --limit 10 --check-monotonic --producer-log <live-producer.log> --compare-local
```

The final helper output includes:

- `udp_digest_check`: signature, signer/source, epoch, DAA score, and blue score
  monotonicity over stored digests
- `udp_digest_compare`: produced provenance vs receiver stored snapshot fields
- `udp_digest_local_compare`: receiver stored snapshot vs receiver-local RPC
  state

## Agreement Case

Command:

```bash
./udp/tools/lora_live_soak_lab.sh \
  --count 4 \
  --duration-seconds 60 \
  --inter-frame-delay-ms 2500 \
  --interval-ms 500 \
  --ack-timeout-ms 6000 \
  --retry-count 8 \
  --snapshot-every 0 \
  --expected-datagram-ms 6500 \
  --signer-id 0 \
  --provenance-report \
  --report /tmp/lora-two-node-agreement-2026-05-13.md
```

Result:

```text
workdir=/tmp/lora-live-soak.5XL6yR
rx datagrams_recovered=4 fragments_received=5 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=5
tx datagrams_sent=4 fragments_sent=5 retries=0 receive_timeouts=0
framesReceived=4 signatureFailures=0
udp_digest_compare snapshot_match=true compared_fields=8 mismatches=[]
udp_digest_local_compare agreement=true compared_fields=5 mismatches=[] divergence_detected=false divergence_epoch=None
```

Interpretation: producer provenance matched receiver storage, receiver storage
matched receiver-local consensus state for the compared fields, and all
signatures were valid.

## Divergence Case

The divergence case uses a validly signed lab digest with one semantic field
intentionally changed:

```text
--lab-diverge-virtual-blue-score
```

This offsets `virtual_blue_score` by one after reading producer RPC state. It
does not change consensus rules and does not invalidate the signature.

Command:

```bash
./udp/tools/lora_live_soak_lab.sh \
  --count 4 \
  --duration-seconds 60 \
  --inter-frame-delay-ms 2500 \
  --interval-ms 500 \
  --ack-timeout-ms 6000 \
  --retry-count 8 \
  --snapshot-every 0 \
  --expected-datagram-ms 6500 \
  --signer-id 0 \
  --provenance-report \
  --lab-diverge-virtual-blue-score \
  --report /tmp/lora-two-node-divergence-2026-05-13-r2.md
```

Result:

```text
workdir=/tmp/lora-live-soak.Vi9M1e
rx datagrams_recovered=4 fragments_received=5 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=5
tx datagrams_sent=4 fragments_sent=5 retries=0 receive_timeouts=0
framesReceived=4 signatureFailures=0
udp_digest_compare snapshot_match=true compared_fields=8 mismatches=[]
udp_digest_local_compare agreement=false compared_fields=5 mismatches=["virtual_blue_score: received=Some(\"1\") local=Some(\"0\")"] divergence_detected=false divergence_epoch=None
```

Interpretation: LoRa delivery and signature verification still succeeded,
producer provenance matched receiver storage, and receiver-local comparison
identified the semantic mismatch in `virtual_blue_score`.

## Compared Fields

`udp_digest_local_compare` currently compares the latest received snapshot
against receiver-local RPC values for:

- `pruning_point`
- `virtual_selected_parent`
- `virtual_blue_score`
- `daa_score`
- `blue_work`

Snapshot `utxo_muhash` is compared between producer provenance and receiver
storage, but not against receiver-local virtual UTXO state because that exact
RPC hook does not exist yet. `pruning_proof_commitment` remains a lab
placeholder, and `kept_headers_mmr_root` remains omitted.

## Current Limits

- The helper detects and names mismatched fields today.
- `getUdpIngestInfo.divergence` is reported, but it did not flip to true in the
  controlled mismatch run. Treat that as a remaining integration gap, not as a
  contradiction of the helper result.
- The receiver can prove signed delivery and local agreement for the compared
  fields. It cannot yet prove canonical pruning-proof or kept-header
  commitments.
- Stronger production read-integrity still needs canonical proof commitments,
  a final state-commitment scope, and production signer/source operations.
