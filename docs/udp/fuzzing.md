# UDP Digest Fuzzing

The UDP side-channel exposes libFuzzer harnesses for the most attack-prone surfaces: frame header parsing and DigestV1 decoding. Both harnesses live under `components/udp-sidechannel/fuzz` and share the golden vectors + malformed seeds committed with the corpora.

## Local setup

1. Install `cargo-fuzz` and a nightly toolchain once:
   ```
   cargo install cargo-fuzz
   rustup toolchain install nightly
   ```
2. Build the seeds (optional, already checked in) by running `cargo run -p kaspa-udp-sidechannel --example dump_vectors -- components/udp-sidechannel/fuzz/seeds_tmp`.
3. Run the harnesses for a quick 60s sanity pass:
   ```
   cd components/udp-sidechannel
   RUSTUP_TOOLCHAIN=nightly cargo fuzz run frame_header -- -max_total_time=60
   RUSTUP_TOOLCHAIN=nightly cargo fuzz run digest_frame -- -max_total_time=60
   ```
   Both commands reuse the shared corpora under `fuzz/corpus/*` and keep crash artifacts in `fuzz/artifacts/`.

## Harness overview

- **frame_header**: Feeds arbitrary datagrams into `SatFrameHeader::parse` with the canonical `HeaderParseContext`. The harness asserts that parsing (or rejection) never panics and benefits greatly from malformed seeds (`bad_magic`, `bad_version`, `truncated`).
- **digest_frame**: Reuses the header parser and, when the frame kind is `Digest`, invokes `DigestParser::parse`. The signer registry is populated with the same key used by the golden test vectors so valid seeds exercise the Schnorr path while malformed seeds explore edge cases.

## CI integration

`.github/workflows/udp-ci-guard.yml` builds both harnesses with `cargo fuzz build` and runs each target for ~60 seconds as part of the UDP soak gate. This keeps the crash-only regression fast while still guarding against parser regressions. Longer fuzzing campaigns can be launched manually by bumping `-max_total_time` or by pointing `cargo fuzz` at a shared artifact directory.
