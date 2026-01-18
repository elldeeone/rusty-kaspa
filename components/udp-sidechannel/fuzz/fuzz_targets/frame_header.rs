#![no_main]

use kaspa_udp_sidechannel::frame::{HeaderParseContext, PayloadCaps, SatFrameHeader};
use libfuzzer_sys::fuzz_target;
use once_cell::sync::Lazy;

static CTX: Lazy<HeaderParseContext> = Lazy::new(|| HeaderParseContext {
    network_tag: 0x01,
    payload_caps: PayloadCaps { digest: 4096, block: 131_072, tx: 4096 },
});

fuzz_target!(|data: &[u8]| {
    let _ = SatFrameHeader::parse(data, &*CTX);
});
