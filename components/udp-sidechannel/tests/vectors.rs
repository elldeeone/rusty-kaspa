use faster_hex::hex_string;
use kaspa_core::time::unix_now;
use kaspa_hashes::Hash;
use kaspa_udp_sidechannel::{
    digest::{DigestParser, DigestVariant, SignerRegistry, TimestampSkew, DIGEST_SIGNATURE_LEN, DIGEST_SIG_DOMAIN},
    frame::{FrameFlags, FrameKind, SatFrameHeader},
    DigestError,
};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use sha2::{Digest as ShaDigest, Sha256};
use std::sync::OnceLock;

const TEST_SOURCE_ID: u16 = 7;
const TEST_SIGNER_ID: u16 = 0;
const SEQ_SNAPSHOT: u64 = 10;
const SEQ_DELTA: u64 = 20;

static SECP: OnceLock<Secp256k1<secp256k1::All>> = OnceLock::new();
static TEST_KEYPAIR: OnceLock<Keypair> = OnceLock::new();
static TEST_SIGNER_HEX: OnceLock<String> = OnceLock::new();

fn secp() -> &'static Secp256k1<secp256k1::All> {
    SECP.get_or_init(Secp256k1::new)
}

fn test_keypair() -> &'static Keypair {
    TEST_KEYPAIR.get_or_init(|| {
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        Keypair::from_secret_key(secp(), &sk)
    })
}

fn test_signer_hex() -> &'static String {
    TEST_SIGNER_HEX.get_or_init(|| {
        let (xonly, _) = XOnlyPublicKey::from_keypair(test_keypair());
        hex_string(&xonly.serialize())
    })
}

#[test]
fn snapshot_roundtrip() {
    let parser = build_parser();
    let fields = snapshot_fields(unix_now(), true);
    let (header, payload) = build_snapshot_vector(&fields);
    let variant = parser.parse(&header, &payload).expect("snapshot parsed");
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
    let first_delta = delta_fields(unix_now());
    let (header, payload) = build_delta_vector(&first_delta);
    let variant = parser.parse(&header, &payload).expect("delta parsed");
    match variant {
        DigestVariant::Delta(delta) => {
            assert!(delta.signature_valid);
            assert_eq!(delta.epoch, first_delta.epoch);
        }
        _ => panic!("expected delta"),
    }

    let mut tampered = payload.clone();
    *tampered.last_mut().unwrap() ^= 0x01;
    let err = parser.parse(&header, &tampered).expect_err("invalid signature rejected");
    assert!(matches!(err, DigestError::SignatureVerificationFailed));

    let second_delta = delta_fields(unix_now() + 100);
    let (header2, payload2) = build_delta_vector(&second_delta);
    let variant2 = parser.parse(&header2, &payload2).expect("second delta parsed");
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
    buf[32..34].copy_from_slice(&TEST_SOURCE_ID.to_le_bytes());
    let ctx = HeaderParseContext { network_tag: 0x11, payload_caps: PayloadCaps { digest: 2048, block: 0 } };
    let err = SatFrameHeader::parse(&buf, &ctx).expect_err("network mismatch");
    assert_eq!(err.reason, DropReason::NetworkMismatch);
}

#[test]
fn non_monotonic_epoch_vector_is_rejected() {
    let parser = build_parser();
    let first = delta_fields(unix_now());
    let (header1, payload1) = build_delta_vector(&first);
    let variant1 = parser.parse(&header1, &payload1).expect("first delta parsed");
    let mut last_epoch = None;
    assert!(accept_variant(&mut last_epoch, &variant1));

    let mut tampered = delta_fields(unix_now());
    tampered.epoch = variant1.epoch() - 1;
    let (header2, payload2) = build_delta_vector(&tampered);
    let variant2 = parser.parse(&header2, &payload2).expect("second delta parsed");
    assert!(!accept_variant(&mut last_epoch, &variant2), "non-monotonic epoch must be rejected");
}

fn build_parser() -> DigestParser {
    let registry = SignerRegistry::from_hex(&[test_signer_hex().to_string()]).expect("registry");
    DigestParser::new(true, registry, TimestampSkew::default())
}

struct SnapshotFields {
    epoch: u64,
    frame_ts_ms: u64,
    pruning_point: Hash,
    pruning_proof_commitment: Hash,
    utxo_muhash: Hash,
    virtual_selected_parent: Hash,
    virtual_blue_score: u64,
    daa_score: u64,
    blue_work: [u8; 32],
    kept_headers_mmr_root: Option<Hash>,
}

struct DeltaFields {
    epoch: u64,
    frame_ts_ms: u64,
    virtual_selected_parent: Hash,
    virtual_blue_score: u64,
    daa_score: u64,
    blue_work: [u8; 32],
}

fn snapshot_fields(frame_ts_ms: u64, include_root: bool) -> SnapshotFields {
    SnapshotFields {
        epoch: 42,
        frame_ts_ms,
        pruning_point: Hash::from_bytes([1; 32]),
        pruning_proof_commitment: Hash::from_bytes([2; 32]),
        utxo_muhash: Hash::from_bytes([3; 32]),
        virtual_selected_parent: Hash::from_bytes([4; 32]),
        virtual_blue_score: 100,
        daa_score: 200,
        blue_work: [5; 32],
        kept_headers_mmr_root: if include_root { Some(Hash::from_bytes([6; 32])) } else { None },
    }
}

fn delta_fields(frame_ts_ms: u64) -> DeltaFields {
    DeltaFields {
        epoch: 43,
        frame_ts_ms,
        virtual_selected_parent: Hash::from_bytes([7; 32]),
        virtual_blue_score: 300,
        daa_score: 400,
        blue_work: [8; 32],
    }
}

