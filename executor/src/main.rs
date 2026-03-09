use alloy::primitives::{Address, U256};

struct TransferRequest {
    from: Address,
    to: Address,
    value: U256
}

struct SimulationResult {
    success: bool,
    gas_used: u64,
    revert_reason: Option<String>,
    new_sender_balance: U256
}

fn main() {
    println!("FF");
}