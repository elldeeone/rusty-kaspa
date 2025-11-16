use super::DigestVariant;
#[cfg(test)]
use super::{DigestSnapshot, DIGEST_SIGNATURE_LEN};
use bincode::{deserialize, serialize};
use kaspa_database::prelude::DB;
use kaspa_database::{
    prelude::StoreError,
    udp_digest::{DbUdpDigestMetadataStore, DbUdpDigestStore, DigestKey, UdpDigestMetadata, UDP_DIGEST_SCHEMA_VERSION},
};
use std::{cmp::Reverse, sync::Arc};

#[derive(Clone)]
pub struct DigestStore {
    inner: DbUdpDigestStore,
}

#[derive(thiserror::Error, Debug)]
pub enum DigestStoreError {
    #[error("db error: {0}")]
    Store(#[from] StoreError),
    #[error("serialization error: {0}")]
    Codec(#[from] Box<bincode::ErrorKind>),
    #[error("udp digest store requires --udp.db_migrate=true to initialize")]
    MigrationRequired,
    #[error("udp digest schema mismatch (found {found}, expected {expected}); rerun with --udp.db_migrate=true")]
    SchemaMismatch { found: u32, expected: u32 },
}

impl DigestStore {
    pub fn open(db: Arc<DB>, allow_create: bool) -> Result<Self, DigestStoreError> {
        let mut metadata = DbUdpDigestMetadataStore::new(db.clone());
        match metadata.read() {
            Ok(record) => {
                if record.schema_version != UDP_DIGEST_SCHEMA_VERSION {
                    if !allow_create {
                        return Err(DigestStoreError::SchemaMismatch {
                            found: record.schema_version,
                            expected: UDP_DIGEST_SCHEMA_VERSION,
                        });
                    }
                    metadata.write(UdpDigestMetadata { schema_version: UDP_DIGEST_SCHEMA_VERSION })?;
                }
            }
            Err(StoreError::KeyNotFound(_)) => {
                if !allow_create {
                    return Err(DigestStoreError::MigrationRequired);
                }
                metadata.write(UdpDigestMetadata { schema_version: UDP_DIGEST_SCHEMA_VERSION })?;
            }
            Err(err) => return Err(DigestStoreError::Store(err)),
        }

        Ok(Self { inner: DbUdpDigestStore::new(db) })
    }

    pub fn insert(&self, variant: &DigestVariant) -> Result<DigestKey, DigestStoreError> {
        let bytes = serialize(variant)?;
        let timestamp_ms = variant.recv_timestamp_ms();
        Ok(self.inner.insert(timestamp_ms, bytes)?)
    }

    pub fn fetch_recent(&self, from_epoch: Option<u64>, limit: usize) -> Result<Vec<DigestVariant>, DigestStoreError> {
        let mut records = Vec::new();
        for entry in self.inner.iterator() {
            let (key, data) = entry?;
            let mut value: DigestVariant = deserialize(&data)?;
            match &mut value {
                DigestVariant::Snapshot(snapshot) => snapshot.recv_timestamp_ms = key.timestamp_ms(),
                DigestVariant::Delta(delta) => delta.recv_timestamp_ms = key.timestamp_ms(),
            }
            records.push(value);
        }
        records.sort_by_key(|value| Reverse(value.recv_timestamp_ms()));
        if let Some(epoch) = from_epoch {
            records.retain(|r| r.epoch() >= epoch);
        }
        records.truncate(limit);
        Ok(records)
    }

    pub fn prune(&self, retention_count: usize, retention_days: u32) -> Result<(), DigestStoreError> {
        let cutoff_ms = if retention_days == 0 {
            None
        } else {
            let days_ms = (retention_days as u64).saturating_mul(24 * 60 * 60 * 1000);
            let now = kaspa_core::time::unix_now();
            Some(now.saturating_sub(days_ms))
        };
        let count_limit = if retention_count == 0 { usize::MAX } else { retention_count };
        let mut keys = Vec::new();
        for entry in self.inner.iterator() {
            let (key, _) = entry?;
            keys.push(key);
        }
        let total = keys.len();
        let mut delete = Vec::new();
        for (idx, key) in keys.into_iter().enumerate() {
            let too_old = cutoff_ms.map(|cutoff| key.timestamp_ms() < cutoff).unwrap_or(false);
            let remaining = total - idx;
            let over_count = remaining > count_limit;
            let should_drop = too_old || over_count;
            if should_drop {
                delete.push(key);
            }
        }
        for key in delete {
            self.inner.delete(key)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_database::{create_temp_db, prelude::ConnBuilder};
    use kaspa_hashes::Hash;

    fn dummy_variant(epoch: u64) -> DigestVariant {
        DigestVariant::Snapshot(DigestSnapshot {
            epoch,
            pruning_point: Hash::from_bytes([epoch as u8; 32]),
            pruning_proof_commitment: Hash::from_bytes([1; 32]),
            utxo_muhash: Hash::from_bytes([2; 32]),
            virtual_selected_parent: Hash::from_bytes([3; 32]),
            virtual_blue_score: epoch * 10,
            daa_score: epoch * 20,
            blue_work: [4; 32],
            kept_headers_mmr_root: None,
            signer_id: 0,
            signature: [0u8; DIGEST_SIGNATURE_LEN],
            signature_valid: true,
            frame_timestamp_ms: epoch,
            recv_timestamp_ms: epoch,
            source_id: 1,
        })
    }

    #[test]
    fn insert_fetch_and_prune_by_count() {
        let (_guard, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let store = DigestStore::open(db, true).expect("store");
        for epoch in 0..3 {
            store.insert(&dummy_variant(epoch)).unwrap();
        }
        let all = store.fetch_recent(None, 10).unwrap();
        assert_eq!(all.len(), 3);
        store.prune(2, 0).unwrap();
        let recent = store.fetch_recent(None, 10).unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().all(|record| record.epoch() >= 1));
    }

    #[test]
    fn insert_fetch_and_prune_by_days() {
        let (_guard, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let store = DigestStore::open(db, true).expect("store");
        let mut old = dummy_variant(1);
        if let DigestVariant::Snapshot(snapshot) = &mut old {
            snapshot.recv_timestamp_ms = 0;
        }
        let mut recent = dummy_variant(2);
        if let DigestVariant::Snapshot(snapshot) = &mut recent {
            snapshot.recv_timestamp_ms = kaspa_core::time::unix_now();
        }
        store.insert(&old).unwrap();
        store.insert(&recent).unwrap();
        store.prune(10, 1).unwrap();
        let remaining = store.fetch_recent(None, 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].epoch(), 2);
    }

    #[test]
    fn disabled_startup_is_safe() {
        let (_guard, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        {
            let store = DigestStore::open(db.clone(), true).expect("store");
            store.insert(&dummy_variant(1)).unwrap();
        }
        // Simulate a restart where migration flag is off but schema exists.
        let reopened = DigestStore::open(db.clone(), false).expect("existing schema should open");
        assert_eq!(reopened.fetch_recent(None, 10).unwrap().len(), 1);

        // Fresh DB without schema should require migration.
        let (_guard2, db2) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        match DigestStore::open(db2, false) {
            Ok(_) => panic!("expected migration guard"),
            Err(err) => assert!(matches!(err, DigestStoreError::MigrationRequired)),
        }
    }
}
