# DigestV1 Wire Format

This document describes the exact on-wire representation for `DigestV1` frames
and the canonical bytes used for signature verification. The intent is to lock
the format so independent producers can interoperate with `rusty-kaspa`.

## Framing

All frames use the Phase‑2 header defined in `components/udp-sidechannel`:

```
bytes 0‥3   : magic = "KUDP"
byte  4      : version = 1
byte  5      : kind (0x01 = DigestV1, 0x02 = BlockV1)
byte  6      : network_id tag (see `UdpConfig::network_tag`)
byte  7      : flags
bytes 8‥15   : seq (little-endian u64)
bytes 16‥19  : group_id (unused for digest)
bytes 20‥21  : group_k
bytes 22‥23  : group_n
bytes 24‥25  : frag_ix
bytes 26‥27  : frag_cnt
bytes 28‥31  : payload_len
bytes 32‥33  : source_id
bytes 34‥37  : header CRC32
```

The flags used for digests are:

| Bit | Meaning                                     |
| --- | ------------------------------------------- |
| 0   | FEC present                                 |
| 1   | Fragmented                                  |
| 2   | Signed payload (must be set for DigestV1)   |
| 3   | Snapshot payload when set (`1 = snapshot`)  |

Every digest payload is stored in little-endian order.

## Snapshot Payload

```
offset size field
0      8    epoch (u64 LE)
8      8    frame_timestamp_ms (u64 LE, wall-clock)
16     32   pruning_point (Hash32)
48     32   pruning_proof_commitment (Hash32)
80     32   utxo_muhash (Hash32)
112    32   virtual_selected_parent (Hash32)
144    8    virtual_blue_score (u64 LE)
152    8    daa_score (u64 LE)
160    32   blue_work (opaque 32 bytes, big-endian numeric)
192    1    kept_headers_mmr_root flag (0 = absent, 1 = present)
193    32   kept_headers_mmr_root (only when flag == 1)
193/225 2   signer_id (u16 LE)
195/227 64  schnorr signature (see below)
```

## Delta Payload

```
offset size field
0      8    epoch (u64 LE)
8      8    frame_timestamp_ms (u64 LE)
16     32   virtual_selected_parent (Hash32)
48     8    virtual_blue_score (u64 LE)
56     8    daa_score (u64 LE)
64     32   blue_work
96     2    signer_id (u16 LE)
98     64   schnorr signature
```

## Signature Canonicalisation

Every digest is signed with secp256k1 Schnorr and SHA‑256, using domain
separation string:

```
const DIGEST_SIG_DOMAIN = "kaspa/udp-digest/v1";
```

The canonical byte sequence (before hashing) is:

```
domain bytes
variant marker (1 for snapshot, 2 for delta)
header.seq (u64 LE)
epoch (u64 LE)
frame_timestamp_ms (u64 LE)
snapshot-only:
    pruning_point (Hash32)
    pruning_proof_commitment (Hash32)
    utxo_muhash (Hash32)
    virtual_selected_parent (Hash32)
delta-only:
    virtual_selected_parent (Hash32)
virtual_blue_score (u64 LE)
daa_score (u64 LE)
blue_work (32 raw bytes)
snapshot-only:
    kept_headers_mmr_root flag byte (0/1) + optional hash
header.source_id (u16 LE)
```

The SHA‑256 digest of these bytes is fed to secp256k1 Schnorr verification.

## Snapshot vs Delta Flag

The header bit `flags.digest_snapshot()` (bit 3) distinguishes snapshots
(`1`) from deltas (`0`). Producers **must** set the bit accordingly.

## Compatibility

* All integers are little-endian.
* `Hash32` values are transmitted as raw 32-byte arrays.
* `blue_work` is treated as an opaque 32-byte big-endian integer; nodes store
  it verbatim.
* The signature domain is fixed at `kaspa/udp-digest/v1`. Any future format
  will use a different domain string and version byte to avoid collisions.
* Golden interoperability vectors (valid + tampered cases) live in
  `components/udp-sidechannel/tests/vectors.rs`. Producers must match these
  fixtures to be considered wire-format compatible.
