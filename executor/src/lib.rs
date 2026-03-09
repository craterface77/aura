use std::sync::Arc;

use alloy::primitives::{Address, U256};
use revm::{
    context::{result::ExecutionResult, TxEnv},
    database::CacheDB,
    database_interface::{DBErrorMarker, DatabaseRef},
    handler::{ExecuteEvm, MainBuilder, MainContext, MainnetContext},
    primitives::{Address as RevmAddress, TxKind, B256, KECCAK_EMPTY, U256 as RevmU256},
    state::{AccountInfo as RevmAccountInfo, Bytecode},
};
use state::{StateBackend, StateEngine};

pub struct TransferRequest {
    pub from: Address,
    pub to: Address,
    pub value: U256,
}

pub struct SimulationResult {
    pub success: bool,
    pub gas_used: u64,
    pub revert_reason: Option<String>,
    pub new_sender_balance: U256,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("State error: {0}")]
    State(#[from] state::StateError),
    #[error("EVM execution failed: {0}")]
    Evm(String),
}

impl DBErrorMarker for ExecutorError {}

pub struct StateEngineDb<S> {
    engine: Arc<StateEngine<S>>,
}

impl<S: StateBackend> StateEngineDb<S> {
    pub fn new(engine: Arc<StateEngine<S>>) -> Self {
        Self { engine }
    }
}

impl<S: StateBackend> DatabaseRef for StateEngineDb<S> {
    type Error = ExecutorError;

    fn basic_ref(
        &self,
        address: RevmAddress,
    ) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let alloy_address = Address::from(address.0 .0);

        let account = self.engine.get_account(&alloy_address)?;

        let revm_balance = RevmU256::from_be_bytes(account.balance.to_be_bytes::<32>());

        Ok(Some(RevmAccountInfo {
            balance: revm_balance,
            nonce: account.nonce.0,
            code_hash: KECCAK_EMPTY,
            account_id: None,
            code: Some(Bytecode::default()),
        }))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(Bytecode::default())
    }

    fn storage_ref(
        &self,
        _address: RevmAddress,
        _index: RevmU256,
    ) -> Result<RevmU256, Self::Error> {
        Ok(RevmU256::ZERO)
    }

    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        Ok(B256::ZERO)
    }
}

pub fn simulate_transfer<S: StateBackend>(
    engine: Arc<StateEngine<S>>,
    request: TransferRequest,
) -> Result<SimulationResult, ExecutorError> {
    let sender_nonce = engine.get_account(&request.from)?.nonce.0;

    let db = StateEngineDb::new(engine);
    let cache_db = CacheDB::new(db);

    let tx = TxEnv {
        caller: RevmAddress::from(request.from.0 .0),
        kind: TxKind::Call(RevmAddress::from(request.to.0 .0)),
        value: RevmU256::from_be_bytes(request.value.to_be_bytes::<32>()),
        gas_limit: 21_000,
        gas_price: 1u128,
        nonce: sender_nonce,
        ..Default::default()
    };

    let mut evm = MainnetContext::<revm::database_interface::EmptyDB>::mainnet()
        .with_ref_db(cache_db)
        .build_mainnet();

    let result_and_state = match evm.transact(tx) {
        Ok(r) => r,
        Err(e) => {
            return Ok(SimulationResult {
                success: false,
                gas_used: 0,
                revert_reason: Some(format!("{:?}", e)),
                new_sender_balance: U256::ZERO,
            });
        }
    };

    let (success, gas_used, revert_reason) = match result_and_state.result {
        ExecutionResult::Success { gas, .. } => (true, gas.used(), None),
        ExecutionResult::Revert { gas, output, .. } => {
            let reason = String::from_utf8_lossy(&output).to_string();
            (false, gas.used(), Some(reason))
        }
        ExecutionResult::Halt { reason, gas, .. } => {
            (false, gas.used(), Some(format!("{:?}", reason)))
        }
    };

    let sender_revm = RevmAddress::from(request.from.0 .0);
    let new_sender_balance = result_and_state
        .state
        .get(&sender_revm)
        .map(|a| {
            let bytes = a.info.balance.to_be_bytes::<32>();
            U256::from_be_bytes::<32>(bytes)
        })
        .unwrap_or(U256::ZERO);

    Ok(SimulationResult {
        success,
        gas_used,
        revert_reason,
        new_sender_balance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::{RocksDbBackend, StateEngine};
    use tempfile::TempDir;

    fn make_engine() -> (Arc<StateEngine<RocksDbBackend>>, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let backend = RocksDbBackend::open(dir.path().to_str().unwrap()).unwrap();
        let engine = Arc::new(StateEngine::new(backend).unwrap());
        (engine, dir)
    }

    fn alice() -> Address {
        Address::from([0xAAu8; 20])
    }

    fn bob() -> Address {
        Address::from([0xBBu8; 20])
    }

    // 1 ETH in wei
    fn one_eth() -> U256 {
        U256::from(1_000_000_000_000_000_000u128)
    }

    #[test]
    fn successful_transfer() {
        let (engine, _dir) = make_engine();

        engine
            .apply_deposit(alice(), one_eth() * U256::from(2))
            .unwrap();

        let result = simulate_transfer(
            Arc::clone(&engine),
            TransferRequest {
                from: alice(),
                to: bob(),
                value: one_eth(),
            },
        )
        .unwrap();

        assert!(result.success, "transfer should succeed");
        assert_eq!(result.gas_used, 21_000, "plain ETH transfer costs exactly 21000 gas");
        assert!(result.revert_reason.is_none());

        // New balance = 2 ETH - 1 ETH - gas_cost (21_000 * 1 wei)
        let gas_cost = U256::from(21_000u64);
        let expected = one_eth() * U256::from(2) - one_eth() - gas_cost;
        assert_eq!(
            result.new_sender_balance, expected,
            "Alice's balance should decrease by transfer amount + gas cost"
        );
    }

    #[test]
    fn transfer_fails_on_insufficient_balance() {
        let (engine, _dir) = make_engine();

        // Alice has no balance (no deposit) - transfer must fail
        let result = simulate_transfer(
            Arc::clone(&engine),
            TransferRequest {
                from: alice(),
                to: bob(),
                value: one_eth(),
            },
        )
        .unwrap();

        assert!(!result.success, "transfer must fail with zero balance");
        assert!(
            result.revert_reason.is_some(),
            "a rejection reason must be present"
        );
    }

    #[test]
    fn simulation_does_not_mutate_state_engine() {
        let (engine, _dir) = make_engine();

        // Give Alice 5 ETH via deposit
        engine
            .apply_deposit(alice(), one_eth() * U256::from(5))
            .unwrap();

        let root_before = engine.state_root();
        let balance_before = engine.get_account(&alice()).unwrap().balance;

        let result = simulate_transfer(
            Arc::clone(&engine),
            TransferRequest {
                from: alice(),
                to: bob(),
                value: one_eth(),
            },
        )
        .unwrap();

        assert!(result.success, "simulation should succeed");

        // StateEngine must be unchanged — simulate_transfer is read-only
        assert_eq!(
            engine.state_root(),
            root_before,
            "simulation must not change the state root"
        );
        assert_eq!(
            engine.get_account(&alice()).unwrap().balance,
            balance_before,
            "simulation must not change Alice's balance in StateEngine"
        );
    }
}
