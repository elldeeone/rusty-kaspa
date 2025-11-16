use crate::{
    digest::{DIGEST_SIGNATURE_LEN, DIGEST_SIG_DOMAIN},
    frame::{FrameFlags, FrameKind, SatFrameHeader, HEADER_LEN, KUDP_MAGIC},
};
use kaspa_hashes::Hash;
use secp256k1::{Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use sha2::{Digest as ShaDigest, Sha256};

/// Default secret key used by tests and tooling when generating golden vectors.
pub const DEFAULT_SIGNER_SECRET: [u8; 32] = [0x11; 32];
/// Default signer identifier embedded inside snapshot/delta payloads.
pub const DEFAULT_SIGNER_ID: u16 = 0;
/// Default UDP source identifier.
pub const DEFAULT_SOURCE_ID: u16 = 7;
/// Default sequence number used for the snapshot golden vector.
pub const DEFAULT_SNAPSHOT_SEQ: u64 = 10;
/// Default sequence number used for the delta golden vector.
pub const DEFAULT_DELTA_SEQ: u64 = 20;

/// Digest snapshot fields used by snapshot vectors.
#[derive(Clone)]
pub struct SnapshotFields {
    pub epoch: u64,
    pub frame_ts_ms: u64,
    pub pruning_point: Hash,
    pub pruning_proof_commitment: Hash,
    pub utxo_muhash: Hash,
    pub virtual_selected_parent: Hash,
    pub virtual_blue_score: u64,
    pub daa_score: u64,
    pub blue_work: [u8; 32],
    pub kept_headers_mmr_root: Option<Hash>,
}

/// Digest delta fields used by delta vectors.
#[derive(Clone)]
pub struct DeltaFields {
    pub epoch: u64,
    pub frame_ts_ms: u64,
    pub virtual_selected_parent: Hash,
    pub virtual_blue_score: u64,
    pub daa_score: u64,
    pub blue_work: [u8; 32],
}

/// Container holding a synthesized header + payload pair.
pub struct DigestVector {
    pub header: SatFrameHeader,
    pub payload: Vec<u8>,
}

impl DigestVector {
    /// Encodes the header/payload pair into a full datagram (header + payload) using the provided network tag.
    pub fn into_datagram(self, network_tag: u8) -> Vec<u8> {
        encode_datagram(&self.header, &self.payload, network_tag)
    }
}

/// Returns the default key pair used by tests/tooling.
pub fn default_keypair() -> Keypair {
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&DEFAULT_SIGNER_SECRET).expect("default signer secret is valid");
    Keypair::from_secret_key(&secp, &sk)
}

/// Returns the hex-encoded x-only public key for the provided key pair.
pub fn signer_hex(keypair: &Keypair) -> String {
    let (xonly, _) = XOnlyPublicKey::from_keypair(keypair);
    faster_hex::hex_string(&xonly.serialize())
}

/// Produces deterministic snapshot fields for the provided epoch.
pub fn snapshot_fields(epoch: u64, frame_ts_ms: u64, include_root: bool) -> SnapshotFields {
    SnapshotFields {
        epoch,
        frame_ts_ms,
        pruning_point: Hash::from_bytes([1; 32]),
        pruning_proof_commitment: Hash::from_bytes([2; 32]),
        utxo_muhash: Hash::from_bytes([3; 32]),
        virtual_selected_parent: Hash::from_bytes([4; 32]),
        virtual_blue_score: 100,
        daa_score: 200,
        blue_work: [5; 32],
        kept_headers_mmr_root: include_root.then(|| Hash::from_bytes([6; 32])),
    }
}

/// Produces deterministic delta fields for the provided epoch.
pub fn delta_fields(epoch: u64, frame_ts_ms: u64) -> DeltaFields {
    DeltaFields {
        epoch,
        frame_ts_ms,
        virtual_selected_parent: Hash::from_bytes([7; 32]),
        virtual_blue_score: 300,
        daa_score: 400,
        blue_work: [8; 32],
    }
}

