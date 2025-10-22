use anyhow::Result;
use mychain_types::{BetRecord, TxFlip, compute_app_hash};
use mychain_util::{Storage, VrfEngine, compute_block_random};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{info, warn, error};

/// Simplified ABCI application state
#[derive(Clone)]
pub struct MyChainApp {
    /// Persistent storage
    storage: Arc<RwLock<Storage>>,
    /// VRF engine for randomness
    vrf_engine: Arc<RwLock<VrfEngine>>,
    /// Chain configuration
    chain_id: String,
    /// Height of latest block
    height: u64,
    /// Mempool of pending transactions
    mempool: Arc<RwLock<HashMap<[u8; 32], TxFlip>>>,
}

impl MyChainApp {
    /// Create a new MyChain application
    pub fn new(storage_path: &str, chain_id: String) -> Result<Self> {
        let storage = Storage::open(storage_path)?;
        let height = storage.get_latest_height()?;
        let storage = Arc::new(RwLock::new(storage));

        // Load or generate VRF key
        let vrf_engine = {
            let storage_read = storage.read().unwrap();
            match storage_read.get_vrf_public_key()? {
                Some(_pub_key_bytes) => {
                    // TODO: Load private key from secure location
                    // For now, generate fresh (not production ready)
                    Arc::new(RwLock::new(VrfEngine::generate()?))
                }
                None => {
                    let engine = VrfEngine::generate()?;
                    let pub_key = engine.public_key_bytes();
                    
                    // Initialize genesis with new VRF key
                    let initial_random = [0u8; 32]; // Genesis block random
                    storage_read.init_genesis(&pub_key, &initial_random)?;
                    
                    Arc::new(RwLock::new(engine))
                }
            }
        };

        let mempool = Arc::new(RwLock::new(HashMap::new()));

        Ok(Self {
            storage,
            vrf_engine,
            chain_id,
            height,
            mempool,
        })
    }

