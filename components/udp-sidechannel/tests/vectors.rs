use kaspa_core::time::unix_now;
use kaspa_udp_sidechannel::{
    digest::{DigestParser, DigestVariant, SignerRegistry, TimestampSkew},
    fixtures::{
        self, build_delta_vector, build_snapshot_vector, delta_fields, signer_hex, snapshot_fields, DEFAULT_SNAPSHOT_SEQ,
        DEFAULT_SOURCE_ID,
    },
    frame::{FrameKind, SatFrameHeader},
    DigestError,
};

#[test]
fn snapshot_roundtrip() {
    let parser = build_parser();
    let keypair = fixtures::default_keypair();
    let fields = snapshot_fields(42, unix_now(), true);
    let vector = build_snapshot_vector(DEFAULT_SNAPSHOT_SEQ, DEFAULT_SOURCE_ID, &fields, &keypair);
    let variant = parser.parse(&vector.header, &vector.payload).expect("snapshot parsed");
    match variant {
        DigestVariant::Snapshot(snapshot) => {
            assert_eq!(snapshot.epoch, fields.epoch);
            assert_eq!(snapshot.pruning_point, fields.pruning_point);
            assert!(snapshot.signature_valid);
        }
        _ => panic!("expected snapshot"),
    }
}

#[test]
fn signature_valid_and_invalid_vectors() {
    let parser = build_parser();
    let keypair = fixtures::default_keypair();
    let first_delta = delta_fields(43, unix_now());
    let vector = build_delta_vector(DEFAULT_SNAPSHOT_SEQ + 1, DEFAULT_SOURCE_ID, &first_delta, &keypair);
    let variant = parser.parse(&vector.header, &vector.payload).expect("delta parsed");
    match variant {
        DigestVariant::Delta(delta) => {
            assert!(delta.signature_valid);
            assert_eq!(delta.epoch, first_delta.epoch);
        }
        _ => panic!("expected delta"),
    }

    let mut tampered = vector.payload.clone();
    *tampered.last_mut().unwrap() ^= 0x01;
    let err = parser.parse(&vector.header, &tampered).expect_err("invalid signature rejected");
    assert!(matches!(err, DigestError::SignatureVerificationFailed));

    let second_delta = delta_fields(44, unix_now() + 100);
    let vector2 = build_delta_vector(DEFAULT_SNAPSHOT_SEQ + 2, DEFAULT_SOURCE_ID, &second_delta, &keypair);
    let variant2 = parser.parse(&vector2.header, &vector2.payload).expect("second delta parsed");
    match variant2 {
        DigestVariant::Delta(delta) => {
            assert!(delta.signature_valid);
            assert_eq!(delta.epoch, second_delta.epoch);
        }
        _ => panic!("expected delta"),
    }
}

#[test]
fn wrong_network_id_is_dropped() {
    use kaspa_udp_sidechannel::frame::header::{HeaderParseContext, PayloadCaps};
    use kaspa_udp_sidechannel::frame::header::{HEADER_LEN, KUDP_MAGIC};
    use kaspa_udp_sidechannel::frame::DropReason;

    let mut buf = vec![0u8; HEADER_LEN + 16];
    buf[..4].copy_from_slice(&KUDP_MAGIC);
    buf[4] = 1;
    buf[5] = FrameKind::Digest as u8;
    buf[6] = 0x22; // incorrect network tag
    buf[7] = 0x0C;
    buf[8..16].copy_from_slice(&1u64.to_le_bytes());
    buf[28..32].copy_from_slice(&(16u32).to_le_bytes());
    buf[32..34].copy_from_slice(&DEFAULT_SOURCE_ID.to_le_bytes());
    let ctx = HeaderParseContext { network_tag: 0x11, payload_caps: PayloadCaps { digest: 2048, block: 0 } };
    let err = SatFrameHeader::parse(&buf, &ctx).expect_err("network mismatch");
    assert_eq!(err.reason, DropReason::NetworkMismatch);
}

#[test]
fn non_monotonic_epoch_vector_is_rejected() {
    let parser = build_parser();
    let keypair = fixtures::default_keypair();
    let first = delta_fields(45, unix_now());
    let vector1 = build_delta_vector(DEFAULT_SNAPSHOT_SEQ + 3, DEFAULT_SOURCE_ID, &first, &keypair);
    let variant1 = parser.parse(&vector1.header, &vector1.payload).expect("first delta parsed");
    let mut last_epoch = None;
    assert!(accept_variant(&mut last_epoch, &variant1));

    let mut tampered = delta_fields(46, unix_now());
    tampered.epoch = variant1.epoch() - 1;
    let vector2 = build_delta_vector(DEFAULT_SNAPSHOT_SEQ + 4, DEFAULT_SOURCE_ID, &tampered, &keypair);
    let variant2 = parser.parse(&vector2.header, &vector2.payload).expect("second delta parsed");
    assert!(!accept_variant(&mut last_epoch, &variant2), "non-monotonic epoch must be rejected");
}

fn build_parser() -> DigestParser {
    let registry = SignerRegistry::from_hex(&[signer_hex(&fixtures::default_keypair())]).expect("registry");
    DigestParser::new(true, registry, TimestampSkew::default())
}

fn accept_variant(state: &mut Option<u64>, variant: &DigestVariant) -> bool {
    if let Some(prev) = *state {
        if variant.epoch() < prev {
            return false;
        }
    }
    *state = Some(variant.epoch());
    true
}
