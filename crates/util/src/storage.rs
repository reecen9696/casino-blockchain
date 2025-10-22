use anyhow::{anyhow, Result};
use mychain_types::{BetRecord, compute_app_hash};
use sled::{Batch, Db, Tree};
use std::path::Path;

/// Storage abstraction over sled database
pub struct Storage {
    db: Db,
    meta_tree: Tree,
    app_tree: Tree,
    state_tree: Tree,
}

impl Storage {
    /// Open or create storage at given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path).map_err(|e| anyhow!("Failed to open sled database: {}", e))?;

        let meta_tree = db
            .open_tree("meta")
            .map_err(|e| anyhow!("Failed to open meta tree: {}", e))?;

        let app_tree = db
            .open_tree("app")
            .map_err(|e| anyhow!("Failed to open app tree: {}", e))?;

        let state_tree = db
            .open_tree("state")
            .map_err(|e| anyhow!("Failed to open state tree: {}", e))?;

        Ok(Self {
            db,
            meta_tree,
            app_tree,
            state_tree,
        })
    }

    /// Initialize storage with genesis data
    pub fn init_genesis(&self, vrf_public_key: &[u8], initial_block_random: &[u8; 32]) -> Result<()> {
        let mut batch = Batch::default();

        // Store VRF public key
        batch.insert(b"vrf_pk", vrf_public_key);

        // Store initial block random for height 1
        batch.insert(b"block_random_1", initial_block_random.as_slice());

        // Set initial height
        batch.insert(b"latest_height", &1u64.to_be_bytes());

        // Compute and store initial app hash
        let app_hash = compute_app_hash(1, initial_block_random);
        batch.insert(b"app_hash_1", app_hash.as_slice());

        self.app_tree
            .apply_batch(batch)
            .map_err(|e| anyhow!("Failed to apply genesis batch: {}", e))?;

        self.db
            .flush()
            .map_err(|e| anyhow!("Failed to flush genesis data: {}", e))?;

        Ok(())
    }

    /// Get latest block height
    pub fn get_latest_height(&self) -> Result<u64> {
        match self.meta_tree.get(b"latest_height")? {
            Some(bytes) => {
                let array: [u8; 8] = bytes
                    .as_ref()
                    .try_into()
                    .map_err(|_| anyhow!("Invalid height bytes"))?;
                Ok(u64::from_be_bytes(array))
            }
            None => Ok(0), // Genesis not initialized
        }
    }

    /// Set latest block height
    pub fn set_latest_height(&self, height: u64) -> Result<()> {
        self.meta_tree
            .insert(b"latest_height", &height.to_be_bytes())
            .map_err(|e| anyhow!("Failed to set latest height: {}", e))?;
        Ok(())
    }

    /// Get VRF public key
    pub fn get_vrf_public_key(&self) -> Result<Option<Vec<u8>>> {
        match self.app_tree.get(b"vrf_pk")? {
            Some(bytes) => Ok(Some(bytes.to_vec())),
            None => Ok(None),
        }
    }

    /// Get block random for given height
    pub fn get_block_random(&self, height: u64) -> Result<Option<[u8; 32]>> {
        let key = format!("block_random_{}", height);
        match self.app_tree.get(key.as_bytes())? {
            Some(bytes) => {
                let array: [u8; 32] = bytes
                    .as_ref()
                    .try_into()
                    .map_err(|_| anyhow!("Invalid block random bytes"))?;
                Ok(Some(array))
            }
            None => Ok(None),
        }
    }

    /// Set block random for given height
    pub fn set_block_random(&self, height: u64, block_random: &[u8; 32]) -> Result<()> {
        let key = format!("block_random_{}", height);
        self.app_tree
            .insert(key.as_bytes(), block_random.as_slice())
            .map_err(|e| anyhow!("Failed to set block random: {}", e))?;
        Ok(())
    }

    /// Store bet record
    pub fn store_bet(&self, tx_hash: &[u8; 32], bet_record: &BetRecord) -> Result<()> {
        let key = format!("bet_{}", hex::encode(tx_hash));
        let bytes = bet_record
            .to_bytes()
            .map_err(|e| anyhow!("Failed to serialize bet record: {}", e))?;

        self.app_tree
            .insert(key.as_bytes(), bytes)
            .map_err(|e| anyhow!("Failed to store bet: {}", e))?;
        Ok(())
    }

    /// Get bet record by transaction hash
    pub fn get_bet(&self, tx_hash: &[u8; 32]) -> Result<Option<BetRecord>> {
        let key = format!("bet_{}", hex::encode(tx_hash));
        match self.app_tree.get(key.as_bytes())? {
            Some(bytes) => {
                let bet_record = BetRecord::from_bytes(&bytes)
                    .map_err(|e| anyhow!("Failed to deserialize bet record: {}", e))?;
                Ok(Some(bet_record))
            }
            None => Ok(None),
        }
    }

    /// Get app hash for given height
    pub fn get_app_hash(&self, height: u64) -> Result<Option<[u8; 32]>> {
        let key = format!("app_hash_{}", height);
        match self.state_tree.get(key.as_bytes())? {
            Some(bytes) => {
                let array: [u8; 32] = bytes
                    .as_ref()
                    .try_into()
                    .map_err(|_| anyhow!("Invalid app hash bytes"))?;
                Ok(Some(array))
            }
            None => Ok(None),
        }
    }

    /// Set app hash for given height
    pub fn set_app_hash(&self, height: u64, app_hash: &[u8; 32]) -> Result<()> {
        let key = format!("app_hash_{}", height);
        self.state_tree
            .insert(key.as_bytes(), app_hash.as_slice())
            .map_err(|e| anyhow!("Failed to set app hash: {}", e))?;
        Ok(())
    }

    /// Atomic commit of all pending changes
    pub fn commit(&self) -> Result<()> {
        self.db
            .flush()
            .map_err(|e| anyhow!("Failed to commit changes: {}", e))?;
        Ok(())
    }

    /// Create a new batch for atomic operations
    pub fn create_batch(&self) -> Batch {
        Batch::default()
    }

    /// Apply batch to app tree
    pub fn apply_app_batch(&self, batch: Batch) -> Result<()> {
        self.app_tree
            .apply_batch(batch)
            .map_err(|e| anyhow!("Failed to apply app batch: {}", e))?;
        Ok(())
    }

    /// Apply batch to state tree
    pub fn apply_state_batch(&self, batch: Batch) -> Result<()> {
        self.state_tree
            .apply_batch(batch)
            .map_err(|e| anyhow!("Failed to apply state batch: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mychain_types::TxFlip;
    use tempfile::TempDir;

    fn create_test_storage() -> (Storage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::open(temp_dir.path()).unwrap();
        (storage, temp_dir)
    }

    #[test]
    fn test_storage_creation() {
        let (storage, _temp_dir) = create_test_storage();
        assert_eq!(storage.get_latest_height().unwrap(), 0);
    }

    #[test]
    fn test_genesis_initialization() {
        let (storage, _temp_dir) = create_test_storage();
        let vrf_pk = vec![1, 2, 3, 4];
        let block_random = [5u8; 32];

        storage.init_genesis(&vrf_pk, &block_random).unwrap();

        assert_eq!(storage.get_latest_height().unwrap(), 1);
        assert_eq!(storage.get_vrf_public_key().unwrap().unwrap(), vrf_pk);
        assert_eq!(storage.get_block_random(1).unwrap().unwrap(), block_random);
    }

    #[test]
    fn test_height_operations() {
        let (storage, _temp_dir) = create_test_storage();

        storage.set_latest_height(42).unwrap();
        assert_eq!(storage.get_latest_height().unwrap(), 42);

        storage.set_latest_height(100).unwrap();
        assert_eq!(storage.get_latest_height().unwrap(), 100);
    }

    #[test]
    fn test_bet_storage() {
        let (storage, _temp_dir) = create_test_storage();

        let tx_hash = [1u8; 32];
        let bet_record = BetRecord {
            wallet: [2u8; 32],
            amount: 1000,
            nonce: 42,
            msg: vec![1, 2, 3],
            proof: vec![4, 5, 6],
            output: vec![7, 8, 9],
            result: true,
            height: 100,
            tx_hash,
        };

        storage.store_bet(&tx_hash, &bet_record).unwrap();
        let retrieved = storage.get_bet(&tx_hash).unwrap().unwrap();

        assert_eq!(retrieved.wallet, bet_record.wallet);
        assert_eq!(retrieved.amount, bet_record.amount);
        assert_eq!(retrieved.result, bet_record.result);
    }

    #[test]
    fn test_block_random_operations() {
        let (storage, _temp_dir) = create_test_storage();

        let block_random = [42u8; 32];
        storage.set_block_random(100, &block_random).unwrap();

        let retrieved = storage.get_block_random(100).unwrap().unwrap();
        assert_eq!(retrieved, block_random);

        assert!(storage.get_block_random(999).unwrap().is_none());
    }

    #[test]
    fn test_app_hash_operations() {
        let (storage, _temp_dir) = create_test_storage();

        let app_hash = [99u8; 32];
        storage.set_app_hash(50, &app_hash).unwrap();

        let retrieved = storage.get_app_hash(50).unwrap().unwrap();
        assert_eq!(retrieved, app_hash);

        assert!(storage.get_app_hash(999).unwrap().is_none());
    }
}