    /// Validate a transaction
    pub fn validate_tx(&self, tx_bytes: &[u8]) -> Result<TxFlip> {
        // Deserialize transaction
        let tx = TxFlip::from_bytes(tx_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid transaction format: {}", e))?;

        // Basic validation
        if tx.version != 1 {
            return Err(anyhow::anyhow!("Unsupported transaction version: {}", tx.version));
        }

        if tx.amount == 0 {
            return Err(anyhow::anyhow!("Bet amount must be greater than 0"));
        }

        // TODO: Add balance checking, signature verification, etc.

        Ok(tx)
    }

    /// Execute a validated transaction and return the bet record
    pub fn execute_tx(&self, tx: &TxFlip, height: u64, tx_hash: [u8; 32]) -> Result<BetRecord> {
        let storage = self.storage.read().unwrap();

        // Get current block random
        let block_random = storage.get_block_random(height)?
            .ok_or_else(|| anyhow::anyhow!("Block random not found for height {}", height))?;

        // Compute VRF message
        let message = {
            let vrf_engine = self.vrf_engine.read().unwrap();
            vrf_engine.compute_message(
                &self.chain_id,
                height,
                &block_random,
                &tx_hash,
                &tx.wallet,
                tx.nonce,
            )
        };

        // Generate VRF proof
        let (proof, output) = {
            let mut vrf_engine = self.vrf_engine.write().unwrap();
            vrf_engine.prove(&message)?
        };

        // Derive flip result
        let result = {
            let vrf_engine = self.vrf_engine.read().unwrap();
            vrf_engine.derive_flip_result(&output)
        };

        // Create bet record
        let bet_record = BetRecord {
            wallet: tx.wallet,
            amount: tx.amount,
            nonce: tx.nonce,
            msg: message,
            proof,
            output,
            result,
            height,
            tx_hash,
        };

        Ok(bet_record)
    }

    /// Simulate processing a block with transactions
    pub fn process_block(&mut self, txs: Vec<Vec<u8>>) -> Result<Vec<BetRecord>> {
        let new_height = self.height + 1;
        
        info!("Processing block at height {}", new_height);

        // Compute new block random from previous block
        let prev_block_hash = [0u8; 32]; // Simplified - no actual block hash
        let prev_vrf_accum = self.storage.read().unwrap()
            .get_block_random(self.height)
            .unwrap_or(Some([0u8; 32]))
            .unwrap_or([0u8; 32]);
        
        let new_block_random = compute_block_random(&prev_block_hash, &prev_vrf_accum);

        // Store new block random
        {
            let storage = self.storage.write().unwrap();
            storage.set_block_random(new_height, &new_block_random)?;
        }

        let mut bet_records = Vec::new();

        // Process transactions
        for (i, tx_bytes) in txs.iter().enumerate() {
            let tx_hash = blake3::hash(tx_bytes);
            let tx_hash_bytes: [u8; 32] = *tx_hash.as_bytes();

            match self.validate_tx(tx_bytes) {
                Ok(tx) => {
                    match self.execute_tx(&tx, new_height, tx_hash_bytes) {
                        Ok(bet_record) => {
                            // Store bet record
                            {
                                let storage = self.storage.read().unwrap();
                                if let Err(e) = storage.store_bet(&tx_hash_bytes, &bet_record) {
                                    error!("Failed to store bet record: {}", e);
                                }
                            }

                            info!("Transaction {}: {} -> {}", 
                                hex::encode(&tx_hash_bytes[..8]),
                                bet_record.amount,
                                if bet_record.result { "heads" } else { "tails" }
                            );

                            bet_records.push(bet_record);
                        }
                        Err(e) => {
                            error!("Failed to execute transaction {}: {}", i, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Invalid transaction {}: {}", i, e);
                }
            }
        }

        // Compute app hash based on bet records and height
        let app_hash = compute_app_hash(new_height, &new_block_random);

        // Store app hash and height
        {
            let storage = self.storage.write().unwrap();
            storage.set_app_hash(new_height, &app_hash)?;
            storage.set_latest_height(new_height)?;
            storage.commit()?;
        }

        self.height = new_height;

        Ok(bet_records)
    }

    /// Query bet record by transaction hash
    pub fn query_bet(&self, tx_hash: &[u8; 32]) -> Result<Option<BetRecord>> {
        let storage = self.storage.read().unwrap();
        storage.get_bet(tx_hash)
    }

    /// Get current height
    pub fn get_height(&self) -> u64 {
        self.height
    }

    /// Get chain ID
    pub fn get_chain_id(&self) -> &str {
        &self.chain_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_app() -> (MyChainApp, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let app = MyChainApp::new(
            temp_dir.path().to_str().unwrap(),
            "test-chain".to_string(),
        ).unwrap();
        (app, temp_dir)
    }

    #[test]
    fn test_app_creation() {
        let (_app, _temp_dir) = create_test_app();
        // App creation should not panic
    }

    #[test]
    fn test_validate_tx() {
        let (app, _temp_dir) = create_test_app();
        
        let tx = TxFlip {
            version: 1,
            wallet: [1u8; 32],
            amount: 1000,
            nonce: 1,
        };
        
        let tx_bytes = tx.to_bytes().unwrap();
        let result = app.validate_tx(&tx_bytes);
        assert!(result.is_ok());
    }

    #[test]
    fn test_process_block() {
        let (mut app, _temp_dir) = create_test_app();
        
        let tx = TxFlip {
            version: 1,
            wallet: [1u8; 32],
            amount: 1000,
            nonce: 1,
        };
        
        let tx_bytes = tx.to_bytes().unwrap();
        let txs = vec![tx_bytes];
        
        let bet_records = app.process_block(txs).unwrap();
        assert_eq!(bet_records.len(), 1);
        assert_eq!(bet_records[0].amount, 1000);
        assert_eq!(app.get_height(), 1);
    }

    #[test]
    fn test_query_bet() {
        let (mut app, _temp_dir) = create_test_app();
        
        let tx = TxFlip {
            version: 1,
            wallet: [1u8; 32],
            amount: 1000,
            nonce: 1,
        };
        
        let tx_bytes = tx.to_bytes().unwrap();
        let tx_hash = blake3::hash(&tx_bytes);
        let tx_hash_bytes: [u8; 32] = *tx_hash.as_bytes();
        
        let txs = vec![tx_bytes];
        app.process_block(txs).unwrap();
        
        let bet_record = app.query_bet(&tx_hash_bytes).unwrap().unwrap();
        assert_eq!(bet_record.amount, 1000);
    }
}