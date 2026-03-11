// Design:
//   - depth = 32  →  capacity = 2^32 ≈ 4 billion accounts
//   - Leaves are stored only when populated (HashMap keyed by (level, index))
//   - Unpopulated nodes are implicitly `zero_hash[level]`
//   - zero_hash[0]  = keccak256([0u8; 64])
//   - zero_hash[i]  = keccak256(zero_hash[i-1] || zero_hash[i-1])
//   - Update is O(depth) = O(32) hash operations
//   - Proof generation is O(depth) = O(32) lookups

use crate::account::{MerkleIndex, StateRoot};
use alloy::hex;
use alloy::primitives::B256;
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

pub const TREE_DEPTH: usize = 32;

/// A depth-32 Sparse Merkle Tree using Keccak256.
///
/// Only populated nodes are stored in memory — unpopulated nodes are the
/// precomputed zero-hash for their level. This allows tracking billions of
/// potential accounts with minimal memory for sparse state.
pub struct SparseMerkleTree {
    /// All populated interior nodes, keyed by (level, index).
    /// Level 0 = leaves, level TREE_DEPTH = root.
    nodes: HashMap<(usize, u64), [u8; 32]>,
    zero_hashes: Vec<[u8; 32]>,
}

impl SparseMerkleTree {
    /// Creates a new empty tree and precomputes zero hashes.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            zero_hashes: Self::precompute_zero_hashes(),
        }
    }

    pub fn root(&self) -> StateRoot {
        let root_bytes = self
            .nodes
            .get(&(TREE_DEPTH, 0))
            .copied()
            .unwrap_or(self.zero_hashes[TREE_DEPTH]);
        StateRoot(B256::from(root_bytes))
    }

    pub fn update(&mut self, index: MerkleIndex, value: [u8; 32]) -> StateRoot {
        let mut current_index = index.0;
        let mut current_hash = value;

        self.nodes.insert((0, current_index), current_hash);
        tracing::debug!(
            level = 0,
            index = current_index,
            hash = hex::encode(current_hash),
            "SMT leaf updated"
        );

        // Walk up the tree, recomputing each parent.
        for level in 0..TREE_DEPTH {
            let sibling_index = current_index ^ 1; // flip lowest bit to get sibling
            let sibling_hash = self
                .nodes
                .get(&(level, sibling_index))
                .copied()
                .unwrap_or(self.zero_hashes[level]);

            // Determine left/right order from the bit at this level.
            let parent_hash = if current_index % 2 == 0 {
                Self::hash_pair(&current_hash, &sibling_hash)
            } else {
                Self::hash_pair(&sibling_hash, &current_hash)
            };

            current_index /= 2;
            current_hash = parent_hash;
            self.nodes.insert((level + 1, current_index), current_hash);
        }

        let root = StateRoot(B256::from(current_hash));
        tracing::debug!(root = %root, "SMT root updated");
        root
    }

    /// Returns the sibling path from the leaf up to (but not including) the root.
    /// `siblings[0]` is the leaf-level sibling, `siblings[31]` is the top-level sibling.
    pub fn get_proof(&self, index: MerkleIndex) -> Vec<[u8; 32]> {
        let mut current_index = index.0;
        let mut siblings = Vec::with_capacity(TREE_DEPTH);

        for level in 0..TREE_DEPTH {
            let sibling_index = current_index ^ 1;
            let sibling_hash = self
                .nodes
                .get(&(level, sibling_index))
                .copied()
                .unwrap_or(self.zero_hashes[level]);
            siblings.push(sibling_hash);
            current_index /= 2;
        }

        siblings
    }

    /// Verifies a Merkle proof. Returns `true` if the proof is valid for the
    /// given `root`, `index`, and `leaf_value`. Used in unit tests and future
    /// L1 withdrawal verification.
    pub fn verify_proof(
        root: &StateRoot,
        index: MerkleIndex,
        leaf_value: &[u8; 32],
        siblings: &[[u8; 32]],
    ) -> bool {
        if siblings.len() != TREE_DEPTH {
            return false;
        }

        let mut current_hash = *leaf_value;
        let mut current_index = index.0;

        for sibling in siblings {
            current_hash = if current_index % 2 == 0 {
                Self::hash_pair(&current_hash, sibling)
            } else {
                Self::hash_pair(sibling, &current_hash)
            };
            current_index /= 2;
        }

        root.0 == B256::from(current_hash)
    }

    fn keccak256(data: &[u8]) -> [u8; 32] {
        let mut hasher = Keccak256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(left);
        buf[32..].copy_from_slice(right);
        Self::keccak256(&buf)
    }

    /// Precomputes zero hashes for all levels.
    /// zero[0] = keccak256([0; 64])
    /// zero[i] = keccak256(zero[i-1] || zero[i-1])
    fn precompute_zero_hashes() -> Vec<[u8; 32]> {
        let mut hashes = Vec::with_capacity(TREE_DEPTH + 1);
        // Level 0: hash of two zero children
        hashes.push(Self::keccak256(&[0u8; 64]));
        for i in 0..TREE_DEPTH {
            let prev = hashes[i];
            hashes.push(Self::hash_pair(&prev, &prev));
        }
        hashes
    }
}

