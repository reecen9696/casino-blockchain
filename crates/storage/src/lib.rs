use anyhow::{Context, Result};
use mychain_types::BetRecord;
use sled::Db;
use std::path::Path;

/// Storage layer using sled with proper keyspace organization
/// 
/// Keyspaces:
/// - /meta/last_height -> u64
/// - /blocks/{height} -> bincode(Block)  
/// - /tx/{tx_hash} -> height:u64
/// - /app/vrf_pk -> bytes
/// - /app/bets/{tx_hash} -> bincode(BetRecord)
/// - /state/app_hash/{height} -> [u8; 32]
pub struct Storage {
    db: Db,
}

/// Simple batch structure for atomic operations
pub struct StorageBatch {
    operations: Vec<BatchOperation>,
}

enum BatchOperation {
    Insert {
        tree_name: String,
        key: Vec<u8>,
        value: Vec<u8>,
    },
}

impl Storage {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path).context("Failed to open sled database")?;
        Ok(Self { db })
    }

    /// Get the last block height
    pub fn get_last_height(&self) -> Result<u64> {
        let tree = self.db.open_tree("meta")?;
        match tree.get("last_height")? {
            Some(bytes) => {
                let height_bytes: [u8; 8] = bytes.as_ref().try_into()
                    .context("Invalid height format")?;
                Ok(u64::from_le_bytes(height_bytes))
            }
            None => Ok(0), // Genesis
        }
    }

    /// Set the last block height  
    pub fn set_last_height(&self, height: u64, batch: &mut StorageBatch) -> Result<()> {
        batch.operations.push(BatchOperation::Insert {
            tree_name: "meta".to_string(),
            key: b"last_height".to_vec(),
            value: height.to_le_bytes().to_vec(),
        });
        Ok(())
    }

    /// Get VRF public key
    pub fn get_vrf_public_key(&self) -> Result<Option<Vec<u8>>> {
        let tree = self.db.open_tree("app")?;
        Ok(tree.get("vrf_pk")?.map(|v| v.to_vec()))
    }

    /// Set VRF public key
    pub fn set_vrf_public_key(&self, vrf_pk: &[u8], batch: &mut StorageBatch) -> Result<()> {
        batch.operations.push(BatchOperation::Insert {
            tree_name: "app".to_string(),
            key: b"vrf_pk".to_vec(),
            value: vrf_pk.to_vec(),
        });
        Ok(())
    }

    /// Store a bet record
    pub fn store_bet(&self, tx_hash: &[u8], bet: &BetRecord, batch: &mut StorageBatch) -> Result<()> {
        let key = format!("bets/{}", hex::encode(tx_hash));
        let encoded = bincode::serialize(bet)?;
        batch.operations.push(BatchOperation::Insert {
            tree_name: "app".to_string(),
            key: key.into_bytes(),
            value: encoded,
        });
        Ok(())
    }

    /// Get a bet record by transaction hash
    pub fn get_bet(&self, tx_hash: &[u8]) -> Result<Option<BetRecord>> {
        let tree = self.db.open_tree("app")?;
        let key = format!("bets/{}", hex::encode(tx_hash));
        match tree.get(key.as_bytes())? {
            Some(bytes) => {
                let bet: BetRecord = bincode::deserialize(&bytes)?;
                Ok(Some(bet))
            }
            None => Ok(None),
        }
    }

    /// Store app hash for a height
    pub fn store_app_hash(&self, height: u64, app_hash: &[u8; 32], batch: &mut StorageBatch) -> Result<()> {
        let key = format!("app_hash/{}", height);
        batch.operations.push(BatchOperation::Insert {
            tree_name: "state".to_string(),
            key: key.into_bytes(),
            value: app_hash.to_vec(),
        });
        Ok(())
    }

    /// Get app hash for a height
    pub fn get_app_hash(&self, height: u64) -> Result<Option<[u8; 32]>> {
        let tree = self.db.open_tree("state")?;
        let key = format!("app_hash/{}", height);
        match tree.get(key.as_bytes())? {
            Some(bytes) => {
                let hash: [u8; 32] = bytes.as_ref().try_into()
                    .context("Invalid app hash format")?;
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }

    /// Store block-transaction mapping
    pub fn store_tx_height(&self, tx_hash: &[u8], height: u64, batch: &mut StorageBatch) -> Result<()> {
        let key = hex::encode(tx_hash);
        batch.operations.push(BatchOperation::Insert {
            tree_name: "tx".to_string(),
            key: key.into_bytes(),
            value: height.to_le_bytes().to_vec(),
        });
        Ok(())
    }

    /// Get height for a transaction hash
    pub fn get_tx_height(&self, tx_hash: &[u8]) -> Result<Option<u64>> {
        let tree = self.db.open_tree("tx")?;
        let key = hex::encode(tx_hash);
        match tree.get(key.as_bytes())? {
            Some(bytes) => {
                let height_bytes: [u8; 8] = bytes.as_ref().try_into()
                    .context("Invalid height format")?;
                Ok(Some(u64::from_le_bytes(height_bytes)))
            }
            None => Ok(None),
        }
    }

    /// Create a new batch for atomic operations
    pub fn batch(&self) -> StorageBatch {
        StorageBatch {
            operations: Vec::new(),
        }
    }

    /// Apply a batch atomically and flush to disk
    pub fn apply_batch(&self, batch: StorageBatch) -> Result<()> {
        // Group operations by tree
        let mut tree_operations: std::collections::HashMap<String, Vec<(Vec<u8>, Vec<u8>)>> = 
            std::collections::HashMap::new();

        for op in batch.operations {
            match op {
                BatchOperation::Insert { tree_name, key, value } => {
                    tree_operations.entry(tree_name)
                        .or_insert_with(Vec::new)
                        .push((key, value));
                }
            }
        }

        // Apply operations to each tree
        for (tree_name, operations) in tree_operations {
            let tree = self.db.open_tree(&tree_name)?;
            let mut tree_batch = sled::Batch::default();
            
            for (key, value) in operations {
                tree_batch.insert(key, value);
            }
            
            tree.apply_batch(tree_batch)?;
        }
        
        // Ensure data is persisted to disk
        self.db.flush()?;
        Ok(())
    }

    /// Compute app hash based on current state
    /// Simple implementation: hash(height || last_vrf_accumulator)
    pub fn compute_app_hash(&self, height: u64) -> Result<[u8; 32]> {
        // For POC: simple hash of height
        // In production: hash of canonical state serialization
        let mut hasher = blake3::Hasher::new();
        hasher.update(&height.to_le_bytes());
        
        // Add some state data to the hash if available
        if let Ok(Some(vrf_pk)) = self.get_vrf_public_key() {
            hasher.update(&vrf_pk);
        }
        
        let hash = hasher.finalize();
        Ok(*hash.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_storage_basic_operations() -> Result<()> {
        let temp_dir = tempdir()?;
        let storage = Storage::open(temp_dir.path())?;

        // Test height operations
        assert_eq!(storage.get_last_height()?, 0);
        
        let mut batch = storage.batch();
        storage.set_last_height(42, &mut batch)?;
        storage.apply_batch(batch)?;
        
        assert_eq!(storage.get_last_height()?, 42);

        // Test VRF key operations  
        let vrf_pk = b"test_vrf_public_key";
        let mut batch = storage.batch();
        storage.set_vrf_public_key(vrf_pk, &mut batch)?;
        storage.apply_batch(batch)?;
        
        assert_eq!(storage.get_vrf_public_key()?.unwrap(), vrf_pk);

        Ok(())
    }
}