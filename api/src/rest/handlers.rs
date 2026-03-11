use std::sync::Arc;

use alloy::primitives::{Address, U256};
use axum::{
    extract::{Path, State},
    Json,
};
use executor::{commit_transfer, TransferRequest};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::error::ApiError;

// ── POST /tx ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SubmitTxBody {
    pub from: String,
    pub to: String,
    pub value: String, // decimal wei string
}

#[derive(Debug, Serialize)]
pub struct SubmitTxResponse {
    pub gas_used: u64,
    pub new_sender_balance: String,
    pub new_state_root: String,
}

pub async fn post_transaction(
    State(app): State<AppState>,
    Json(body): Json<SubmitTxBody>,
) -> Result<Json<SubmitTxResponse>, ApiError> {
    let from: Address = body
        .from
        .parse()
        .map_err(|_| ApiError::InvalidAddress(body.from.clone()))?;
    let to: Address = body
        .to
        .parse()
        .map_err(|_| ApiError::InvalidAddress(body.to.clone()))?;
    let value: U256 = body
        .value
        .parse()
        .map_err(|_| ApiError::InvalidValue(body.value.clone()))?;

    let engine = Arc::clone(&app.engine);
    let result = tokio::task::spawn_blocking(move || {
        commit_transfer(engine, TransferRequest { from, to, value })
    })
    .await
    .map_err(|e| ApiError::InvalidValue(format!("join error: {e}")))?
    .map_err(ApiError::Executor)?;

    Ok(Json(SubmitTxResponse {
        gas_used: result.gas_used,
        new_sender_balance: result.new_sender_balance.to_string(),
        new_state_root: format!("0x{}", hex::encode(result.new_state_root.0)),
    }))
}

// ── GET /account/:address ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AccountResponse {
    pub address: String,
    pub balance: String,
    pub nonce: u64,
}

pub async fn get_account(
    State(app): State<AppState>,
    Path(address_str): Path<String>,
) -> Result<Json<AccountResponse>, ApiError> {
    let address: Address = address_str
        .parse()
        .map_err(|_| ApiError::InvalidAddress(address_str.clone()))?;

    let account = app.engine.get_account(&address).map_err(ApiError::State)?;

    Ok(Json(AccountResponse {
        address: format!("{address:#x}"),
        balance: account.balance.to_string(),
        nonce: account.nonce.0,
    }))
}

// ── GET /account/:address/proof ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AccountProofResponse {
    pub address: String,
    pub balance: String,
    pub leaf_index: u64,
    pub leaf_value: String,        // 0x-prefixed hex, 32 bytes
    pub siblings: Vec<String>,     // 32 entries, 0x-prefixed hex
    pub state_root: String,        // 0x-prefixed hex
}

pub async fn get_account_proof(
    State(app): State<AppState>,
    Path(address_str): Path<String>,
) -> Result<Json<AccountProofResponse>, ApiError> {
    let address: Address = address_str
        .parse()
        .map_err(|_| ApiError::InvalidAddress(address_str.clone()))?;

    let account = app.engine.get_account(&address).map_err(ApiError::State)?;
    let proof = app.engine.get_proof(&address).map_err(ApiError::State)?;

    Ok(Json(AccountProofResponse {
        address: format!("{address:#x}"),
        balance: account.balance.to_string(),
        leaf_index: proof.leaf_index.0,
        leaf_value: format!("0x{}", hex::encode(proof.leaf_value)),
        siblings: proof.siblings.iter().map(|s| format!("0x{}", hex::encode(s))).collect(),
        state_root: format!("0x{}", hex::encode(proof.root.0)),
    }))
}

// ── GET /state/root ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct StateRootResponse {
    pub state_root: String,
}

pub async fn get_state_root(State(app): State<AppState>) -> Json<StateRootResponse> {
    let root = app.engine.state_root();
    Json(StateRootResponse {
        state_root: format!("0x{}", hex::encode(root.0)),
    })
}