fn build_snapshot_vector(fields: &SnapshotFields) -> (SatFrameHeader, Vec<u8>) {
    let mut header = SatFrameHeader {
        kind: FrameKind::Digest,
        flags: FrameFlags::from_bits(0x0C),
        seq: SEQ_SNAPSHOT,
        group_id: 0,
        group_k: 0,
        group_n: 0,
        frag_ix: 0,
        frag_cnt: 1,
        payload_len: 0,
        source_id: TEST_SOURCE_ID,
    };
    let signature = sign_snapshot(&header, fields);
    let payload = snapshot_payload(fields, &signature);
    header.payload_len = payload.len() as u32;
    (header, payload)
}

fn build_delta_vector(fields: &DeltaFields) -> (SatFrameHeader, Vec<u8>) {
    let mut header = SatFrameHeader {
        kind: FrameKind::Digest,
        flags: FrameFlags::from_bits(0x04),
        seq: SEQ_DELTA,
        group_id: 0,
        group_k: 0,
        group_n: 0,
        frag_ix: 0,
        frag_cnt: 1,
        payload_len: 0,
        source_id: TEST_SOURCE_ID,
    };
    let signature = sign_delta(&header, fields);
    let payload = delta_payload(fields, &signature);
    header.payload_len = payload.len() as u32;
    (header, payload)
}

fn snapshot_payload(fields: &SnapshotFields, signature: &[u8; DIGEST_SIGNATURE_LEN]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&fields.epoch.to_le_bytes());
    buf.extend_from_slice(&fields.frame_ts_ms.to_le_bytes());
    push_hash(&mut buf, fields.pruning_point);
    push_hash(&mut buf, fields.pruning_proof_commitment);
    push_hash(&mut buf, fields.utxo_muhash);
    push_hash(&mut buf, fields.virtual_selected_parent);
    buf.extend_from_slice(&fields.virtual_blue_score.to_le_bytes());
    buf.extend_from_slice(&fields.daa_score.to_le_bytes());
    buf.extend_from_slice(&fields.blue_work);
    if let Some(root) = fields.kept_headers_mmr_root {
        buf.push(1);
        push_hash(&mut buf, root);
    } else {
        buf.push(0);
    }
    buf.extend_from_slice(&TEST_SIGNER_ID.to_le_bytes());
    buf.extend_from_slice(signature);
    buf
}

fn delta_payload(fields: &DeltaFields, signature: &[u8; DIGEST_SIGNATURE_LEN]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&fields.epoch.to_le_bytes());
    buf.extend_from_slice(&fields.frame_ts_ms.to_le_bytes());
    push_hash(&mut buf, fields.virtual_selected_parent);
    buf.extend_from_slice(&fields.virtual_blue_score.to_le_bytes());
    buf.extend_from_slice(&fields.daa_score.to_le_bytes());
    buf.extend_from_slice(&fields.blue_work);
    buf.extend_from_slice(&TEST_SIGNER_ID.to_le_bytes());
    buf.extend_from_slice(signature);
    buf
}

fn sign_snapshot(header: &SatFrameHeader, fields: &SnapshotFields) -> [u8; DIGEST_SIGNATURE_LEN] {
    let mut buf = Vec::new();
    buf.extend_from_slice(DIGEST_SIG_DOMAIN.as_bytes());
    buf.push(1);
    buf.extend_from_slice(&header.seq.to_le_bytes());
    buf.extend_from_slice(&fields.epoch.to_le_bytes());
    buf.extend_from_slice(&fields.frame_ts_ms.to_le_bytes());
    push_hash(&mut buf, fields.pruning_point);
    push_hash(&mut buf, fields.pruning_proof_commitment);
    push_hash(&mut buf, fields.utxo_muhash);
    push_hash(&mut buf, fields.virtual_selected_parent);
    buf.extend_from_slice(&fields.virtual_blue_score.to_le_bytes());
    buf.extend_from_slice(&fields.daa_score.to_le_bytes());
    buf.extend_from_slice(&fields.blue_work);
    if let Some(root) = fields.kept_headers_mmr_root {
        buf.push(1);
        push_hash(&mut buf, root);
    } else {
        buf.push(0);
    }
    buf.extend_from_slice(&header.source_id.to_le_bytes());
    sign_preimage(&buf)
}

fn sign_delta(header: &SatFrameHeader, fields: &DeltaFields) -> [u8; DIGEST_SIGNATURE_LEN] {
    let mut buf = Vec::new();
    buf.extend_from_slice(DIGEST_SIG_DOMAIN.as_bytes());
    buf.push(2);
    buf.extend_from_slice(&header.seq.to_le_bytes());
    buf.extend_from_slice(&fields.epoch.to_le_bytes());
    buf.extend_from_slice(&fields.frame_ts_ms.to_le_bytes());
    push_hash(&mut buf, fields.virtual_selected_parent);
    buf.extend_from_slice(&fields.virtual_blue_score.to_le_bytes());
    buf.extend_from_slice(&fields.daa_score.to_le_bytes());
    buf.extend_from_slice(&fields.blue_work);
    buf.extend_from_slice(&header.source_id.to_le_bytes());
    sign_preimage(&buf)
}

fn sign_preimage(bytes: &[u8]) -> [u8; DIGEST_SIGNATURE_LEN] {
    let hash = Sha256::digest(bytes);
    let msg = Message::from_digest_slice(&hash).unwrap();
    let sig = secp().sign_schnorr(&msg, test_keypair());
    let mut out = [0u8; DIGEST_SIGNATURE_LEN];
    out.copy_from_slice(sig.as_ref());
    out
}

fn push_hash(buf: &mut Vec<u8>, hash: Hash) {
    let bytes = hash.as_bytes();
    buf.extend_from_slice(&bytes);
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
