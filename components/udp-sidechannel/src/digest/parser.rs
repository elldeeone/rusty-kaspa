use crate::{
    digest::types::{DigestDelta, DigestError, DigestSnapshot, DigestVariant, DIGEST_SIGNATURE_LEN, DIGEST_SIG_DOMAIN},
    frame::SatFrameHeader,
};
use kaspa_core::time::unix_now;
use kaspa_hashes::Hash;
use secp256k1::{schnorr, Message, Secp256k1, XOnlyPublicKey};
use sha2::{Digest as ShaDigest, Sha256};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy)]
pub struct TimestampSkew {
    pub future_ms: u64,
    pub past_ms: u64,
}

impl Default for TimestampSkew {
    fn default() -> Self {
        Self { future_ms: 120_000, past_ms: 600_000 }
    }
}

#[derive(Debug, Clone)]
pub struct SignerRegistry {
    keys: Arc<RwLock<Vec<XOnlyPublicKey>>>,
}

impl SignerRegistry {
    pub fn empty() -> Self {
        Self { keys: Arc::new(RwLock::new(Vec::new())) }
    }

    pub fn parse_hex_keys(input: &[String]) -> Result<Vec<XOnlyPublicKey>, SignerError> {
        let mut keys = Vec::with_capacity(input.len());
        for (idx, key) in input.iter().enumerate() {
            if key.len() != 64 {
                return Err(SignerError::InvalidLength(idx as u16, key.clone()));
            }
            let mut bytes = [0u8; 32];
            faster_hex::hex_decode(key.as_bytes(), &mut bytes).map_err(|_| SignerError::InvalidHex(idx as u16))?;
            let parsed = XOnlyPublicKey::from_slice(&bytes).map_err(|_| SignerError::InvalidKey(idx as u16))?;
            keys.push(parsed);
        }
        Ok(keys)
    }

    pub fn from_hex(input: &[String]) -> Result<Self, SignerError> {
        Ok(Self { keys: Arc::new(RwLock::new(Self::parse_hex_keys(input)?)) })
    }

    pub fn replace(&self, keys: Vec<XOnlyPublicKey>) {
        *self.keys.write().expect("signer registry poisoned") = keys;
    }

    pub fn len(&self) -> usize {
        self.keys.read().expect("signer registry poisoned").len()
    }

    pub fn replace_from_hex(&self, input: &[String]) -> Result<usize, SignerError> {
        let keys = Self::parse_hex_keys(input)?;
        let len = keys.len();
        self.replace(keys);
        Ok(len)
    }

    pub fn get(&self, id: u16) -> Option<XOnlyPublicKey> {
        let guard = self.keys.read().expect("signer registry poisoned");
        guard.get(id as usize).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug)]
pub enum SignerError {
    InvalidLength(u16, String),
    InvalidHex(u16),
    InvalidKey(u16),
}

impl std::fmt::Display for SignerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignerError::InvalidLength(idx, value) => write!(f, "signer {idx} has invalid length (got `{value}`)"),
            SignerError::InvalidHex(idx) => write!(f, "signer {idx} contains invalid hex characters"),
            SignerError::InvalidKey(idx) => write!(f, "signer {idx} is not a valid x-only public key"),
        }
    }
}

impl std::error::Error for SignerError {}

pub struct DigestParser {
    secp: Secp256k1<secp256k1::VerifyOnly>,
    require_signature: bool,
    skew: TimestampSkew,
    signers: SignerRegistry,
}

impl DigestParser {
    pub fn new(require_signature: bool, signers: SignerRegistry, skew: TimestampSkew) -> Self {
        Self { secp: Secp256k1::verification_only(), require_signature, skew, signers }
    }

    pub fn reload_signers(&self, input: &[String]) -> Result<usize, SignerError> {
        self.signers.replace_from_hex(input)
    }

