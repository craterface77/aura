use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::eth::Filter;
use alloy::sol;
use dotenvy::dotenv;
use futures_util::StreamExt;
use state::{RocksDbBackend, StateEngine};
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing_subscriber::{EnvFilter, fmt};

sol! {
    #[sol(abi)]
    interface L1Bridge {
        event Deposit(address indexed user, uint256 amount, uint256 indexed depositId);
    }
}

pub struct Ingestor {
    provider_url: String,
    bridge_contract: String,
    event_sender: mpsc::Sender<IngestorEvent>,
}

#[derive(Debug)]
pub enum IngestorEvent {
    L1Deposit {
        user: Address,
        amount: alloy::primitives::U256,
    },
    NewBlock {
        number: u64,
        hash: alloy::primitives::B256,
    },
}

impl Ingestor {
    pub fn new(url: String, bridge_address: String, sender: mpsc::Sender<IngestorEvent>) -> Self {
        Self {
            provider_url: url,
            bridge_contract: bridge_address,
            event_sender: sender,
        }
    }

    pub async fn run(self) -> eyre::Result<()> {
        let ws = WsConnect::new(&self.provider_url);
        let provider_connection = ProviderBuilder::new().connect_ws(ws).await?;
        let provider = std::sync::Arc::new(provider_connection);

        tracing::info!("Ingestor connected to L1 RPC");

        let logs_handle = tokio::spawn(Self::listen_logs(
            provider.clone(),
            self.bridge_contract.clone(),
            self.event_sender.clone(),
        ));

        let blocks_handle = tokio::spawn(Self::listen_blocks(
            provider.clone(),
            self.event_sender.clone(),
        ));

        tokio::select! {
            res = logs_handle => res?,
            res = blocks_handle => res?,
        }
    }

    async fn listen_logs(
        provider: std::sync::Arc<impl Provider>,
        bridge_contract: String,
        tx: mpsc::Sender<IngestorEvent>,
    ) -> eyre::Result<()> {
        let bridge_address = Address::from_str(bridge_contract.as_str())?;

        let filter = Filter::new()
            .address(bridge_address)
            .event("Deposit(address,uint256,uint256)");

        let sub = provider.subscribe_logs(&filter).await?;
        let mut stream = sub.into_stream();

        while let Some(log) = stream.next().await {
            if let Ok(decoded) = log.log_decode::<L1Bridge::Deposit>() {
                let deposit = decoded.data();
                tracing::info!(user = ?deposit.user, amount = ?deposit.amount, "New L1 Deposit detected");
                tx.send(IngestorEvent::L1Deposit {
                    user: deposit.user,
                    amount: deposit.amount,
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn listen_blocks(
        provider: std::sync::Arc<impl Provider>,
        tx: mpsc::Sender<IngestorEvent>,
    ) -> eyre::Result<()> {
        let sub = provider.subscribe_blocks().await?;
        let mut stream = sub.into_stream();

        while let Some(header) = stream.next().await {
            let number = header.number;
            let hash = header.hash;

            tracing::debug!(number, ?hash, "New L1 Block");
            tx.send(IngestorEvent::NewBlock { number, hash }).await?;
        }
        Ok(())
    }
}

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

    // Open RocksDB and initialize StateEngine.
    // StateEngine::new() rebuilds the Sparse Merkle Tree from all persisted accounts,
    // so state survives process restarts without replaying L1 history.
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
