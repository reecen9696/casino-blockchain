pub mod vrf;

use anyhow::{Context, Result};
use mychain_storage::Storage;
use mychain_types::{BetRecord, TxFlip};
use std::path::Path;
use tower::service_fn;
use tower_abci::v038::ServerBuilder;
use tendermint::v0_38::abci::{request, response};
use tendermint::AppHash;
use vrf::VrfEngine;
use tracing::{info, warn, error};

/// MyChain ABCI application state
#[derive(Clone)]
pub struct MyChainApp {
    storage_path: String,
}

// Ensure MyChainApp is Send + Sync
unsafe impl Send for MyChainApp {}
unsafe impl Sync for MyChainApp {}

impl MyChainApp {
    pub fn new<P: AsRef<Path>>(storage_path: P) -> Result<Self> {
        let storage_path = storage_path.as_ref().to_string_lossy().to_string();
        
        // Test storage connection
        let _storage = Storage::open(&storage_path)
            .context("Failed to open storage")?;
        
        Ok(Self { storage_path })
    }

    /// Get a storage instance (for per-request access)
    fn storage(&self) -> Result<Storage> {
        Storage::open(&self.storage_path)
            .context("Failed to open storage")
    }

    /// Process a flip transaction and generate VRF result
    fn process_flip(
        &self,
        tx: &TxFlip,
        height: u64,
        vrf_engine: &VrfEngine,
        chain_id: &str,
    ) -> Result<BetRecord> {
        // Create VRF message from transaction data
        let tx_hash = tx.hash()?;
        let block_random = height.to_le_bytes(); // Simplified for POC
        
        // Process VRF computation
        let (vrf_message, vrf_proof, vrf_output, flip_result) = vrf_engine.process_flip(
            chain_id,
            height,
            &block_random,
            &tx_hash,
            &tx.wallet,
            tx.nonce,
        )?;

        // Create bet record
        let record = BetRecord {
            wallet: tx.wallet,
            amount: tx.amount,
            nonce: tx.nonce,
            vrf_message,
            vrf_proof,
            vrf_output,
            result: flip_result,
            height,
            tx_hash,
        };

        Ok(record)
    }

