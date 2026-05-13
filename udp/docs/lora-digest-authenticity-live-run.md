# LoRa Digest Authenticity Live Run

Run date: 2026-05-13

This is the committed evidence artifact for the real-state DigestV1 producer
run after adding field provenance and replacing the snapshot `utxo_muhash`
placeholder with sink header `utxo_commitment`.

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
  --report /tmp/lora-schema-live-run-2026-05-13-r2.md
```

Configuration:

- Lab progress counter: `0`
- Provenance report: `1`
- TX serial: `/dev/lora-left`
- RX serial: `/dev/lora-right`
- Signer id: `0`
- Workdir: `/tmp/lora-live-soak.YiGDMx`

Bridge result:

```text
rx datagrams_recovered=4 fragments_received=5 retries=0 duplicate_fragments=0 missing_fragments=0 corrupt_frames=0 receive_timeouts=0 reassembly_failures=0 acks_sent=5
tx datagrams_sent=4 fragments_sent=5 retries=0 receive_timeouts=0
```

Process result:

```text
live producer exit=0
lora tx exit=0
lora rx exit=0
result=complete
```

Receiver kaspad result:

```text
framesReceived=4
bytesTotal=897
sourceCount=1
signatureFailures=0
```

Receiver digest check:

```text
udp_digest_check count=4 all_signature_valid=true epoch_monotonic=true daa_score_monotonic=true virtual_blue_score_monotonic=true sources={7} signers={0}
udp_digest_compare snapshot_match=true compared_fields=8 mismatches=[]
```

Accepted snapshot fields:

```text
epoch=0
pruningPoint=4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2
pruningProofCommitment=5fe1d48e1c3812c68ef51d42c095e141efc81f85932d0de77ee9e39574914744
utxoMuhash=544eb3142c000f0ad2c76ac41f4222abbababed830eeafee4b6dc56b52d5cac0
virtualSelectedParent=4cb48d0b2073b802360145a15ad1abdc01d89b5c2fe4722630ab9b5fe9dfc4f2
virtualBlueScore=0
daaScore=0
blueWorkHex=0000000000000000000000000000000000000000000000000000000000000000
keptHeadersMmrRoot=null
signerId=0
signatureValid=true
sourceId=7
```

Producer provenance for the accepted snapshot classified:

- `epoch`: real, `getBlockDagInfo.virtual_daa_score`
- `frame_timestamp_ms`: lab-derived, producer wall clock
- `pruning_point`: real, `getBlockDagInfo.pruning_point_hash`
- `virtual_selected_parent`: real, `getBlockDagInfo.sink`
- `virtual_blue_score`: real, `getSinkBlueScore.blue_score`
- `daa_score`: real, `getBlockDagInfo.virtual_daa_score`
- `blue_work`: real, `getBlock(sink).header.blue_work`
- `utxo_muhash`: real, `getBlock(sink).header.utxo_commitment`
- `pruning_proof_commitment`: placeholder, deterministic lab SHA-256
- `kept_headers_mmr_root`: omitted
- `signer_id`, `signature`, `source_id`: lab-derived identity/signing inputs

The idle devnet node reported zero DAA/blue score throughout the run, so all
accepted digests have epoch `0`. This is real node state, not lab progression.