impl Default for SparseMerkleTree {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::AccountData;
    use alloy::primitives::{Address, U256};

    fn make_leaf(address_seed: u8, balance: u64) -> ([u8; 32], Address, U256) {
        let mut addr_bytes = [0u8; 20];
        addr_bytes[19] = address_seed;
        let address = Address::from(addr_bytes);
        let balance = U256::from(balance);
        let account = AccountData::zero(MerkleIndex(address_seed as u64));
        let mut data = account;
        data.balance = balance;
        let leaf = data.leaf_hash(&address);
        (leaf, address, balance)
    }

    #[test]
    fn empty_tree_has_deterministic_root() {
        let tree1 = SparseMerkleTree::new();
        let tree2 = SparseMerkleTree::new();
        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn root_changes_after_update() {
        let mut tree = SparseMerkleTree::new();
        let root_before = tree.root();

        let (leaf, _, _) = make_leaf(1, 1_000_000);
        tree.update(MerkleIndex(1), leaf);

        assert_ne!(
            tree.root(),
            root_before,
            "Root must change after leaf update"
        );
    }

    #[test]
    fn same_updates_produce_same_root() {
        let (leaf_a, _, _) = make_leaf(1, 500);
        let (leaf_b, _, _) = make_leaf(2, 300);

        let mut tree1 = SparseMerkleTree::new();
        tree1.update(MerkleIndex(1), leaf_a);
        tree1.update(MerkleIndex(2), leaf_b);

        let mut tree2 = SparseMerkleTree::new();
        tree2.update(MerkleIndex(1), leaf_a);
        tree2.update(MerkleIndex(2), leaf_b);

        assert_eq!(
            tree1.root(),
            tree2.root(),
            "Trees with same updates must have same root"
        );
    }

    #[test]
    fn merkle_proof_round_trip() {
        let mut tree = SparseMerkleTree::new();
        let (leaf, _, _) = make_leaf(42, 1_000);
        let index = MerkleIndex(42);

        let root = tree.update(index, leaf);
        let proof = tree.get_proof(index);

        assert!(
            SparseMerkleTree::verify_proof(&root, index, &leaf, &proof),
            "Proof must verify against the root that was returned by update()"
        );
    }

    #[test]
    fn proof_fails_for_wrong_leaf() {
        let mut tree = SparseMerkleTree::new();
        let (leaf, _, _) = make_leaf(10, 999);
        let (wrong_leaf, _, _) = make_leaf(10, 1); // different balance
        let index = MerkleIndex(10);

        let root = tree.update(index, leaf);
        let proof = tree.get_proof(index);

        assert!(
            !SparseMerkleTree::verify_proof(&root, index, &wrong_leaf, &proof),
            "Proof must NOT verify for a tampered leaf value"
        );
    }

    #[test]
    fn proof_fails_for_wrong_root() {
        let mut tree = SparseMerkleTree::new();
        let (leaf, _, _) = make_leaf(5, 100);
        let index = MerkleIndex(5);

        tree.update(index, leaf);
        let proof = tree.get_proof(index);

        // Update a different leaf to change the root
        let (leaf2, _, _) = make_leaf(6, 200);
        let different_root = tree.update(MerkleIndex(6), leaf2);

        // Original leaf with old proof should still verify against old root... but root changed
        // So using `different_root` should fail for the first leaf's proof
        // (the proof was generated before the second update, so it's stale)
        assert!(
            !SparseMerkleTree::verify_proof(&different_root, index, &leaf, &proof),
            "Stale proof must NOT verify against a newer root"
        );
    }

    #[test]
    fn hundred_accounts_deterministic() {
        let mut tree = SparseMerkleTree::new();

        for i in 0..100u64 {
            let (leaf, _, _) = make_leaf(i as u8 % 200, i * 1000);
            tree.update(MerkleIndex(i), leaf);
        }

        // Rebuild from scratch with same updates
        let mut tree2 = SparseMerkleTree::new();
        for i in 0..100u64 {
            let (leaf, _, _) = make_leaf(i as u8 % 200, i * 1000);
            tree2.update(MerkleIndex(i), leaf);
        }

        assert_eq!(
            tree.root(),
            tree2.root(),
            "Determinism: same updates must always produce the same root"
        );
    }
}