    pub fn parse(&self, header: &SatFrameHeader, payload: &[u8]) -> Result<DigestVariant, DigestError> {
        let recv_ts_ms = unix_now();
        if header.flags.digest_snapshot() {
            let mut reader = PayloadReader::new(payload);
            let epoch = reader.read_u64("snapshot.epoch")?;
            let frame_ts_ms = reader.read_u64("snapshot.frame_ts")?;
            let pruning_point = reader.read_hash("snapshot.pruning_point")?;
            let pruning_proof_commitment = reader.read_hash("snapshot.pruning_proof_commitment")?;
            let utxo_muhash = reader.read_hash("snapshot.utxo_muhash")?;
            let virtual_selected_parent = reader.read_hash("snapshot.virtual_selected_parent")?;
            let virtual_blue_score = reader.read_u64("snapshot.virtual_blue_score")?;
            let daa_score = reader.read_u64("snapshot.daa_score")?;
            let blue_work = reader.read_bytes_const::<32>("snapshot.blue_work")?;
            let kept_headers_mmr_root = reader.read_optional_hash("snapshot.kept_headers_mmr_root")?;
            let signer_id = reader.read_u16("snapshot.signer_id")?;
            let signature = reader.read_signature()?;
            reader.expect_finished()?;

            self.check_skew(frame_ts_ms, recv_ts_ms)?;

            let mut snapshot = DigestSnapshot {
                epoch,
                pruning_point,
                pruning_proof_commitment,
                utxo_muhash,
                virtual_selected_parent,
                virtual_blue_score,
                daa_score,
                blue_work,
                kept_headers_mmr_root,
                signer_id,
                signature,
                signature_valid: false,
                frame_timestamp_ms: frame_ts_ms,
                recv_timestamp_ms: recv_ts_ms,
                source_id: header.source_id,
            };
            snapshot.signature_valid = self.verify_signature(header, &snapshot)?;
            Ok(DigestVariant::Snapshot(snapshot))
        } else {
            let mut reader = PayloadReader::new(payload);
            let epoch = reader.read_u64("delta.epoch")?;
            let frame_ts_ms = reader.read_u64("delta.frame_ts")?;
            let virtual_selected_parent = reader.read_hash("delta.virtual_selected_parent")?;
            let virtual_blue_score = reader.read_u64("delta.virtual_blue_score")?;
            let daa_score = reader.read_u64("delta.daa_score")?;
            let blue_work = reader.read_bytes_const::<32>("delta.blue_work")?;
            let signer_id = reader.read_u16("delta.signer_id")?;
            let signature = reader.read_signature()?;
            reader.expect_finished()?;

            self.check_skew(frame_ts_ms, recv_ts_ms)?;

            let mut delta = DigestDelta {
                epoch,
                virtual_selected_parent,
                virtual_blue_score,
                daa_score,
                blue_work,
                signer_id,
                signature,
                signature_valid: false,
                frame_timestamp_ms: frame_ts_ms,
                recv_timestamp_ms: recv_ts_ms,
                source_id: header.source_id,
            };
            delta.signature_valid = self.verify_signature(header, &delta)?;
            Ok(DigestVariant::Delta(delta))
        }
    }

    fn check_skew(&self, frame_ts: u64, recv_ts: u64) -> Result<(), DigestError> {
        if frame_ts > recv_ts {
            let delta = frame_ts - recv_ts;
            if delta > self.skew.future_ms {
                return Err(DigestError::TimestampFuture(delta));
            }
        } else {
            let delta = recv_ts - frame_ts;
            if delta > self.skew.past_ms {
                return Err(DigestError::TimestampStale(delta));
            }
        }
        Ok(())
    }

    fn verify_signature<T>(&self, header: &SatFrameHeader, digest: &T) -> Result<bool, DigestError>
    where
        T: CanonicalDigest,
    {
        let signer_id = digest.signer_id();
        let pubkey = match self.signers.get(signer_id) {
            Some(key) => key,
            None => {
                if self.require_signature {
                    return Err(DigestError::UnknownSigner(signer_id));
                }
                return Ok(false);
            }
        };
        let canonical = digest.canonical_bytes(header);
        let hash = Sha256::digest(&canonical);
        let message = Message::from_digest_slice(&hash).map_err(|_| DigestError::InvalidSignature)?;
        let signature = schnorr::Signature::from_slice(digest.signature()).map_err(|_| DigestError::InvalidSignature)?;
        match self.secp.verify_schnorr(&signature, &message, &pubkey) {
            Ok(_) => Ok(true),
            Err(_) if self.require_signature => Err(DigestError::SignatureVerificationFailed),
            Err(_) => Ok(false),
        }
    }
}

trait CanonicalDigest {
    fn signer_id(&self) -> u16;
    fn signature(&self) -> &[u8; DIGEST_SIGNATURE_LEN];
    fn canonical_bytes(&self, header: &SatFrameHeader) -> Vec<u8>;
}

