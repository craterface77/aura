// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract AuraL1Bridge {
    address public operator;
    uint256 public depositCount;

    event Deposit(address indexed user, uint256 amount, uint256 indexed depositId);
    event Withdraw(address indexed user, uint256 amount);

    error OnlyOperator();
    error TransferFailed();

    constructor() {
        operator = msg.sender;
    }

    function deposit() external payable {
        require(msg.value > 0, "Amount must be > 0");

        uint256 depositId = depositCount;
        depositCount++;

        emit Deposit(msg.sender, msg.value, depositId);
    }

    function withdraw(address payable user, uint256 amount) external {
        // TODO: make Merkle Proof instead of operator logic
        if (msg.sender != operator) revert OnlyOperator();

        (bool success, ) = user.call{value: amount}("");
        if (!success) revert TransferFailed();

        emit Withdraw(user, amount);
    }

    function setOperator(address _newOperator) external {
        require(msg.sender == operator, "Not authorized");
        operator = _newOperator;
    }

    function getBridgeBalance() external view returns (uint256) {
        return address(this).balance;
    }
}
