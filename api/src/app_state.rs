use std::sync::Arc;
use state::{RocksDbBackend, StateEngine};

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<StateEngine<RocksDbBackend>>,
}

impl AppState {
    pub fn new(engine: Arc<StateEngine<RocksDbBackend>>) -> Self {
        Self { engine }
    }

    /// Sync secondary RocksDB instance with primary before reading state.
    /// Logs and ignores catch-up errors so the API stays up with stale data
    /// rather than crashing when the primary is temporarily unavailable.
    pub fn catch_up(&self) {
        if let Err(e) = self.engine.catch_up_with_primary() {
            tracing::warn!(error = %e, "catch_up_with_primary failed — serving stale state");
        }
    }
}
