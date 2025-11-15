# UDP Side-Channel Audit — Phases 1 & 2

## Executive summary
Phase 1/2 scaffolding is largely in place: config/CLI wiring, binding safety, header/assembler/runtime logic, and the ingest service pipeline all conform to the plan, and the test suite plus workspace checks are green. Two gaps remain: the promised admin RPC/locality guard (Phase 1 item 2) is entirely absent, and frame counters are recorded before the bounded mpsc accepts a message, which misreports throughput under back-pressure. The former is a blocking defect, the latter is a medium-risk observability bug with a small patch prepared.

## Checklist
| # | Item | Status | Evidence | Notes |
|---|------|--------|----------|-------|
| 1 | Config & CLI defaults, mutual exclusion, tests | ✅ | `kaspad/src/args.rs:117-208,273-327,493-652`; `components/udp-sidechannel/src/service.rs:592-658` | `[udp]` defaults match the PRD, CLI enforces `listen` vs `listen_unix`, and unit tests cover CLI/TOML plus the non-local bind rejection. |
| 2 | Admin RPC locality guard & token overrides | ❌ | `components/udp-sidechannel/src/config.rs:24-43`; `kaspad/src/args.rs:136-207`; no RPC references in `rpc/*` | Config + CLI expose `admin_remote_allowed`/`admin_token_file`, but there are zero RPC handlers (`git grep` finds no `udp` admin RPCs), so the Phase 1 admin controls/locality guard are missing entirely. |
| 3 | Runtime wiring gated on `--udp.enable` | ✅ | `kaspad/src/daemon.rs:633-706` | `UdpIngestService` is only registered when the flag is true; disabled builds never spawn the task, keeping default behaviour unchanged. |
| 4 | Bind safety & Unix socket hygiene | ✅ | `components/udp-sidechannel/src/service.rs:107-205,592-658` | Loopback binds are enforced unless `allow_non_local_bind`, Unix sockets get 0640 perms and are removed via `UnixSocketGuard`; tests cover non-loopback rejection and Unix perms. |
| 5 | Logging & metrics skeleton | ✅ | `components/udp-sidechannel/src/service.rs:32-70,366-409`; `components/udp-sidechannel/src/metrics.rs:6-40`; `grep -R 'udp.event' … = 14` | Structured `udp.event=*` logs exist for binds, frames, drops, and CRC storms, labels are bounded, and static greps confirm no payload/signer exposure in info/warn logs. |
| 6 | Header parser (magic/version/network/payload cap) | ✅ | `components/udp-sidechannel/src/frame/header.rs:4-232,268-296` | Parser enforces `KUDP`, version==1, network tags, CRC/payload caps before buffering, exposes `digest_snapshot` flag, and emits bounded drop reasons with rate-limited future-version warnings. |
| 7 | Fragmentation/FEC assembler bounds & drops | ✅ | `components/udp-sidechannel/src/frame/assembler.rs:6-164,166-205` | Fragment groups are bounded by max_groups/buffer bytes, expire via TTL, treat `fec_present` groups specially, and drop with `fragment_timeout`/`fec_incomplete`; out-of-order handling is covered by the unit test. |
| 8 | Runtime dedup + token bucket limiter | ✅ | `components/udp-sidechannel/src/runtime/mod.rs:6-247,284-341` | Per-kind dedup windows key on `(seq, blake3 payload hash)`, stale/duplicate decisions map to bounded reasons, and the token bucket uses monotonic `Instant` with snapshot overdraft and `DropClass` only in logs. |
| 9 | Service integration & bounded queues | ⚠ | `components/udp-sidechannel/src/service.rs:39-90,524-547`; patch `udp/docs/patches/01-fix-frame-metrics.patch` | Parser→assembler→runtime pipeline works, but `record_frame` runs before `try_send`, so queue drops still inflate frame counters; the attached patch defers metrics until the send succeeds. |
| 10 | Unit/integ tests (header/assembler/runtime/service/adverse) | ✅ | `kaspad/src/args.rs:273-327`; `components/udp-sidechannel/src/frame/header.rs:268-296`; `components/udp-sidechannel/src/frame/assembler.rs:166-205`; `components/udp-sidechannel/src/runtime/mod.rs:284-341`; `components/udp-sidechannel/src/service.rs:592-658`; `components/udp-sidechannel/tests/adverse.rs:1-120` | Coverage matches the checklist: parser bounds, assembler TTL/FEC, runtime dedup/rate-limit, bind safety, and an adverse stream test that drives wrong-network/oversize/fragment timeout scenarios. |
| 11 | Observability hygiene (bounded labels, rate-limited logs, no payload leaks) | ✅ | `components/udp-sidechannel/src/service.rs:366-409,455-520`; greps from Step B | Drop logging is rate-limited for future-version/CRC storms, metrics expose fixed label sets, and no info/warn logs include payload or signature data (`grep -R 'payload' … | grep -Ei 'log|info|warn'` returned empty). |
| 12 | Dependency & code quality | ✅ | `components/udp-sidechannel/Cargo.toml:1-24`; `rg -n 'unsafe' components/udp-sidechannel/src` (no matches) | The crate only depends on core/unasync-safe crates (tokio/bytes/etc.), MSRV matches workspace, and there is no `unsafe` code on the ingest path. |

## Findings & fixes
1. **High – Admin RPCs and locality guard missing entirely (Phase 1.2)**  
   *Evidence:* `udp/plan.md:245-248` mandates `getUdpIngestInfo`, `udp.enable/disable/updateSigners`, plus locality controls keyed off `--udp.admin_remote_allowed`/`--udp.admin_token_file`, yet the only references are the config/CLI structs (`components/udp-sidechannel/src/config.rs:24-43`, `kaspad/src/args.rs:136-208`). No RPC module or guard exists, so operators cannot observe/disable the ingest task at runtime, nor can they rely on the advertised locality/tokens.  
   *Fix guidance:* Implement the UDP admin RPC surface in `rpc/{core,service}` (status queries + enable/disable/updateSigners), wire it through the gRPC/wRPC layers, and enforce remote access via the global admin flag or the UDP-specific override + token file. Add tests for loopback success vs remote rejection, plus token-file validation.  
   *Patch:* Not included here because the change requires multi-crate RPC additions and coordination with the runtime.

2. **Medium – Frame counters increment before the mpsc accepts a message**  
   *Evidence:* `components/udp-sidechannel/src/service.rs:524-543` calls `self.metrics.record_frame` before reserving queue space or checking `try_send`. When the queue is saturated (or closed), `queue_full` drops still increment `frames_total`, so throughput metrics and rate calculations are overstated exactly when operators debug back-pressure.  
   *Fix:* Move `record_frame` to run only after `try_send` succeeds. The ready-to-apply diff lives in `udp/docs/patches/01-fix-frame-metrics.patch` and simply wraps the send in a `match`, logging/recording metrics only on `Ok(())`.

## Tests run
- `cargo fmt -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace -q` (second run with longer timeout; noisy UPnP warnings only)
- `cargo test -p kaspa-udp-sidechannel -- --nocapture`
- `cargo test -p kaspad -- --nocapture`
- `grep -R 'udp.event' components/udp-sidechannel/src | wc -l` → `14`
- `grep -R 'payload' components/udp-sidechannel/src | grep -Ei 'log|warn|info'` → *(no matches)*
- `grep -R 'labels' components/udp-sidechannel/src/metrics.rs` → *(no matches)*
