use std::sync::Arc;

use alloy::primitives::{Address, U256};
use tonic::{Request, Response, Status};

use crate::app_state::AppState;
use crate::error::ApiError;
use executor::{simulate_transfer, TransferRequest};

use super::proto::aura_l2_server::AuraL2;
use super::proto::{
    AccountProofRequest, AccountProofResponse, Empty, StateRootResponse, TransactionResponse,
    TransferRequest as ProtoTransferRequest,
};

pub struct AuraL2Service {
    state: AppState,
}

impl AuraL2Service {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl AuraL2 for AuraL2Service {
    async fn submit_transaction(
        &self,
        request: Request<ProtoTransferRequest>,
    ) -> Result<Response<TransactionResponse>, Status> {
        self.state.catch_up();
        let req = request.into_inner();

        let from: Address = req
            .from
            .parse()
            .map_err(|_| Status::from(ApiError::InvalidAddress(req.from.clone())))?;
        let to: Address = req
            .to
            .parse()
            .map_err(|_| Status::from(ApiError::InvalidAddress(req.to.clone())))?;
        let value: U256 = req
            .value
            .parse()
            .map_err(|_| Status::from(ApiError::InvalidValue(req.value.clone())))?;

        // simulate_transfer is synchronous (revm); run on blocking thread pool (Rustcamp 3.11)
        let engine = Arc::clone(&self.state.engine);
        let result = tokio::task::spawn_blocking(move || {
            simulate_transfer(engine, TransferRequest { from, to, value })
        })
        .await
        .map_err(|e| Status::internal(format!("join error: {e}")))?
        .map_err(|e| Status::from(ApiError::Executor(e)))?;

        Ok(Response::new(TransactionResponse {
            success: result.success,
            gas_used: result.gas_used,
            revert_reason: result.revert_reason.unwrap_or_default(),
            new_sender_balance: result.new_sender_balance.to_string(),
        }))
    }

    async fn get_account_proof(
        &self,
        request: Request<AccountProofRequest>,
    ) -> Result<Response<AccountProofResponse>, Status> {
        self.state.catch_up();
        let addr_str = request.into_inner().address;
        let address: Address = addr_str
            .parse()
            .map_err(|_| Status::from(ApiError::InvalidAddress(addr_str)))?;

        let account = self
            .state
            .engine
            .get_account(&address)
            .map_err(|e| Status::from(ApiError::State(e)))?;

        let proof = self
            .state
            .engine
            .get_proof(&address)
            .map_err(|e| Status::from(ApiError::State(e)))?;

        Ok(Response::new(AccountProofResponse {
            balance: account.balance.to_string(),
            nonce: account.nonce.0,
            state_root: format!("0x{}", hex::encode(proof.root.0)),
            leaf_value: proof.leaf_value.to_vec(),
            siblings: proof.siblings.iter().map(|s| s.to_vec()).collect(),
            leaf_index: proof.leaf_index.0,
        }))
    }

    async fn get_state_root(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<StateRootResponse>, Status> {
        self.state.catch_up();
        let root = self.state.engine.state_root();
        Ok(Response::new(StateRootResponse {
            state_root: format!("0x{}", hex::encode(root.0)),
        }))
    }
}
