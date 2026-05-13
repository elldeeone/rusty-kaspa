# LoRa DigestV1 Authenticity Report

Run date: 2026-05-13

This note tracks the semantic status of `udp-live-digest-producer`. It is
devnet/simnet lab tooling. It does not use mainnet, does not change consensus
rules, and does not claim production-authentic digest semantics where the node
does not expose the needed state.

## Current Producer Behavior

The producer queries a local kaspad gRPC endpoint and emits signed `DigestV1`
`KUDP` datagrams. Pass `--provenance-report` to emit one JSON provenance report
per produced digest:

```bash
target/debug/udp-live-digest-producer \
  --rpc-url grpc://127.0.0.1:16111 \
  --network devnet \
  --output file \
  --out-dir /tmp/live-digests \
  --count 3 \
  --interval-ms 10 \
  --provenance-report
```

Each report includes field name, value, source, authenticity level, and notes.
Use `--lab-progress-counter` only when an idle devnet/simnet node needs
monotonic demo output; it changes `epoch`, `daa_score`, and
`virtual_blue_score` from real RPC state into lab-derived values.

The real LoRa run for this change is summarized in
[`lora-digest-authenticity-live-run.md`](lora-digest-authenticity-live-run.md).

## Field Provenance

| Field | Current source | Authenticity | Semantic intent | Full-authenticity gap |
| --- | --- | --- | --- | --- |
| `epoch` | `getBlockDagInfo.virtual_daa_score`, unless `--lab-progress-counter` adds the output index | Real without lab counter; lab-derived with lab counter | Digest ordering epoch | No gap for current virtual DAA interpretation; production should define whether this is virtual DAA score, wall-clock epoch, or sidecar sequence. |
| `frame_timestamp_ms` | Producer wall clock via `unix_now()` | Lab-derived | Time the sidecar framed/signed the digest | Needs either accepted node timestamp semantics or explicit schema language that this is sidecar observation time. |
| `pruning_point` | `getBlockDagInfo.pruning_point_hash` | Real | Node's current pruning point | No gap for the hash itself. |
| `pruning_proof_commitment` | Deterministic lab SHA-256 placeholder | Placeholder | Compact commitment to the actual pruning-point proof | Need node/RPC hook exposing the pruning-point proof or a consensus-defined proof commitment. Internal source: `PruningProofManager::get_pruning_point_proof()`. |
| `utxo_muhash` | `getBlock(sink).header.utxo_commitment` | Real, but sink-header scoped | UTXO commitment/MuHash-like state commitment | For exact virtual-state semantics, expose the current virtual UTXO multiset commitment. Internal source is virtual processor state; public RPC currently exposes block header `utxo_commitment`, not a virtual-state commitment. |
| `virtual_selected_parent` | `getBlockDagInfo.sink` | Real | Receiver-side comparison point for the selected chain tip/sink | No gap for matching the current receiver divergence monitor, which compares snapshots to `async_get_sink()`. If schema wants first virtual parent instead, document and update both producer and monitor together. |
| `virtual_blue_score` | `getSinkBlueScore.blue_score`, unless `--lab-progress-counter` adds the output index | Real without lab counter; lab-derived with lab counter | Blue score of the selected/sink chain state | No gap for sink blue score; production schema should clarify whether this is sink or virtual score. |
| `daa_score` | `getBlockDagInfo.virtual_daa_score`, unless `--lab-progress-counter` adds the output index | Real without lab counter; lab-derived with lab counter | Current virtual DAA score | No gap for virtual DAA score. |
| `blue_work` | `getBlock(sink).header.blue_work`, left-padded to 32 bytes | Real | Accumulated blue work for the selected/sink chain state | No gap for sink-header blue work; production schema should clarify sink vs virtual state. |
| `kept_headers_mmr_root` | Omitted (`None`) | Omitted | Optional commitment to kept headers | Need a consensus-defined kept-headers MMR and a node/RPC hook. No current public RPC exposes this root. |
| `signer_id` | Fixture builder encodes signer id `0` | Lab-derived | Identifies which configured signer key signed the digest | Production needs signer registry, rotation, revocation, and policy binding. |
| `signature` | Schnorr signature from configured lab signer secret | Lab-derived key, cryptographically valid | Authenticates the digest payload under configured signer id | Production needs operational signer infrastructure; the wire signature format itself verifies today. |
| `source_id` | `--source-id`, default `7` | Lab-derived | Operator-selected sidecar/source identity | Production needs source identity policy and collision/authorization rules. |

## Placeholder Replaced

`utxo_muhash` is no longer synthesized when the sink block is available. The
producer now uses `getBlock(sink).header.utxo_commitment`, which is real
consensus header state. This is better than the previous deterministic lab
placeholder, but it is intentionally described as sink-header scoped because it
is not necessarily the same as a future schema's virtual-state UTXO commitment.

`virtual_selected_parent` now uses `getBlockDagInfo.sink`, matching the existing
receiver divergence monitor in `kaspad/src/udp.rs`, which compares accepted
snapshots against `async_get_sink()`.

## Receiver Verification

The receiver already exposes accepted digest fields through `getUdpDigests`.
For a compact receiver-side summary, use:

```bash
target/debug/udp-rpc-digests \
  --rpc-url grpc://127.0.0.1:16110 \
  digests \
  --limit 10 \
  --check-monotonic
```

The helper prints the JSON RPC response and a compact check line containing:

- accepted digest count
- `all_signature_valid`
- `epoch_monotonic`
- `daa_score_monotonic`
- `virtual_blue_score_monotonic`
- observed source IDs
- observed signer IDs

## Remaining API/RPC Gaps

- Pruning proof commitment: expose a compact commitment for
  `PruningProofManager::get_pruning_point_proof()` or expose the proof and let
  the sidecar commit to a canonical encoding.
- Virtual UTXO commitment: expose the current virtual UTXO multiset commitment
  if the digest schema requires virtual-state semantics rather than sink-header
  semantics.
- Kept headers root: define the kept-headers MMR contents and expose its root,
  or remove the optional field from production-minimum digest expectations.
- Signer policy: define signer id allocation, key rotation, allowed signer
  distribution, revocation, and source id binding.
- Schema language: decide whether production `DigestV1` anchors to sink-header
  state, virtual state, or both. The current lab producer is explicit about
  this instead of overclaiming.

## Minimum Useful Production Digest

For LoRa/satellite read-integrity, the minimum useful digest is likely:

- network id
- source id
- signer id and signature
- virtual DAA score or equivalent ordering field
- pruning point hash
- selected/sink hash
- blue score
- blue work
- a UTXO/state commitment with clearly defined scope

`pruning_proof_commitment` and `kept_headers_mmr_root` are valuable for stronger
remote auditability, but they require node-supported canonical commitments
before the sidecar should claim production authenticity.
