use alloy::primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateRoot(pub B256);

impl Default for StateRoot {
    fn default() -> Self {
        Self(B256::ZERO)
    }
}

impl std::fmt::Display for StateRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccountNonce(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleIndex(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountData {
    pub balance: U256,
    pub nonce: AccountNonce,
    pub merkle_index: MerkleIndex,
}

impl AccountData {
    pub fn new(balance: U256, nonce: AccountNonce, merkle_index: MerkleIndex) -> Self {
        Self {
            balance,
            nonce,
            merkle_index,
        }
    }

    /// Creates a zeroed account at the given tree position (first deposit).
    pub fn zero(merkle_index: MerkleIndex) -> Self {
        Self {
            balance: U256::ZERO,
            nonce: AccountNonce(0),
            merkle_index,
        }
    }

    /// Computes the leaf hash for this account as: keccak256(address || balance_be).
    /// This is the value stored at `merkle_index` in the Sparse Merkle Tree.
    pub fn leaf_hash(&self, address: &Address) -> [u8; 32] {
        use sha3::{Digest, Keccak256};
        let mut inner = Keccak256::new();
        inner.update(address.as_slice());
        inner.update(self.balance.to_be_bytes::<32>());
        let inner_hash: [u8; 32] = inner.finalize().into();
        let mut outer = Keccak256::new();
        outer.update(inner_hash);
        outer.finalize().into()
    }
}

/// A Merkle inclusion proof for a single account leaf.
/// Can be used to verify account balance without trusting the full state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// The leaf position in the tree.
    pub leaf_index: MerkleIndex,
    /// keccak256(address || balance_be) — the leaf value.
    pub leaf_value: [u8; 32],
    /// Sibling hashes from leaf (level 0) up to the root (level 31).
    pub siblings: Vec<[u8; 32]>,
    /// The state root at the time this proof was generated.
    pub root: StateRoot,
}
