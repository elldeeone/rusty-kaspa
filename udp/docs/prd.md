# Product Requirements Document (PRD)

**Feature:** UDP Side‑Channel Ingest for `rusty‑kaspa` (satellite‑first)
**Branch:** `udp`
**Owner:** Luke
**Stakeholders:** Networking, Consensus, DevOps/Infra, Wallets
**Rollout:** Devnet/Testnet first (feature‑gated); mainnet later if warranted
**Status:** Developer‑ready (v1.0)

---

## 1) Problem Statement

During severe outages or censorship, normal TCP P2P paths can be unreliable. We want a **parallel, receive‑only UDP side‑channel** that a node can consume alongside P2P. In v1 this will be fed by a **satellite downlink**, but the design must remain transport‑agnostic so LoRa/HF/mesh gateways can reuse it later. The payload of this side‑channel is primarily **compact consensus digests** (for global “read” integrity) with an *optional* block payload path for lab/testnet use.

---

## 2) Goals & Non‑Goals

### Goals

1. **Parallel ingest:** Node listens on localhost UDP for structured frames while continuing normal TCP P2P.
2. **Digest mode (primary):** Ingest, verify, store, and surface compact consensus digests at **~10 kbps** sustained (**≤ 50 kbps** hard cap).
3. **Optional block mode:** Reassemble and inject block payloads into existing validation flows for dev/test (may be disabled in prod).
4. **Safety:** UDP input is untrusted and cannot bypass consensus; blocks still undergo full validation.
5. **Operability:** Simple config, metrics, and RPCs; default **off**.
6. **Phased rollout:** Devnet/Testnet only at first; mainnet decision after telemetry.

### Non‑Goals

* No new consensus rules or tokenomics.
* No guarantee of IBD via UDP.
* No commitment to maintain “block mode” in production.

---

## 3) Assumptions & Constraints

* **Data rate targets:**

  * **Digest mode:** design for ~**10 kbps** continuous; enforce **≤ 50 kbps** cap.
  * **Block mode (opt):** assume **256 kbps–1 Mbps** channels (test uses).
* **Transport:** v1 producer is a satellite receiver stack that presents a repaired UDP stream to localhost. Future producers (LoRa/HF/mesh) can map into the same UDP port.
* **Security posture:** Treat UDP frames as hostile; require signed digests on testnet; never grant priority over TCP peers.

---

## 4) User Stories

* **Operator:** “When I enable UDP ingest, my node shows a live, signed digest heartbeat even if my internet is flaky.”
* **Engineer:** “I can replay a UDP capture into a node and see the same digests/blocks flow through normal validation.”
* **Auditor/Forensics:** “I can query stored digests and later check they match data fetched over P2P.”

---

## 5) Success Metrics (Testnet SLOs)

* **Staleness:** Latest accepted digest ≤ **120 s** old during healthy broadcast.
* **Overhead:** CPU ≤ **+15%** vs baseline at 10 kbps; memory bounded (no unbounded queues).
* **Safety:** No consensus deviations attributable to UDP input in integration tests.
* **Observability:** `getUdpIngestInfo` exposes enough for alerting on silence, signature failures, or divergence.

---

## 6) Scope

### In‑Scope

* UDP listener, frame header parsing, optional FEC group handling, fragmentation, bounded queues, de‑dup.
* **Digest mode** end‑to‑end: signature verify, persistence, RPCs/metrics, divergence signalling.
* **Block mode (optional):** Reassembly and injection via a virtual P2P peer.
* Feature flag, config, logging, docs, tests.

### Out‑of‑Scope

* RF demod/FEC; satellite leases/teleports; any hardware.
* New cryptographic primitives beyond readily available crates (we’ll note TBDs).

---

## 7) Functional Requirements

### FR‑1: Enable/Disable & Modes

* Config/TOML & CLI:

  * `--udp.enable=[true|false]` (default **false**)
  * `--udp.listen=127.0.0.1:28515`
  * `--udp.mode=[digest|blocks|both]` (default **digest**)
  * `--udp.max_kbps=10`
  * `--udp.require_signature=true` (testnet default)
  * `--udp.allowed_signers=[hex,...]`
  * Queues: `--udp.digest_queue=1024`, `--udp.block_queue=32`

### FR‑2: Framing & Parsing

* Frame header with: magic, version, kind (`DigestV1`/`BlockV1`), network id, flags, seq, optional FEC group (K/N), fragmentation (ix/cnt), payload_len, header CRC32.
* Reject wrong network id, bad CRC, unknown version/kind, oversize payloads.

