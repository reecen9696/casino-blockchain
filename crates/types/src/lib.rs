use serde::{Deserialize, Serialize};

/// Transaction for a coin flip bet
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TxFlip {
    /// Version for future compatibility
    pub version: u8,
    /// Wallet address (32 bytes)
    pub wallet: [u8; 32],
    /// Bet amount in minimal units
    pub amount: u64,
    /// Nonce to prevent replay attacks
    pub nonce: u64,
}

impl TxFlip {
    /// Create a new flip transaction
    pub fn new(wallet: [u8; 32], amount: u64, nonce: u64) -> Self {
        Self {
            version: 1,
            wallet,
            amount,
            nonce,
        }
    }

    /// Serialize to bytes using bincode
    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize from bytes using bincode
    pub fn from_bytes(data: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(data)
    }

    /// Get transaction hash (BLAKE3 of serialized data)
    pub fn hash(&self) -> Result<[u8; 32], bincode::Error> {
        let bytes = self.to_bytes()?;
        Ok(*blake3::hash(&bytes).as_bytes())
    }
}

/// Record of a completed bet stored in state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetRecord {
    /// Wallet address
    pub wallet: [u8; 32],
    /// Bet amount
    pub amount: u64,
    /// Nonce used
    pub nonce: u64,
    /// VRF message that was signed
    pub vrf_message: Vec<u8>,
    /// VRF proof
    pub vrf_proof: Vec<u8>,
    /// VRF output
    pub vrf_output: Vec<u8>,
    /// Coin flip result (true = heads, false = tails)
    pub result: bool,
    /// Block height where bet was processed
    pub height: u64,
    /// Transaction hash
    pub tx_hash: [u8; 32],
}

impl BetRecord {
    /// Serialize to bytes using bincode
    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize from bytes using bincode
    pub fn from_bytes(data: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(data)
    }
}

/// Application state hash computation
pub fn compute_app_hash(height: u64, block_random: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&height.to_be_bytes());
    hasher.update(block_random);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_flip_serialization() {
        let tx = TxFlip::new([1u8; 32], 1000, 42);
        let bytes = tx.to_bytes().unwrap();
        let recovered = TxFlip::from_bytes(&bytes).unwrap();
        assert_eq!(tx, recovered);
    }

    #[test]
    fn test_tx_flip_hash() {
        let tx = TxFlip::new([1u8; 32], 1000, 42);
        let hash1 = tx.hash().unwrap();
        let hash2 = tx.hash().unwrap();
        assert_eq!(hash1, hash2);

        // Different nonce should produce different hash
        let tx2 = TxFlip::new([1u8; 32], 1000, 43);
        let hash3 = tx2.hash().unwrap();
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_bet_record_serialization() {
        let record = BetRecord {
            wallet: [2u8; 32],
            amount: 500,
            nonce: 123,
            msg: vec![1, 2, 3],
            proof: vec![4, 5, 6],
            output: vec![7, 8, 9],
            result: true,
            height: 100,
            tx_hash: [3u8; 32],
        };

        let bytes = record.to_bytes().unwrap();
        let recovered = BetRecord::from_bytes(&bytes).unwrap();
        assert_eq!(record.wallet, recovered.wallet);
        assert_eq!(record.amount, recovered.amount);
        assert_eq!(record.result, recovered.result);
    }

    #[test]
    fn test_app_hash_deterministic() {
        let hash1 = compute_app_hash(100, &[1u8; 32]);
        let hash2 = compute_app_hash(100, &[1u8; 32]);
        assert_eq!(hash1, hash2);

        // Different inputs should produce different hashes
        let hash3 = compute_app_hash(101, &[1u8; 32]);
        let hash4 = compute_app_hash(100, &[2u8; 32]);
        assert_ne!(hash1, hash3);
        assert_ne!(hash1, hash4);
    }
}