/// Builds the canonical snapshot header/payload pair for the provided fields.
pub fn build_snapshot_vector(seq: u64, source_id: u16, fields: &SnapshotFields, signer: &Keypair) -> DigestVector {
    let mut header = SatFrameHeader {
        kind: FrameKind::Digest,
        flags: FrameFlags::from_bits(0x0C),
        seq,
        group_id: 0,
        group_k: 0,
        group_n: 0,
        frag_ix: 0,
        frag_cnt: 1,
        payload_len: 0,
        source_id,
    };
    let signature = sign_snapshot(&header, fields, signer);
    let payload = snapshot_payload(fields, &signature);
    header.payload_len = payload.len() as u32;
    DigestVector { header, payload }
}

/// Builds the canonical delta header/payload pair for the provided fields.
pub fn build_delta_vector(seq: u64, source_id: u16, fields: &DeltaFields, signer: &Keypair) -> DigestVector {
    let mut header = SatFrameHeader {
        kind: FrameKind::Digest,
        flags: FrameFlags::from_bits(0x04),
        seq,
        group_id: 0,
        group_k: 0,
        group_n: 0,
        frag_ix: 0,
        frag_cnt: 1,
        payload_len: 0,
        source_id,
    };
    let signature = sign_delta(&header, fields, signer);
    let payload = delta_payload(fields, &signature);
    header.payload_len = payload.len() as u32;
    DigestVector { header, payload }
}

/// Returns the canonical snapshot + delta golden vectors used by fuzz/tests/tooling.
pub fn golden_vectors(frame_ts_ms: u64) -> (DigestVector, DigestVector) {
    let signer = default_keypair();
    let snapshot = build_snapshot_vector(DEFAULT_SNAPSHOT_SEQ, DEFAULT_SOURCE_ID, &snapshot_fields(42, frame_ts_ms, true), &signer);
    let delta = build_delta_vector(DEFAULT_DELTA_SEQ, DEFAULT_SOURCE_ID, &delta_fields(43, frame_ts_ms), &signer);
    (snapshot, delta)
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
    buf.extend_from_slice(&DEFAULT_SIGNER_ID.to_le_bytes());
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
    buf.extend_from_slice(&DEFAULT_SIGNER_ID.to_le_bytes());
    buf.extend_from_slice(signature);
    buf
}

fn sign_snapshot(header: &SatFrameHeader, fields: &SnapshotFields, signer: &Keypair) -> [u8; DIGEST_SIGNATURE_LEN] {
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
    sign_preimage(&buf, signer)
}

fn sign_delta(header: &SatFrameHeader, fields: &DeltaFields, signer: &Keypair) -> [u8; DIGEST_SIGNATURE_LEN] {
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
    sign_preimage(&buf, signer)
}

fn sign_preimage(bytes: &[u8], signer: &Keypair) -> [u8; DIGEST_SIGNATURE_LEN] {
    let hash = Sha256::digest(bytes);
    let msg = Message::from_digest_slice(&hash).expect("digest sized");
    let secp = Secp256k1::new();
    let sig = secp.sign_schnorr(&msg, signer);
    let mut out = [0u8; DIGEST_SIGNATURE_LEN];
    out.copy_from_slice(sig.as_ref());
    out
}

fn push_hash(buf: &mut Vec<u8>, hash: Hash) {
    let bytes = hash.as_bytes();
    buf.extend_from_slice(&bytes);
}

fn encode_datagram(header: &SatFrameHeader, payload: &[u8], network_tag: u8) -> Vec<u8> {
    use crc32fast::Hasher;

    let mut buf = vec![0u8; HEADER_LEN + payload.len()];
    buf[..4].copy_from_slice(&KUDP_MAGIC);
    buf[4] = 1;
    buf[5] = header.kind as u8;
    buf[6] = network_tag;
    buf[7] = header.flags.bits();
    buf[8..16].copy_from_slice(&header.seq.to_le_bytes());
    buf[16..20].copy_from_slice(&header.group_id.to_le_bytes());
    buf[20..22].copy_from_slice(&header.group_k.to_le_bytes());
    buf[22..24].copy_from_slice(&header.group_n.to_le_bytes());
    buf[24..26].copy_from_slice(&header.frag_ix.to_le_bytes());
    buf[26..28].copy_from_slice(&header.frag_cnt.to_le_bytes());
    buf[28..32].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    buf[32..34].copy_from_slice(&header.source_id.to_le_bytes());
    buf[HEADER_LEN..].copy_from_slice(payload);
    let mut hasher = Hasher::new();
    hasher.update(&buf[..HEADER_LEN - 4]);
    buf[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&hasher.finalize().to_le_bytes());
    buf
}
