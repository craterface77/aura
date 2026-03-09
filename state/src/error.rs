use thiserror::Error;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("RocksDB error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),

    #[error("Account not found: {0:?}")]
    AccountNotFound(alloy::primitives::Address),

    #[error("Merkle proof verification failed")]
    MerkleProofInvalid,

    #[error("Lock poisoned")]
    LockPoisoned,
}

pub type StateResult<T> = Result<T, StateError>;
