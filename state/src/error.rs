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

    #[error("Insufficient balance: have {have}, need {need}")]
    InsufficientBalance {
        have: alloy::primitives::U256,
        need: alloy::primitives::U256,
    },

    #[error("Nonce mismatch: expected {expected}, got {got}")]
    NonceMismatch { expected: u64, got: u64 },

    #[error("Lock poisoned")]
    LockPoisoned,
}

pub type StateResult<T> = Result<T, StateError>;
