#![no_main]

use kaspa_udp_sidechannel::digest::{DigestParser, SignerRegistry, TimestampSkew};
use kaspa_udp_sidechannel::fixtures;
use kaspa_udp_sidechannel::frame::{FrameKind, HeaderParseContext, PayloadCaps, SatFrameHeader};
use libfuzzer_sys::fuzz_target;
use once_cell::sync::Lazy;

static CTX: Lazy<HeaderParseContext> = Lazy::new(|| HeaderParseContext {
    network_tag: 0x01,
    payload_caps: PayloadCaps { digest: 4096, block: 0, tx: 0 },
});

static PARSER: Lazy<DigestParser> = Lazy::new(|| {
    let hex = fixtures::signer_hex(&fixtures::default_keypair());
    let registry = SignerRegistry::from_hex(&[hex]).expect("signer registry");
    DigestParser::new(true, registry, TimestampSkew::default())
});

fuzz_target!(|data: &[u8]| {
    if let Ok(parsed) = SatFrameHeader::parse(data, &*CTX) {
        if parsed.header.kind == FrameKind::Digest {
            let _ = PARSER.parse(&parsed.header, parsed.payload);
        }
    }
});
