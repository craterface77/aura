mod app_state;
mod config;
mod error;
mod grpc;
mod rest;

use std::sync::Arc;

use ::state::{RocksDbBackend, StateEngine};
use ingestor::{Ingestor, IngestorEvent};
use tokio::sync::mpsc;
use tonic::transport::Server as TonicServer;
use tracing_subscriber::{fmt, EnvFilter};

use app_state::AppState;
use config::Config;
use grpc::{proto::aura_l2_server::AuraL2Server, AuraL2Service};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    dotenvy::dotenv().ok();

    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    tracing::info!(
        grpc_port = config.grpc_port.0,
        rest_port = config.rest_port.0,
        db_path   = %config.state_db_path,
        "Aura L2 API starting"
    );

    // Open RocksDB as primary — API is the single writer (sequencer + API in one process).
    let backend = RocksDbBackend::open(&config.state_db_path)
        .expect("Failed to open RocksDB");
    let engine = Arc::new(StateEngine::new(backend).expect("Failed to init StateEngine"));
    tracing::info!(root = %engine.state_root(), "StateEngine ready");

    // Spawn ingestor as a background task inside this process.
    // It writes deposits into the same StateEngine via the event channel.
    let provider_url = std::env::var("PROVIDER_URL").expect("PROVIDER_URL must be set");
    let bridge_contract = std::env::var("BRIDGE_CONTRACT").expect("BRIDGE_CONTRACT must be set");

    let (tx, mut rx) = mpsc::channel::<IngestorEvent>(100);

    let engine_ingestor = Arc::clone(&engine);
    tokio::spawn(async move {
        tracing::info!("L1 event processor started");
        while let Some(event) = rx.recv().await {
            match event {
                IngestorEvent::L1Deposit { user, amount } => {
                    tracing::warn!(user = %user, amount = %amount, "Applying L1 deposit");
                    match engine_ingestor.apply_deposit(user, amount) {
                        Ok(root) => tracing::info!(user = %user, root = %root, "Deposit applied"),
                        Err(e) => tracing::error!(error = %e, "Failed to apply deposit"),
                    }
                }
                IngestorEvent::NewBlock { number, .. } => {
                    tracing::info!(block = number, root = %engine_ingestor.state_root(), "L1 block synced");
                }
            }
        }
    });

    let provider_url_clone = provider_url.clone();
    let bridge_clone = bridge_contract.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            let ingestor = Ingestor::new(provider_url_clone.clone(), bridge_clone.clone(), tx_clone.clone());
            if let Err(e) = ingestor.run().await {
                tracing::error!(error = %e, "Ingestor disconnected — reconnecting in 5s");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    });

    let app_state = AppState::new(Arc::clone(&engine));

    let grpc_addr = config.grpc_port.socket_addr();
    let grpc_service = AuraL2Server::new(AuraL2Service::new(app_state.clone()));
    let grpc_server = TonicServer::builder()
        .add_service(grpc_service)
        .serve(grpc_addr);

    let rest_addr = config.rest_port.socket_addr();
    let rest_router = rest::router(app_state);
    let rest_listener = tokio::net::TcpListener::bind(rest_addr).await?;

    tracing::info!(%grpc_addr, "gRPC listening");
    tracing::info!(%rest_addr, "REST listening");

    tokio::try_join!(
        async { grpc_server.await.map_err(eyre::Report::from) },
        async {
            axum::serve(rest_listener, rest_router)
                .await
                .map_err(eyre::Report::from)
        },
    )?;

    Ok(())
}
