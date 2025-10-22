mod simple;

use anyhow::Result;
use simple::MyChainApp;
use mychain_types::TxFlip;
use std::env;
use tracing::info;
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting MyChain application (simplified)");

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    
    let storage_path = args.get(1)
        .cloned()
        .unwrap_or_else(|| "./data".to_string());

    let chain_id = args.get(2)
        .cloned()
        .unwrap_or_else(|| "mychain".to_string());

    info!("Configuration:");
    info!("  Storage path: {}", storage_path);
    info!("  Chain ID: {}", chain_id);

    // Create the application
    let mut app = MyChainApp::new(&storage_path, chain_id)?;
    
    info!("MyChain application initialized successfully");
    info!("Current height: {}", app.get_height());

    // Demo: Create and process some test transactions
    info!("Processing demo transactions...");
    
    let demo_txs = vec![
        TxFlip {
            version: 1,
            wallet: [1u8; 32],
            amount: 1000,
            nonce: 1,
        },
        TxFlip {
            version: 1,
            wallet: [2u8; 32],
            amount: 500,
            nonce: 1,
        },
        TxFlip {
            version: 1,
            wallet: [3u8; 32],
            amount: 2000,
            nonce: 1,
        },
    ];

    // Convert to bytes
    let tx_bytes: Result<Vec<Vec<u8>>> = demo_txs
        .into_iter()
        .map(|tx| tx.to_bytes().map_err(Into::into))
        .collect();
    
    let tx_bytes = tx_bytes?;
    
    // Process the block
    let bet_records = app.process_block(tx_bytes.clone())?;
    
    info!("Processed {} transactions in block {}", bet_records.len(), app.get_height());
    
    // Display results
    for (i, bet_record) in bet_records.iter().enumerate() {
        let tx_hash = blake3::hash(&tx_bytes[i]);
        info!("  Bet {}: {} coins -> {} ({})", 
            hex::encode(&tx_hash.as_bytes()[..8]),
            bet_record.amount,
            if bet_record.result { "heads" } else { "tails" },
            if bet_record.result { "WIN" } else { "LOSE" }
        );
    }

    info!("Demo completed successfully!");
    info!("Application is ready for real ABCI integration with CometBFT");

    Ok(())
}