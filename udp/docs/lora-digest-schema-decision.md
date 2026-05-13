# LoRa Digest Schema Decision

Date: 2026-05-13

## Decision

Keep `DigestV1` for the current devnet/simnet LoRa alpha and do not introduce
a wire-level `DigestV2` yet.

Rationale:

- `DigestV1` already carries the minimum useful digest shape for the lab:
  network, source id, signer id/signature, ordering score, pruning point,
  selected/sink hash, blue score, blue work, and a UTXO/state commitment field.
- The largest remaining issues are data-source semantics, not byte layout.
  `pruning_proof_commitment` and `kept_headers_mmr_root` need canonical node
  data before a new schema would make them more authentic.
- A premature `DigestV2` would either duplicate `DigestV1` with different names
  or bless placeholders as production fields.
- LoRa capacity favors keeping the digest compact until the node exposes
  canonical commitments worth adding.

## Production-Minimum Schema

For a production-relevant read-integrity digest over LoRa/satellite, the
minimum useful schema is:

- network id
- source id
- signer id and signature
- virtual DAA score or another clearly defined ordering field
- pruning point hash
- selected/sink hash with explicit semantics
- blue score
- blue work
- UTXO/state commitment with explicit scope

`pruning_proof_commitment` and `kept_headers_mmr_root` remain useful audit
extensions, but they should stay lab/optional until rusty-kaspa exposes
canonical commitments.

## Source-Of-Truth Audit

| Field/gap | Current source | Sufficient now? | Required hook if not sufficient |
| --- | --- | --- | --- |
| Pruning point hash | `getBlockDagInfo.pruning_point_hash` | Yes | None. |
| Pruning proof commitment | deterministic lab SHA-256 placeholder | No | Expose canonical commitment for `PruningProofManager::get_pruning_point_proof()` or expose the proof with a canonical sidecar commitment rule. |
| UTXO commitment / MuHash | `getBlock(sink).header.utxo_commitment` | Yes for sink-header semantics; no for exact virtual-state semantics | Expose virtual UTXO multiset commitment if production schema chooses virtual state rather than sink-header state. |
| Kept headers MMR root | omitted | No | Define kept-header MMR contents and expose the root, or remove this field from production-minimum requirements. |
| Virtual selected parent / sink | `getBlockDagInfo.sink` | Yes for receiver divergence monitor semantics | If schema chooses first virtual parent instead of sink, update producer and monitor together. |
| Blue score | `getSinkBlueScore.blue_score` | Yes for sink semantics | Expose a separate virtual score only if production schema requires it. |
| DAA score | `getBlockDagInfo.virtual_daa_score` | Yes | None. |
| Blue work | `getBlock(sink).header.blue_work` | Yes for sink-header semantics | Expose virtual blue work only if production schema requires virtual state. |

## Implemented Hook/Policy Step

The safe implementation step in this pass is devnet/simnet signer-source
policy support:

- `SnapshotFields` and `DeltaFields` now carry `signer_id`.
- `udp-live-digest-producer --signer-id N` encodes signer id `N` in produced
  snapshots/deltas.
- Receiver verification already maps signer id to the `--udp.allowed_signers`
  list index through `SignerRegistry`.
- `source_id` remains an operator-selected frame source, configured with
  `--source-id`.
- The receiver already rejects unknown signers when `--udp.require_signature`
  is enabled and rejects a source that switches signer id after first use.

This is not production signer infrastructure. It is the narrow devnet/simnet
policy needed to make the lab behavior explicit and testable.

## Remaining Blockers

- Canonical pruning proof commitment.
- Canonical kept headers commitment, or a schema decision to remove it.
- Virtual-state UTXO commitment if sink-header state is not the final scope.
- Production signer operations: key generation, rotation, revocation,
  distribution, source binding, and replay policy.
