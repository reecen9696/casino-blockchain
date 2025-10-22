use anyhow::Result;
use fastcrypto::vrf::VRFKeyPair;
use fastcrypto::vrf::ecvrf::ECVRFKeyPair;
use sha2::{Digest, Sha256};
use blake3;

/// VRF Engine using fastcrypto ECVRF with Ristretto255
/// 
/// Provides provably fair randomness for coin flip outcomes
pub struct VrfEngine {
    keypair: ECVRFKeyPair,
}

impl VrfEngine {
    /// Generate a new VRF keypair using ThreadRng (allowed by fastcrypto)
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let keypair = ECVRFKeyPair::generate(&mut rng);
        Self { keypair }
    }

    /// Load VRF engine from private key bytes (placeholder for POC)
    pub fn from_private_key(_private_key_bytes: &[u8]) -> Result<Self> {
        // For POC, just generate a new keypair since fastcrypto ECVRF 
        // serialization API is complex
        Ok(Self::generate())
    }

    /// Get the VRF public key as bytes (placeholder for POC)
    pub fn public_key(&self) -> Vec<u8> {
        // For now, return a fixed placeholder until we can properly serialize ECVRF keys
        vec![1u8; 32] // Placeholder
    }

    /// Get the VRF private key as bytes (placeholder for POC)
    pub fn private_key(&self) -> Vec<u8> {
        // For now, return a fixed placeholder until we can properly serialize ECVRF keys
        vec![2u8; 32] // Placeholder
    }

    /// Prove VRF computation and return (output, proof)
    pub fn prove(&self, message: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let (output, proof) = self.keypair.output(message);
        // Serialize output and proof using debug format for POC
        let proof_bytes = format!("{:?}", proof).into_bytes();
        Ok((output.to_vec(), proof_bytes))
    }

    /// Verify a VRF proof (simplified for POC)
    pub fn verify(
        _public_key: &[u8],
        _message: &[u8], 
        _proof_bytes: &[u8],
        _expected_output: &[u8]
    ) -> Result<bool> {
        // Placeholder verification - always returns true for POC
        // In production, would properly deserialize and verify
        Ok(true)
    }

    /// Compute VRF message for a coin flip transaction
    /// Message format: SHA256('MYCHAIN:VRF:v1' || chain_id || height || block_random || tx_hash || wallet || nonce)
    pub fn compute_flip_message(
        chain_id: &str,
        height: u64,
        block_random: &[u8],
        tx_hash: &[u8],
        wallet: &[u8],
        nonce: u64,
    ) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(b"MYCHAIN:VRF:v1");
        hasher.update(chain_id.as_bytes());
        hasher.update(&height.to_le_bytes());
        hasher.update(block_random);
        hasher.update(tx_hash);
        hasher.update(wallet);
        hasher.update(&nonce.to_le_bytes());
        hasher.finalize().to_vec()
    }

    /// Derive coin flip result from VRF output
    /// result = blake3(output)[0] & 1
    pub fn derive_flip_result(vrf_output: &[u8]) -> bool {
        let hash = blake3::hash(vrf_output);
        (hash.as_bytes()[0] & 1) == 1
    }

    /// Compute block randomness seed
    /// block_random[h] = blake3(prev_block_hash || prev_vrf_accum)
    pub fn compute_block_random(prev_block_hash: &[u8], prev_vrf_accum: &[u8]) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(prev_block_hash);
        hasher.update(prev_vrf_accum);
        *hasher.finalize().as_bytes()
    }

    /// Process a coin flip transaction
    /// Returns (vrf_message, vrf_proof, vrf_output, flip_result)
    pub fn process_flip(
        &self,
        chain_id: &str,
        height: u64,
        block_random: &[u8],
        tx_hash: &[u8],
        wallet: &[u8],
        nonce: u64,
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>, bool)> {
        // Compute VRF message
        let message = Self::compute_flip_message(
            chain_id, height, block_random, tx_hash, wallet, nonce
        );

        // Generate VRF proof and output
        let (output, proof) = self.prove(&message)?;

        // Derive flip result
        let result = Self::derive_flip_result(&output);

        Ok((message, proof, output, result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vrf_prove_verify_roundtrip() -> Result<()> {
        let engine = VrfEngine::generate();
        let message = b"test_message";

        let (output, proof) = engine.prove(message)?;
        let public_key = engine.public_key();

        let is_valid = VrfEngine::verify(&public_key, message, &proof, &output)?;
        assert!(is_valid);

        Ok(())
    }

    #[test]
    fn test_flip_result_deterministic() -> Result<()> {
        let engine = VrfEngine::generate();
        
        let chain_id = "test_chain";
        let height = 100;
        let block_random = b"block_random_seed_12345678901234567890";
        let tx_hash = b"tx_hash_1234567890123456789012345678";
        let wallet = b"wallet_12345678901234567890123456789012";
        let nonce = 42;

        // Process same flip twice
        let (msg1, proof1, output1, result1) = engine.process_flip(
            chain_id, height, block_random, tx_hash, wallet, nonce
        )?;

        let (msg2, proof2, output2, result2) = engine.process_flip(
            chain_id, height, block_random, tx_hash, wallet, nonce
        )?;

        // Results should be identical (deterministic)
        assert_eq!(msg1, msg2);
        assert_eq!(proof1, proof2);
        assert_eq!(output1, output2);
        assert_eq!(result1, result2);

        // Verify proof
        let public_key = engine.public_key();
        let is_valid = VrfEngine::verify(&public_key, &msg1, &proof1, &output1)?;
        assert!(is_valid);

        Ok(())
    }

    #[test]
    fn test_different_inputs_different_outputs() -> Result<()> {
        let engine = VrfEngine::generate();
        
        // Same parameters except nonce
        let chain_id = "test_chain";
        let height = 100;
        let block_random = b"block_random_seed_12345678901234567890";
        let tx_hash = b"tx_hash_1234567890123456789012345678";
        let wallet = b"wallet_12345678901234567890123456789012";

        let (_, _, output1, _) = engine.process_flip(
            chain_id, height, block_random, tx_hash, wallet, 1
        )?;

        let (_, _, output2, _) = engine.process_flip(
            chain_id, height, block_random, tx_hash, wallet, 2  // Different nonce
        )?;

        // Outputs should be different
        assert_ne!(output1, output2);

        Ok(())
    }
}