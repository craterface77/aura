use crate::account::{AccountData, AccountNonce, MerkleIndex, MerkleProof, StateRoot};
use crate::error::{StateError, StateResult};
use crate::merkle::SparseMerkleTree;
use crate::store::{RocksDbBackend, StateBackend};
use alloy::primitives::{Address, U256};
use std::sync::{Arc, RwLock};

pub struct StateEngine<S> {
    backend: Arc<S>,
    /// In-memory SMT. Write-locked only during `apply_deposit`.
    tree: Arc<RwLock<SparseMerkleTree>>,
}

impl<S> Clone for StateEngine<S> {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            tree: Arc::clone(&self.tree),
        }
    }
}

impl<S: StateBackend> StateEngine<S> {
    /// Creates a new `StateEngine` and rebuilds the in-memory SMT from all
    /// accounts stored in `backend`. This O(n) scan ensures the tree root is
    /// consistent with persisted data after a restart.
    pub fn new(backend: S) -> StateResult<Self> {
        let mut tree = SparseMerkleTree::new();

        // Reconstruct the SMT from all accounts in persistent storage.
        let accounts = backend.iter_accounts()?;
        let account_count = accounts.len();
        for (address, data) in accounts {
            let leaf = data.leaf_hash(&address);
            tree.update(data.merkle_index, leaf);
        }

        let root = tree.root();
        tracing::info!(
            accounts = account_count,
            root = %root,
            "StateEngine initialized - SMT rebuilt from storage"
        );

        Ok(Self {
            backend: Arc::new(backend),
            tree: Arc::new(RwLock::new(tree)),
        })
    }

    /// Applies a deposit: adds `amount` to `address` balance, persists to
    /// RocksDB, updates the SMT leaf, and returns the new `StateRoot`.
    ///
    /// Called from the Tokio event-processor task (Rustcamp 3.11).
    /// `apply_deposit` is synchronous - RocksDB writes are fast enough to call
    /// directly from async context via `tokio::spawn`. If this ever becomes a
    /// bottleneck, migrate to `tokio::task::spawn_blocking`.
    pub fn apply_deposit(&self, address: Address, amount: U256) -> StateResult<StateRoot> {
        let mut account = match self.backend.get_account(&address)? {
            Some(existing) => {
                tracing::debug!(?address, balance = ?existing.balance, "Existing account found");
                existing
            }
            None => {
                // First deposit for this address — allocate a new SMT leaf.
                let index = self.backend.increment_next_index()?;
                tracing::info!(?address, index, "New account registered");
                AccountData::zero(MerkleIndex(index))
            }
        };

        account.balance = account
            .balance
            .checked_add(amount)
            .ok_or_else(|| StateError::MerkleProofInvalid)?; // overflow guard
        account.nonce = AccountNonce(account.nonce.0 + 1);

        self.backend.put_account(&address, &account)?;

        let leaf = account.leaf_hash(&address);
        let new_root = self
            .tree
            .write()
            .map_err(|_| StateError::LockPoisoned)?
            .update(account.merkle_index, leaf);

        tracing::info!(
            ?address,
            balance = ?account.balance,
            root = %new_root,
            "Deposit applied — state root updated"
        );

        Ok(new_root)
    }

    /// Returns the current account state. If the address has never deposited,
    /// returns a zeroed `AccountData` at index 0 (not yet persisted).
    pub fn get_account(&self, address: &Address) -> StateResult<AccountData> {
        match self.backend.get_account(address)? {
            Some(data) => Ok(data),
            None => Ok(AccountData::zero(MerkleIndex(0))),
        }
    }

    /// Returns the current Merkle root without acquiring a write lock.
    pub fn state_root(&self) -> StateRoot {
        self.tree.read().map(|t| t.root()).unwrap_or_default()
    }

    /// Generates a Merkle inclusion proof for an address.
    /// Returns an error if the address has never deposited (no leaf exists).
    pub fn get_proof(&self, address: &Address) -> StateResult<MerkleProof> {
        let account = self
            .backend
            .get_account(address)?
            .ok_or(StateError::AccountNotFound(*address))?;

        let tree = self.tree.read().map_err(|_| StateError::LockPoisoned)?;
        let siblings = tree.get_proof(account.merkle_index);
        let leaf_value = account.leaf_hash(address);
        let root = tree.root();

        Ok(MerkleProof {
            leaf_index: account.merkle_index,
            leaf_value,
            siblings,
            root,
        })
    }
}

impl StateEngine<RocksDbBackend> {
    /// Pull latest writes from the primary RocksDB instance and rebuild the SMT.
    /// Call this before reads in the API to get fresh state.
    pub fn catch_up_with_primary(&self) -> StateResult<()> {
        self.backend.try_catch_up_with_primary()?;

        // Rebuild the in-memory SMT from the refreshed backend.
        let accounts = self.backend.iter_accounts()?;
        let mut tree = SparseMerkleTree::new();
        for (address, data) in accounts {
            let leaf = data.leaf_hash(&address);
            tree.update(data.merkle_index, leaf);
        }

        *self.tree.write().map_err(|_| StateError::LockPoisoned)? = tree;
        Ok(())
    }
}

#[cfg(test)]
mod thread_safety {
    use super::*;
    use crate::store::RocksDbBackend;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn state_engine_is_send_sync() {
        _assert_send_sync::<StateEngine<RocksDbBackend>>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merkle::SparseMerkleTree;
    use crate::store::RocksDbBackend;
    use tempfile::TempDir;

    fn make_engine() -> (StateEngine<RocksDbBackend>, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let backend = RocksDbBackend::open(dir.path().to_str().unwrap()).unwrap();
        let engine = StateEngine::new(backend).unwrap();
        (engine, dir)
    }

    #[test]
    fn deposit_updates_state_root() {
        let (engine, _dir) = make_engine();
        let addr = Address::from([1u8; 20]);
        let root_before = engine.state_root();

        engine.apply_deposit(addr, U256::from(1_000u64)).unwrap();

        assert_ne!(
            engine.state_root(),
            root_before,
            "Root must change after deposit"
        );
    }

    #[test]
    fn two_deposits_accumulate_balance() {
        let (engine, _dir) = make_engine();
        let addr = Address::from([2u8; 20]);

        engine.apply_deposit(addr, U256::from(500u64)).unwrap();
        engine.apply_deposit(addr, U256::from(300u64)).unwrap();

        let account = engine.get_account(&addr).unwrap();
        assert_eq!(account.balance, U256::from(800u64));
    }

    #[test]
    fn proof_verifies_against_current_root() {
        let (engine, _dir) = make_engine();
        let addr = Address::from([3u8; 20]);

        engine.apply_deposit(addr, U256::from(1_000u64)).unwrap();
        let proof = engine.get_proof(&addr).unwrap();

        assert!(
            SparseMerkleTree::verify_proof(
                &proof.root,
                proof.leaf_index,
                &proof.leaf_value,
                &proof.siblings
            ),
            "Proof must verify against current root"
        );
    }

    #[test]
    fn multiple_addresses_independent_state() {
        let (engine, _dir) = make_engine();
        let addr_a = Address::from([10u8; 20]);
        let addr_b = Address::from([20u8; 20]);

        engine.apply_deposit(addr_a, U256::from(1000u64)).unwrap();
        engine.apply_deposit(addr_b, U256::from(2000u64)).unwrap();

        assert_eq!(
            engine.get_account(&addr_a).unwrap().balance,
            U256::from(1000u64)
        );
        assert_eq!(
            engine.get_account(&addr_b).unwrap().balance,
            U256::from(2000u64)
        );
    }
}
