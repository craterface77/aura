use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::eth::Filter;
use alloy::sol;
use futures_util::StreamExt;
use std::str::FromStr;
use tokio::sync::mpsc;
use tracing_subscriber;
use dotenvy::dotenv;
use std::env;
use tracing_subscriber::{fmt, EnvFilter};

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
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let provider_url = env::var("PROVIDER_URL")
        .expect("PROVIDER_URL must be set");

    let bridge_contract = env::var("BRIDGE_CONTRACT")
        .expect("BRIDGE_CONTRACT must be set");

    let (tx, mut rx) = mpsc::channel::<IngestorEvent>(100);

    tokio::spawn(async move {
        tracing::info!("Event Processor started");
        while let Some(event) = rx.recv().await {
            match event {
                IngestorEvent::L1Deposit { user, amount } => {
                    tracing::warn!(
                        "PROCESSOR: Processing deposit for {} - {} wei",
                        user,
                        amount
                    );
                    // TODO: CALL STATE ENGINE (Merkle Tree)
                }
                IngestorEvent::NewBlock { number, .. } => {
                    tracing::info!("PROCESSOR: Synced with block {}", number);
                }
            }
        }
    });

    loop {
        tracing::info!("Starting Ingestor instance...");

        let ingestor = Ingestor::new(provider_url.clone(), bridge_contract.clone(), tx.clone());

        if let Err(e) = ingestor.run().await {
            tracing::error!("Ingestor error: {}. Reconnecting in 5 seconds...", e);
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}
