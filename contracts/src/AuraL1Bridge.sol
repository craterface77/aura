// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract AuraL1Bridge {
    address public operator;
    uint256 public depositCount;

    /// Current L2 state root — updated by operator after each L2 batch.
    bytes32 public stateRoot;

    event Deposit(address indexed user, uint256 amount, uint256 indexed depositId);
    event Withdraw(address indexed user, uint256 amount);
    event StateRootUpdated(bytes32 indexed newRoot);

    error OnlyOperator();
    error TransferFailed();
    error InvalidProof();
    error InsufficientL2Balance();
    error ZeroStateRoot();
    error WithdrawLimitExceeded();

    /// Tracks withdrawn amounts per (stateRoot, user).
    /// Using the root as a key means the counter automatically resets to zero
    /// when a new root is posted — no expensive mapping wipe needed.
    mapping(bytes32 => mapping(address => uint256)) public withdrawnAmount;

    uint256 private constant TREE_DEPTH = 32;

    constructor() {
        operator = msg.sender;
    }

    /// Called by the operator (off-chain sequencer) to anchor the latest L2
    /// state root on L1. All subsequent withdrawal proofs are verified against
    /// this root.
    function updateStateRoot(bytes32 newRoot) external {
        if (msg.sender != operator) revert OnlyOperator();
        if (newRoot == bytes32(0)) revert ZeroStateRoot();
        stateRoot = newRoot;
        emit StateRootUpdated(newRoot);
    }

    function setOperator(address newOperator) external {
        if (msg.sender != operator) revert OnlyOperator();
        operator = newOperator;
    }

    function deposit() external payable {
        require(msg.value > 0, "Amount must be > 0");
        uint256 depositId = depositCount++;
        emit Deposit(msg.sender, msg.value, depositId);
    }

    /// Withdraw `amount` wei from L2 back to L1 by providing a Merkle inclusion
    /// proof that the caller has at least `amount` balance in the L2 state.
    ///
    /// @param amount       Amount to withdraw (must be <= L2 balance).
    /// @param l2Balance    The full L2 balance encoded in the leaf.
    /// @param leafIndex    Position of the leaf in the depth-32 SMT.
    /// @param siblings     32 sibling hashes, leaf-level first (siblings[0] is
    ///                     the sibling at depth 0, siblings[31] at depth 31).
    function withdraw(uint256 amount, uint256 l2Balance, uint256 leafIndex, bytes32[32] calldata siblings) external {
        if (stateRoot == bytes32(0)) revert ZeroStateRoot();
        if (amount > l2Balance) revert InsufficientL2Balance();

        // Double-withdrawal guard: total withdrawn against this root cannot
        // exceed the proven l2Balance.
        uint256 alreadyWithdrawn = withdrawnAmount[stateRoot][msg.sender];
        if (alreadyWithdrawn + amount > l2Balance) revert WithdrawLimitExceeded();

        // Recompute leaf: keccak256(keccak256(address || balance_be32))
        // Double hash protects against second preimage attacks (OpenZeppelin standard).
        // Must match Rust: leaf_hash(&self, address: &Address) in account.rs
        bytes32 leaf = keccak256(bytes.concat(keccak256(abi.encodePacked(msg.sender, bytes32(l2Balance)))));

        // Walk up the tree exactly as Rust verify_proof() does.
        bytes32 current = leaf;
        uint256 idx = leafIndex;

        for (uint256 i = 0; i < TREE_DEPTH; i++) {
            bytes32 sibling = siblings[i];
            if (idx % 2 == 0) {
                current = keccak256(abi.encodePacked(current, sibling));
            } else {
                current = keccak256(abi.encodePacked(sibling, current));
            }
            idx >>= 1;
        }

        if (current != stateRoot) revert InvalidProof();

        // Record withdrawal before transfer (checks-effects-interactions pattern).
        withdrawnAmount[stateRoot][msg.sender] += amount;

        (bool success,) = msg.sender.call{value: amount}("");
        if (!success) revert TransferFailed();

        emit Withdraw(msg.sender, amount);
    }

    function getBridgeBalance() external view returns (uint256) {
        return address(this).balance;
    }
}
