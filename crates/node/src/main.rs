use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mychain_app::MyChainApp;
use std::path::PathBuf;
use tracing::{info, error};
use tracing_subscriber;

#[derive(Parser)]
#[command(name = "mychain-node")]
#[command(about = "MyChain ABCI node for coin flip blockchain")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the ABCI server
    Start {
        /// ABCI server bind address
        #[arg(long, default_value = "127.0.0.1:26658")]
        abci_addr: String,
        
        /// Storage directory path
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
        
        /// HTTP API server address
        #[arg(long, default_value = "127.0.0.1:3000")]
        api_addr: String,
    },
    /// Initialize node configuration
    Init {
        /// Storage directory path
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start { abci_addr, data_dir, api_addr } => {
            start_node(abci_addr, data_dir, api_addr).await
        }
        Commands::Init { data_dir } => {
            init_node(data_dir).await
        }
    }
}

async fn start_node(abci_addr: String, data_dir: PathBuf, api_addr: String) -> Result<()> {
    info!("Starting MyChain node...");
    info!("ABCI server: {}", abci_addr);
    info!("Data directory: {}", data_dir.display());
    info!("API server: {}", api_addr);

    // Ensure data directory exists
    std::fs::create_dir_all(&data_dir)
        .context("Failed to create data directory")?;

    // Create ABCI application
    let app = MyChainApp::new(&data_dir)
        .context("Failed to create MyChain application")?;

    // Start ABCI server
    info!("ABCI server listening on: {}", abci_addr);

    // Start both ABCI server and HTTP API concurrently
    let abci_handle = tokio::spawn(async move {
        // Use tower-abci to create the server
        match app.create_server().await {
            Ok(server) => {
                if let Err(e) = server.listen_tcp(&abci_addr).await {
                    error!("ABCI server error: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to create ABCI server: {}", e);
            }
        }
    });

    let api_handle = tokio::spawn(async move {
        if let Err(e) = start_api_server(api_addr).await {
            error!("API server error: {}", e);
        }
    });

    // Wait for either server to exit
    tokio::select! {
        _ = abci_handle => {
            error!("ABCI server exited");
        }
        _ = api_handle => {
            error!("API server exited");
        }
    }

    Ok(())
}

async fn start_api_server(api_addr: String) -> Result<()> {
    use axum::{
        extract::State,
        http::StatusCode,
        response::Json,
        routing::{get, post},
        Router,
    };
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize)]
    struct FlipRequest {
        wallet: String,
        amount: u64,
        nonce: u64,
    }

    #[derive(Serialize)]
    struct FlipResponse {
        tx_hash: String,
        height: u64,
        result: String,
        vrf_proof: String,
        vrf_output: String,
        vrf_public_key: String,
    }

    #[derive(Clone)]
    struct ApiState {
        cometbft_rpc_url: String,
    }

    async fn health() -> &'static str {
        "MyChain API Server"
    }

    async fn flip(
        State(state): State<ApiState>,
        Json(request): Json<FlipRequest>,
    ) -> Result<Json<FlipResponse>, StatusCode> {
        // Decode wallet hex
        let wallet_bytes = hex::decode(&request.wallet)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        
        if wallet_bytes.len() != 32 {
            return Err(StatusCode::BAD_REQUEST);
        }

        let mut wallet = [0u8; 32];
        wallet.copy_from_slice(&wallet_bytes);

        // Create TxFlip
        let tx = mychain_types::TxFlip {
            version: 1,
            wallet,
            amount: request.amount,
            nonce: request.nonce,
        };

        // Serialize transaction
        let tx_bytes = bincode::serialize(&tx)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Submit to CometBFT via broadcast_tx_commit
        let client = reqwest::Client::new();
        let tx_base64 = base64::encode(&tx_bytes);
        let rpc_url = format!("{}/broadcast_tx_commit", state.cometbft_rpc_url);
        
        let response = client
            .get(&rpc_url)
            .query(&[("tx", &tx_base64)])
            .send()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

        if !response.status().is_success() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }

        // For now, return a placeholder response
        // In production: parse CometBFT response and extract results from events
        let tx_hash = hex::encode(blake3::hash(&tx_bytes).as_bytes());
        
        Ok(Json(FlipResponse {
            tx_hash,
            height: 0, // TODO: extract from response
            result: "pending".to_string(), // TODO: extract from events
            vrf_proof: "".to_string(), // TODO: extract from events
            vrf_output: "".to_string(), // TODO: extract from events  
            vrf_public_key: "".to_string(), // TODO: extract from events
        }))
    }

    let state = ApiState {
        cometbft_rpc_url: "http://127.0.0.1:26657".to_string(),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/flip", post(flip))
        .with_state(state);

    info!("API server listening on: {}", api_addr);

    let listener = tokio::net::TcpListener::bind(&api_addr).await
        .context("Failed to bind API server")?;

    axum::serve(listener, app).await
        .context("API server failed")?;

    Ok(())
}

async fn init_node(data_dir: PathBuf) -> Result<()> {
    info!("Initializing MyChain node...");
    info!("Data directory: {}", data_dir.display());

    // Create data directory
    std::fs::create_dir_all(&data_dir)
        .context("Failed to create data directory")?;

    // Initialize storage
    let app = MyChainApp::new(&data_dir)
        .context("Failed to initialize application")?;

    info!("Node initialized successfully");
    info!("To start the node: mychain-node start --data-dir {}", data_dir.display());
    info!("ABCI server will listen on: 127.0.0.1:26658");
    info!("API server will listen on: 127.0.0.1:3000");

    Ok(())
}