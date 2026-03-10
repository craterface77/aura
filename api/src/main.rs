mod app_state;
mod config;
mod error;
mod grpc;
mod rest;

use std::sync::Arc;

use ::state::{RocksDbBackend, StateEngine};
use tonic::transport::Server as TonicServer;
use tracing_subscriber::{fmt, EnvFilter};

use app_state::AppState;
use config::Config;
use grpc::{proto::aura_l2_server::AuraL2Server, AuraL2Service};

#[tokio::main]
async fn main() -> eyre::Result<()> {
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

    // Open RocksDB in secondary mode — ingestor holds the primary lock.
    // Secondary path is a scratch dir for WAL catch-up metadata.
    let secondary_path = format!("{}-api-secondary", config.state_db_path);
    let backend = RocksDbBackend::open_secondary(&config.state_db_path, &secondary_path)
        .expect("Failed to open RocksDB as secondary");
    let engine = Arc::new(StateEngine::new(backend).expect("Failed to init StateEngine"));
    tracing::info!(root = %engine.state_root(), "StateEngine ready (secondary)");

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
