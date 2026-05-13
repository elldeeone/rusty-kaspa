# UDP Branch Review Readiness

Date: 2026-05-13

This is the review entrypoint for the `udp` branch. It separates mergeable core
UDP side-channel work from LoRa lab tooling and experimental block/tx paths.
Start here before reading the older phase notes and lab reports.

## Branch Inventory

Relative to `origin/master`, the branch adds or changes these areas:

| Area | Files / modules | Review posture |
| --- | --- | --- |
| Workspace/package wiring | root `Cargo.toml`, `Cargo.lock`, `kaspad/Cargo.toml`, tool manifests | Required for all staged pieces. Review dependency additions closely. |
| Core UDP side-channel crate | `components/udp-sidechannel/*` | Main review target for framing, runtime, queues, digest parsing/storage, metrics, and tests. |
| Kaspad runtime/config | `kaspad/src/args.rs`, `kaspad/src/daemon.rs`, `kaspad/src/udp.rs`, `kaspad/src/udp_tx.rs` | Default-off runtime wiring. Divergence monitor belongs with digest ingest; tx submitter is experimental. |
| RPC surface | `rpc/core/*udp*`, `rpc/grpc/*`, `rpc/wrpc/*`, `rpc/service/src/service.rs`, `cli/src/modules/rpc.rs` | Required for observability/admin operations. Review locality/token guard and serialization compatibility. |
| Database | `database/src/udp_digest.rs`, registry/lib additions | Stores accepted digests only when migration is explicitly enabled. Review schema guard and pruning. |
| Metrics | `metrics/core/src/data.rs`, UDP custom metrics in RPC service | Bounded counters/gauges only; no dynamic field labels. |
| P2P/flow injection | `components/connectionmanager/src/injector.rs`, `protocol/flows/*block_injector*`, `sat_virtual_peer.rs` | Experimental block path. Do not merge with core digest stage unless reviewers explicitly accept it. |
| UDP tools | `udp/tools/generator/*`, `udp/tools/soak.sh` | Lab/dev tooling. Useful for verification, not production runtime. |
| LoRa bridge/tools | `udp/tools/lora-bridge/*`, `lora_live_soak_lab.sh`, `lora_reliability_harness.sh` | Lab-only hardware tooling. Keep separate from core merge. |
| Docs/reports | `udp/docs/*`, `docs/perf/udp.md`, `docs/udp/*`, `udp/plan.md`, `udp/research/gpt.md` | Evidence plus design context. Older reports are evidence snapshots, not primary runbooks. |
| CI/fuzz | `.github/workflows/udp-ci-guard.yml`, `components/udp-sidechannel/fuzz/*` | Good parser/runtime guardrail; review CI cost before enabling on mainline. |
| Experimental flags | `--udp.danger_accept_blocks`, `--udp.tx_enable`, `--udp.allow_non_local_bind`, `--udp.admin_remote_allowed`, `--udp.db_migrate` | Must remain default-off and documented as non-production unless promoted later. |

## Staged Merge Plan

### Stage A: Core UDP Side-Channel Framing/Runtime/Digest Storage/RPC

- Purpose: introduce default-off UDP digest ingest, parser/runtime safety,
  signed `DigestV1` acceptance, storage, RPC/admin status, and bounded metrics.
- Risk: medium. It touches `kaspad`, RPC models, database registry, and a new
  component crate, but should not affect consensus while disabled.
- Reviewers: node runtime, RPC, database, and security/safety reviewers.
- Required tests: `cargo test -p kaspa-udp-sidechannel`, RPC/admin tests,
  vector/adverse/crash-only tests, `cargo build -p kaspad`.
- Feature/config gate: runtime disabled by default via `--udp.enable=false`;
  storage migration guarded by `--udp.db_migrate`.
- Production/lab status: production-shaped alpha, not mainnet-ready without
  signer/source operations and incident runbooks.

