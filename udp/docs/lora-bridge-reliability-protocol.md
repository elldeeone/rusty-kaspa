# LoRa Bridge Reliability Protocol

Date: 2026-05-15

This note documents the bridge-local reliability modes used by `lora-bridge`.
They are RF adapter behavior only. They do not change consensus, do not change
the recovered `KUDP` datagram bytes, and do not add `BlockV1`, transaction, or
mempool relay semantics.

## Current ACK Mode

`--reliable-fragments --reliability-mode ack` wraps every outgoing datagram in
`KLR2` frames and waits for a `KLA1` ACK after each frame.

`KLR2` data frame:

```text
bytes 0..3   magic = "KLR2"
bytes 4..7   session_id/group_id (u32 LE)
bytes 8..11  datagram_id (u32 LE)
bytes 12..13 frag_ix (u16 LE)
bytes 14..15 frag_cnt (u16 LE)
bytes 16..17 original_datagram_len (u16 LE)
bytes 18..   KUDP byte slice
```

`KLA1` ACK frame:

```text
bytes 0..3   magic = "KLA1"
bytes 4..7   session_id/group_id (u32 LE)
bytes 8..11  datagram_id (u32 LE)
bytes 12..13 frag_ix (u16 LE)
```

The sender retries the same frame until the matching ACK arrives or
`--retry-count` is exhausted. `--ack-timeout-ms` controls the wait between
attempts. `--session-id` identifies the reliability domain, and `--group-id`
overrides it for operators who prefer group naming on a shared RF channel.

The receiver accepts out-of-order fragments, detects missing fragments while
reassembly is pending, rejects malformed frames, and treats duplicate fragments
as safe. Completed datagrams are emitted byte-for-byte as the original `KUDP`
payload.

Real testnet capacity runs showed this mode has an ACK-dependent failure shape:
50 and 60 mixed live datagrams passed, while 75 and 90 runs exhausted retries on
single-fragment datagrams. Increasing ACK timeout and retry count did not improve
that boundary. The observed class is RF loss or ACK loss, not kaspad ingest
backpressure, producer burst behavior, or reassembly timeout.

## Redundant Mode

`--reliable-fragments --reliability-mode redundant` keeps the same `KLR2`
datagram/frame identifiers and receiver reassembly path, but the sender does
not wait for `KLA1` ACKs. Instead, it sends each reliable frame
`--redundant-copies` times with `--inter-frame-delay-ms` between copies.

The receiver still ACKs received frames for compatibility and observability, but
the redundant-mode sender retires a frame after its configured copy count. The
receiver suppresses duplicate fragments before completion and also keeps a
bounded cache of delivered datagram ids so a late duplicate copy cannot cause a
second UDP delivery.

This mode is intentionally simple:

- ACK loss cannot stall the sender.
- Data-frame loss can be recovered when at least one copy reaches the receiver.
- Out-of-order fragments still reassemble through the existing pending-state
  map.
- Memory is bounded by the reassembly timeout and a 512-entry delivered-id
  cache.
- Sender retirement is deterministic: `redundant-copies` copies, then advance.
- The cost is lower throughput because every frame consumes duplicate airtime.

Recommended current live testnet setting for the two-HAT lab:

```bash
--reliable-fragments \
--reliability-mode redundant \
--redundant-copies 2 \
--inter-frame-delay-ms 2500 \
--expected-datagram-ms 13000
```

This is not FEC and it is not a production RF reliability claim. It is a
bridge-local alpha hardening step that improved the measured mixed live digest
operating point from the old 50/60-datagram safe range to completed 75- and
90-datagram runs on the same hardware.

## Remaining Limits

- Redundant mode doubles the frame airtime with `--redundant-copies 2`.
- The SX126X UART modules do not expose enough per-packet diagnostics here to
  separate forward data loss from reverse ACK loss.
- There is no congestion control, RF channel sensing, FEC, encryption, or
  production key-management.
- Longer runs still need hardware-specific validation before claiming a stable
  operating envelope.
