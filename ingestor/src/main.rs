use dotenvy::dotenv;
use ingestor::{Ingestor, IngestorEvent};
use state::{RocksDbBackend, StateEngine};
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    dotenv().ok();

    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let provider_url = env::var("PROVIDER_URL").expect("PROVIDER_URL must be set");
    let bridge_contract = env::var("BRIDGE_CONTRACT").expect("BRIDGE_CONTRACT must be set");
    let db_path = env::var("STATE_DB_PATH").unwrap_or_else(|_| "./data/state".to_string());

    let backend = RocksDbBackend::open(&db_path)
        .expect("Failed to open RocksDB — check STATE_DB_PATH and permissions");
    let engine = Arc::new(StateEngine::new(backend).expect("Failed to initialize StateEngine"));

    tracing::info!(db_path, root = %engine.state_root(), "StateEngine ready");

    let (tx, mut rx) = mpsc::channel::<IngestorEvent>(100);

    let engine_processor = Arc::clone(&engine);
    tokio::spawn(async move {
        tracing::info!("Event Processor started");
        while let Some(event) = rx.recv().await {
            match event {
                IngestorEvent::L1Deposit { user, amount } => {
                    tracing::warn!(
                        user = %user,
                        amount = %amount,
                        "PROCESSOR: Applying L1 deposit to L2 state"
                    );
                    match engine_processor.apply_deposit(user, amount) {
                        Ok(root) => {
                            tracing::info!(
                                user = %user,
                                root = %root,
                                "STATE: Deposit applied — new state root"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "STATE: Failed to apply deposit");
                        }
                    }
                }
                IngestorEvent::NewBlock { number, .. } => {
                    tracing::info!(
                        block = number,
                        root = %engine_processor.state_root(),
                        "PROCESSOR: Synced with L1 block"
                    );
                }
            }
        }
    });

    loop {
        tracing::info!("Starting Ingestor instance...");

        let ingestor = Ingestor::new(provider_url.clone(), bridge_contract.clone(), tx.clone());

        if let Err(e) = ingestor.run().await {
            tracing::error!(error = %e, "Ingestor disconnected — reconnecting in 5 seconds");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}
