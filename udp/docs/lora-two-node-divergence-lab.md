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

The helper comparison and receiver-native `getUdpIngestInfo.divergence` flag
now agree in the lab. The helper remains useful because it prints received and
local values for each mismatched field, while the core receiver state exposes
the bounded divergence flag and last mismatch epoch.

## Native Divergence Audit

The receiver-native monitor lives in `kaspad/src/udp.rs` and runs as the
`udp-divergence-monitor` async service when digest ingest is enabled. It checks
the latest accepted verified snapshot every five seconds against the receiver's
local consensus session.

The previous controlled `virtual_blue_score` mismatch did not flip
`getUdpIngestInfo.divergence` for two reasons:

- the monitor service existed but was not registered in daemon startup, so it
  never ticked in the two-node lab
- `UdpDigestManager::latest_snapshot()` only returned the last digest if the
  last digest was a snapshot; later deltas replaced that state before the final
  RPC check

This was a wiring/stale-state bug, not a narrower intended semantic. The fix is
to retain the latest verified snapshot separately, register the monitor when
the digest manager starts, and update divergence with bounded mismatch field
names.

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
  --report /tmp/lora-native-divergence-agreement-2026-05-13-r4.md
```

Result:

```text
workdir=/tmp/lora-live-soak.2AO42q
rx datagrams_recovered=4 fragments_received=5 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=5
tx datagrams_sent=4 fragments_sent=5 retries=0 receive_timeouts=0
framesReceived=4 signatureFailures=0
udp_digest_compare snapshot_match=true compared_fields=8 mismatches=[]
udp_digest_local_compare agreement=true compared_fields=5 mismatches=[] divergence_detected=false divergence_epoch=None compared_at_ms=1778672323369 source_id=7 signer_id=0 recv_ts_ms=1778672296106
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
  --report /tmp/lora-native-divergence-mismatch-2026-05-13-r3.md
```

Result:

```text
workdir=/tmp/lora-live-soak.ytKJgO
rx datagrams_recovered=4 fragments_received=5 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=5
tx datagrams_sent=4 fragments_sent=5 retries=0 receive_timeouts=0
framesReceived=4 signatureFailures=0
udp_digest_compare snapshot_match=true compared_fields=8 mismatches=[]
udp_digest_local_compare agreement=false compared_fields=5 mismatches=["virtual_blue_score: received=Some(\"1\") local=Some(\"0\")"] divergence_detected=true divergence_epoch=Some(0) compared_at_ms=1778672367905 source_id=7 signer_id=0 recv_ts_ms=1778672340641
receiver log: udp.event=divergence state=entered reason=snapshot_mismatch epoch=Some(0) fields=virtual_blue_score
```

Interpretation: LoRa delivery and signature verification still succeeded,
producer provenance matched receiver storage, receiver-local comparison
identified the semantic mismatch in `virtual_blue_score`, and the native
receiver divergence flag flipped to true.

## Compared Fields

The native monitor and `udp_digest_local_compare` both compare the latest
verified received snapshot against receiver-local values for:

- `pruning_point`
- `virtual_selected_parent`
- `virtual_blue_score`
- `daa_score`
- `blue_work`

Snapshot `utxo_muhash` is compared between producer provenance and receiver
storage, but not against receiver-local virtual UTXO state because that exact
RPC hook does not exist yet. `pruning_proof_commitment` remains a lab
placeholder, and `kept_headers_mmr_root` remains omitted.

`getUdpIngestInfo.divergence` carries the native boolean and last mismatch
epoch. Detailed received/local values are intentionally kept in the helper
output for this alpha instead of expanding the RPC schema. Logs use bounded
field names only, and custom metrics expose `udp_divergence_detected` plus
`udp_divergence_mismatch_total`.

## Current Limits

- Native divergence currently evaluates the latest verified snapshot, not
  advisory delta-only fields.
- The helper detects and names mismatched received/local values; the RPC status
  carries only the bounded divergence state.
- The receiver can prove signed delivery and local agreement for the compared
  fields. It cannot yet prove canonical pruning-proof or kept-header
  commitments.
- Stronger production read-integrity still needs canonical proof commitments,
  a final state-commitment scope, and production signer/source operations.