### FR‑3: De‑dup & Ordering

* Sliding windows per kind keyed by `(seq, content‑hash)`; tolerate reordering; drop duplicates and stale seq.

### FR‑4: Digest Mode (Primary)

* Verify signatures against `allowed_signers` when required.
* Persist latest N digests (configurable).
* Provide RPCs:

  * `getUdpIngestInfo` (status/health snapshot).
  * `getUdpDigests { from_epoch?: u64, limit?: u32 }`.
* Do not mutate consensus from digests; use for hints/telemetry and later verification.

### FR‑5: Block Mode (Optional)

* Reassemble `BlockV1` payloads into `pb::BlockMessage` (or `pb::BlockHeaders` + compact body if implemented).
* Inject via a **virtual Router peer** so blocks traverse existing flows and validation.
* Respect bounded queues and avoid starving TCP peers.

### FR‑6: Divergence Signalling (Advisory)

* If latest **verified** snapshot digest conflicts with local virtual/pruning point, set a divergence flag (RPC/metrics).

### FR‑7: Rate Control & Safety

* Enforce `max_kbps` by downsampling deltas before snapshots; never exceed hard caps; drop gracefully with counters.

---

## 8) Non‑Functional Requirements

* **Security:** Signed digests (testnet), strict parser limits, defensive defaults, no consensus bypass.
* **Performance:** Bounded memory, steady CPU; back‑pressure on queues.
* **Compatibility:** Default off; on → coexists with current P2P.
* **Observability:** Structured logs + Prometheus metrics + RPCs.

---

## 9) Interfaces

### 9.1 Config (TOML)

```toml
[udp]
enable = true
listen = "127.0.0.1:28515"
mode = "digest"
max_kbps = 10
require_signature = true
allowed_signers = ["02ab...","03cd..."]
digest_queue = 1024
block_queue = 32
```

### 9.2 RPCs

* **`getUdpIngestInfo` → { ... }**

  * `enabled, mode, bind, rx_kbps`
  * `frames_received, frames_dropped{reason}, last_frame_ts_ms`
  * `last_digest: { epoch, pruning_point, vsp, daa_score, virtual_blue_score, signer_id, sig_ok, recv_ts_ms } | null`
  * `divergence: { detected, last_mismatch_epoch | null }`
* **`getUdpDigests { from_epoch?: u64, limit?: u32 }` → [digest records]**

### 9.3 Metrics (Prometheus)

* `udp_frames_total{kind}`
* `udp_frames_dropped_total{reason}`
* `udp_rx_kbps`
* `udp_digest_latest_epoch`
* `udp_digest_sig_fail_total`
* `udp_divergence_detected` (0/1)
* `udp_block_injected_total`

---

## 10) Architecture

### 10.1 Diagram

```mermaid
flowchart LR
  U[UDP localhost] --> ING[udp_ingest (tokio)]
  ING --> FR[frame_reassembler]
  FR -->|Digest| DP[digest_parser + verify]
  FR -->|Block (opt)| BP[block_parser + reassembly]
  DP --> DS[digest_store (RocksDB CF)]
  BP --> IQ[inject_queue] --> VR[Virtual Router peer] --> P2P[Existing flows] --> CM[Consensus Manager] --> CP[Consensus pipeline] --> DB[(RocksDB)]
```

### 10.2 Components

* **udp_ingest:** Binds socket; tracks rate; dispatches to reassembler.
* **frame_reassembler:** Fragment/FEC windows; emits complete payloads.
* **digest_parser:** Parses/validates `DigestV1`; signature check; persists; updates metrics.
* **block_parser (opt):** Parses block payloads; builds P2P messages.
* **Virtual Router peer:** In‑process peer that reuses existing P2P flows.

---

## 11) Data Model

### 11.1 Digest record (stored)

* `network_id: u8`
* `epoch: u64` (e.g., blue‑score/DAA epoch)
* `pruning_point: Hash32`
* `pruning_proof_commitment: Hash32`
* `utxo_muhash: Hash32`
* `virtual_selected_parent: Hash32`
* `virtual_blue_score: u64`
* `daa_score: u64`
* `blue_work: [u8; 32]` (or compressed)
* `kept_headers_mmr_root: Hash32` (optional/TBD)
* `signer_id: u16`
* `sig_ok: bool`
* `recv_ts_ms: u64`

**Storage:** new RocksDB CF `udp_digest` (+ small meta CF for head pointers and signer set).

### 11.2 Frame Header (illustrative)