### Stage B: Digest Producer/Helper Tools

- Purpose: provide devnet/simnet fixture generation, live digest production,
  and RPC inspection/comparison helpers.
- Risk: low to medium. Tools are outside runtime but can mislead if lab fields
  are read as production-authentic.
- Reviewers: tooling and docs reviewers; RPC reviewer for helper assumptions.
- Required tests: `cargo test -p udp-generator --bin udp-live-digest-producer
  --bin udp-rpc-digests`, generator builds.
- Feature/config gate: no runtime gate; keep tools under `udp/tools`.
- Production/lab status: lab-only. `udp-live-digest-producer` explicitly rejects
  mainnet.

### Stage C: LoRa Bridge/Lab Tooling

- Purpose: demonstrate byte-exact UDP digest transport over SX126X LoRa with
  bridge-local ACK/retry, harnesses, and soak evidence.
- Risk: low for main node if kept isolated; high if treated as production RF
  reliability.
- Reviewers: tooling/hardware reviewers; node reviewers only for ingest
  interfaces used by the lab.
- Required tests: `cargo build -p lora-bridge`, `cargo test -p lora-bridge`,
  script syntax checks, hardware reports when changing RF behavior.
- Feature/config gate: separate binary and scripts; no `kaspad` runtime path.
- Production/lab status: lab-only.

### Stage D: Divergence Monitor

- Purpose: receiver-native comparison of verified snapshots against local
  consensus fields and `getUdpIngestInfo.divergence` reporting.
- Risk: low to medium. It reads consensus state but must never mutate it.
- Reviewers: consensus API/runtime reviewers and RPC reviewers.
- Required tests: sidechannel divergence state tests, `cargo build -p kaspad`,
  two-node devnet agreement/divergence lab evidence.
- Feature/config gate: monitor starts only with digest manager.
- Production/lab status: alpha diagnostic. It currently evaluates verified
  snapshots only.

### Stage E: Experimental Block/Tx Paths

- Purpose: early BlockV1 virtual-peer and tx submission experiments.
- Risk: high relative to digest ingest because these paths can affect node
  behavior if enabled.
- Reviewers: consensus, mempool, P2P, RPC, and security reviewers.
- Required tests: existing flow harness plus broader integration tests; do not
  rely on LoRa digest labs.
- Feature/config gate: keep default-off. Block mode additionally requires
  `--udp.danger_accept_blocks`; tx submission is now disallowed on mainnet even
  when `--udp.tx_enable=true`.
- Production/lab status: not merge-ready with Stage A. Split or leave out unless
  reviewers explicitly take it on.

## Safety Audit

- Default-off: UDP ingest defaults to disabled. UDP listen has defaults, but the
  service does not bind until enabled.
- Bind protection: non-loopback UDP bind is rejected unless
  `--udp.allow_non_local_bind=true`; Unix socket mode is available for local
  deployments.
- Admin RPC protection: UDP admin/status RPCs require loopback/Unix unless
  `--udp.admin_remote_allowed=true`; remote admin can require
  `--udp.admin_token_file`.
- Queues/memory: digest, block, and tx queues are bounded; payload caps are
  configured separately for digest/block/tx.
- Parser limits: frame header parser validates magic/version/kind/network/CRC,
  fragment counts, lengths, and payload caps before dispatch.
- Rate limiting: runtime rate cap is covered by `dedup_and_ratecap`.
- Duplicate/stale handling: runtime and digest manager reject duplicates and
  non-monotonic source epochs.
- Signature behavior: digest ingest defaults to requiring signatures and can
  restrict signer public keys. Unsigned/unverified snapshots do not feed trusted
  divergence state.
- Signer/source policy: signer id and source id are surfaced in RPC/helper
  output; production source operations remain unresolved.
- Divergence safety: monitor reads receiver consensus state and updates UDP
  status only; it does not mutate consensus.
- Database migration: digest store requires explicit migration when enabled;
  startup is safe when the store is disabled.