impl CanonicalDigest for DigestSnapshot {
    fn signer_id(&self) -> u16 {
        self.signer_id
    }

    fn signature(&self) -> &[u8; DIGEST_SIGNATURE_LEN] {
        &self.signature
    }

    fn canonical_bytes(&self, header: &SatFrameHeader) -> Vec<u8> {
        let mut buf = Vec::with_capacity(512);
        buf.extend_from_slice(DIGEST_SIG_DOMAIN.as_bytes());
        buf.push(1);
        buf.extend_from_slice(&header.seq.to_le_bytes());
        buf.extend_from_slice(&self.epoch.to_le_bytes());
        buf.extend_from_slice(&self.frame_timestamp_ms.to_le_bytes());
        let pruning_point = self.pruning_point.as_bytes();
        buf.extend_from_slice(&pruning_point);
        let commitment = self.pruning_proof_commitment.as_bytes();
        buf.extend_from_slice(&commitment);
        let muhash = self.utxo_muhash.as_bytes();
        buf.extend_from_slice(&muhash);
        let selected_parent = self.virtual_selected_parent.as_bytes();
        buf.extend_from_slice(&selected_parent);
        buf.extend_from_slice(&self.virtual_blue_score.to_le_bytes());
        buf.extend_from_slice(&self.daa_score.to_le_bytes());
        buf.extend_from_slice(&self.blue_work);
        if let Some(root) = self.kept_headers_mmr_root {
            buf.push(1);
            let root_bytes = root.as_bytes();
            buf.extend_from_slice(&root_bytes);
        } else {
            buf.push(0);
        }
        buf.extend_from_slice(&header.source_id.to_le_bytes());
        buf
    }
}

impl CanonicalDigest for DigestDelta {
    fn signer_id(&self) -> u16 {
        self.signer_id
    }

    fn signature(&self) -> &[u8; DIGEST_SIGNATURE_LEN] {
        &self.signature
    }

    fn canonical_bytes(&self, header: &SatFrameHeader) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(DIGEST_SIG_DOMAIN.as_bytes());
        buf.push(2);
        buf.extend_from_slice(&header.seq.to_le_bytes());
        buf.extend_from_slice(&self.epoch.to_le_bytes());
        buf.extend_from_slice(&self.frame_timestamp_ms.to_le_bytes());
        let selected_parent = self.virtual_selected_parent.as_bytes();
        buf.extend_from_slice(&selected_parent);
        buf.extend_from_slice(&self.virtual_blue_score.to_le_bytes());
        buf.extend_from_slice(&self.daa_score.to_le_bytes());
        buf.extend_from_slice(&self.blue_work);
        buf.extend_from_slice(&header.source_id.to_le_bytes());
        buf
    }
}

struct PayloadReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> PayloadReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_u64(&mut self, label: &'static str) -> Result<u64, DigestError> {
        let bytes = self.read_bytes_const::<8>(label)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn read_u16(&mut self, label: &'static str) -> Result<u16, DigestError> {
        let bytes = self.read_bytes_const::<2>(label)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_hash(&mut self, label: &'static str) -> Result<Hash, DigestError> {
        let bytes = self.read_bytes_const::<32>(label)?;
        Ok(Hash::from(bytes))
    }

    fn read_optional_hash(&mut self, label: &'static str) -> Result<Option<Hash>, DigestError> {
        let flag = self.read_bytes_const::<1>(label)?;
        match flag[0] {
            0 => Ok(None),
            1 => Ok(Some(self.read_hash(label)?)),
            _ => Err(DigestError::PayloadTooShort(label)),
        }
    }

    fn read_bytes_const<const N: usize>(&mut self, label: &'static str) -> Result<[u8; N], DigestError> {
        if self.pos + N > self.buf.len() {
            return Err(DigestError::PayloadTooShort(label));
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&self.buf[self.pos..self.pos + N]);
        self.pos += N;
        Ok(out)
    }

    fn read_signature(&mut self) -> Result<[u8; DIGEST_SIGNATURE_LEN], DigestError> {
        self.read_bytes_const::<DIGEST_SIGNATURE_LEN>("signature")
    }

    fn expect_finished(&self) -> Result<(), DigestError> {
        if self.pos != self.buf.len() {
            Err(DigestError::PayloadTooShort("trailing_bytes"))
        } else {
            Ok(())
        }
    }
}