    /// Create the ABCI server using tower-abci v0.19 API
    pub async fn create_server(&self) -> Result<tower_abci::v038::Server<
        impl tower::Service<tendermint::v0_38::abci::ConsensusRequest, Response = tendermint::v0_38::abci::ConsensusResponse, Error = tower_abci::BoxError> + Clone,
        impl tower::Service<tendermint::v0_38::abci::MempoolRequest, Response = tendermint::v0_38::abci::MempoolResponse, Error = tower_abci::BoxError> + Clone,
        impl tower::Service<tendermint::v0_38::abci::InfoRequest, Response = tendermint::v0_38::abci::InfoResponse, Error = tower_abci::BoxError> + Clone,
        impl tower::Service<tendermint::v0_38::abci::SnapshotRequest, Response = tendermint::v0_38::abci::SnapshotResponse, Error = tower_abci::BoxError> + Clone,
    >> {
        let app = self.clone();

        // Info service
        let info = {
            let app = app.clone();
            service_fn(move |request: tendermint::v0_38::abci::InfoRequest| {
                let app = app.clone();
                async move {
                    use tendermint::v0_38::abci::{InfoRequest, InfoResponse};
                    
                    match request {
                        InfoRequest::Echo(echo) => {
                            Ok(InfoResponse::Echo(response::Echo {
                                message: echo.message,
                            }))
                        }
                        InfoRequest::Info(_) => {
                            let storage = match app.storage() {
                                Ok(storage) => storage,
                                Err(e) => {
                                    error!("Failed to open storage: {}", e);
                                    return Ok(InfoResponse::Info(response::Info {
                                        data: "MyChain Casino".to_string(),
                                        version: "0.1.0".to_string(),
                                        app_version: 1,
                                        last_block_height: 0u32.into(),
                                        last_block_app_hash: AppHash::try_from(vec![]).unwrap_or_default(),
                                    }));
                                }
                            };

                            let last_block_height = storage.get_last_height().unwrap_or(0);
                            let last_app_hash = storage.get_app_hash(last_block_height)
                                .unwrap_or(None)
                                .unwrap_or([0u8; 32]);

                            info!("Info request: height={}, app_hash={}", 
                                  last_block_height, hex::encode(&last_app_hash));

                            Ok(InfoResponse::Info(response::Info {
                                data: "MyChain Casino".to_string(),
                                version: "0.1.0".to_string(),
                                app_version: 1,
                                last_block_height: (last_block_height as u32).into(),
                                last_block_app_hash: AppHash::try_from(last_app_hash.to_vec()).unwrap_or_default(),
                            }))
                        }
                        InfoRequest::Query(query) => {
                            let response = app.handle_query(query).await.unwrap_or_default();
                            Ok(InfoResponse::Query(response))
                        }
                    }
                }
            })
        };

        // Mempool service (CheckTx)
        let mempool = {
            service_fn(move |request: tendermint::v0_38::abci::MempoolRequest| async move {
                // Basic transaction validation
                let tx_bytes = match request {
                    tendermint::v0_38::abci::MempoolRequest::CheckTx(ref req) => &req.tx,
                };
                match bincode::deserialize::<TxFlip>(tx_bytes) {
                    Ok(tx) => {
                        // Validate transaction format
                        if tx.amount == 0 {
                            return Ok(tendermint::v0_38::abci::MempoolResponse::CheckTx(response::CheckTx {
                            code: 1u32.into(),
                            log: "Invalid amount: must be greater than 0".to_string(),
                            ..Default::default()
                        }));
                        }

                        if tx.wallet == [0u8; 32] {
                            return Ok(tendermint::v0_38::abci::MempoolResponse::CheckTx(response::CheckTx {
                            code: 2u32.into(),
                            log: "Invalid wallet: cannot be zero".to_string(),
                            ..Default::default()
                        }));
                    }

                    Ok(tendermint::v0_38::abci::MempoolResponse::CheckTx(response::CheckTx {
                        code: 0u32.into(),
                        log: "Transaction valid".to_string(),
                        ..Default::default()
                    }))
                }
                }
                Err(e) => Ok(tendermint::v0_38::abci::MempoolResponse::CheckTx(response::CheckTx {
                    code: 3u32.into(),
                    log: format!("Failed to decode transaction: {}", e),
                    ..Default::default()
                }))
            })
        };

        // Consensus service
        let consensus = {
            let app = app.clone();
            service_fn(move |request: tendermint::v0_38::abci::ConsensusRequest| {
                let app = app.clone();
                async move {
                    use tendermint::v0_38::abci::{ConsensusRequest, ConsensusResponse};
                    
                    match request {
                        ConsensusRequest::InitChain(req) => {
                            info!("InitChain request: chain_id={}", req.chain_id);
                            
                            let storage = match app.storage() {
                                Ok(storage) => storage,
                                Err(e) => {
                                    error!("Failed to open storage: {}", e);
                                    return Ok(ConsensusResponse::InitChain(response::InitChain {
                                        consensus_params: Some(req.consensus_params),
                                        validators: req.validators,
                                        app_hash: AppHash::try_from(vec![0u8; 32]).unwrap_or_default(),
                                    }));
                                }
                            };

                            // Initialize VRF engine
                            let vrf_engine = VrfEngine::generate();
                            
                            // Store initial state
                            let mut batch = storage.batch();
                            if let Err(e) = storage.set_last_height(0, &mut batch) {
                                error!("Failed to set initial height: {}", e);
                            }
                            if let Err(e) = storage.set_vrf_public_key(&vrf_engine.public_key(), &mut batch) {
                                error!("Failed to set VRF public key: {}", e);
                            }
                            if let Err(e) = storage.apply_batch(batch) {
                                error!("Failed to apply initial batch: {}", e);
                            }

                            Ok(ConsensusResponse::InitChain(response::InitChain {
                                consensus_params: Some(req.consensus_params),
                                validators: req.validators,
                                app_hash: AppHash::try_from(vec![0u8; 32]).unwrap_or_default(),
                            }))
                        }
                        ConsensusRequest::FinalizeBlock(req) => {
                            let height = req.height.value();
                            info!("FinalizeBlock: height={}, tx_count={}", height, req.txs.len());

                            let storage = match app.storage() {
                                Ok(storage) => storage,
                                Err(e) => {
                                    error!("Failed to open storage: {}", e);
                                    return Ok(ConsensusResponse::FinalizeBlock(response::FinalizeBlock {
                                        events: vec![],
                                        tx_results: vec![],
                                        validator_updates: vec![],
                                        consensus_param_updates: None,
                                        app_hash: AppHash::try_from(vec![]).unwrap_or_default(),
                                    }));
                                }
                            };

                            // Initialize VRF engine for this block
                            let vrf_engine = match storage.get_vrf_public_key() {
                                Ok(Some(_)) => VrfEngine::generate(), // Simplified: generate new each time
                                Ok(None) => {
                                    warn!("No VRF key found, generating new one");
                                    VrfEngine::generate()
                                }
                                Err(e) => {
                                    error!("Failed to get VRF key: {}", e);
                                    return Ok(ConsensusResponse::FinalizeBlock(response::FinalizeBlock {
                                        events: vec![],
                                        tx_results: vec![],
                                        validator_updates: vec![],
                                        consensus_param_updates: None,
                                        app_hash: AppHash::try_from(vec![]).unwrap_or_default(),
                                    }));
                                }
                            };

                            let mut all_events = Vec::new();
                            let mut bet_records = Vec::new();

                            // Process each transaction
                            for (tx_index, tx_bytes) in req.txs.iter().enumerate() {
                                match bincode::deserialize::<TxFlip>(tx_bytes) {
                                    Ok(tx) => {
                                        match app.process_flip(&tx, height, &vrf_engine, "mychain") {
                                            Ok(record) => {
                                                bet_records.push((tx_bytes.clone(), record.clone()));

                                                // Create event
                                                let event = tendermint::abci::Event {
                                                    kind: "flip".to_string(),
                                                    attributes: vec![
                                                        ("wallet".to_string(), hex::encode(record.wallet)).into(),
                                                        ("amount".to_string(), record.amount.to_string()).into(),
                                                        ("result".to_string(), if record.result { "heads" } else { "tails" }.to_string()).into(),
                                                        ("tx_hash".to_string(), hex::encode(record.tx_hash)).into(),
                                                        ("vrf_proof".to_string(), hex::encode(&record.vrf_proof)).into(),
                                                        ("vrf_output".to_string(), hex::encode(&record.vrf_output)).into(),
                                                    ],
                                                };
                                                all_events.push(event);
                                            }
                                            Err(e) => {
                                                error!("Failed to process flip {}: {}", tx_index, e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to parse transaction {}: {}", tx_index, e);
                                    }
                                }
                            }

                            // Store records
                            let mut batch = storage.batch();
                            for (tx_bytes, record) in bet_records {
                                let tx_hash = blake3::hash(&tx_bytes);
                                if let Err(e) = storage.store_bet(tx_hash.as_bytes(), &record, &mut batch) {
                                    error!("Failed to store bet record: {}", e);
                                }
                                if let Err(e) = storage.store_tx_height(tx_hash.as_bytes(), height, &mut batch) {
                                    error!("Failed to store tx height: {}", e);
                                }
                            }

                            // Update height
                            if let Err(e) = storage.set_last_height(height, &mut batch) {
                                error!("Failed to set height: {}", e);
                            }

                            // Compute and store app hash
                            let app_hash = storage.compute_app_hash(height).unwrap_or([0u8; 32]);
                            if let Err(e) = storage.store_app_hash(height, &app_hash, &mut batch) {
                                error!("Failed to store app hash: {}", e);
                            }

                            // Apply batch atomically
                            if let Err(e) = storage.apply_batch(batch) {
                                error!("Failed to apply finalize batch: {}", e);
                            }

                            info!("Finalized block: height={}, app_hash={}", 
                                  height, hex::encode(&app_hash));

                            Ok(ConsensusResponse::FinalizeBlock(response::FinalizeBlock {
                                events: all_events,
                                tx_results: vec![], // Will be filled by CometBFT
                                validator_updates: vec![],
                                consensus_param_updates: None,
                                app_hash: AppHash::try_from(app_hash.to_vec()).unwrap_or_default(),
                            }))
                        }
                        ConsensusRequest::Commit => {
                            info!("Commit");
                            // Storage is already committed in finalize_block
                            Ok(ConsensusResponse::Commit(response::Commit {
                                retain_height: 0u32.into(),
                                data: vec![].into(),
                            }))
                        }
                        // ABCI++ methods
                        ConsensusRequest::PrepareProposal(req) => {
                            info!("PrepareProposal: tx_count={}", req.txs.len());
                            // Pass through transactions unchanged for POC
                            Ok(ConsensusResponse::PrepareProposal(response::PrepareProposal {
                                txs: req.txs,
                            }))
                        }
                        ConsensusRequest::ProcessProposal(_req) => {
                            info!("ProcessProposal");
                            // Accept all proposals for POC
                            Ok(ConsensusResponse::ProcessProposal(response::ProcessProposal::Accept))
                        }
                        ConsensusRequest::ExtendVote(_req) => {
                            info!("ExtendVote");
                            // No vote extensions for POC
                            Ok(ConsensusResponse::ExtendVote(response::ExtendVote {
                                vote_extension: bytes::Bytes::new(),
                            }))
                        }
                        ConsensusRequest::VerifyVoteExtension(_req) => {
                            info!("VerifyVoteExtension");
                            // Accept all vote extensions for POC
                            Ok(ConsensusResponse::VerifyVoteExtension(response::VerifyVoteExtension::Accept))
                        }
                    }
                }
            })
        };

        // Snapshot service (stubbed for now)
        let snapshot = {
            service_fn(|_request: tendermint::v0_38::abci::SnapshotRequest| async move {
                use tendermint::v0_38::abci::SnapshotResponse;
                // Stubbed - no snapshots for POC
                Ok(SnapshotResponse::ListSnapshots(
                    response::ListSnapshots { snapshots: vec![] }
                ))
            })
        };

        let server = ServerBuilder::default()
            .consensus(consensus)
            .mempool(mempool)
            .info(info)
            .snapshot(snapshot)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("Failed to build ABCI server"))?;

        Ok(server)
    }

    /// Handle query requests  
    pub async fn handle_query(&self, request: request::Query) -> Result<response::Query> {
        let storage = match self.storage() {
            Ok(storage) => storage,
            Err(e) => {
                return Ok(response::Query {
                    code: 1u32.into(),
                    log: format!("Storage error: {}", e),
                    ..Default::default()
                });
            }
        };

        let path = &request.path;
        
        match path.as_ref() {
            "/bet" => {
                // Query bet by transaction hash
                if request.data.len() < 32 {
                    return Ok(response::Query {
                        code: 2u32.into(),
                        log: "Invalid tx hash length".to_string(),
                        ..Default::default()
                    });
                }

                match storage.get_bet(&request.data) {
                    Ok(Some(bet)) => {
                        match bincode::serialize(&bet) {
                            Ok(data) => Ok(response::Query {
                                code: 0u32.into(),
                                value: data.into(),
                                ..Default::default()
                            }),
                            Err(e) => Ok(response::Query {
                                code: 3u32.into(),
                                log: format!("Failed to serialize bet: {}", e),
                                ..Default::default()
                            })
                        }
                    }
                    Ok(None) => Ok(response::Query {
                        code: 4u32.into(),
                        log: "Bet not found".to_string(),
                        ..Default::default()
                    }),
                    Err(e) => Ok(response::Query {
                        code: 5u32.into(),
                        log: format!("Storage error: {}", e),
                        ..Default::default()
                    })
                }
            }
            _ => Ok(response::Query {
                code: 6u32.into(),
                log: format!("Unknown query path: {}", path),
                ..Default::default()
            })
        }
    }
}