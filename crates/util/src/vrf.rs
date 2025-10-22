use anyhow::{anyhow, Result};
use p256::ecdsa::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use vrf::openssl::{CipherSuite, ECVRF};
use vrf::VRF;

/// VRF implementation using P-256 curve
pub struct VrfEngine {
    vrf: ECVRF,
    private_key: SigningKey,
    public_key: VerifyingKey,
}

impl VrfEngine {
    /// Generate a new VRF keypair
    pub fn generate() -> Result<Self> {
        let private_key = SigningKey::random(&mut OsRng);
        let public_key = VerifyingKey::from(&private_key);
        let vrf = ECVRF::from_suite(CipherSuite::SECP256K1_SHA256_TAI)
            .map_err(|e| anyhow!("Failed to create VRF: {:?}", e))?;

        Ok(Self {
            vrf,
            private_key,
            public_key,
        })
    }

    /// Load VRF from existing private key bytes
    pub fn from_private_key(private_key_bytes: &[u8]) -> Result<Self> {
        let private_key = SigningKey::from_slice(private_key_bytes)
            .map_err(|e| anyhow!("Invalid private key: {}", e))?;
        let public_key = VerifyingKey::from(&private_key);
        let vrf = ECVRF::from_suite(CipherSuite::SECP256K1_SHA256_TAI)
            .map_err(|e| anyhow!("Failed to create VRF: {:?}", e))?;

        Ok(Self {
            vrf,
            private_key,
            public_key,
        })
    }

    /// Get the public key bytes
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.public_key.to_encoded_point(false).as_bytes().to_vec()
    }

    /// Get the private key bytes (for persistence)
    pub fn private_key_bytes(&self) -> Vec<u8> {
        self.private_key.to_bytes().to_vec()
    }

    /// Compute VRF message according to specification
    pub fn compute_message(
        &self,
        chain_id: &str,
        height: u64,
        block_random: &[u8; 32],
        tx_hash: &[u8; 32],
        wallet: &[u8; 32],
        nonce: u64,
    ) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(b"MYCHAIN:VRF:v1");
        hasher.update(chain_id.as_bytes());
        hasher.update(&height.to_be_bytes());
        hasher.update(block_random);
        hasher.update(tx_hash);
        hasher.update(wallet);
        hasher.update(&nonce.to_be_bytes());
        hasher.finalize().to_vec()
    }

    /// Prove VRF for given message
    pub fn prove(&mut self, message: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let proof = self
            .vrf
            .prove(&self.private_key.to_bytes(), message)
            .map_err(|e| anyhow!("VRF prove failed: {:?}", e))?;

        let output = self
            .vrf
            .proof_to_hash(&proof)
            .map_err(|e| anyhow!("Failed to extract VRF output: {:?}", e))?;

        Ok((proof, output))
    }

    /// Verify VRF proof
    pub fn verify(&mut self, message: &[u8], proof: &[u8], public_key: &[u8]) -> Result<Vec<u8>> {
        let output = self
            .vrf
            .verify(public_key, proof, message)
            .map_err(|e| anyhow!("VRF verification failed: {:?}", e))?;

        Ok(output)
    }

    /// Derive coin flip result from VRF output
    pub fn derive_flip_result(&self, vrf_output: &[u8]) -> bool {
        let result_hash = blake3::hash(vrf_output);
        let result_bytes = result_hash.as_bytes();
        (result_bytes[0] & 1) == 1
    }
}

/// Compute block randomness seed
pub fn compute_block_random(prev_block_hash: &[u8], prev_vrf_accum: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(prev_block_hash);
    hasher.update(prev_vrf_accum);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vrf_engine_generation() {
        let engine = VrfEngine::generate().unwrap();
        let pub_key = engine.public_key_bytes();
        assert!(!pub_key.is_empty());
        
        let priv_key = engine.private_key_bytes();
        assert!(!priv_key.is_empty());
    }

    #[test]
    fn test_vrf_prove_verify_round_trip() {
        let mut engine = VrfEngine::generate().unwrap();
        let message = b"test message";
        
        let (proof, output) = engine.prove(message).unwrap();
        let pub_key = engine.public_key_bytes();
        
        let verified_output = engine.verify(message, &proof, &pub_key).unwrap();
        assert_eq!(output, verified_output);
    }

    #[test]
    fn test_vrf_deterministic() {
        let mut engine = VrfEngine::generate().unwrap();
        let message = b"test message";
        
        let (proof1, output1) = engine.prove(message).unwrap();
        let (proof2, output2) = engine.prove(message).unwrap();
        
        assert_eq!(proof1, proof2);
        assert_eq!(output1, output2);
    }

    #[test]
    fn test_flip_result_derivation() {
        let engine = VrfEngine::generate().unwrap();
        let output = vec![0u8; 32]; // Even first byte -> false
        let result1 = engine.derive_flip_result(&output);
        assert!(!result1);
        
        let output = vec![1u8; 32]; // Odd first byte -> true
        let result2 = engine.derive_flip_result(&output);
        assert!(result2);
    }

    #[test]
    fn test_compute_message() {
        let engine = VrfEngine::generate().unwrap();
        
        let msg1 = engine.compute_message(
            "test-chain",
            100,
            &[1u8; 32],
            &[2u8; 32],
            &[3u8; 32],
            42,
        );
        
        let msg2 = engine.compute_message(
            "test-chain",
            100,
            &[1u8; 32],
            &[2u8; 32],
            &[3u8; 32],
            42,
        );
        
        assert_eq!(msg1, msg2); // Same inputs = same message
        
        let msg3 = engine.compute_message(
            "test-chain",
            101, // Different height
            &[1u8; 32],
            &[2u8; 32],
            &[3u8; 32],
            42,
        );
        
        assert_ne!(msg1, msg3); // Different inputs = different message
    }

    #[test]
    fn test_block_random_computation() {
        let random1 = compute_block_random(&[1u8; 32], &[2u8; 32]);
        let random2 = compute_block_random(&[1u8; 32], &[2u8; 32]);
        assert_eq!(random1, random2); // Deterministic
        
        let random3 = compute_block_random(&[2u8; 32], &[2u8; 32]);
        assert_ne!(random1, random3); // Different inputs
    }
}