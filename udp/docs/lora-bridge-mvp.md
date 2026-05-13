# LoRa Bridge MVP Handoff

This note captures the current hardware facts and the smallest useful MVP for
renewing the UDP side-channel work over a real LoRa UART link.

The goal is not to redesign the `KUDP` protocol and not to add consensus
behavior. The first LoRa milestone is a transport adapter: carry the existing
UDP side-channel datagrams over the Waveshare SX126X UART radios and recover
the original bytes on the other side.

## Hardware

Observed setup:

- 2x Waveshare SX126X LoRa HATs connected over USB.
- Each HAT enumerates as a Silicon Labs CP2102 USB-UART bridge.
- Both CP2102 bridges report the same USB serial number, `0001`.
- Do not rely on `/dev/serial/by-id` to distinguish the boards.
- Use stable path-based aliases such as `/dev/lora-left` and
  `/dev/lora-right`.

Example udev rules from the lab machine:

```udev
SUBSYSTEM=="tty", ENV{ID_VENDOR_ID}=="10c4", ENV{ID_MODEL_ID}=="ea60", ENV{ID_PATH}=="pci-0000:07:1b.0-usb-0:3:1.0", SYMLINK+="lora-left"
SUBSYSTEM=="tty", ENV{ID_VENDOR_ID}=="10c4", ENV{ID_MODEL_ID}=="ea60", ENV{ID_PATH}=="pci-0000:07:1b.0-usb-0:4:1.0", SYMLINK+="lora-right"
```

The physical left/right mapping follows the USB port path. If the boards are
unplugged and moved between ports, re-check the mapping with a UART activity
blink before testing.

## Jumper Modes

Use the USB UART selector position:

```text
UART selector: A
```

Normal transmit/receive mode:

```text
M0: shorted
M1: shorted
```

Configuration mode:

```text
M0: shorted
M1: open
```

Avoid leaving both M0 and M1 open; that is deep sleep on these modules.

With USB passthrough only, software can use the CP2102 UART but cannot
directly control M0/M1. Moving between configuration mode and transmit/receive
mode is a physical jumper operation unless M0/M1 are wired to external GPIO.

## Known Working Module Config

Both boards were configured and read back with:

```text
c1 00 09 00 00 00 62 20 41 c3 00 00
```

Decoded lab assumptions:

- UART baud: `9600`
- node address: `0`
- network ID: `0`
- LoRa air rate: `2400`
- frequency offset: `0x41`
- crypt key: `0`

The lab tests used the Waveshare fixed-send packet prefix:

```text
dest_hi dest_lo freq src_hi src_lo src_freq payload...
```

For the working config that was:

```text
00 00 41 00 00 41 payload...
```

The receiver outputs:

```text
src_hi src_lo src_freq payload... trailing_status_or_rssi_byte
```

Bridge code should strip the 3-byte source prefix and the trailing status/RSSI
byte before comparing or forwarding recovered `KUDP` datagrams.

## Measured LoRa Limits

Empirical single-send ceiling with this setup:

```text
Max application payload per LoRa send: 234 bytes
```

This comes from a 240-byte module packet setting minus the 6-byte Waveshare
fixed-send prefix.

Larger writes are silently truncated by the radio module, so the bridge must
fragment any `KUDP` datagram larger than 234 bytes before sending.

## Fit With Existing UDP Side-Channel

The existing side-channel frame header is 38 bytes (`KUDP` header). Current
frame kinds include:

- `DigestV1`
- `BlockV1`
- `Tx`

The LoRa MVP should start with `DigestV1`.

Measured from the repository's own fixture generator:

```text
delta.bin    200 bytes
snapshot.bin 329 bytes
```

Implications:

- `DigestV1` delta fits in one LoRa packet.
- `DigestV1` snapshot needs fragmentation.
- `BlockV1` is out of scope for the LoRa MVP.
- `Tx` relay is out of scope for the first pass.

Lab proof already performed:

```text
delta.bin over LoRa: exact byte-for-byte match
snapshot.bin split into 234-byte and 133-byte fragments: both fragments exact
```

## MVP 1: Single-Packet Delta Bridge

Build a small `lora-bridge` tool under `udp/tools/` that can:

1. Read a `KUDP` datagram from a file or local UDP socket.
2. Send it over a serial LoRa device such as `/dev/lora-left`.
3. Receive from a serial LoRa device such as `/dev/lora-right`.
4. Recover the original `KUDP` datagram to a file, stdout, or local UDP socket.
5. Prove the existing signed `DigestV1` delta vector survives byte-for-byte.

The first acceptance test does not require `kaspad`, mainnet, or a synced node.
It only proves that the transport can preserve the existing protocol bytes.

## MVP 2: Fragmented Snapshot Bridge

Extend the bridge with local fragmentation/reassembly for `KUDP` datagrams over
234 bytes:

1. Fragment the repository's 329-byte `DigestV1` snapshot vector.
2. Send all fragments over LoRa.
3. Reassemble the original datagram on the receiving side.
4. Prove byte-for-byte equality with the original snapshot datagram.

This fragmentation belongs in the bridge layer for the MVP. Do not change
consensus behavior and do not change the core `KUDP` wire format unless a later
design review explicitly chooses that direction.

## Later End-To-End Step

After byte-preserving LoRa transport works, the bridge can forward recovered
datagrams into the existing ingest socket:

```text
udp-generator or digest producer
  -> lora-bridge tx
  -> /dev/lora-left
  -> LoRa RF
  -> /dev/lora-right
  -> lora-bridge rx
  -> UDP 127.0.0.1:28515
  -> kaspad UDP side-channel ingest
```

That later step can use devnet or simnet. Mainnet and fully synced nodes are
not required for the MVP.

## Out Of Scope

- Full Kaspa block relay over LoRa.
- Mempool gossip over LoRa.
- Mainnet requirements.
- Synced node requirements.
- LoRaWAN.
- FEC.
- Encryption.
- Direct consensus changes.
- Direct `kaspad` LoRa integration before the standalone bridge proves the
  transport.

