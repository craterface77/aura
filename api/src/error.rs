use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid value: {0}")]
    InvalidValue(String),

    #[error("State error: {0}")]
    State(#[from] state::StateError),

    #[error("Executor error: {0}")]
    Executor(#[from] executor::ExecutorError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::InvalidAddress(_) | ApiError::InvalidValue(_) => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            ApiError::State(state::StateError::AccountNotFound(_)) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

impl From<ApiError> for tonic::Status {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::InvalidAddress(_) | ApiError::InvalidValue(_) => {
                tonic::Status::invalid_argument(e.to_string())
            }
            ApiError::State(state::StateError::AccountNotFound(_)) => {
                tonic::Status::not_found(e.to_string())
            }
            _ => tonic::Status::internal(e.to_string()),
        }
    }
}