```
magic="KUDP" | ver=1 | kind=DigestV1|BlockV1 | network_id | flags
seq:u64 | group_id:u32 | group_k:u16 | group_n:u16
frag_ix:u16 | frag_cnt:u16 | payload_len:u32 | header_crc32:u32
payload[..payload_len]
```

---

## 12) Security / Threat Model

* **Local spoofing:** Require signatures for digests (testnet). Allow unsigned only in lab with explicit flag.
* **DOS:** Strict size caps; bounded queues; early drops on parse errors; rate limiting.
* **Priority:** Virtual peer marked low priority; cannot starve TCP peers.
* **Trust:** Digests are advisory; blocks must pass full validation.

---

## 13) Acceptance Criteria

* **AC‑1:** With UDP disabled, node behaviour identical to baseline.
* **AC‑2:** With digest mode enabled, node accepts signed digests, persists them, exposes RPCs/metrics, and never mutates consensus from digests.
* **AC‑3:** Sustained 10 kbps stream processed with ≤ 15% CPU overhead vs baseline; memory bounded under long runs.
* **AC‑4:** Duplicate/replayed frames cause no unbounded growth; counters increment correctly.
* **AC‑5 (opt):** In block mode on devnet, injected blocks yield identical chainstate vs TCP path.
* **AC‑6:** Divergence flag set when digest snapshot conflicts with local state; clears on reconciliation.
* **AC‑7:** Parser fuzz tests show no panics; invalid frames are safely rejected.

---

## 14) Test Plan

### Unit

* Header parse/CRC/version/kind; fragmentation join; seq windows; signature verification.

### Integration

* **Digest‑only:** Replay synthetic stream (1 snapshot / 30–60 s; 1 delta / s) for ≥ 10 min; verify RPC/metrics; simulate silence to test staleness alert.
* **Adverse:** 10–20% loss; reorder; duplicate/replay; bad signatures; oversize payloads.
* **Block mode (opt):** Inject a small devnet block set; assert identical DAG state to TCP.

### Performance

* Soak at 10 kbps for 24 h; check memory boundedness; CPU headroom; queue health.

---

## 15) Rollout Plan

* **M1 – Digest path (core):**
  UDP bind → header parser → digest verify/store → RPC/metrics → fuzz/unit tests.

* **M2 – Virtual Router (optional blocks):**
  Virtual peer → block injection → devnet tests → bounded queues.

* **M3 – Hardening & Docs:**
  Adverse‑condition tests; rate cap enforcement; operator guide; dashboards.

* **M4 – Testnet Pilot:**
  Fixed signer keys; public builds; telemetry review → mainnet go/no‑go.

---

## 16) Risks & Mitigations

* **Parser bugs / crashes:** Fuzzing, strict bounds, early exits, comprehensive tests.
* **Local DOS via UDP spam:** Signatures, rate caps, bounded queues, low‑priority injection.
* **Scope creep (transports):** Keep v1 strictly UDP; treat satellite as the first producer; defer transport‑specific code.
* **Digest crypto TBDs:** Start with pragmatic primitives (e.g., secp256k1 Schnorr; BLAKE3/SHA‑256); document upgrade path.

---

## 17) Open Questions

1. Exact digest commitments (e.g., MMR for kept headers) and hash family choice.
2. Snapshot/delta cadence defaults and adaptive back‑off policy.
3. Retention policy (rolling window vs full archive) for `udp_digest`.
4. Whether “block mode” remains after testnet or stays a dev‑only tool.
5. Signer key distribution/rotation process on testnet.

---

## 18) Deliverables

* Code: new module/crate **`kaspa-udp-sidechannel`** (name flexible), feature flag, config, RPCs, metrics, tests.
* Documentation: operator runbook, developer notes (module boundaries, adding fields), security note.
* CI: unit + integration + fuzz harness.
* Dashboards: sample Prometheus/Grafana panels.

---

## Appendix A — “Transport” Note (to include near the top)

> **Transport:** v1 feeds this UDP side‑channel from a satellite downlink.
> The implementation must remain transport‑agnostic: any future gateway (LoRaWAN/HF/mesh) may deliver the same framed UDP packets to `127.0.0.1:PORT`. No transport‑specific code lives in the node.

---

## Appendix B — Minimal PoC (no satellite)

**Goal:** Prove ingest path works with a local generator.

1. Start node with:

   ```
   --udp.enable=true --udp.mode=digest --udp.listen=127.0.0.1:28515
   ```
2. Run a tiny generator that sends valid `DigestV1` frames to that port at ~1 Hz.
3. Call `getUdpIngestInfo`; confirm counters advance; digests stored; no consensus mutation.