use crate::{
    prelude::{CachePolicy, CachedDbAccess, CachedDbItem, DirectDbWriter, StoreError, StoreResult},
    registry::DatabaseStorePrefixes,
};
use faster_hex::hex_string;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Display, Formatter},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use super::prelude::DB;

pub const UDP_DIGEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DigestKey([u8; 16]);

impl DigestKey {
    pub fn new(timestamp_ms: u64, seq: u64) -> Self {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&timestamp_ms.to_be_bytes());
        bytes[8..].copy_from_slice(&seq.to_be_bytes());
        Self(bytes)
    }

    pub fn timestamp_ms(&self) -> u64 {
        u64::from_be_bytes(self.0[..8].try_into().unwrap())
    }
}

impl AsRef<[u8]> for DigestKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Display for DigestKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex_string(&self.0))
    }
}

#[derive(Clone)]
pub struct DbUdpDigestStore {
    db: Arc<DB>,
    access: CachedDbAccess<DigestKey, Vec<u8>>,
    seq: Arc<AtomicU64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct UdpDigestMetadata {
    pub schema_version: u32,
}

pub struct DbUdpDigestMetadataStore {
    db: Arc<DB>,
    access: CachedDbItem<UdpDigestMetadata>,
}

impl DbUdpDigestMetadataStore {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db: Arc::clone(&db), access: CachedDbItem::new(db, DatabaseStorePrefixes::UdpDigestMetadata.into()) }
    }

    pub fn read(&self) -> Result<UdpDigestMetadata, StoreError> {
        self.access.read()
    }

    pub fn write(&mut self, metadata: UdpDigestMetadata) -> Result<(), StoreError> {
        self.access.write(DirectDbWriter::new(&self.db), &metadata)
    }
}

impl DbUdpDigestStore {
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db: Arc::clone(&db),
            access: CachedDbAccess::new(db, CachePolicy::Empty, DatabaseStorePrefixes::UdpDigest.into()),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn insert(&self, timestamp_ms: u64, data: Vec<u8>) -> StoreResult<DigestKey> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let key = DigestKey::new(timestamp_ms, seq);
        self.access.write(DirectDbWriter::new(&self.db), key, data)?;
        Ok(DigestKey::new(timestamp_ms, seq))
    }

    pub fn get(&self, key: DigestKey) -> StoreResult<Vec<u8>> {
        self.access.read(key)
    }

    pub fn delete(&self, key: DigestKey) -> StoreResult<()> {
        self.access.delete(DirectDbWriter::new(&self.db), key)
    }

    pub fn iterator(&self) -> impl Iterator<Item = StoreResult<(DigestKey, Vec<u8>)>> + '_ {
        self.access.iterator().map(|result| match result {
            Ok((raw_key, data)) => {
                let mut key = [0u8; 16];
                key.copy_from_slice(&raw_key);
                Ok((DigestKey(key), data))
            }
            Err(err) => Err(StoreError::DataInconsistency(err.to_string())),
        })
    }
}
