# Phase 3 Prep Checklist

- Implement the UDP admin RPC surface (`getUdpIngestInfo`, `getUdpDigests`, `udp.enable/disable/updateSigners`) with the promised locality guard and token-file authentication so operators can observe/stop the task at runtime.
- Wire `UdpIngestService::take_reassembled_rx` into the upcoming digest/block parser so accepted frames are actually consumed before we add signature/DB logic.
- Confirm the documented framing constants (magic, version, flag bits, network tags) stay in sync across `udp/docs/prd.md`, `udp/plan.md`, and the codebase before publishing golden vectors.
- Add golden test vectors/property tests for header fragmentation and future digest payload parsing to guard against regressions once signature verification lands.
- Extend Unix-domain socket coverage to prove non-member processes cannot write to the 0640 path (e.g., via a negative test using a different UID) before shipping packages.
