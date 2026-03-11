mod handlers;

use axum::{
    routing::{get, post},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::app_state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/tx", post(handlers::post_transaction))
        .route("/account/{address}", get(handlers::get_account))
        .route("/account/{address}/proof", get(handlers::get_account_proof))
        .route("/state/root", get(handlers::get_state_root))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
