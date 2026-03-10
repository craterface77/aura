use crate::account::AccountData;
use crate::error::StateResult;
use alloy::primitives::Address;
use rocksdb::{DB, IteratorMode, Options};

mod private {
    pub trait Sealed {}
}

pub struct RocksDbBackend {
    db: DB,
}

impl private::Sealed for RocksDbBackend {}

impl RocksDbBackend {
    pub fn open(path: &str) -> StateResult<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path)?;
        tracing::info!(path, "RocksDB opened");
        Ok(Self { db })
    }

    /// Open in secondary (read-only) mode.
    ///
    /// `primary_path` must point to the same directory the primary (ingestor) uses.
    /// `secondary_path` is a scratch directory for WAL catch-up — can be a temp dir.
    /// Call `try_catch_up_with_primary` on the returned backend to refresh state.
    pub fn open_secondary(primary_path: &str, secondary_path: &str) -> StateResult<Self> {
        let opts = Options::default();
        let db = DB::open_as_secondary(&opts, primary_path, secondary_path)?;
        tracing::info!(primary_path, secondary_path, "RocksDB opened as secondary");
        Ok(Self { db })
    }

    /// Pull latest writes from the primary instance.
    /// Call this before every read in the API to get fresh state.
    pub fn try_catch_up_with_primary(&self) -> StateResult<()> {
        self.db.try_catch_up_with_primary()?;
        Ok(())
    }

    fn address_key(address: &Address) -> [u8; 20] {
        address.0.0
    }
}

pub trait StateBackend: private::Sealed + Send + Sync {
    fn get_account(&self, address: &Address) -> StateResult<Option<AccountData>>;
    fn put_account(&self, address: &Address, data: &AccountData) -> StateResult<()>;
    fn get_next_index(&self) -> StateResult<u64>;
    fn increment_next_index(&self) -> StateResult<u64>;
    fn iter_accounts(&self) -> StateResult<Vec<(Address, AccountData)>>;
}

const INDEX_COUNTER_KEY: &[u8] = b"__next_leaf_index__";

impl StateBackend for RocksDbBackend {
    fn get_account(&self, address: &Address) -> StateResult<Option<AccountData>> {
        let key = Self::address_key(address);
        match self.db.get(key)? {
            Some(bytes) => {
                let data: AccountData = bincode::deserialize(&bytes)?;
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    fn put_account(&self, address: &Address, data: &AccountData) -> StateResult<()> {
        let key = Self::address_key(address);
        let bytes = bincode::serialize(data)?;
        self.db.put(key, bytes)?;
        Ok(())
    }

    fn get_next_index(&self) -> StateResult<u64> {
        match self.db.get(INDEX_COUNTER_KEY)? {
            Some(bytes) => {
                let val: u64 = bincode::deserialize(&bytes)?;
                Ok(val)
            }
            None => Ok(0),
        }
    }

    fn increment_next_index(&self) -> StateResult<u64> {
        let current = self.get_next_index()?;
        let next = current + 1;
        let bytes = bincode::serialize(&next)?;
        self.db.put(INDEX_COUNTER_KEY, bytes)?;
        Ok(current)
    }

    fn iter_accounts(&self) -> StateResult<Vec<(Address, AccountData)>> {
        let mut result = Vec::new();
        let iter = self.db.iterator(IteratorMode::Start);
        for item in iter {
            let (key_bytes, val_bytes) = item?;
            // Skip the index counter key (not a 20-byte address)
            if key_bytes.as_ref() == INDEX_COUNTER_KEY {
                continue;
            }
            if key_bytes.len() != 20 {
                continue;
            }
            let mut addr_bytes = [0u8; 20];
            addr_bytes.copy_from_slice(&key_bytes);
            let address = Address::from(addr_bytes);
            let data: AccountData = bincode::deserialize(&val_bytes)?;
            result.push((address, data));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{AccountNonce, MerkleIndex};
    use alloy::primitives::U256;
    use tempfile::TempDir;

    fn make_backend() -> (RocksDbBackend, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let backend = RocksDbBackend::open(dir.path().to_str().unwrap()).expect("open RocksDB");
        (backend, dir)
    }

    #[test]
    fn put_and_get_account_round_trip() {
        let (backend, _dir) = make_backend();
        let address = Address::from([1u8; 20]);
        let data = AccountData::new(U256::from(1_000_000u64), AccountNonce(5), MerkleIndex(0));

        backend.put_account(&address, &data).unwrap();
        let retrieved = backend.get_account(&address).unwrap();

        assert_eq!(retrieved, Some(data));
    }

    #[test]
    fn get_missing_account_returns_none() {
        let (backend, _dir) = make_backend();
        let address = Address::from([99u8; 20]);
        assert_eq!(backend.get_account(&address).unwrap(), None);
    }

    #[test]
    fn index_counter_starts_at_zero() {
        let (backend, _dir) = make_backend();
        assert_eq!(backend.get_next_index().unwrap(), 0);
    }

    #[test]
    fn increment_index_returns_previous_value() {
        let (backend, _dir) = make_backend();
        assert_eq!(backend.increment_next_index().unwrap(), 0); // was 0, now 1
        assert_eq!(backend.increment_next_index().unwrap(), 1); // was 1, now 2
        assert_eq!(backend.get_next_index().unwrap(), 2);
    }

    #[test]
    fn iter_accounts_returns_all_stored() {
        let (backend, _dir) = make_backend();

        let addr1 = Address::from([1u8; 20]);
        let addr2 = Address::from([2u8; 20]);
        let data1 = AccountData::zero(MerkleIndex(0));
        let data2 = AccountData::zero(MerkleIndex(1));

        backend.put_account(&addr1, &data1).unwrap();
        backend.put_account(&addr2, &data2).unwrap();

        let all = backend.iter_accounts().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn iter_accounts_excludes_counter_key() {
        let (backend, _dir) = make_backend();

        // Trigger counter creation
        backend.increment_next_index().unwrap();

        // No accounts stored yet
        let all = backend.iter_accounts().unwrap();
        assert_eq!(all.len(), 0, "Counter key must not appear as an account");
    }

    #[test]
    fn put_account_overwrites_balance() {
        let (backend, _dir) = make_backend();
        let address = Address::from([7u8; 20]);

        let v1 = AccountData::new(U256::from(100u64), AccountNonce(0), MerkleIndex(0));
        backend.put_account(&address, &v1).unwrap();

        let v2 = AccountData::new(U256::from(999u64), AccountNonce(1), MerkleIndex(0));
        backend.put_account(&address, &v2).unwrap();

        let retrieved = backend.get_account(&address).unwrap().unwrap();
        assert_eq!(retrieved.balance, U256::from(999u64));
    }
}
