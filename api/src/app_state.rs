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
}
