use kaspa_hashes::Hash;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum length for Schnorr signatures used by DigestV1 payloads.
pub const DIGEST_SIGNATURE_LEN: usize = 64;

/// Signature domain separation tag for DigestV1 messages.
pub const DIGEST_SIG_DOMAIN: &str = "kaspa/udp-digest/v1";

/// Parsed digest payload (snapshot or delta).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DigestVariant {
    Snapshot(DigestSnapshot),
    Delta(DigestDelta),
}

impl DigestVariant {
    pub fn epoch(&self) -> u64 {
        match self {
            DigestVariant::Snapshot(snapshot) => snapshot.epoch,
            DigestVariant::Delta(delta) => delta.epoch,
        }
    }

    pub fn signer_id(&self) -> u16 {
        match self {
            DigestVariant::Snapshot(snapshot) => snapshot.signer_id,
            DigestVariant::Delta(delta) => delta.signer_id,
        }
    }

    pub fn source_id(&self) -> u16 {
        match self {
            DigestVariant::Snapshot(snapshot) => snapshot.source_id,
            DigestVariant::Delta(delta) => delta.source_id,
        }
    }

    pub fn recv_timestamp_ms(&self) -> u64 {
        match self {
            DigestVariant::Snapshot(snapshot) => snapshot.recv_timestamp_ms,
            DigestVariant::Delta(delta) => delta.recv_timestamp_ms,
        }
    }

    pub fn frame_timestamp_ms(&self) -> u64 {
        match self {
            DigestVariant::Snapshot(snapshot) => snapshot.frame_timestamp_ms,
            DigestVariant::Delta(delta) => delta.frame_timestamp_ms,
        }
    }

    pub fn signature_valid(&self) -> bool {
        match self {
            DigestVariant::Snapshot(snapshot) => snapshot.signature_valid,
            DigestVariant::Delta(delta) => delta.signature_valid,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestSnapshot {
    pub epoch: u64,
    pub pruning_point: Hash,
    pub pruning_proof_commitment: Hash,
    pub utxo_muhash: Hash,
    pub virtual_selected_parent: Hash,
    pub virtual_blue_score: u64,
    pub daa_score: u64,
    pub blue_work: [u8; 32],
    pub kept_headers_mmr_root: Option<Hash>,
    pub signer_id: u16,
    #[serde(with = "sig_bytes")]
    pub signature: [u8; DIGEST_SIGNATURE_LEN],
    pub signature_valid: bool,
    pub frame_timestamp_ms: u64,
    pub recv_timestamp_ms: u64,
    pub source_id: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestDelta {
    pub epoch: u64,
    pub virtual_selected_parent: Hash,
    pub virtual_blue_score: u64,
    pub daa_score: u64,
    pub blue_work: [u8; 32],
    pub signer_id: u16,
    #[serde(with = "sig_bytes")]
    pub signature: [u8; DIGEST_SIGNATURE_LEN],
    pub signature_valid: bool,
    pub frame_timestamp_ms: u64,
    pub recv_timestamp_ms: u64,
    pub source_id: u16,
}

#[derive(Debug)]
pub enum DigestError {
    PayloadTooShort(&'static str),
    InvalidSignature,
    UnknownSigner(u16),
    SignatureVerificationFailed,
    TimestampFuture(u64),
    TimestampStale(u64),
}

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DigestError::PayloadTooShort(section) => write!(f, "digest payload truncated near {section}"),
            DigestError::InvalidSignature => write!(f, "invalid digest signature bytes"),
            DigestError::UnknownSigner(id) => write!(f, "unknown signer id {id}"),
            DigestError::SignatureVerificationFailed => write!(f, "digest signature verification failed"),
            DigestError::TimestampFuture(delta) => write!(f, "digest timestamp {}ms in the future", delta),
            DigestError::TimestampStale(delta) => write!(f, "digest timestamp {}ms stale", delta),
        }
    }
}

impl std::error::Error for DigestError {}

mod sig_bytes {
    use super::DIGEST_SIGNATURE_LEN;
    use serde::{de::Error as DeError, ser::Serializer, Deserialize, Deserializer};

    pub fn serialize<S>(bytes: &[u8; DIGEST_SIGNATURE_LEN], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; DIGEST_SIGNATURE_LEN], D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        if bytes.len() != DIGEST_SIGNATURE_LEN {
            return Err(D::Error::custom(format!("expected {} signature bytes", DIGEST_SIGNATURE_LEN)));
        }
        let mut sig = [0u8; DIGEST_SIGNATURE_LEN];
        sig.copy_from_slice(&bytes);
        Ok(sig)
    }
}
