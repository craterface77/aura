use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::eth::Filter;
use alloy::sol;
use futures_util::StreamExt;
use tokio::sync::mpsc;

sol! {
    #[sol(abi)]
    interface L1Bridge {
        event Deposit(address indexed user, uint256 amount, uint256 indexed depositId);
    }
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

pub struct Ingestor {
    provider_url: String,
    bridge_contract: String,
    event_sender: mpsc::Sender<IngestorEvent>,
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
        let bridge_address = bridge_contract.parse::<Address>()?;

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