- Lab-only flags: live producer divergence/progression flags are documented as
  lab-derived and should not be used for authenticity claims.
- Safety fix made in this pass: UDP tx submission is now rejected on mainnet via
  `UdpConfig::tx_allowed()` even when explicitly enabled.

## Test Audit

| Risk | Current evidence | Gap / action |
| --- | --- | --- |
| Header/parser bounds | `frame::header` tests, fuzz corpus, `tests/crash_only.rs` | Adequate for review; run longer fuzz separately before production. |
| Fragment assembly/loss/reorder | `frame::assembler` tests and adverse stream test | Adequate for Stage A. |
| Digest signatures/vectors | `tests/vectors.rs`, parser tests through manager | Adequate for allowed-signer and invalid-signature cases. |
| Storage/retention | `digest::store` tests | Adequate for count/day pruning and migration guard. |
| RPC/admin locality/token | `rpc/service/src/service.rs` unit tests | Adequate for status/admin guard review. |
| Queue shutdown/lifecycle | `ingest_shutdown`, flow shutdown tests | Adequate for current scope. |
| LoRa bridge reliability | `cargo test -p lora-bridge`, hardware reports | Good for lab alpha, not production RF proof. |
| Divergence monitor | sidechannel divergence state tests plus real two-node LoRa reports | Build/runtime evidence exists; deeper unit seams for monitor comparison would help later. |
| Block/tx paths | flow harness and config gates | Not enough for merge with Stage A; Stage E needs separate review. |
| Mainnet safety for tx | Missing before this pass | Added `tx_submit_disallowed_on_mainnet_unit` and testnet positive case. |

## Documentation Map

- Current review entrypoint: this file.
- Core design context: `udp/plan.md`, `udp/docs/prd.md`.
- Wire format: `udp/docs/digest-wire-format.md`.
- Operator/prototype runbook: `udp/docs/lora-prototype.md`.
- LoRa reliability evidence: `udp/docs/lora-reliability-report.md`.
- Completed live soak evidence: `udp/docs/lora-live-soak-report.md`.
- Digest authenticity audit: `udp/docs/lora-digest-authenticity.md` and
  `udp/docs/lora-digest-schema-decision.md`.
- Native divergence lab: `udp/docs/lora-two-node-divergence-lab.md`.
- Older phase notes and research files are background evidence, not the primary
  reviewer path.

## Real Hardware Evidence

- Reliable LoRa byte-equality sweep: 100 deltas and 100 snapshots passed at
  250 ms with reliable fragments.
- 30-minute live soak: 276 produced, recovered, and ingested datagrams with zero
  signature failures, corrupt frames, or reassembly failures.
- Two-node agreement: 4/4 recovered/ingested,
  `divergence_detected=false`.
- Two-node controlled divergence: 4/4 recovered/ingested,
  `virtual_blue_score` mismatch detected by helper and native
  `getUdpIngestInfo.divergence`.

## Remaining Review Risks

- The branch is too broad for a single comfortable merge; split or stage it.
- RPC/protobuf/wRPC changes need compatibility review.
- Database migration and retention behavior needs DB reviewer sign-off.
- Block/tx paths are experimental and should not ride along with the core
  digest ingest merge.
- Production signer/key management, canonical pruning-proof commitments,
  kept-header MMR semantics, alerting, and mainnet operations are still open.
- LoRa bridge evidence is hardware-lab evidence only; it is not a production RF
  reliability claim.

## Review Commands

```bash
cargo fmt --check
cargo test -p kaspa-udp-sidechannel
cargo test -p udp-generator --bin udp-live-digest-producer --bin udp-rpc-digests
cargo build -p udp-generator --bin udp-live-digest-producer --bin udp-rpc-digests --bin udp-digest-fixtures
cargo build -p lora-bridge
cargo test -p lora-bridge
cargo build -p kaspad
```
